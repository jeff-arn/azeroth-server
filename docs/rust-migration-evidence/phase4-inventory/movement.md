# Phase 4 evidence inventory — Movement

- Workspace: `/Users/jeffarn/Programming/azeroth-server`, branch
  `rust-migration`, HEAD `1bec69f`. Port oracle: C++ at checkout HEAD
  (`vmangos/src/`); production binary `2bae5f9` (see §f/Divergence).
- Method: every implementation body cited below was opened with
  `read_file` (full file or offset window); grep only located
  candidates. Anchors are `file:line` at HEAD.
- Build variant pinned: default `SUPPORTED_CLIENT_BUILD` =
  `CLIENT_BUILD_1_12_1` (`vmangos/cmake/options.cmake:11`, materialised
  via `vmangos/cmake/generators/Progression.h.in:36`). Therefore every
  `#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_9_4` and
  `> CLIENT_BUILD_1_8_4` branch cited below is the **taken** branch
  (packed GUIDs, `movementCounter u32` present in ACKs, root-on-landing
  1.14-client workarounds active); `== 1_9_4` / `<= 1_8_4` branches are
  dead at this build and are flagged as such where they appear.

## (a) Verified structure / flow

### A1. `MovementInfo` — the wire record (critic anchors re-verified)

Prior critic anchors **confirmed exactly**: class at
`vmangos/src/game/Objects/MovementInfo.h:157-247`; `Read`/`CorrectData`/
`Write` at `vmangos/src/game/Objects/Object.cpp:61-172`.

Fields (`MovementInfo.h:230-246`, all public): `moveFlags u32`,
`stime u32` (server), `ctime u32` (client), `pos{x,y,z,o f32}`,
`t_guid ObjectGuid`, `t_pos{x,y,z,o}`, `s_pitch f32`, `fallTime u32`,
`jump{zspeed,sinAngle,cosAngle,xyspeed f32; start Position;
startClientTime u32}` (`MovementInfo.h:219-225` — note struct declares
sin before cos, wire writes cos first, see below),
`splineElevation f32`, `sourceSessionGuid u32` (server-only bookkeeping,
never on wire).

`Read` (`Object.cpp:61-103`), in order:

1. `stime = WorldTimer::getMSTime()` **before** any byte is read
   (`:63`) — later overwritten by handlers with packet time.
2. `moveFlags u32` (`:64`), **`ctime u32` (`:65`)**, `pos.x/y/z/o f32`
   (`:66-69`).
3. If `MOVEFLAG_ONTRANSPORT (0x02000000)`: `t_guid` then
   `t_pos.x/y/z/o f32` (`:71-78`).
4. If `MOVEFLAG_SWIMMING (0x00200000)`: `s_pitch f32` (`:79-80`).
5. `fallTime u32` unconditionally (`:82`).
6. If `MOVEFLAG_JUMPING (0x00002000)`: `jump.zspeed, jump.cosAngle,
   jump.sinAngle, jump.xyspeed f32` (`:86-89`, **cos before sin**);
   side effect: if `jump.startClientTime==0`, latch
   `startClientTime=ctime, start=pos` (`:90-94`); else-branch (not
   jumping) resets `jump.startClientTime=0` (`:96-97`). This latch
   depends on the *previous* value in the destination struct — see the
   copy-seeding note in A3.
7. If `MOVEFLAG_SPLINE_ELEVATION (0x04000000)`: `splineElevation f32`
   (`:99-102`).

`Write` (`Object.cpp:138-172`) is the mirror image with one deliberate
asymmetry: it writes **`stime` (`:141`) where `Read` read `ctime`
(`:65`)** — i.e. relayed packets carry server time in the client-time
slot. All conditional blocks, order and cos-before-sin are identical
(`:147-171`). `sourceSessionGuid`, `jump.start/startClientTime` are
never serialised.

Byte counts (little-endian, packed GUID excluded — GUID is written by
callers, not by `Write`): base = flags 4 + time 4 + pos 16 + fallTime 4
= **28 B**. Optional: ONTRANSPORT +24 B (guid 8 + 4×f32; note `Read`
reads `t_guid` via `operator>>` on `ObjectGuid` = raw u64 here, not
packed — `Object.cpp:73/149`), SWIMMING +4 B, JUMPING +16 B,
SPLINE_ELEVATION +4 B. Max = 28+24+4+16+4 = **76 B**.

`CorrectData` (`Object.cpp:105-136`): strips mutually-exclusive flag
pairs via macro (`:108-110`): ROOT removed if ROOT && MASK_MOVING
(`:117`, anti-freeze), TURN_LEFT|TURN_RIGHT both removed (`:124`),
STRAFE_LEFT|STRAFE_RIGHT (`:127`), PITCH_UP|PITCH_DOWN (`:130`),
FORWARD|BACKWARD (`:133`). HOVER-vs-aura check is commented out
(`:119-121`). No other normalisation (no NaN/coordinate check here).

`SetAsServerSide` (`MovementInfo.h:205-216`): `stime=now`, forced
strictly monotonic (`oldTime>=stime ⇒ stime=oldTime+1`), `ctime=0`
("pauses extrapolation"), `sourceSessionGuid=0`.
`WasSentBySession` = `ctime!=0 && sourceSessionGuid==guid` (`:217`).

Flags enum: 32 bits + 3 masks (`MovementInfo.h:27-70`):
`MASK_MOVING` = FORWARD|BACKWARD|STRAFE_L|STRAFE_R|PITCH_UP|PITCH_DOWN|
JUMPING|FALLINGFAR|SPLINE_ELEVATION (`:64`); `MASK_MOVING_OR_TURN`
(`:65`); `MASK_XZ` = FORWARD|BACKWARD|STRAFE_L|STRAFE_R (`:69`, melee
leeway). Legacy `SplineFlags` enum (`:147-155`, WALKMODE 0x100, FLYING
0x200, SPLINE 0x2000) coexists with the real spline flag word (A5).

