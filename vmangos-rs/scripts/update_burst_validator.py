#!/usr/bin/env python3
"""Capture and strictly parse the initial world-login UpdateData burst.

This is the real-client-facing counterpart to the opcode-tally in
``phase4_scripted_world.py``. The parser below is transcribed from
the C++ builders in ``Object.cpp`` / ``UpdateData.cpp`` and rejects
truncated blocks, wrong value-mask widths, out-of-range field bits,
trailing bytes, and unit/player creates whose display fields are
zero or absent from the extracted 5875 DBCs when those are present.

Defaults exercise TESTUSER/Groktar on Kalimdor. Override with
``UPDATE_BURST_ACCOUNT``, ``UPDATE_BURST_ACCOUNT_ID``,
``UPDATE_BURST_GUID``, ``UPDATE_BURST_MAP``, ``UPDATE_BURST_HOST``,
``UPDATE_BURST_PORT``, ``UPDATE_BURST_CAPTURE_SECS``, and
``MANGOS_DBC_DIR``.

Run from the repository root. Prints ``UPDATE-BURST-PASS`` on success.
"""

from __future__ import annotations

import math
import os
import pathlib
import select
import struct
import subprocess
import sys
import time
import zlib
from collections import Counter

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import phase3_scripted_login as p3  # noqa: E402
from phase3_scripted_login import Client  # noqa: E402

CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
CMSG_LOGOUT_REQUEST = 75
SMSG_LOGOUT_COMPLETE = 77
SMSG_UPDATE_OBJECT = 169
SMSG_COMPRESSED_UPDATE_OBJECT = 502
SMSG_LOGIN_VERIFY_WORLD = 566

UPDATETYPE_VALUES = 0
UPDATETYPE_MOVEMENT = 1
UPDATETYPE_CREATE_OBJECT = 2
UPDATETYPE_CREATE_OBJECT2 = 3
UPDATETYPE_OUT_OF_RANGE_OBJECTS = 4

UPDATEFLAG_TRANSPORT = 0x02
UPDATEFLAG_MELEE_ATTACKING = 0x04
UPDATEFLAG_HIGHGUID = 0x08
UPDATEFLAG_ALL = 0x10
UPDATEFLAG_LIVING = 0x20
UPDATEFLAG_HAS_POSITION = 0x40

MOVEFLAG_JUMPING = 0x0000_2000
MOVEFLAG_SWIMMING = 0x0020_0000
MOVEFLAG_SPLINE_ENABLED = 0x0040_0000
MOVEFLAG_ONTRANSPORT = 0x0200_0000
MOVEFLAG_SPLINE_ELEVATION = 0x0400_0000

SPLINE_FINAL_POINT = 0x0001_0000
SPLINE_FINAL_TARGET = 0x0002_0000
SPLINE_FINAL_ANGLE = 0x0004_0000

# TypeId -> values word count, from UpdateFields_1_12_1.cpp.
FIELD_COUNT_BY_TYPE = {
    0: 6,  # OBJECT_END
    1: 48,  # ITEM_END
    2: 122,  # CONTAINER_END
    3: 188,  # UNIT_END
    4: 1282,  # PLAYER_END
    5: 26,  # GAMEOBJECT_END
    6: 16,  # DYNAMICOBJECT_END
    7: 38,  # CORPSE_END
}

OBJECT_FIELD_ENTRY = 3
OBJECT_FIELD_SCALE_X = 4
UNIT_FIELD_FACTIONTEMPLATE = 35
UNIT_VIRTUAL_ITEM_SLOT_DISPLAY = 37
UNIT_FIELD_AURA = 47
UNIT_FIELD_AURA_END = 95
UNIT_FIELD_BOUNDINGRADIUS = 129
UNIT_FIELD_COMBATREACH = 130
UNIT_FIELD_DISPLAYID = 131
UNIT_FIELD_NATIVEDISPLAYID = 132
UNIT_FIELD_MOUNTDISPLAYID = 133
GAMEOBJECT_DISPLAYID = 8
GAMEOBJECT_FACTION = 20
DYNAMICOBJECT_SPELLID = 9


class ParseError(ValueError):
    pass


