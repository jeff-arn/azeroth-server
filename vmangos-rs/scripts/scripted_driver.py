#!/usr/bin/env python3
"""High-level navigation and combat driver for scripted wire probes.

This is Option 1 from the Tier 4 real-client follow-up:
instead of piloting the graphical client from
screenshots, probes issue high-level commands
("walk there", "approach this spawn", "cast until
dead", "loot it") over the real 1.12.1 wire
protocol. The driver emits ordinary movement
heartbeats at run speed, tracks object creates,
movement blocks, and value updates from
SMSG_UPDATE_OBJECT, and exposes the tracked
positions/health that a screenshot loop cannot see
reliably.

Movement is intentionally simple: straight-line legs
between caller-chosen waypoints over open ground. It
is a verification driver, not a pathfinding bot; z is
interpolated between leg endpoints, so routes should
use known spawn/NPC heights at their endpoints.

Run offline parser/geometry checks with:
    python3 vmangos-rs/scripts/scripted_driver.py --self-test
"""

from __future__ import annotations

import math
import pathlib
import select
import struct
import sys
import time
import zlib
from dataclasses import dataclass, field

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import two_client_smoke as smoke  # noqa: E402
from two_client_smoke import Client  # noqa: E402
from update_burst_validator import (  # noqa: E402
    FIELD_COUNT_BY_TYPE,
    MOVEFLAG_JUMPING,
    MOVEFLAG_ONTRANSPORT,
    MOVEFLAG_SPLINE_ELEVATION,
    MOVEFLAG_SPLINE_ENABLED,
    MOVEFLAG_SWIMMING,
    SPLINE_FINAL_ANGLE,
    SPLINE_FINAL_POINT,
    SPLINE_FINAL_TARGET,
    UPDATEFLAG_ALL,
    UPDATEFLAG_HAS_POSITION,
    UPDATEFLAG_HIGHGUID,
    UPDATEFLAG_LIVING,
    UPDATEFLAG_MELEE_ATTACKING,
    UPDATEFLAG_TRANSPORT,
    ParseError,
    Reader,
)

SMSG_UPDATE_OBJECT = 169
SMSG_COMPRESSED_UPDATE_OBJECT = 502
CMSG_CAST_SPELL = 302
SMSG_CAST_RESULT = 304
SMSG_SPELL_GO = 306
CMSG_LOOT = 349
CMSG_LOOT_MONEY = 350
CMSG_LOOT_RELEASE = 351
SMSG_LOOT_RESPONSE = 352
CMSG_AUTOSTORE_LOOT_ITEM = 264

SPELL_FAILED_NOT_READY = 60
SPELL_FAILED_NOT_INFRONT = 54
SPELL_FAILED_NO_POWER = 77
SPELL_FAILED_SPELL_IN_PROGRESS = 97
SMSG_MONSTER_MOVE = 221

UPDATETYPE_VALUES = 0
UPDATETYPE_MOVEMENT = 1
UPDATETYPE_CREATE_OBJECT = 2
UPDATETYPE_CREATE_OBJECT2 = 3
UPDATETYPE_OUT_OF_RANGE_OBJECTS = 4

OBJECT_FIELD_ENTRY = 3
UNIT_FIELD_HEALTH = 22
UNIT_FIELD_MAXHEALTH = 28
UNIT_FIELD_LEVEL = 34

Position = tuple[float, float, float, float]


def distance2d(a: Position, b: tuple[float, float] | Position) -> float:
    return math.hypot(b[0] - a[0], b[1] - a[1])


def facing_toward(position: Position, x: float, y: float) -> float:
    return math.atan2(y - position[1], x - position[0]) % (2 * math.pi)