### A2. Cross-check vs `22-opcode-catalog.md` §7 — line by line

Catalog §7 movement bullet is at
`docs/rust-migration-evidence/22-opcode-catalog.md:142-157`. Verdict:
**every field claim matches the bodies**:

| Catalog claim (§7) | Body evidence | Verdict |
|---|---|---|
| 36 rows: `HandleMovementOpcodes` (22) | `Opcodes.cpp` grep count 22 (0x0B5-0x0DB block + heartbeat 0x0EE + `CMSG_MOVE_FALL_RESET` 0x2CA) | ✓ exact |
| `HandleForceSpeedChangeAckOpcodes` (6) | count 6; switch at `MovementHandler.cpp:435-452` | ✓ |
| `HandleMovementFlagChangeToggleAck` (3) | count 3 (0x0F6, 0x2CF, 0x2D0) | ✓ |
| `HandleMoveRootAck` (2) | count 2 (0x0E9 ROOT, 0x0EB UNROOT) | ✓ |
| TeleportAck / KnockBackAck / SplineDone (1 each) | 0x0C7 / 0x0F0 / 0x2C9, one `StoreOpcode` each | ✓ (36 total) |
| `moveFlags u32, ctime u32, pos.x/y/z/o f32` | `Object.cpp:64-69` | ✓ |
| ONTRANSPORT: `t_guid u64, t_pos.x/y/z/o f32` | `Object.cpp:71-78` | ✓ |
| SWIMMING: `s_pitch f32`; then `fallTime u32` | `Object.cpp:79-82` | ✓ (order: s_pitch before fallTime) |
| JUMPING: `zspeed, cosAngle, sinAngle, xyspeed` (cos first) | `Object.cpp:86-89` | ✓ |
| SPLINE_ELEVATION: `splineElevation f32` | `Object.cpp:99-102` | ✓ |
| SplineDone appends `splineId u32` + `Unused<float>` | `MovementHandler.cpp:830-831` | ✓ |
| "parser NOT yet ported; mangos-movement = spline only" | `vmangos-rs/crates/mangos-movement/src/` = `spline.rs`+`move_spline.rs`+`packet_builder.rs`, no `MovementInfo` symbol | ✓ still true at HEAD |

Two precision notes the catalog does not state (not disagreements):
(a) `Read` also stamps `stime` and latches/clears jump-start state
(A1.1/A1.6) — a port that only copies fields will diverge on
anticheat/extrapolation inputs; (b) ACK handlers each read extra fields
around the `MovementInfo` (guid, movementCounter, speed/apply) — see A3.

### A3. Client→server handlers (`MovementHandler.cpp`, 1201 lines)

Opcode registration: `vmangos/src/game/Server/Protocol/Opcodes.cpp:241-307,690,785-807`
(all `STATUS_LOGGEDIN, PACKET_PROCESS_MOVEMENT` except worldport-ack
`STATUS_TRANSFER, PACKET_PROCESS_WORLD` at `:280`).

**`HandleMovementOpcodes` (`:285-403`)** — the 22-opcode hot path
(MSG_MOVE_* start/stop/turn/pitch/run/walk/fall/swim/facing/heartbeat +
CMSG_MOVE_FALL_RESET). Gate order, each silently dropping the packet:
packet time `<= m_moveRejectTime` (`:290`); no confirmed mover
(`GetConfirmedMover`, `:293`); `HasPendingSplineDone` (`:297`); spline
not finalized = server is moving the unit (`:301`); player mover being
teleported (`:307`). Then the key seeding detail: the parse target is
**copy-initialised from the mover's current `m_movementInfo`** for
player movers (fresh zeroed for creature movers) (`:311`) — this is
what makes the `jump.startClientTime` latch in `Read` stateful across
packets. `recvData >> movementInfo` (`:312`), then
`UpdateTime(recvData.GetPacketTime())` overwrites the `stime` stamped
in `Read` (`:313`). Validation: `VerifyMovementInfo` (`:316`, body at
`:1078-1095`: `IsValidMapCoord(pos)`; if ONTRANSPORT, |t_pos| ≤
250/250/100 and pos+t_pos valid). Anticheat (player movers only):
`HandleFlagTests` then `HandlePositionTests`; either returning non-zero
sets `m_moveRejectTime` and drops (`:321-324`) — bodies live in
`Anticheat/MovementAnticheat/MovementAnticheat.h:86-88` (interface
inspected; test internals are the anticheat surface, not opened here).
Side effects: `MSG_MOVE_JUMP`→`SetJumpInitialSpeed(7.95797334f)`,
`FALL_LAND`→`-9.645f` (`:328-331`); fall damage `HandleFall` on
FALL_LAND unless taxi-flying (`:334-335`); launched-state reset +
`UpdateFallInformationIfNeed` (`:341-344`); relocation (`:347`, see
`HandleMoverRelocation` below); root-on-landing enforcement incl.
`KickPlayer` (`:353-365`, 1.12.1 build ⇒ active).
**Relay**: `CMSG_MOVE_FALL_RESET` is applied but **never relayed**
(`:371-372`, no client handler). Otherwise a new packet with the
*same opcode* is built: packed mover GUID (`:396`, 1.12.1 branch) +
`movementInfo.Write` (server `stime` in time slot) and sent via
`SendMovementMessageToSet(data, true, _player)` — to observers,
excluding the mover's own session player (`:393-402`).

**ACK family** — common shape: `guid`, `movementCounter u32` (1.12.1),
`MovementInfo`, trailing payload; mover authorised via
`GetMoverFromGuid` (`:1059-1069`: must be current mover, the player, or
`m_clientMoverGuid`); a matching *pending change* must exist or the ACK
is logged and, if counter is 0 or ahead of server counter,
`OnWrongAckData()` anticheat penalty fires (pattern repeated at
`:463-477,551-587,678-694,781-795`).