class Reader:
    def __init__(self, body: bytes):
        self.body = body
        self.pos = 0

    @property
    def remaining(self) -> int:
        return len(self.body) - self.pos

    def read(self, count: int) -> bytes:
        if count < 0 or self.pos + count > len(self.body):
            raise ParseError(f"truncated at byte {self.pos}: need {count}, have {self.remaining}")
        out = self.body[self.pos : self.pos + count]
        self.pos += count
        return out

    def u8(self) -> int:
        return self.read(1)[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.read(4))[0]

    def i32(self) -> int:
        return struct.unpack("<i", self.read(4))[0]

    def u64(self) -> int:
        return struct.unpack("<Q", self.read(8))[0]

    def f32(self) -> float:
        return struct.unpack("<f", self.read(4))[0]

    def packed_guid(self) -> int:
        mask = self.u8()
        guid = 0
        for byte_index in range(8):
            if mask & (1 << byte_index):
                guid |= self.u8() << (byte_index * 8)
        return guid


def load_dbc_ids(path: pathlib.Path) -> set[int] | None:
    if not path.is_file():
        return None
    data = path.read_bytes()
    if len(data) < 20 or data[:4] != b"WDBC":
        raise ParseError(f"invalid DBC header in {path}")
    record_count, _field_count, record_size, _string_size = struct.unpack_from("<IIII", data, 4)
    if record_size < 4 or 20 + record_count * record_size > len(data):
        raise ParseError(f"truncated DBC records in {path}")
    return {struct.unpack_from("<I", data, 20 + index * record_size)[0] for index in range(record_count)}