def approach_point(
    position: Position, target: tuple[float, float, float], standoff: float
) -> tuple[float, float, float]:
    """Point on the current-to-target line `standoff` yards short."""
    dist = distance2d(position, target)
    if dist <= standoff or dist == 0:
        return target
    frac = (dist - standoff) / dist
    return (
        position[0] + (target[0] - position[0]) * frac,
        position[1] + (target[1] - position[1]) * frac,
        position[2] + (target[2] - position[2]) * frac,
    )


@dataclass
class TrackedObject:
    guid: int
    type_id: int | None = None
    position: Position | None = None
    spline_destination: tuple[float, float, float] | None = None
    values: dict[int, int] = field(default_factory=dict)

    @property
    def entry(self) -> int | None:
        return self.values.get(OBJECT_FIELD_ENTRY)

    @property
    def health(self) -> int | None:
        return self.values.get(UNIT_FIELD_HEALTH)

    @property
    def max_health(self) -> int | None:
        return self.values.get(UNIT_FIELD_MAXHEALTH)

    @property
    def level(self) -> int | None:
        return self.values.get(UNIT_FIELD_LEVEL)


class WorldState:
    """Best-effort object tracker fed by update packets.

    Parse failures are recorded in `errors` rather
    than raised: a driver should keep navigating if
    an unrelated create block uses a spline variant
    this lightweight parser does not model.
    """

    def __init__(self) -> None:
        self.objects: dict[int, TrackedObject] = {}
        self.errors: list[str] = []

    def get(self, guid: int) -> TrackedObject | None:
        return self.objects.get(guid)

    def position(self, guid: int) -> Position | None:
        obj = self.objects.get(guid)
        return None if obj is None else obj.position

    def by_entry(self, entry: int) -> list[TrackedObject]:
        return [obj for obj in self.objects.values() if obj.entry == entry]

    def observe(self, opcode: int, body: bytes) -> None:
        if opcode == SMSG_MONSTER_MOVE:
            try:
                self._parse_monster_move(body)
            except (ParseError, struct.error) as exc:
                self.errors.append(str(exc))
            return
        if opcode not in (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT):
            return
        try:
            self._parse_update(opcode, body)
        except (ParseError, struct.error, zlib.error) as exc:
            self.errors.append(str(exc))

    def _object(self, guid: int) -> TrackedObject:
        return self.objects.setdefault(guid, TrackedObject(guid=guid))

    def _parse_monster_move(self, body: bytes) -> None:
        reader = Reader(body)
        guid = reader.packed_guid()
        start = (reader.f32(), reader.f32(), reader.f32())
        reader.u32()  # spline id
        move_type = reader.u8()
        obj = self._object(guid)
        old_orientation = 0.0 if obj.position is None else obj.position[3]
        obj.position = (start[0], start[1], start[2], old_orientation)
        if move_type == 1:  # MonsterMoveStop
            obj.spline_destination = None
            return
        if move_type == 3:  # facing target guid
            reader.u64()
        elif move_type == 4:  # facing angle
            reader.f32()
        elif move_type == 2:  # facing spot
            for _ in range(3):
                reader.f32()
        elif move_type != 0:
            raise ParseError(f"unsupported monster-move facing type {move_type}")
        flags = reader.u32()
        reader.u32()  # duration
        if flags & 0x0000_0200:  # Catmull-Rom (FLYING mask in this fork)
            count = reader.u32()
            if count > 100_000:
                raise ParseError(f"implausible monster-move point count {count}")
            points = [
                (reader.f32(), reader.f32(), reader.f32()) for _ in range(count)
            ]
            obj.spline_destination = points[-1] if points else start
        else:
            count = reader.u32()
            if count > 100_000:
                raise ParseError(f"implausible monster-move point count {count}")
            destination = (reader.f32(), reader.f32(), reader.f32())
            if count > 1:
                reader.read((count - 1) * 4)  # packed destination-relative points
            obj.spline_destination = destination

    def _parse_update(self, opcode: int, body: bytes) -> None:
        if opcode == SMSG_COMPRESSED_UPDATE_OBJECT:
            if len(body) < 4:
                raise ParseError("compressed update body too short")
            expected = struct.unpack_from("<I", body)[0]
            body = zlib.decompress(body[4:])
            if len(body) != expected:
                raise ParseError(
                    f"decompressed {len(body)} bytes, header promised {expected}"
                )
        reader = Reader(body)
        block_count = reader.u32()
        reader.u8()  # hasTransport
        for _ in range(block_count):
            self._parse_block(reader)
        if reader.remaining:
            raise ParseError(f"{reader.remaining} trailing update bytes")

    def _parse_block(self, reader: Reader) -> None:
        update_type = reader.u8()
        if update_type == UPDATETYPE_OUT_OF_RANGE_OBJECTS:
            for _ in range(reader.u32()):
                self.objects.pop(reader.packed_guid(), None)
            return
        if update_type == UPDATETYPE_MOVEMENT:
            guid = reader.u64()
            position, destination = self._parse_movement(reader)
            obj = self._object(guid)
            if position is not None:
                obj.position = position
            obj.spline_destination = destination
            return

        guid = reader.packed_guid()
        obj = self._object(guid)
        if update_type == UPDATETYPE_VALUES:
            obj.values.update(self._parse_values(reader, obj.type_id))
            return
        if update_type not in (UPDATETYPE_CREATE_OBJECT, UPDATETYPE_CREATE_OBJECT2):
            raise ParseError(f"unsupported update type {update_type}")
        obj.type_id = reader.u8()
        position, destination = self._parse_movement(reader)
        if position is not None:
            obj.position = position
        obj.spline_destination = destination
        obj.values.update(self._parse_values(reader, obj.type_id))

    def _parse_values(
        self, reader: Reader, type_id: int | None
    ) -> dict[int, int]:
        block_count = reader.u8()
        mask = reader.read(block_count * 4)
        field_count = FIELD_COUNT_BY_TYPE.get(type_id) if type_id is not None else None
        if field_count is not None:
            expected = (field_count + 31) // 32
            if block_count != expected:
                raise ParseError(
                    f"values mask has {block_count} blocks, expected {expected}"
                )
        values: dict[int, int] = {}
        for index in range(block_count * 32):
            if mask[index // 8] & (1 << (index % 8)):
                values[index] = reader.u32()
        return values

    def _parse_movement(
        self, reader: Reader
    ) -> tuple[Position | None, tuple[float, float, float] | None]:
        update_flags = reader.u8()
        position: Position | None = None
        destination: tuple[float, float, float] | None = None
        if update_flags & UPDATEFLAG_LIVING:
            move_flags = reader.u32()
            reader.u32()  # server time
            position = (reader.f32(), reader.f32(), reader.f32(), reader.f32())
            if move_flags & MOVEFLAG_ONTRANSPORT:
                reader.u64()
                for _ in range(4):
                    reader.f32()
            if move_flags & MOVEFLAG_SWIMMING:
                reader.f32()
            reader.u32()  # fall time
            if move_flags & MOVEFLAG_JUMPING:
                for _ in range(4):
                    reader.f32()
            if move_flags & MOVEFLAG_SPLINE_ELEVATION:
                reader.f32()
            for _ in range(6):
                reader.f32()  # speeds
            if move_flags & MOVEFLAG_SPLINE_ENABLED:
                destination = self._parse_spline(reader)
        elif update_flags & UPDATEFLAG_HAS_POSITION:
            position = (reader.f32(), reader.f32(), reader.f32(), reader.f32())

        if update_flags & UPDATEFLAG_HIGHGUID:
            reader.u32()
        if update_flags & UPDATEFLAG_ALL:
            reader.u32()
        if update_flags & UPDATEFLAG_MELEE_ATTACKING:
            reader.packed_guid()
        if update_flags & UPDATEFLAG_TRANSPORT:
            reader.u32()
        return position, destination

    def _parse_spline(
        self, reader: Reader
    ) -> tuple[float, float, float] | None:
        spline_flags = reader.u32()
        if spline_flags & SPLINE_FINAL_ANGLE:
            reader.f32()
        elif spline_flags & SPLINE_FINAL_TARGET:
            reader.u64()
        elif spline_flags & SPLINE_FINAL_POINT:
            for _ in range(3):
                reader.f32()
        reader.i32()  # time passed
        reader.i32()  # duration
        reader.u32()  # spline id
        nodes = reader.u32()
        if nodes > 100_000:
            raise ParseError(f"implausible spline node count {nodes}")
        points = [reader.f32() for _ in range(nodes * 3 + 3)]
        return (points[-3], points[-2], points[-1])


@dataclass
class CastOutcome:
    ok: bool
    result_reason: int | None = None
    packets: list[tuple[int, bytes]] = field(default_factory=list)


@dataclass
class LootResult:
    gold: int
    items: dict[int, int]


class ScriptedDriver:
    """High-level commands over one logged-in scripted client."""

    def __init__(
        self, client: Client, position: Position, own_guid: int | None = None
    ):
        self.client = client
        self.position: Position = position
        self.world = WorldState()
        self.events: list[tuple[int, bytes]] = []
        self.own_guid = own_guid
        self.last_cast_outcome: CastOutcome | None = None
        self._move_clock_ms = 2000

    def observe(self, opcode: int, body: bytes) -> None:
        self.events.append((opcode, body))
        self.world.observe(opcode, body)

    def poll(self, timeout: float = 0.0) -> list[tuple[int, bytes]]:
        """Read currently available packets, waiting up to timeout."""
        sock = getattr(self.client, "sock", None)
        if sock is None:
            if timeout > 0:
                time.sleep(timeout)
            return []
        seen: list[tuple[int, bytes]] = []
        deadline = time.monotonic() + max(timeout, 0.0)
        while True:
            remaining = deadline - time.monotonic()
            ready, _, _ = select.select([sock], [], [], max(remaining, 0.0))
            if not ready:
                break
            packet = self.client.read_packet()
            self.observe(*packet)
            seen.append(packet)
            if time.monotonic() >= deadline:
                break
        return seen

    def drain_idle(
        self, idle_secs: float = 0.5, max_secs: float = 3.0
    ) -> list[tuple[int, bytes]]:
        packets = smoke.drain_until_idle(
            self.client, idle_secs=idle_secs, max_secs=max_secs
        )
        for opcode, body in packets:
            self.observe(opcode, body)
        return packets

    def wait_for_any(
        self, opcodes: set[int], timeout: float, label: str
    ) -> tuple[int, bytes, list[tuple[int, bytes]]]:
        skipped: list[tuple[int, bytes]] = []
        deadline = time.monotonic() + timeout
        sock = getattr(self.client, "sock", None)
        while time.monotonic() < deadline:
            if sock is not None:
                ready, _, _ = select.select(
                    [sock], [], [], min(0.25, max(deadline - time.monotonic(), 0.0))
                )
                if not ready:
                    continue
            opcode, body = self.client.read_packet()
            self.observe(opcode, body)
            if opcode in opcodes:
                return opcode, body, skipped
            skipped.append((opcode, body))
        raise AssertionError(
            f"timeout waiting for {label}; skipped opcodes "
            f"{[opcode for opcode, _ in skipped]}"
        )

    def wait_for(
        self, opcode: int, timeout: float, label: str
    ) -> tuple[bytes, list[tuple[int, bytes]]]:
        _opcode, body, skipped = self.wait_for_any({opcode}, timeout, label)
        return body, skipped

    def _heartbeat(self, position: Position) -> None:
        self._move_clock_ms = (self._move_clock_ms + 100) & 0xFFFF_FFFF
        self.client.send_packet(
            smoke.MSG_MOVE_HEARTBEAT,
            smoke.heartbeat_body(self._move_clock_ms, *position),
        )
        self.position = position

    def face(self, x: float, y: float) -> Position:
        facing = facing_toward(self.position, x, y)
        self._heartbeat((self.position[0], self.position[1], self.position[2], facing))
        return self.position

    def walk_to(
        self,
        x: float,
        y: float,
        z: float | None = None,
        *,
        speed: float = 7.0,
        tick: float = 0.2,
    ) -> Position:
        if speed <= 0:
            raise ValueError("speed must be positive")
        if tick <= 0:
            raise ValueError("tick must be positive")
        start = self.position
        tz = start[2] if z is None else z
        dist = distance2d(start, (x, y))
        facing = facing_toward(start, x, y) if dist > 0 else start[3]
        steps = max(1, math.ceil(dist / (speed * tick)))
        for index in range(1, steps + 1):
            frac = index / steps
            self._heartbeat(
                (
                    start[0] + (x - start[0]) * frac,
                    start[1] + (y - start[1]) * frac,
                    start[2] + (tz - start[2]) * frac,
                    facing,
                )
            )
            self.poll(tick)
        return self.position

    def walk_route(
        self, waypoints: list[tuple[float, float] | tuple[float, float, float]]
    ) -> Position:
        for waypoint in waypoints:
            self.walk_to(*waypoint)
        return self.position

    def approach(
        self,
        target: tuple[float, float, float],
        standoff: float,
        *,
        speed: float = 7.0,
        tick: float = 0.2,
    ) -> Position:
        point = approach_point(self.position, target, standoff)
        self.walk_to(point[0], point[1], point[2], speed=speed, tick=tick)
        self.face(target[0], target[1])
        return self.position

    def approach_guid(
        self,
        guid: int,
        standoff: float,
        *,
        fallback: tuple[float, float, float] | None = None,
        attempts: int = 3,
    ) -> Position:
        for _ in range(attempts):
            obj = self.world.get(guid)
            target = None if obj is None else obj.position
            if target is None and fallback is not None:
                target = (fallback[0], fallback[1], fallback[2], 0.0)
            if target is None:
                self.poll(0.25)
                continue
            self.approach((target[0], target[1], target[2]), standoff)
            self.poll(0.1)
            obj = self.world.get(guid)
            if obj is None or obj.position is None:
                return self.position
            if distance2d(self.position, obj.position) <= standoff + 2.5:
                self.face(obj.position[0], obj.position[1])
                return self.position
        return self.position

    def cast_spell(
        self,
        spell_id: int,
        target_raw: int | None = None,
        *,
        timeout: float = 8.0,
    ) -> CastOutcome:
        if target_raw is None:
            body = struct.pack("<IH", spell_id, 0)
        else:
            body = struct.pack("<IH", spell_id, 0x0002) + smoke.pack_guid(target_raw)
        self.client.send_packet(CMSG_CAST_SPELL, body)
        try:
            opcode, response, skipped = self.wait_for_any(
                {SMSG_SPELL_GO, SMSG_CAST_RESULT}, timeout, f"cast {spell_id}"
            )
        except AssertionError:
            self.last_cast_outcome = CastOutcome(ok=False)
            return self.last_cast_outcome
        packets = [*skipped, (opcode, response)]
        if opcode == SMSG_CAST_RESULT:
            reason = response[4] if len(response) > 4 else None
            self.last_cast_outcome = CastOutcome(
                ok=False, result_reason=reason, packets=packets
            )
            return self.last_cast_outcome
        self.last_cast_outcome = CastOutcome(ok=True, packets=packets)
        return self.last_cast_outcome

    def target_health(self, guid: int) -> int | None:
        obj = self.world.get(guid)
        return None if obj is None else obj.health

    def self_health(self) -> int | None:
        if self.own_guid is None:
            return None
        return self.target_health(self.own_guid)

    def wait_self_health_at_least(
        self, minimum: int, timeout: float = 45.0
    ) -> int | None:
        """Poll while out-of-combat regen restores health."""
        deadline = time.monotonic() + timeout
        health = self.self_health()
        while health is not None and health < minimum and time.monotonic() < deadline:
            self.poll(1.0)
            health = self.self_health()
        return health

    def cast_until_dead(
        self,
        target_raw: int,
        spell_id: int,
        *,
        max_casts: int = 10,
        settle_secs: float = 1.6,
    ) -> bool:
        successful_casts = 0
        attempts = 0
        no_power_retries = 0
        max_attempts = max_casts * 6
        while successful_casts < max_casts and attempts < max_attempts:
            attempts += 1
            if self.target_health(target_raw) == 0:
                return True
            if self.self_health() == 0:
                return False
            target_pos = self.world.position(target_raw)
            if target_pos is not None:
                self.face(target_pos[0], target_pos[1])
            outcome = self.cast_spell(spell_id, target_raw)
            if not outcome.ok and outcome.result_reason == SPELL_FAILED_NO_POWER:
                # Mana is a recoverable pacing limit,
                # especially after a buff plus several
                # nukes. Wait through the five-second
                # rule and at least one regen tick.
                no_power_retries += 1
                if no_power_retries > 3:
                    return False
                self.poll(9.0)
                continue
            if not outcome.ok and outcome.result_reason in (
                SPELL_FAILED_NOT_READY,
                SPELL_FAILED_NOT_INFRONT,
                SPELL_FAILED_SPELL_IN_PROGRESS,
            ):
                # NOT_READY/SPELL_IN_PROGRESS: the
                # server sets GCD when the previous cast
                # executes, and pushback can leave that
                # cast active after an earlier packet was
                # observed. NOT_INFRONT: a charging target
                # crossed behind us between casts. These
                # are pacing/facing results, not evidence
                # that the target is dead or invalid.
                self.poll(1.6)
                continue
            if outcome.ok:
                successful_casts += 1
            self.poll(settle_secs)
            if self.target_health(target_raw) == 0:
                return True
            if not outcome.ok:
                # Other failures usually mean the target
                # is not a usable live target; a corpse
                # commonly rejects further casts before
                # its final values update arrives.
                self.poll(0.25)
                return self.target_health(target_raw) == 0
        return self.target_health(target_raw) == 0

    @staticmethod
    def parse_loot_response(body: bytes) -> LootResult | None:
        if len(body) < 14 or body[8] == 0:
            return None
        gold = struct.unpack_from("<I", body, 9)[0]
        count = body[13]
        items: dict[int, int] = {}
        offset = 14
        for _ in range(count):
            if offset + 22 > len(body):
                raise ParseError("truncated loot item row")
            item_id, item_count = struct.unpack_from("<II", body, offset + 1)
            items[item_id] = items.get(item_id, 0) + item_count
            offset += 22
        return LootResult(gold=gold, items=items)

    def loot(self, guid: int, *, timeout: float = 6.0) -> LootResult | None:
        self.client.send_packet(CMSG_LOOT, struct.pack("<Q", guid))
        try:
            body, _ = self.wait_for(SMSG_LOOT_RESPONSE, timeout, f"loot {guid:#x}")
        except AssertionError:
            return None
        try:
            result = self.parse_loot_response(body)
        except ParseError as exc:
            self.world.errors.append(str(exc))
            return None
        if result is None:
            return None
        if len(body) >= 14:
            count = body[13]
            for index in range(count):
                slot = body[14 + index * 22]
                self.client.send_packet(
                    CMSG_AUTOSTORE_LOOT_ITEM, struct.pack("<B", slot)
                )
                self.poll(0.05)
        self.client.send_packet(CMSG_LOOT_MONEY, b"")
        self.poll(0.05)
        self.client.send_packet(CMSG_LOOT_RELEASE, struct.pack("<Q", guid))
        self.poll(0.1)
        return result

    def kill_and_loot(
        self,
        target_raw: int,
        spell_id: int,
        *,
        fallback: tuple[float, float, float] | None = None,
        cast_range: float = 20.0,
        max_casts: int = 10,
    ) -> LootResult | None:
        self.approach_guid(target_raw, cast_range, fallback=fallback)
        if not self.cast_until_dead(target_raw, spell_id, max_casts=max_casts):
            # Health is only a hint; a loot response is
            # the authoritative corpse check. Still
            # close to corpse range before asking.
            self.approach_guid(target_raw, 2.0, fallback=fallback)
            result = self.loot(target_raw)
            if result is not None:
                return result
            return None
        self.approach_guid(target_raw, 2.0, fallback=fallback)
        return self.loot(target_raw)


class _FakeClient:
    def __init__(self) -> None:
        self.packets: list[tuple[int, bytes]] = []

    def send_packet(self, opcode: int, body: bytes) -> None:
        self.packets.append((opcode, body))


def run_self_test() -> int:
    fake = _FakeClient()
    driver = ScriptedDriver(fake, (0.0, 0.0, 10.0, 0.0))  # type: ignore[arg-type]
    final = driver.walk_to(7.0, 0.0, 12.0, speed=7.0, tick=0.25)
    assert final == (7.0, 0.0, 12.0, 0.0), final
    assert len(fake.packets) == 4, fake.packets
    assert all(opcode == smoke.MSG_MOVE_HEARTBEAT for opcode, _ in fake.packets)
    print("SELFTEST walk_to heartbeat route")

    point = approach_point((0.0, 0.0, 0.0, 0.0), (10.0, 0.0, 0.0), 3.0)
    assert point == (7.0, 0.0, 0.0), point
    print("SELFTEST approach point")

    state = WorldState()
    guid = 0xF130_0000_0000_0001
    state.objects[guid] = TrackedObject(guid=guid, type_id=3)
    mask_blocks = (188 + 31) // 32
    mask = bytearray(mask_blocks * 4)
    mask[UNIT_FIELD_HEALTH // 8] |= 1 << (UNIT_FIELD_HEALTH % 8)
    values_packet = (
        struct.pack("<IB", 1, 0)
        + bytes([UPDATETYPE_VALUES])
        + smoke.pack_guid(guid)
        + bytes([mask_blocks])
        + bytes(mask)
        + struct.pack("<I", 0)
    )
    state.observe(SMSG_UPDATE_OBJECT, values_packet)
    assert not state.errors, state.errors
    assert state.get(guid) is not None and state.get(guid).health == 0  # type: ignore[union-attr]
    print("SELFTEST update-object health tracking")

    monster_move = (
        smoke.pack_guid(guid)
        + struct.pack("<fffI", 1.0, 2.0, 3.0, 7)
        + bytes([0])
        + struct.pack("<II", 0x100, 1000)
        + struct.pack("<Ifff", 2, 10.0, 2.0, 3.0)
        + b"\0\0\0\0"
    )
    state.observe(SMSG_MONSTER_MOVE, monster_move)
    tracked = state.get(guid)
    assert not state.errors, state.errors
    assert tracked is not None and tracked.position == (1.0, 2.0, 3.0, 0.0)
    assert tracked.spline_destination == (10.0, 2.0, 3.0)
    print("SELFTEST monster-move tracking")

    # GUID, loot type, gold, count, then slot + item fields.
    body = (
        struct.pack("<Q", guid)
        + bytes([1])
        + struct.pack("<I", 123)
        + bytes([1, 4])
        + struct.pack("<IIIII", 750, 2, 99, 0, 0)
        + bytes([0])
    )
    result = ScriptedDriver.parse_loot_response(body)
    assert result == LootResult(gold=123, items={750: 2}), result
    print("SELFTEST loot response parse")

    print("SCRIPTED-DRIVER-SELFTEST-PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if args == ["--self-test"]:
        return run_self_test()
    print("usage: scripted_driver.py --self-test", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