- `HandleForceSpeedChangeAckOpcodes` (`:414-519`, 6 opcodes): trailing
  `speedReceived f32` (`:427-428`); opcode→`UnitMoveType` map
  (`:435-452`); pending match via `FindPendingMovementSpeedChange`;
  **speed applied before relocation** (`newSpeedRate =
  speedReceived / baseMoveSpeed[type]`, `SetSpeedRateReal`, `:496-497`
  — ordering comment at `:491-495` is load-bearing: outdoor-aura case);
  relocation only if mover is active and not teleporting, else only
  flags copied + `CorrectData` (`:501-514`); observers get
  `SendSpeedChangeToObservers` (`:518`).
- `HandleMovementFlagChangeToggleAck` (`:527-650`, 3): trailing
  `applyInt u32` (`:541-543`); WATER_WALK/HOVER/FEATHER_FALL →
  `SetWaterWalkingReal/SetHoverReal/SetFeatherFallReal` (`:634-642`);
  feather-fall also `SetJumpInitialSpeed(max(cur,7.0f))` (`:590`);
  position checks decoupled from ACK acceptance via do/while(false)
  (`:593-630` — change applies even if position rejected);
  observers via `SendMovementFlagChangeToObservers` (`:649`).
- `HandleMoveRootAck` (`:657-760`, 2): apply = (opcode==ROOT_ACK);
  1.14-client root-on-landing dance incl. kick (`:737-756`);
  `SetRootedReal` + observer relay (`:758-759`).
- `HandleMoveKnockBackAck` (`:762-820`, 1): matched via
  `FindPendingMovementKnockbackChange(movementInfo,…)`; clears fall
  info; relays `jump.cos/sin/xyspeed/zspeed` to observers (`:819`).
- `HandleMoveTeleportAckOpcode` (`:213-240`): **no MovementInfo** —
  `guid, movementCounter, time u32`; requires `IsBeingTeleportedNear`
  and guid match; executes near teleport (`ExecuteTeleportNear`
  `:242-283`: `MSG_MOVE_TELEPORT` to observers sent **twice**, before
  and after relocation, `:254,257`).
- `HandleMoveSplineDoneOpcode` (`:822-876`): `MovementInfo`,
  `splineId u32`, `Unused<float>`; requires mover guid ==
  `m_clientMoverGuid`, `movespline->GetId()==splineId`, and pending
  spline-done (cleared only after id check, `:844-848`); anticheat
  `HandleSplineDone` + flag tests; relayed to observers rewritten as
  `MSG_MOVE_HEARTBEAT` if MASK_MOVING else `MSG_MOVE_STOP` (`:866`).
- `HandleMoveNotActiveMoverOpcode` (`:922-999`): old-mover guid +
  MovementInfo (1.12.1 branch); validates/relocates the *old* mover;
  same heartbeat/stop rewrite (`:989`).
- `HandleSetActiveMoverOpcode` (`:878-920`): guid only; mismatched guid
  is corrected server-side, not kicked (`:886-891`).
- `HandleMoveTimeSkippedOpcode` (`:1020-1056`): `guid, lag u32`; adds
  lag to both `stime` and `ctime` if ctime≠0 (`:1031-1035`); relayed as
  `MSG_MOVE_TIME_SKIPPED` (packGUID+lag, 12 B nominal) unless player
  just boarded a transport (`:1047-1055`).
- `HandleMoveWorldportAckOpcode` (`:43-211`): far-teleport completion;
  map creation, BG/instance handling, flight resume (`:149-163`) — map
  surface, only its movement touchpoints noted here.
- `HandleMountSpecialAnimOpcode` (`:1001-1007`): relays
  `SMSG_MOUNTSPECIAL_ANIM`+guid to set **including self** (`self=false`
  arg = do-not-exclude semantics of the impl; packet to all observers).

**`HandleMoverRelocation` (`:1097-1201`)** — shared commit step:
stamps `sourceSessionGuid`, `CorrectData()` (`:1101-1102`); re-adds
ROOT if server has it and client dropped it (`:1105-1106`); **wholesale
assignment** `pMover->m_movementInfo = movementInfo` (`:1108`);
transport board/leave + local→world position conversion via
`CalculatePassengerPosition` (`:1110-1137`); player branch: loot
release on MASK_MOVING (`:1141-1146`), `SetPosition`, anti-undermap 1
(FALLINGFAR + 100 yd below height, or WSG z<250 ⇒ recall, `:1151-1164`),
safe-position save at z+3 (`:1166`), anti-undermap 2 (z<-500 ⇒ void
damage + graveyard repop, `:1169-1194`); creature branch:
`HandleInterruptsOnMovement` + `CreatureRelocation` (`:1198-1199`).

**Observer delivery**: `SendMovementMessageToSet`
(`Object.cpp:2150-2164`) — if the broadcaster is enabled and sender is
a player, packet is queued to that player's `PlayerBroadcaster`
(async, `MovementBroadcaster.cpp:107-117` drains per-player queues on
N worker threads, sharded by `GUID % threads` at `:46`); otherwise
synchronous `ObjectViewersMovementDeliverer` cell visit. Ordering
guarantee therefore differs by config (see §c).

### A4. Server→client forced changes (`MovementPacketSender`, 487+118 lines)

Three-step protocol documented in the header itself
(`MovementPacketSender.h:29-87`): server→controller (pending change +
counter), client ACK (A3), server→observers. Server-controlled units
skip steps 1-2 (`…ToAll`, spline opcodes).

- Opcode matrix `moveTypeToOpcode[6][3]` = {spline, force, observer}
  per move type (`MovementPacketSender.cpp:27-33`; 1.12.1 row uses
  `SMSG_SPLINE_SET_*`).
- To controller: packGUID + `movementCounter u32` + flat speed f32
  (`:56-75`); pending change pushed **before** send (`:35-54`), counter
  from `GetMovementCounterAndInc`.
- To observers (player mover, spline finalized): `MSG_MOVE_SET_*_SPEED`
  = packGUID + full `m_movementInfo` + speed f32 (`:128-140`); if
  spline active: `SMSG_SPLINE_SET_*` = packGUID + speed only
  (`:141-153`).