class UpdateValidator:
    def __init__(self, dbc_dir: pathlib.Path):
        self.creature_displays = load_dbc_ids(dbc_dir / "CreatureDisplayInfo.dbc")
        self.gameobject_displays = load_dbc_ids(dbc_dir / "GameObjectDisplayInfo.dbc")
        self.spells = load_dbc_ids(dbc_dir / "Spell.dbc")
        self.faction_templates = load_dbc_ids(dbc_dir / "FactionTemplate.dbc")
        self.item_displays = load_dbc_ids(dbc_dir / "ItemDisplayInfo.dbc")
        self.types_by_guid: dict[int, int] = {}
        self.last_movement: dict[str, object] = {}
        self.errors: list[str] = []
        self.warnings: list[str] = []
        for label, ids in (
            ("CreatureDisplayInfo.dbc", self.creature_displays),
            ("GameObjectDisplayInfo.dbc", self.gameobject_displays),
            ("Spell.dbc", self.spells),
            ("FactionTemplate.dbc", self.faction_templates),
            ("ItemDisplayInfo.dbc", self.item_displays),
        ):
            if ids is None:
                self.warnings.append(f"{label} not found; its ID checks were skipped")
        self.block_types: Counter[int] = Counter()
        self.object_types: Counter[int] = Counter()
        self.blocks = 0
        self.create_blocks = 0
        self.self_creates = 0
        self.self_seen = False
        self.creates_before_self = 0
        self.current_packet_has_transport = False

    def check(self, condition: bool, message: str) -> None:
        if not condition:
            self.errors.append(message)

    def read_finite_f32(self, reader: Reader, where: str, label: str) -> float:
        value = reader.f32()
        self.check(math.isfinite(value), f"{where}: non-finite {label} {value}")
        return value

    def check_dbc_id(self, ids: set[int] | None, value: int, label: str, where: str) -> None:
        if value and ids is not None and value not in ids:
            self.errors.append(f"{where}: {label} {value} not in client DBC")

    def check_positive_float_field(self, values: dict[int, int], index: int, label: str, where: str) -> None:
        if index not in values:
            return
        value = struct.unpack("<f", struct.pack("<I", values[index]))[0]
        self.check(
            math.isfinite(value) and value > 0,
            f"{where}: {label} must be finite and positive, got {value}",
        )

    def parse_packet(self, opcode: int, body: bytes, packet_index: int) -> None:
        if opcode == SMSG_COMPRESSED_UPDATE_OBJECT:
            if len(body) < 4:
                self.errors.append(f"packet {packet_index}: compressed body too short")
                return
            expected = struct.unpack_from("<I", body)[0]
            try:
                body = zlib.decompress(body[4:])
            except zlib.error as exc:
                self.errors.append(f"packet {packet_index}: zlib failure: {exc}")
                return
            self.check(
                len(body) == expected,
                f"packet {packet_index}: decompressed {len(body)} bytes, header promised {expected}",
            )

        reader = Reader(body)
        try:
            block_count = reader.u32()
            has_transport = reader.u8()
            self.current_packet_has_transport = has_transport == 1
            self.check(
                has_transport in (0, 1),
                f"packet {packet_index}: hasTransport byte is {has_transport}",
            )
            for block_index in range(block_count):
                self.parse_block(reader, packet_index, block_index)
            self.check(
                reader.remaining == 0,
                f"packet {packet_index}: {reader.remaining} trailing bytes",
            )
        except ParseError as exc:
            self.errors.append(f"packet {packet_index}: {exc}")

    def parse_block(self, reader: Reader, packet_index: int, block_index: int) -> None:
        start = reader.pos
        update_type = reader.u8()
        self.blocks += 1
        self.block_types[update_type] += 1
        where = f"packet {packet_index} block {block_index} at byte {start}"

        if update_type == UPDATETYPE_OUT_OF_RANGE_OBJECTS:
            count = reader.u32()
            for _ in range(count):
                reader.packed_guid()
            return

        if update_type == UPDATETYPE_MOVEMENT:
            guid = reader.u64()  # BuildMovementUpdateBlock writes a full GUID.
            self.parse_movement(reader, f"{where} movement guid=0x{guid:016x}")
            return

        guid = reader.packed_guid()
        if update_type == UPDATETYPE_VALUES:
            type_id = self.types_by_guid.get(guid)
            values = self.parse_values(reader, type_id, f"{where} values guid=0x{guid:016x}")
            if type_id in (3, 4):
                self.validate_unit_values(guid, type_id, values, where)
            return

        if update_type not in (UPDATETYPE_CREATE_OBJECT, UPDATETYPE_CREATE_OBJECT2):
            raise ParseError(f"{where}: unsupported update type {update_type}")

        type_id = reader.u8()
        self.types_by_guid[guid] = type_id
        self.object_types[type_id] += 1
        self.create_blocks += 1
        self.check(
            type_id in FIELD_COUNT_BY_TYPE,
            f"{where}: invalid object TypeId {type_id}",
        )
        update_flags = self.parse_movement(reader, f"{where} create guid=0x{guid:016x} type={type_id}")
        values = self.parse_values(reader, type_id, f"{where} create guid=0x{guid:016x}")
        if not self.self_seen:
            if type_id == 4 and update_flags & 0x01:  # UPDATEFLAG_SELF
                self.self_seen = True
                # C++ SendInitSelf runs before Map::Add sets
                # m_isNewObject, so the local player create is
                # CREATE_OBJECT, not CREATE_OBJECT2.
                self.check(
                    update_type == UPDATETYPE_CREATE_OBJECT,
                    f"{where}: init-self uses update type {update_type}, expected CREATE_OBJECT",
                )
            elif not (type_id == 5 and self.current_packet_has_transport):
                # C++ Map::Add sends init transports, then init-self,
                # then visibility creates. Non-transport world creates
                # must not precede the local player.
                self.creates_before_self += 1
        self.validate_create(guid, type_id, update_flags, values, where)

    def parse_movement(self, reader: Reader, where: str) -> int:
        update_flags = reader.u8()
        self.last_movement = {"update_flags": update_flags}

        if update_flags & UPDATEFLAG_LIVING:
            move_flags = reader.u32()
            server_time = reader.u32()
            position = tuple(self.read_finite_f32(reader, where, f"position {axis}") for axis in ("x", "y", "z", "o"))
            self.last_movement.update(move_flags=move_flags, server_time=server_time, position=position)
            if move_flags & MOVEFLAG_ONTRANSPORT:
                transport_guid = reader.u64()
                transport_position = tuple(
                    self.read_finite_f32(reader, where, f"transport position {axis}") for axis in ("x", "y", "z", "o")
                )
                self.last_movement.update(
                    transport_guid=transport_guid,
                    transport_position=transport_position,
                )
            if move_flags & MOVEFLAG_SWIMMING:
                self.last_movement["swim_pitch"] = self.read_finite_f32(reader, where, "swim pitch")
            self.last_movement["fall_time"] = reader.u32()
            if move_flags & MOVEFLAG_JUMPING:
                self.last_movement["jump"] = tuple(
                    self.read_finite_f32(reader, where, label)
                    for label in ("jump zspeed", "jump cos", "jump sin", "jump xyspeed")
                )
            if move_flags & MOVEFLAG_SPLINE_ELEVATION:
                self.last_movement["spline_elevation"] = self.read_finite_f32(reader, where, "spline elevation")
            speeds = []
            for _ in range(6):
                speed = self.read_finite_f32(reader, where, "movement speed")
                self.check(speed >= 0, f"{where}: negative movement speed {speed}")
                speeds.append(speed)
            self.last_movement["speeds"] = tuple(speeds)
            if move_flags & MOVEFLAG_SPLINE_ENABLED:
                self.parse_spline(reader, where)
        elif update_flags & UPDATEFLAG_HAS_POSITION:
            self.last_movement["position"] = tuple(
                self.read_finite_f32(reader, where, f"stationary position {axis}") for axis in ("x", "y", "z", "o")
            )

        if update_flags & UPDATEFLAG_HIGHGUID:
            value = reader.u32()
            self.check(value == 0, f"{where}: HIGHGUID word is {value}, expected 0")
        if update_flags & UPDATEFLAG_ALL:
            value = reader.u32()
            self.check(value == 1, f"{where}: ALL word is {value}, expected 1")
        if update_flags & UPDATEFLAG_MELEE_ATTACKING:
            reader.packed_guid()
        if update_flags & UPDATEFLAG_TRANSPORT:
            reader.u32()
        return update_flags

    def parse_spline(self, reader: Reader, where: str) -> None:
        spline_flags = reader.u32()
        summary: dict[str, object] = {"flags": spline_flags}
        if spline_flags & SPLINE_FINAL_ANGLE:
            summary["final_angle"] = self.read_finite_f32(reader, where, "spline final angle")
        elif spline_flags & SPLINE_FINAL_TARGET:
            summary["final_target"] = reader.u64()
        elif spline_flags & SPLINE_FINAL_POINT:
            summary["final_point"] = tuple(
                self.read_finite_f32(reader, where, f"spline final point {axis}") for axis in ("x", "y", "z")
            )
        summary["time_passed"] = reader.i32()
        summary["duration"] = reader.i32()
        summary["id"] = reader.u32()
        nodes = reader.u32()
        self.check(nodes <= 100_000, f"{where}: implausible spline node count {nodes}")
        points = [self.read_finite_f32(reader, where, "spline point") for _ in range(nodes * 3 + 3)]
        summary["nodes"] = nodes
        summary["destination"] = tuple(points[-3:])
        self.last_movement["spline"] = summary

    def parse_values(self, reader: Reader, type_id: int | None, where: str) -> dict[int, int]:
        block_count = reader.u8()
        mask = reader.read(block_count * 4)
        values: dict[int, int] = {}
        field_count = FIELD_COUNT_BY_TYPE.get(type_id) if type_id is not None else None
        if field_count is None:
            self.warnings.append(f"{where}: values mask width not checked (unknown object type)")
        else:
            expected_blocks = (field_count + 31) // 32
            self.check(
                block_count == expected_blocks,
                f"{where}: mask has {block_count} blocks, expected {expected_blocks} for TypeId {type_id}",
            )
        for index in range(block_count * 32):
            if mask[index // 8] & (1 << (index % 8)):
                if field_count is not None:
                    self.check(
                        index < field_count,
                        f"{where}: mask sets field {index}, beyond TypeId {type_id} count {field_count}",
                    )
                values[index] = reader.u32()
        return values

    def validate_create(
        self,
        guid: int,
        type_id: int,
        update_flags: int,
        values: dict[int, int],
        where: str,
    ) -> None:
        if 0 in values and 1 in values:
            value_guid = values[0] | (values[1] << 32)
            self.check(
                value_guid == guid,
                f"{where}: OBJECT_FIELD_GUID 0x{value_guid:016x} does not match block GUID 0x{guid:016x}",
            )
        self.check_positive_float_field(values, OBJECT_FIELD_SCALE_X, "OBJECT_FIELD_SCALE_X", where)
        if type_id in (1, 2):
            self.check(
                values.get(OBJECT_FIELD_ENTRY, 0) != 0,
                f"{where}: item/container create has zero/missing OBJECT_FIELD_ENTRY",
            )
        elif type_id == 6:
            self.check_dbc_id(
                self.spells,
                values.get(DYNAMICOBJECT_SPELLID, 0),
                "DYNAMICOBJECT_SPELLID",
                where,
            )
        if type_id == 4:
            if update_flags & 0x01:  # UPDATEFLAG_SELF
                self.self_creates += 1
            self.validate_unit_values(guid, type_id, values, where)
        elif type_id == 3:
            self.check(
                values.get(OBJECT_FIELD_ENTRY, 0) != 0,
                f"{where}: creature create has zero/missing OBJECT_FIELD_ENTRY",
            )
            self.validate_unit_values(guid, type_id, values, where)
        elif type_id == 5:
            self.check(
                values.get(OBJECT_FIELD_ENTRY, 0) != 0,
                f"{where}: gameobject create has zero/missing OBJECT_FIELD_ENTRY",
            )
            self.check_dbc_id(
                self.gameobject_displays,
                values.get(GAMEOBJECT_DISPLAYID, 0),
                "GAMEOBJECT_DISPLAYID",
                where,
            )
            self.check_dbc_id(
                self.faction_templates,
                values.get(GAMEOBJECT_FACTION, 0),
                "GAMEOBJECT_FACTION",
                where,
            )

    def validate_unit_values(self, guid: int, _type_id: int, values: dict[int, int], where: str) -> None:
        self.check_dbc_id(
            self.faction_templates,
            values.get(UNIT_FIELD_FACTIONTEMPLATE, 0),
            "UNIT_FIELD_FACTIONTEMPLATE",
            where,
        )
        for slot in range(3):
            self.check_dbc_id(
                self.item_displays,
                values.get(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + slot, 0),
                f"UNIT_VIRTUAL_ITEM_SLOT_DISPLAY[{slot}]",
                where,
            )
        self.check_positive_float_field(values, UNIT_FIELD_BOUNDINGRADIUS, "UNIT_FIELD_BOUNDINGRADIUS", where)
        self.check_positive_float_field(values, UNIT_FIELD_COMBATREACH, "UNIT_FIELD_COMBATREACH", where)
        for field, label in (
            (UNIT_FIELD_DISPLAYID, "UNIT_FIELD_DISPLAYID"),
            (UNIT_FIELD_NATIVEDISPLAYID, "UNIT_FIELD_NATIVEDISPLAYID"),
        ):
            if field not in values:
                continue
            display = values[field]
            self.check(display != 0, f"{where}: {label} is zero for guid 0x{guid:016x}")
            self.check_dbc_id(self.creature_displays, display, label, where)
        self.check_dbc_id(
            self.creature_displays,
            values.get(UNIT_FIELD_MOUNTDISPLAYID, 0),
            "UNIT_FIELD_MOUNTDISPLAYID",
            where,
        )
        for index in range(UNIT_FIELD_AURA, UNIT_FIELD_AURA_END):
            spell = values.get(index, 0)
            self.check_dbc_id(self.spells, spell, f"UNIT_FIELD_AURA[{index}]", where)


def env_int(name: str, default: int) -> int:
    return int(os.environ.get(name, str(default)))


def sql_scalar(db: str, stmt: str) -> str:
    result = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{p3.bench_password()}", "-N", "-B", db, "-e", stmt],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"mysql query failed for {db}")
    return result.stdout.strip()


def ensure_character_offline(guid: int) -> None:
    if os.environ.get("UPDATE_BURST_FORCE") == "1":
        return
    online = sql_scalar("characters", f"SELECT online FROM characters WHERE guid={guid}")
    if online == "1":
        raise RuntimeError(f"character guid {guid} is marked online; aborting scripted login")


def is_update_packet(opcode: int) -> bool:
    return opcode in (SMSG_UPDATE_OBJECT, SMSG_COMPRESSED_UPDATE_OBJECT)


def read_until(client: Client, wanted: int, captured: list[tuple[int, bytes]]) -> bytes:
    while True:
        opcode, body = client.read_packet()
        if is_update_packet(opcode):
            captured.append((opcode, body))
        if opcode == wanted:
            return body


def capture_for(client: Client, seconds: float, captured: list[tuple[int, bytes]]) -> None:
    client.sock.settimeout(2)
    deadline = time.time() + seconds
    while time.time() < deadline:
        ready, _, _ = select.select([client.sock], [], [], 0.25)
        if not ready:
            continue
        opcode, body = client.read_packet()
        if is_update_packet(opcode):
            captured.append((opcode, body))


def logout_and_close(client: Client | None, captured: list[tuple[int, bytes]]) -> None:
    if client is None:
        return
    try:
        client.send_packet(CMSG_LOGOUT_REQUEST, b"")
        client.sock.settimeout(2)
        deadline = time.time() + 3
        while time.time() < deadline:
            ready, _, _ = select.select([client.sock], [], [], 0.25)
            if not ready:
                continue
            opcode, body = client.read_packet()
            if is_update_packet(opcode):
                captured.append((opcode, body))
            if opcode == SMSG_LOGOUT_COMPLETE:
                break
    except Exception:
        pass
    finally:
        client.sock.close()


def main() -> int:
    account = os.environ.get("UPDATE_BURST_ACCOUNT", "TESTUSER")
    account_id = env_int("UPDATE_BURST_ACCOUNT_ID", 142)
    guid = env_int("UPDATE_BURST_GUID", 2)
    expected_map = env_int("UPDATE_BURST_MAP", 1)
    capture_secs = float(os.environ.get("UPDATE_BURST_CAPTURE_SECS", "5"))

    p3.ACCOUNT = account
    p3.ACCOUNT_ID = account_id
    p3.HOST = os.environ.get("UPDATE_BURST_HOST", p3.HOST)
    p3.PORT = env_int("UPDATE_BURST_PORT", p3.PORT)

    ensure_character_offline(guid)
    p3.setup_account()

    captured: list[tuple[int, bytes]] = []
    client: Client | None = None
    logged_in = False
    try:
        client = Client()
        client.auth()
        client.send_packet(CMSG_CHAR_ENUM, b"")
        opcode, _ = client.read_packet()
        assert opcode == SMSG_CHAR_ENUM, opcode

        client.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", guid))
        body = read_until(client, SMSG_LOGIN_VERIFY_WORLD, captured)
        map_id, x, y, z, orientation = struct.unpack("<Iffff", body)
        print(f"VERIFY-WORLD map={map_id} pos=({x:.2f},{y:.2f},{z:.2f}) o={orientation:.2f}")
        assert map_id == expected_map, (map_id, expected_map)
        logged_in = True
        capture_for(client, capture_secs, captured)
    finally:
        if logged_in:
            logout_and_close(client, captured)
        elif client is not None:
            client.sock.close()

    dbc_dir = pathlib.Path(os.environ.get("MANGOS_DBC_DIR", "~/azeroth-dev-fixtures/datadir/5875/dbc")).expanduser()
    validator = UpdateValidator(dbc_dir)
    if not captured:
        validator.errors.append("no SMSG_UPDATE_OBJECT packets captured")
    for index, (opcode, body) in enumerate(captured):
        validator.parse_packet(opcode, body, index)
    validator.check(validator.create_blocks > 0, "no object-create blocks captured")
    validator.check(validator.self_creates > 0, "no self player create captured")
    validator.check(
        validator.creates_before_self == 0,
        f"{validator.creates_before_self} non-transport creates precede init-self",
    )

    print(
        "BURST-SUMMARY "
        f"packets={len(captured)} blocks={validator.blocks} "
        f"creates={validator.create_blocks} "
        f"block_types={dict(sorted(validator.block_types.items()))} "
        f"object_types={dict(sorted(validator.object_types.items()))}"
    )
    for warning in validator.warnings[:20]:
        print(f"WARNING: {warning}")
    if validator.errors:
        for error in validator.errors[:50]:
            print(f"ERROR: {error}")
        if len(validator.errors) > 50:
            print(f"ERROR: ... {len(validator.errors) - 50} more")
        return 1

    print("UPDATE-BURST-PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