- To all (server unit): `SMSG_SPLINE_SET_*` packGUID + `rate *
  baseMoveSpeed` (`:156-168`).
- Teleport-to-controller masquerades as `MSG_MOVE_TELEPORT_ACK` (41 B
  nominal) with `SetAsServerSide` MovementInfo (`:170-203`); observers
  get `MSG_MOVE_TELEPORT` via `SendObjectMessageToSet` (immediate, not
  queued — `:205-223`).
- Knockback: controller `SMSG_MOVE_KNOCK_BACK` = guid+counter+4×f32
  (`:225-263`); observers `MSG_MOVE_KNOCK_BACK` = guid+MovementInfo+
  4×f32 (`:265-288`).
- Flag toggles: only ROOT/WATERWALKING/HOVER/SAFE_FALL supported, other
  flags log-and-drop (`:299-317`); observer form is guid+MovementInfo
  with opcode implying apply for ROOT but **same opcode for apply and
  remove** for the other three (`:377-405`) — the MovementInfo flags
  carry the state. Server-unit form uses `SMSG_SPLINE_MOVE_*` pair
  opcodes, guid only (`:407-473`).
- Run/walk toggle: `SMSG_SPLINE_MOVE_SET_RUN/WALK_MODE`, 9 B nominal
  (`:475-487`).

### A5. Spline engine + `SMSG_MONSTER_MOVE`

`MoveSpline` (`spline/MoveSpline.h:40-133`, `.cpp` 301 lines): holds
`Spline<int32>` (ms timestamps), `FacingInfo` union, spline id,
transport low-guid, `MoveSplineFlag`, `time_passed/point_Idx/
last_point_sent_Idx/point_Idx_offset`. `MoveSplineFlag` is a packed
32-bit bitfield struct (`MoveSplineFlag.h:33-138`): Done 0x1, Falling
0x2, Runmode 0x100, Flying 0x200 (= CatmullRom mask), No_Spline 0x400,
Final_Point/Target/Angle 0x10000/0x20000/0x40000, Cyclic 0x100000,
Enter_Cycle 0x200000, Frozen 0x400000; `Mask_No_Monster_Move =
Mask_Final_Facing|Done` stripped on wire (`:75`).

Timing model (`MoveSpline.cpp:105-164`): non-falling segments get
cumulative ms via `CommonInitializer` (1000/velocity × seg length,
starting at `minimal_duration=1`); falling via `FallInitializer`
(`computeFallTime(startZ - pointZ)`). Zero-length spline is clamped to
1 ms (1000 ms if cyclic) with an error log (`:158-162`). Mode =
CatmullRom iff `flags.isSmooth()` (Flying), else Linear (`:133-143`).
`_updateState` (`:226-264`): advance `time_passed` by
min(diff, segment remaining); crossing a timestamp advances
`point_Idx` → NextSegment; at last point: cyclic wraps
(`time_passed %= Duration`), else `_Finalize` (done=true,
point_Idx=last-1, time_passed=Duration, `:287-292`) → Arrived.
`ComputePosition` (`:61-93`): u = segment fraction, spline evaluate;
falling overrides z (clamped ≥ final z, `:95-103`); orientation from
derivative `atan2`, or final facing when done. Fall physics constants
(`spline/util.cpp:24-32`): gravity **19.29110527038574**, terminal
velocity 60.148003, safe-fall terminal 7.0.

Validation (`MoveSpline.cpp:184-197`): only `path.size()>1` and
`velocity>0` enforced; `_checkPathBounds` (11-bit packed-offset limit,
±1024) exists but its CHECK is commented out (`:194,201-222`).

Launch (`spline/MoveSplineInit.cpp:65-185`): computes real position
from in-flight spline if not finalized (`:77-85`), forces
`path[0]=real_position` (`:97`); moveFlags bookkeeping — done⇒clear
SPLINE_ENABLED|MASK_MOVING, else set SPLINE_ENABLED|FORWARD and
WALK_MODE per runmode (`:100-113`); velocity 0 ⇒
`unit.GetSpeed(SelectSpeedType(flags))` (`:120-121`, selector at
`:33-51`: swim>walk>back>run precedence); spline id from a
**`thread_local` counter starting at 1** (`:63,126` — ids are
per-thread, not globally unique!); player/possessed units get
`SetSplineDonePending(true)` (`:131-132`); packet = `SMSG_MONSTER_MOVE`
(221) or `SMSG_MONSTER_MOVE_TRANSPORT` (686) if transport
(`Opcodes_1_12_1.h:224,689`), packGUID (+transport packGUID), then
either stop form (pos + splineId + u8 1, `:162-167`) or
`PacketBuilder::WriteMonsterMove`; flag/run-walk correction packets
may precede/follow (`:171-182`); sent with `self=true` (controller
also receives it — that's how a possessed/player spline is driven).
Returns duration ms.

`PacketBuilder` (`spline/packet_builder.cpp:39-175`):
common part = first spline point (3×f32), splineId u32, move-type u8
(0 normal/1 stop/2 facing-spot/3 facing-target/4 facing-angle) +
facing payload (none / — / 3×f32 / u64 target / f32 angle), flags u32
= `(flags & ~Mask_No_Monster_Move) | Runmode` with Enter_Cycle faked
from cyclic (`:65-68` — **Runmode is always set on wire** even for
walk splines), duration u32 (ms). Linear path (`:72-106`): count u32,
destination 3×f32, then intermediate points as **offsets from
destination, `appendPackXYZ` 11-bit packed**, capped at
`Movement.MaxPointsPerPacket` (default 80) points per packet with
continuation index returned; near-zero offsets nudged in z (+0.26 /
+0.51) to avoid a client freeze (`:94-101`). CatmullRom (`:108-121`):
raw count + full Vector3 array (never chunked); cyclic prepends a
fake first point the client erases after cycle 1. Chunked linear
moves rewrite the duration field in place to the partial duration
minus elapsed (`:140-143`). Continuation packets are emitted from
`Unit::UpdateSplineMovement` when the unit comes within 3 points of
the last sent index (`Unit.cpp:10689-10699`).

`WriteCreate` (object-create embedding, `packet_builder.cpp:147-175`):
raw spline flags u32 (**unmasked**, includes Done/facing bits),
facing payload, timePassed i32, Duration i32, splineId u32, node
count u32, **all control points incl. spline padding** as Vector3s,
final destination (zero if cyclic).

Spline tick: `Unit::Update` → `UpdateSplineMovement(diff)`
(`Unit.cpp:316,10674-10717`): updateState, on arrival `DisableSpline`
(clears SPLINE_ENABLED|FORWARD, ctime=0, `:10743-10750`); compute
position; transport-local→world conversion; invalid coords silently
skip relocation; player `SetPosition` / creature `CreatureRelocation`.
`StopMoving` launches a done-spline (stop packet) unless
uninterruptible (`Unit.cpp:9136-9153`, cited from grep window +
call sites; body beyond :9153 not opened — see §f).

Create-block embedding: `Object::BuildMovementUpdate`
(`Object.cpp:447-478`, 1.12.1 branch): updateFlags u8; if LIVING:
MovementInfo — but if `ctime==0` (server-side), stime is faked to
`now+1000` and position refreshed from the object (`:458-462`);
creatures have ROOT stripped (`:463-464`); then **6 speeds as flat
f32** in order WALK, RUN, RUN_BACK, SWIM, SWIM_BACK, TURN_RATE
(`:469-474`); then `WriteCreate` iff SPLINE_ENABLED (`:476-477`).

### A6. MotionMaster stack + generators

`MotionMaster` (`MotionMaster.h:82-179`, `.cpp` 945 lines) is a
`std::stack<MovementGenerator*>` per Unit. Bottom = default generator
(never popped by expire; `DirectExpire` no-ops at size 1,
`MotionMaster.cpp:281`). Generator interface
(`MovementGenerator.h:34-69`): `Initialize/Finalize/Interrupt/Reset/
Update(diff)->bool` (false ⇒ expire self), optional `UpdateAsync`
(thread-safe contract: pathfinding only, no AI/world mutation),
`UnitSpeedChanged`, `GetResetPosition`. `MovementGeneratorMedium<T,D>`
CRTP dispatches Player vs Creature (`:71-121`); factory registry for
DB-selectable types (`:123-138`).

Type enum (`MotionMaster.h:36-59`): DB-legal 0-3 (IDLE, RANDOM,
WAYPOINT, CYCLIC; `MAX_DB_MOTION_TYPE=4`), then CONFUSED 5, CHASE 6,
HOME 7, FLIGHT 8, POINT 9, FLEEING 10, DISTRACT 11, ASSISTANCE 12,
ASSISTANCE_DISTRACT 13, TIMED_FLEEING 14, FOLLOW 15, EFFECT 16,
PATROL 17, CHARGE 18, DISTANCING 19.

Stack semantics (all bodies read):
- `Initialize` (`:43-79`): stop, clear all, push factory-selected
  default for alive unpossessed creatures (waypoint/cyclic get
  `InitializeWaypointPath` immediately), else static `si_idleMovement`
  singleton (never deleted, `isStatic` `:41`).
- `Mutate` = push (`:672-699`): if top is CHASE (unless new is
  DISTANCING), HOME, DISTRACT or DISTANCING it is **expired instead of
  interrupted** (fallthrough switch `:676-691`); otherwise top gets
  `Interrupt`; new gen `Initialize`d **before** push (`:697-698` —
  Initialize can itself launch splines / recurse).
- `UpdateMotion` (`:173-209`): skipped entirely under
  `UNIT_STATE_CAN_NOT_MOVE`; sets MMCF_UPDATE so Clear/Expire during
  Update become delayed (finalize now, delete after update via
  `m_expList`); false return ⇒ `MovementExpired`; after deferred
  deletion, empty⇒`Initialize`, MMCF_RESET⇒`top()->Reset`.
- Expire (`:279-350`): pop top; also pop any CHASE/FOLLOW exposed
  underneath (unless expiring gen is DISTANCING); `Finalize` may push
  new gens — Reset only applied if top is unchanged (`:301-313`).
- `Clear(reset,all)` (`:223-277`): pop to size 1 (or empty), Reset
  survivor **before** Finalize of popped (Finalize→AI MovementInform
  may push again). `ClearType` (`:895-908`) finalizes+erases mid-stack
  without Interrupt/Reset of neighbours — sharp edge.
- Named pushes (`:352-670`): MoveIdle, MoveRandom (creature only),
  MoveTargetedHome (clears first; pet-follow/stay/linked/transport
  branches), MoveChase (stops spline for creatures first), MoveFollow
  (clears), MovePoint, MoveConfused/Fleeing/Feared (Player vs Creature
  instantiations), MoveSeekAssistance(+Distract), MoveWaypoint /
  MoveWaypointAsDefault (inserts *under* current top, `:524-561`) /
  MoveCyclicWaypoint, MoveTaxiFlight ×2, MoveDistract, MoveJump
  (**no-op stub — "doesn't exist in 1.12"**, `:848-859`), MoveCharge,
  MoveDistance (LOS + z≤+10 checks, `:870-893`).
- `PropagateSpeedChange` calls `UnitSpeedChanged` on **every** stack
  entry (`:701-706`); waypoint helpers search top-down (`:708-724`).

Generator families (bodies/headers inspected as noted):
- **Idle/Distract/AssistanceDistract** — `IdleMovementGenerator.*`
  (65+75 lines); shared static instance.
- **Point/Distancing/Assistance/Effect/Charge** —
  `PointMovementGenerator.cpp:34-376` read: Initialize launches a
  spline via `MoveSplineInit` mapping MoveOptions→spline setters
  (`:41-57`); Update = `!movespline->Finalized()` with one-shot speed
  recalc re-Initialize (`:83-102`); Finalize→`AI::MovementInform(type,
  id)` (+ summoner chain for temp summons, `:114-139`); Effect gen is
  a spline-completion notifier (`:215-225`).
- **Chase/Follow** — `TargetedMovementGenerator.cpp` 913 lines
  (function map inspected; `_setTargetLocation:84`, `Update:248`,
  async pathfinding in `UpdateAsync:68`): repaths to moving target,
  back-off/spread behaviours (`:418-490`).
- **Waypoint** — see A7. **Cyclic** — `CyclicMovementGenerator.*`
  (51+118): closed-loop variant sharing waypoint path loading.
- **Random** (63+140), **Home** (47+89: evade-to-home),
  **Confused** (48+112), **Fleeing/TimedFleeing** (67+248),
  **Fear/TimedFear** (69+258): headers + MotionMaster push sites
  inspected; per-generator wander-point algorithms not line-read here
  (§f).

### A7. Waypoints

`WaypointMovementGenerator<Creature>` (609 lines, read `:1-500`) +
`FlightPathMovementGenerator` + `PatrolMovementGenerator` in the same
file. Path = `std::map<pointId, WaypointNode>` (ordered;
`WaypointManager.h:38-52`: x,y,z,orientation f32, delay u32 ms,
wander_distance f32, script_id u32, path_id u32). Origins
(`WaypointManager.h:30-36`): GUID → `creature_movement`, ENTRY →
`creature_movement_template` (key `entry<<8 | pathId`, pathId<0xFF),
SPECIAL → `creature_movement_special`; default = GUID else ENTRY
(`:76-92`).

Behaviour: `StartMove` (`WaypointMovementGenerator.cpp:181-239`):
advance to next node (wrap to begin if repeating; if not repeating,
set home to last node and `MotionMaster::Initialize`, `:200-207`);
if node has `path_id`, the *entire special sub-path* is launched as
one spline (`MovebyPath`, fly if CanFly), else `MoveTo` single node
with `MOVE_PATHFINDING` (or `MOVE_STRAIGHT_PATH` when the path itself
is SPECIAL-origin, `:232`); facing applied only if
`orientation != 100 && delay != 0` (100.0 = sentinel "no facing",
`:234-235`); walk unless UNIT_STATE_RUNNING/levitating.
`OnArrived` (`:125-179`): fire `creature_movement_scripts` script_id,
`AI::MovementInform(WAYPOINT,node)`, group-leader bookkeeping; delay
⇒ wait (ShortTimeTracker) or, with wander_distance, push
`MoveRandom(true, dist, delay)` on top (`:165-176`, generator nesting
in action). `Update` (`:241-287`): waits under CAN_NOT_MOVE/DISTRACTED
or while a no-movement spell is cast; if creature stopped externally,
`Stop(STOP_TIME_FOR_PLAYER=30 s)` (`:279`); spline finalized ⇒
OnArrived⇒StartMove. Always returns true (never self-expires).
`Reset` after wander flies back to last node at speed =
dist/(1000·0.001) i.e. 1000 ms duration (`:105-123` — literal
`distance / 1.0f` velocity quirk, preserve or consciously fix).

Flight paths (`:329-462`): nodes from DBC taxi data
(`TaxiPathNodeList`), speed **32.0** (`PLAYER_FLIGHT_SPEED`, `:384`),
CatmullRom (`SetFly`), map-transition node handling; Finalize unmounts
and clears taxi state. Patrol (`:466-609`): formation followers offset
from a leader's waypoint progress (`CreatureGroups`).

### A8. Speeds

`UnitMoveType` order fixed by `baseMoveSpeed[6]` (`Unit.cpp:66-73`):
WALK **2.5**, RUN **7.0**, RUN_BACK **4.5**, SWIM **4.722222**,
SWIM_BACK **2.5**, TURN_RATE **3.141594** (π rad/s). Per-unit
`m_speedRates[6]` init 1.0 (`Unit.cpp:155-156`);
`GetSpeed(t) = rate × base` (`:7229`) — wire always carries the flat
product, never the rate (except rates are what ACKs divide back out).

`UpdateSpeed` (`Unit.cpp:7087-7227`, read fully): pet-follow copies
owner rate (`:7090-7105`); RUN uses mounted vs foot aura sets,
max(stack, non-stack) bonus formula (`:7111-7147`); normalization cap
via SPELL_AURA_USE_NORMAL_MOVEMENT_SPEED (`:7156-7162`); ghost
multiplier from config (`:7170-7174`); strongest slow (`:7182-7187`);
creature base-rate multiplier — DB `creature_template.speed_walk/
speed_run` if non-zero, else display-addon DBC speeds, else defaults
WALK 1.0 / RUN **1.14286** (`Creature.cpp:1148-1192`,
`CreatureDefines.h:230-231`); player pets post-1.9 normalized to
1.14286 run (`Unit.cpp:7194-7209`); wounded slowdown ×0.7/0.6/0.5 at
15/10/5 % HP (`:7212-7220`, `CreatureDefines.h:226-228`). SWIM_BACK
returns early (never recomputed, `:7138-7139`).

`SetSpeedRate` (`:7270-7296`): player-moved+in-world ⇒ pending-change
to controller (rate **not** applied until ACK); not-in-world ⇒ applied
directly (documented hack vs retail, comment `:7290-7295`);
server-moved ⇒ applied + `SendSpeedChangeToAll`. `SetSpeedRateReal`
(`:7298-7303`) also propagates to all stack generators + controlled
units. `GetXZFlagBasedSpeed` (`:7233-7260`): flag→speed selection for
anticheat (0 if no XZ flag; swim/walk/back precedence).

## (b) Sizes / counts

| Item | Size / count |
|---|---|
| `vmangos/src/game/Movement/` (32 files incl. spline/) + MovementHandler + MovementInfo.h | **10,834 lines** total (`wc -l`) |
| — MotionMaster.cpp / Targeted / WaypointMgr+Gen / MovementPacketSender | 945 / 913 / 626+609 / 487 lines |
| — spline subdir (11 files) | 1,904 lines (MoveSpline 301, spline.cpp 293, util 204, packet_builder 176…) |
| MovementHandler.cpp | 1,201 lines, 11 WorldSession handlers + `Player::ExecuteTeleportNear` |
| MovementInfo wire size | 28 B base; 76 B max; +packGUID (1-9 B) per relayed packet |
| Client movement opcodes routed | 22 generic + 6 speed-ACK + 3 flag-ACK + 2 root-ACK + teleport/knockback/spline-done/worldport/set-active/not-active/time-skipped = **41** StoreOpcode entries (36 carry MovementInfo) |
| MovementFlags / MoveSplineFlag bits | 32 / 32 |
| Generator types | 19 named (0-19 excl. MAX_DB sentinel 4), 4 DB-selectable |
| Move types (speeds) | 6 |
| SMSG_MONSTER_MOVE / _TRANSPORT | opcode 221 / 686 (1.12.1 table) |
| Linear spline chunk cap | 80 points/packet default (config 5..10000) |
| Existing Rust port `mangos-movement` | 2,140 lines (spline 504 + move_spline 633 + packet_builder 510 + lib 26 + tests 467); **zero** MovementInfo/MotionMaster/generators/waypoint/speed code |

## (c) DB / config inputs

World DB (all loaded at startup by `WaypointManager::Load`,
`WaypointManager.cpp:33-376`, read `:1-420`):
- `creature_movement` (id=creature guid, point, position_x/y/z,
  waittime, wander_distance, script_id, orientation, path_id) →
  `m_pathMap`; points stored 0-based (`path[point-1]`, `:118`).
- `creature_movement_template` (entry,…) → `m_pathTemplateMap[entry<<8]`.
- `creature_movement_special` (id,…) → `m_pathSpecialMap` (sub-paths +
  straight-line paths).
- Validation with **DB write-back**: unknown script_id ⇒ row skipped
  (`:93-99` — note `continue` drops the whole node); unknown creature
  guid/entry ⇒ skipped; invalid coords ⇒ normalized and `UPDATE`d back
  to the DB (`:133-150`); `Cleanup()` renumbers non-contiguous points
  with DDL (`CREATE TEMPORARY TABLE…`, `:378-420+`). A Rust loader must
  decide whether to keep this self-healing (writes to world DB at boot).
- `creature_movement_scripts` (via `sCreatureMovementScripts`): ids
  referenced by nodes; unused scripts logged (`:374-375`).
- `creature_template.MovementType` (0-3 selects default generator),
  `speed_walk/speed_run` (0 ⇒ fallback chain, A8); `creature` spawn
  movement column feeds `GetDefaultMovementType` (selection plumbing
  in ObjectMgr/Creature, not opened — §f).
- DBC: `CreatureDisplayInfoAddon.speed_walk/speed_run` fallback
  (`Creature.cpp:1152-1189`); `TaxiPathNode` DBC-derived tables
  (`sTaxiPathNodesByPath`, used at `MotionMaster.cpp:609-612`); spell
  auras drive most speed modifiers (spell surface).

Config (`World.cpp` registrations / `mangosd.conf.dist.in`):
`Movement.MaxPointsPerPacket=80` (min 5 max 10000, `World.cpp:967`);
`Movement.PendingAckResponseTime=4000` ms (`:966` — ACK timeout
enforcement lives in Unit pending-change sweep, not opened, §f);
`Movement.ExtrapolateChargePosition/PetPosition=1`,
`Movement.RelocationVmapsCheckDelay=0` (conf `:331-336`);
`Death.Ghost.RunSpeed.World/Battleground=1.0` (`World.cpp:758-759`);
`Compression.Movement.Count=300` (conf `:260`, packet-compression
threshold); broadcaster threads/frequency (MovementBroadcaster ctor
args — registration site not opened, §f); ~60 `Anticheat.*` movement
thresholds/penalties (`World.h:275-331+`, anticheat surface).

## (d) Wire-visible semantics a Rust port must replicate

1. **Read/Write asymmetry**: inbound slot 2 = client `ctime`, outbound
   slot 2 = server `stime`; handlers overwrite parsed `stime` with
   *packet arrival time* before relay. Observers extrapolate from
   server time; `ctime==0` on stored info means server-side (pauses
   extrapolation, triggers `now+1000` fake in create blocks).
2. **Conditional-field order is fixed**: transport → pitch → fallTime
   → jump → splineElevation; jump order **z, cos, sin, xy** (struct
   declares sin/cos opposite — easy porting trap).
3. **Stateful parse**: destination struct is seeded from previous
   MovementInfo; jump-start latch and any flag-absent optional fields
   retain stale values and are used (knockback relay reads jump fields
   from the ACK's own parse; stored `m_movementInfo` keeps last
   transport/pitch values after flags drop).
4. **Relay identity**: same opcode out as in (generic path), mover
   packGUID prepended, originator excluded; FALL_RESET swallowed;
   SplineDone/NotActiveMover rewritten to HEARTBEAT/STOP by MASK_MOVING.
   Relay happens **after** relocation, so relayed flags include
   CorrectData stripping + server ROOT re-add — observers never see raw
   client flags.
5. **Pending-change ACK protocol**: every server-forced change on a
   player-moved unit is counter-tagged, not applied server-side until
   ACK, applied *before* the ACK's relocation (speeds), and only then
   forwarded to observers. Rates on wire are flat speeds; server stores
   rate = flat/base. Wrong/unsolicited ACKs feed anticheat.
6. **CorrectData + Verify are the only generic validation** in this
   surface: coordinate validity, transport box limits, exclusive-flag
   stripping. All speed/position cheat detection is delegated to
   `MovementAnticheat` (flag+position tests may set a reject-until
   timestamp that silently eats later packets).
7. **MONSTER_MOVE quirks**: Runmode bit always set; facing bits and
   Done stripped; Enter_Cycle faked; linear points are
   destination-relative 11-bit packed with anti-freeze z-nudge;
   CatmullRom points raw; duration field patched for chunks;
   continuation packets share the same splineId and re-send from the
   *original* first point; stop form is a different shape (pos+id+u8 1,
   no flags/duration/count).
8. **Create-block movement**: 6 flat speeds always; WriteCreate (raw
   flags, elapsed/duration, full padded path) only under
   SPLINE_ENABLED; creature ROOT hidden in create blocks.
9. **Spline timing**: durations are integer-ms cumulative segment
   times from velocity at launch; later speed changes do **not**
   retime an active spline except via generator re-Initialize
   (Point-gen one-shot recalc). Cyclic wrap uses modulo duration.
10. **Generator stack order effects are observable**: Mutate-expires
    (chase/home/distract/distancing) vs interrupts (others);
    Finalize→MovementInform can push new movement mid-expire; waypoint
    delay+wander nests a Random gen above the waypoint gen.
11. **Sentinels/constants**: waypoint orientation 100.0 = none; flight
    speed 32.0; jump initial speeds 7.95797334 / −9.645; gravity
    19.29110527038574; NPC run rate 1.14286; transport bounds
    250/250/100; undermap z<−500 / +100-below-height rules.
12. **Delivery**: player-originated movement may be queued through the
    threaded broadcaster (per-player FIFO, cross-player unordered)
    while teleports use the immediate path — a port must not "fix"
    this into a single ordered channel without checking client impact.

## (e) Smallest viable first slice + entry dependencies

**Slice M1 — MovementInfo codec + generic relay (no NPC movement).**
In `mangos-movement` (extend the existing spline-only crate):
`MovementInfo` struct + `read`/`write`/`correct_data` with bit-exact
golden tests (base + each conditional + all-combined = 28/52/32/44/32/
76 B vectors), `MovementFlags` bitflags + masks, and
`VerifyMovementInfo` equivalent. Behavioural tests must cover: cos/
sin order, stime-in-ctime-slot on write, jump latch across a seeded
parse sequence, CorrectData pair stripping.

Entry dependencies (must exist in the Rust world loop first):
ByteBuffer-equivalent cursor with the same underflow behaviour;
ObjectGuid raw-u64 + packGUID codecs; a monotonic ms clock
(`WorldTimer::getMSTime` analogue) and per-packet arrival time;
`IsValidMapCoord` bounds (map-visibility surface); Unit/Player mover
resolution + `m_movementInfo` storage (object surface); session
`m_moveRejectTime`/`m_clientMoverGuid` state. Anticheat can be a
trait stub returning 0 for M1 **only** if the reject-time plumbing is
already wired (it changes drop semantics).

**Slice M2 — pending-change ACK loop + speeds.** `baseMoveSpeed[6]`,
`speedRates`, `UpdateSpeed` aura inputs reduced to an interface
(spell surface supplies modifiers), `MovementPacketSender` 3-step
flows, the 12 ACK handlers. Depends on: aura modifier queries,
creature template/DBC speed lookup.

**Slice M3 — spline launch path (NPCs move).** `MoveSplineInit::Launch`
port on top of the existing Rust `MoveSpline`/`packet_builder`
(already at parity per crate tests — not re-audited here), plus
MotionMaster enum-stack with Idle+Point+Waypoint generators,
`WaypointManager` loader for the three `creature_movement*` tables
(skip DB write-back initially, log instead), `UpdateSplineMovement`
tick + create-block embedding. Depends on: PathFinder (detour surface)
for `MOVE_PATHFINDING`, map relocation/grid, Script/AI MovementInform
callbacks (AI surface). Chase/Follow/fear-family and taxi flights are
M4 (combat/script coupling).

## (f) Unresolved / not inspected

- `MovementAnticheat` test internals (flag/position/spline-done
  bodies; only the header interface at
  `Anticheat/MovementAnticheat/MovementAnticheat.h:86-97` verified) —
  belongs to the anticheat surface; M1 stub risk noted in (e).
- Generator algorithm bodies not line-read: Targeted (chase/follow),
  Random, Home, Confused, Fleeing, Fear, Cyclic, Idle, Patrol tail
  (`WaypointMovementGenerator.cpp:500-609`). Stack-level semantics in
  A6 are verified; per-generator wander/evade point selection is not.
- `Unit::StopMoving` tail (after `Unit.cpp:9153`), pending-change
  storage/timeout sweep (`FindPendingMovement*` bodies,
  `Movement.PendingAckResponseTime` enforcement), `Player::HandleFall`
  damage formula, `SendMovementPacket`/heartbeat generation
  (`Unit.cpp:486-498` located, not opened).
- Default-generator selection plumbing (`FactorySelector::
  selectMovementGenerator`, `creature(_template).MovementType` load)
  and broadcaster config registration (thread count / frequency
  source) not opened.
- Rust `mangos-movement` parity tests (`spline_parity.rs`,
  `spline_oracle.rs`) not re-run or re-audited in this pass; A5 claims
  are C++-side only.
- `Spline<T>` curve math (`spline/spline.cpp/.h`, 509 lines) treated
  as black box behind MoveSpline; CatmullRom coefficient parity is a
  Phase-2 claim, not re-verified here.

### Divergence vs production binary `2bae5f9` (inspected)

`git diff 2bae5f9..HEAD` over this surface touches 49 files
(3383+/3584-), but spot-verification shows it is formatting churn:
for `MovementInfo::Read/Write/CorrectData` the whitespace-stripped
token streams at `2bae5f9` and HEAD are **identical** except line
wrapping (37 diff lines, all macro-continuation reflows), and the
`MovementInfo.h` ignore-whitespace diff contains only include order
and brace reflow. The full 414+/527- ignore-whitespace remainder was
**not** line-audited, so no claim is made beyond the wire codec:
**no inspected wire-visible divergence between 2bae5f9 and HEAD**;
generator-behaviour equivalence remains unverified.
