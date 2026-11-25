# Phase 4 inventory — Entity model + update fields

Oracle: C++ at HEAD `1bec69f` (branch `rust-migration`), primary tree
`vmangos/src/game/Objects/` + `vmangos/src/game/ObjectGuid.h`.
Production binary `2bae5f9`: `git diff -w 2bae5f9 HEAD` on this surface was
inspected; `UpdateFields_1_12_1.cpp` differs only in brace formatting
(324-row table identical) and no create/values logic lines in `Object.cpp`
matched the divergence grep. No other prod-vs-HEAD divergence is claimed;
a full line-by-line `Object.cpp/Object.h/ObjectGuid.h` audit was not done
(see §f). All bodies cited below were opened with `read_file`; search was
used only to locate candidates.

## (a) Verified structure / flow

### A.1 Class hierarchy (single rooted tree, no ECS)

```
Object                                  Object.h:111
├── Item : Object                       Item.h:38 (class at Item.h, 162 lines)
│   └── Bag : Item                      Bag.h:33
└── WorldObject : Object                Object.h:453
    ├── DynamicObject : WorldObject     DynamicObject.h:37
    ├── Corpse : WorldObject            Corpse.h:53
    └── SpellCaster : WorldObject       SpellCaster.h:315
        ├── GameObject : SpellCaster    GameObject.h:41
        └── Unit : SpellCaster          Unit.h:321 (abstract: pure virtuals Unit.h:462-471,620,991,1008,1404-1406)
            ├── Creature : Unit         Creature.h:80
            │   ├── Pet : Creature      Pet.h:142
            │   ├── Totem : Creature    Totem.h:33
            │   └── TemporarySummon : Creature  TemporarySummon.h:27
            │       └── TemporarySummonWaypoint  TemporarySummon.h:52
            └── Player final : Unit     Player.h:746
```

- `Object` owns identity, the values array, and all wire-update building;
  copy is deleted (`Object.h:447-448`). `WorldObject` adds position
  (`Position m_position`, `Object.h:815`), map/instance ids
  (`Object.h:812-814`), `MovementInfo m_movementInfo` + transport
  (`Object.h:655-656`), visibility/viewpoint, and the client-update-list
  overrides (`Object.h:746-748`).
- `SpellCaster` is the shared Unit/GameObject spell engine: cooldown maps
  `m_GCDCatMap/m_lockoutMap/m_cooldownMap` (`SpellCaster.h:437-439`),
  current spells array (`SpellCaster.h:445`), dynobject list
  (`SpellCaster.h:441`), `EventProcessor m_Events` (`SpellCaster.h:429`).
  `Object::IsSpellCaster()` = `IsUnit() || IsGameObject()` (`Object.h:394`).
- Creature subtypes are discriminated twice: C++ subclass **and**
  `CreatureSubtype m_subtype` (`Creature.h:642`; enum
  `CreatureDefines.h:655-660`: GENERIC/PET/TOTEM/TEMPORARY_SUMMON).
  `Creature::IsCorpse()` is death-state, not TypeID (`Creature.h:152`),
  while `Object::IsCorpse()` is TypeID (`Object.h:398`) — a port must keep
  both meanings distinct.
- Downcasts are unchecked `static_cast` gated by TypeId/TypeMask
  (`Object.h:373-404`, `822-829`; per-class inlines e.g.
  `Creature.h:689-695`, `Player.h:2554-2560`). Rust equivalent: an enum or
  trait-object downcast table keyed by `TypeId`+subtype, not RTTI.

### A.2 TypeID / TypeMask / GUID highs

- `TypeID` 0..7: OBJECT, ITEM, CONTAINER, UNIT, PLAYER, GAMEOBJECT,
  DYNAMICOBJECT, CORPSE (`ObjectGuid.h:33-43`); `MAX_TYPE_ID 8`
  (`ObjectGuid.h:45`).
- `TypeMask` bits: OBJECT 0x01, ITEM 0x02, CONTAINER 0x04, UNIT 0x08
  (players also carry it), PLAYER 0x10, GAMEOBJECT 0x20,
  DYNAMICOBJECT 0x40, CORPSE 0x80; `TYPEMASK_WORLDOBJECT` =
  UNIT|PLAYER|GAMEOBJECT|DYNAMICOBJECT|CORPSE (`ObjectGuid.h:47-64`).
  Stored per object as `uint16 m_objectType` + `uint8 m_objectTypeId`
  (`Object.h:420-421`); `OBJECT_FIELD_TYPE` on the wire is the mask
  (`Object.cpp:230`).
- `HighGuid` (bits 63..48): ITEM/CONTAINER `0x4000`, PLAYER `0x0000`,
  GAMEOBJECT `0xF110`, TRANSPORT `0xF120`, UNIT `0xF130`, PET `0xF140`,
  DYNAMICOBJECT `0xF100`, CORPSE `0xF101`, MO_TRANSPORT `0x1FC0`
  (`ObjectGuid.h:66-78`).
- GUID layout: `counter | entry<<24 | high<<48`; zero counter ⇒ whole GUID
  0 (`ObjectGuid.h:129-130`). Entry-bearing highs (GAMEOBJECT, TRANSPORT,
  UNIT, PET; default also true) use 24-bit counter + 24-bit entry; others
  use full 32-bit counter (`ObjectGuid.h:153-158,216-235`).
  Max counter `0xFFFFFF` vs `0xFFFFFFFF` (`ObjectGuid.h:158`).
- `ObjectGuid::GetTypeId(HighGuid)`: ITEM→ITEM (CONTAINER unreachable,
  same high), UNIT/PET→UNIT, PLAYER→PLAYER, GAMEOBJECT/MO_TRANSPORT→
  GAMEOBJECT, DYNAMICOBJECT, CORPSE; **TRANSPORT has no arm → OBJECT**
  (`ObjectGuid.h:176-201`). Rust `mangos-guid` already preserves this
  quirk (`vmangos-rs/crates/mangos-guid/src/lib.rs:91-102`, 549 lines).
- Packed GUID: mask byte + present bytes, min buffer 9
  (`ObjectGuid.h:250-279`); cached per object as `PackedGuid m_PackGUID`,
  set once in `_Create` (`Object.h:443`, `Object.cpp:231`).
- Generators: `ObjectGuidGenerator<high>` with freed-guid queue, plus a
  mutex-guarded `ObjectSafeGuidGenerator` (`ObjectGuid.h:281-326`);
  seeded from DB max (`LoadFromDB/SetMaxUsedGuid`, `ObjectGuid.h:288-289`).

### A.3 The values array (`m_uint32Values`) — the entity's wire state

- One heap array per object, viewed through a union
  `int32*/uint32*/float*` (`Object.h:424-429`), plus a same-size
  `m_uint32Values_mirror` shadow (`Object.h:431`) and
  `uint16 m_valuesCount` (`Object.h:433`). Allocated zeroed, together,
  in `_InitValues()` (`Object.cpp:212-221`); freed in `~Object`
  (`Object.cpp:206-209`). The mirror is "last broadcast" state:
  dirty = `mirror[i] != values[i]`.
- `m_valuesCount` is set in each leaf constructor to the build's `*_END`:
  Item→`ITEM_END` (`Item.cpp:190`), Bag→`CONTAINER_END` (`Bag.cpp:31`),
  Creature→`UNIT_END` (`Creature.cpp:199`), Player→`PLAYER_END`
  (`Player.cpp:116`), GameObject→`GAMEOBJECT_END` (`GameObject.cpp:69`),
  DynamicObject→`DYNAMICOBJECT_END` (`DynamicObject.cpp:37`),
  Corpse→`CORPSE_END` (`Corpse.cpp:41`). Unit itself sets no count
  (abstract); Object base sets 0 (`Object.cpp:182`).
- Constructors also stamp type/mask/update-flag:
  Object `TYPEID_OBJECT/TYPEMASK_OBJECT` (`Object.cpp:177-178`);
  Unit adds `TYPEMASK_UNIT/TYPEID_UNIT`, update flag
  `ALL|LIVING|HAS_POSITION` (`Unit.cpp:80-83`); Player adds
  `TYPEMASK_PLAYER/TYPEID_PLAYER` (`Player.cpp:113-114`);
  GameObject/DynamicObject/Corpse `ALL|HAS_POSITION`
  (`GameObject.cpp:64-69`, `DynamicObject.cpp:32-37`, `Corpse.cpp:34-41`;
  Corpse pre-1.8.4 = 0); Item `ALL` only (`Item.cpp:185-190`); Bag
  `ITEM|CONTAINER` mask, `TYPEID_CONTAINER` (`Bag.cpp:28-31`).
  Flags only exist for `> CLIENT_BUILD_1_8_4` (same anchors).
- `_Create(guidlow, entry, high)` lazily inits values, writes field 0..1
  GUID and field 2 TYPE (`Object.cpp:223-232`). Note it does **not** write
  `OBJECT_FIELD_ENTRY`; each `Create()` does (`SetEntry`, e.g.
  `Item.cpp:203`, `GameObject.cpp` Create body). `WorldObject::_Create`
  is the 2-arg variant used by DynamicObject/Corpse
  (`Object.h:489`; `DynamicObject.cpp:63`, `Corpse.cpp:87`).

### A.4 Field accessors and dirty marking

- Typed access is raw reinterpretation of the word array: `uint64` spans
  two words (`Object.h:190-194`), bytes/shorts address sub-word lanes
  (`Object.h:202-221`), GUID = reinterpreted `uint64` (`Object.h:223`).
  Bounds are `MANGOS_ASSERT(index < m_valuesCount)` (`Object.h:178-200`).
- Every setter writes only on change, then calls `MarkForClientUpdate()`:
  Int32/UInt32/Float/Byte/UInt16 (`Object.cpp:1090-1183`), flags
  (`Object.cpp:1233-1313`). `SetUInt64Value` additionally poisons both
  mirror words (`mirror = value+1`) to force both halves onto the wire,
  with a long comment on client half-GUID bugs (`Object.cpp:1112-1136`).
  `ForceValuesUpdateAtIndex` does the same poison for one index
  (body after `Object.cpp:2779` region; declared `Object.h:241`).
- `MarkForClientUpdate()`: no-op if not in world; Item/Container enqueue
  immediately, everything else sets a delayed action bit
  (`Object.cpp:1355` region; enum `Object.h:102-107`).
  `ExecuteDelayedActions` promotes to `AddToClientUpdateList()` +
  `m_objectUpdated=true`. `WorldObject` list ops go to the map
  (`AddUpdateObject/RemoveUpdateObject`, `Object.cpp` ~2747-2750);
  base `Object::AddToClientUpdateList/BuildUpdateData` assert-fail
  (`Object.cpp:1337-1353`) — only WorldObject and Item are legal
  (comment `Object.h:155`).
- `ClearUpdateMask(remove)` = `memcpy(mirror, values)` + dequeue
  (`Object.cpp:942-954`). Called on `AddToWorld` (sync, `Object.h:118-127`),
  `RemoveFromWorld` (`Object.h:128-133`), and after fan-out in
  `WorldObject::BuildUpdateData` (`Object.cpp:2779` region).
  **Consequence:** the mirror is global, not per-viewer. Per-viewer
  filtering happens only via field flags at build time; a field consumed
  for viewer A is clean for viewer B in the same tick. New viewers get a
  full CREATE block instead (§A.6). A Rust port must replicate this
  single-mirror semantics or visibility/login bursts will diverge.
- Value helpers built on top: `ApplyMod*`, `ApplyPercentModFloatValue`
  (clamps −100% to −99.9%, `Object.h:244-248`), stat clamp setters
  (`Object.cpp:1185-1199`), flag/byte/short/64-bit flag ops
  (`Object.h:250-360`). `LoadValues` parses a space-separated string of
  exactly `m_valuesCount` tokens (`Object.cpp:956-972`);
  `_LoadIntoDataField` loads a slice and poisons its mirror (+1) so it
  will broadcast (`Object.cpp:1035-1050`).

### A.5 UpdateField tables: selection, shape, flags

- **Compile-time, single-build selection.** `UpdateFields.h:27-49` and
  `UpdateFields.cpp:22-44` `#include` exactly one
  `UpdateFields_<v>.{h,cpp}` by `SUPPORTED_CLIENT_BUILD >= CLIENT_BUILD_*`
  cascade (1.12.1 → 1.11.2 → … → 1.2.4 fallback). The build constants and
  the `@supported_build@` substitution live in
  `vmangos/cmake/generators/Progression.h.in:24-36` (4222, 4297, 4375,
  4449, 4544, 4695, 4878, 5086, 5302, 5464, 5875); CMake default is
  `CLIENT_BUILD_1_12_1` (`vmangos/cmake/options.cmake:11`). The same
  header defines accepted client builds per target (1.12.1 accepts
  {5875, 6005, 6141}, `Progression.h.in:39-40`) and `MAX_CONTENT_PATCH`
  (10 for 1.12, `Progression.h.in:79-80`). There is **no runtime
  per-session field translation**: one binary serves one field layout.
  A Rust port targeting 1.12.1 only needs the 5875 table; multi-build
  support would need all 11 tables compiled in and selected per build.
- Each variant header is auto-generated enums with chained bases
  (`ITEM_* = OBJECT_END+…`, `PLAYER_* = UNIT_END+…`); the matching `.cpp`
  holds `g_updateFieldsData`: rows of
  `{objectTypeMask, name, offset, size, valueType, flags}`
  (`UpdateFields.h:136-146`; 1.12.1 table head
  `UpdateFields_1_12_1.cpp:5-12`). Value types: NONE/INT/TWO_SHORT/
  FLOAT/GUID/BYTES/BYTES2 (`UpdateFields.h:111-120`). Flags: PUBLIC 0x001,
  PRIVATE 0x002, OWNER_ONLY 0x004, UNK1 0x008, UNK2 0x010,
  SPECIAL_INFO 0x020, GROUP_ONLY 0x040, UNK5 0x080, DYNAMIC 0x100
  (`UpdateFields.h:122-134`).
- At static init, rows are expanded into per-type **flags arrays** indexed
  by field offset: container (OBJECT|ITEM|CONTAINER), player
  (OBJECT|UNIT|PLAYER), gameobject, dynamicobject, corpse
  (`UpdateFields.cpp:46-67`). `GetUpdateFieldFlagsArray(TypeId)` maps
  ITEM+CONTAINER→container array and **UNIT+PLAYER→player array**
  (`UpdateFields.cpp:69-98`); unhandled TypeId logs and returns null.
  Creatures therefore index a PLAYER_END-sized array but loop only to
  `UNIT_END` — safe, but the port should model flags per *field*, not
  per class.
- Script/DB compatibility shim: DB scripts address fields by 5875 index;
  `GetIndexOfUpdateFieldForCurrentBuild` remaps 14 known indices
  (`UpdateFields.h:51-109`).
- **Header-comment trap (verified):** in `UpdateFields_1_12_1.h` the
  trailing `// 0x…` absolute-offset comments for the ITEM-derived
  CONTAINER block and UNIT-derived PLAYER block understate by 6
  (e.g. `CONTAINER_FIELD_NUM_SLOTS` commented `0x02A`, real `0x30`;
  `PLAYER_END` commented `0x4FC`, real `0x502`). The enum arithmetic and
  the `.cpp` offsets agree (`UpdateFields_1_12_1.cpp:30,111,292-293`:
  CONTAINER_FIELD_NUM_SLOTS 0x30, PLAYER_DUEL_ARBITER 0xBC,
  PLAYER_END 0x502). Generate Rust tables from the `.cpp` rows, never
  from header comments.

### A.6 Building wire updates (create / values / movement / destroy)

Opcodes (1.12.1): `SMSG_UPDATE_OBJECT=169`, `SMSG_DESTROY_OBJECT=170`,
`SMSG_COMPRESSED_UPDATE_OBJECT=502`
(`Server/Protocol/Opcodes_1_12_1.h:172-173,505`).
Block types: VALUES 0, MOVEMENT 1, CREATE_OBJECT 2, CREATE_OBJECT2 3
(only >1.8.4), OUT_OF_RANGE, NEAR (`UpdateData.h:32-42`).
Create flags: SELF 0x01, TRANSPORT 0x02, MELEE_ATTACKING 0x04,
HIGHGUID 0x08, ALL 0x10, LIVING 0x20, HAS_POSITION 0x40
(`UpdateData.h:45-57`, last five only >1.8.4).

- `BuildCreateUpdateBlockForPlayer` (`Object.cpp:260-320`): picks
  CREATE_OBJECT vs CREATE_OBJECT2 (`m_isNewObject`, >1.8.4), adds SELF if
  `target==this`, MELEE_ATTACKING if unit is melee-attacking a victim;
  writes `uint8 type, PackedGuid (raw uint64 pre-1.9), uint8 TypeId`,
  then `BuildMovementUpdate`, then values with a create mask.
  Pre-1.9 also writes flags/attack-cycle/timer/victim/transport words
  (`Object.cpp:295-314`).
- Create mask `_SetCreateBits`: bit set iff `values[i]!=0 && visible`
  (`Object.cpp:1077-1088`). Zero-valued public fields are **omitted**
  (client defaults them) — except forced bits in `BuildValuesUpdate`
  (GO dyn-flags always; UNIT_DYNAMIC_FLAGS for debug-target cheat;
  ITEM_FIELD_FLAGS pre-1.7) (`Object.cpp:590-611`).
- Update mask `_SetUpdateBits`: bit set iff `mirror[i]!=values[i] &&
  visible` (`Object.cpp:1064-1075`).
  `BuildValuesUpdateBlockForPlayer` emits nothing if the mask is empty
  (`Object.cpp:394-402`); the flags variant selects by flag instead of
  dirtiness (`Object.cpp:404-411`, `1052-1062`).
- Visibility `GetUpdateFieldFlagsForTarget` (`Object.cpp:974-1033`):
  base = PUBLIC|DYNAMIC; +PRIVATE if target is self; Item/Container
  owner gets OWNER_ONLY|UNK2; Unit/Player: debug-target cheat gets
  OWNER_ONLY|SPECIAL_INFO|GROUP_ONLY, else owner/charmer gets
  OWNER_ONLY, SPECIAL_INFO only if dynflag set *and*
  `CanSeeSpecialInfoOf`, GROUP_ONLY if same raid as owner-or-self.
  GameObject/DynamicObject/Corpse owner arms are commented out
  (no owner-only fields) (`Object.cpp:1012-1029`).
- `BuildMovementUpdate` (>1.8.4 path, `Object.cpp:447-539`): `uint8
  flags`; if LIVING: full `MovementInfo` (synthesized from position with
  `stime=now+1000` if `ctime==0`; creatures strip `MOVEFLAG_ROOT`), then
  6 speeds (walk/run/run-back/swim/swim-back/turn), then spline create
  if spline-enabled; else if HAS_POSITION: 4 floats (stationary pos for
  transports, world pos otherwise); then HIGHGUID→`uint32 0`,
  ALL→`uint32 1`, MELEE_ATTACKING→victim PackedGuid (or `uint8 0`),
  TRANSPORT→path progress or server mstime.
- Fan-out: `WorldObject::BuildUpdateData` visits cameras in visibility
  range; players always get their own block first (camera may be far),
  others only if `IsInVisibleList_Unsafe`, then `ClearUpdateMask(false)`
  (`Object.cpp:2752-2785` region). `SendForcedObjectUpdate` flushes one
  object immediately (`Object.cpp:236-248`).
  `DirectSendPublicValueUpdate` bypasses the queue: builds a VALUES
  block for explicit indices, updates the mirror inline, broadcasts to
  the object set (`Object.cpp:331-392`).
- Overrides that prepend child creates: `Player::` emits owned Item
  creates first (equipment, bags→bank-bags, buyback >1.7.1, keyring)
  **only when target==self**, then the Unit block (`Player.cpp:4199-4236`);
  `Bag::` emits itself then each contained item (`Bag.cpp:160-167`).
  Item's own create is the base Object one.
- Destroy/out-of-range: `BuildOutOfRangeUpdateBlock` collects GUIDs into
  the packet's out-of-range section (`Object.cpp:427`);
  `DestroyForPlayer` sends standalone `SMSG_DESTROY_OBJECT` with the
  full (unpacked) GUID (`Object.cpp:438-445`);
  `SendOutOfRangeUpdateToPlayer` builds a one-off packet
  (`Object.cpp:429-436`). Movement-only blocks:
  `BuildMovementUpdateBlock` = MOVEMENT type + **unpacked** `ObjectGuid`
  + movement (`Object.cpp:250-258`) — note the GUID encoding differs
  from create/values (packed >1.8.4).
- Packet assembly `UpdateData` (`UpdateData.cpp:43-194`): blocks append
  into `UpdatePacket`s split at `MAX_UNCOMPRESSED_PACKET_SIZE 0x8000`
  checked *before* each append (a block may exceed it); header =
  `uint32 blockCount (+1 if out-of-range present), uint8 hasTransport`;
  out-of-range section = type byte + `uint32 count` + PackedGuids
  (>1.8.4, else raw GUIDs); zlib-deflate to
  `SMSG_COMPRESSED_UPDATE_OBJECT` (prefixed `uint32` raw size) iff size
  > `Compression.Update.Size` (default 128), level
  `Compression.Level` (default 1, 1..9) (`World.cpp:557-559`);
  ≥900000 bytes logs `[CRASH-CLIENT]`. `Send` clears out-of-range GUIDs
  after the first sub-packet (`UpdateData.cpp:177-194`).
  `MovementData` (>1.7.1) batches movement packets into
  `SMSG_COMPRESSED_MOVES` with `uint8` per-packet length
  (`UpdateData.cpp:202-243`).
- `UpdateMask`: bitset of `ceil(count/32)` uint32 blocks; wire form =
  `uint8 blockCount` + blocks (`UpdateMask.h:51-60`;
  written at `Object.cpp:633-634`). Copy-assign reallocates
  (`UpdateMask.h:69-75`) — allocation-heavy; Rust should use a fixed
  bitset/`SmallVec`.

### A.7 Per-viewer value rewriting inside `BuildValuesUpdate`

The serializer does **not** copy words verbatim for units/GObjects/corpses
(`Object.cpp:576-940`). A Rust port needs the same target-dependent
rewrites, in this priority order for `TYPEMASK_UNIT`:

1. `UNIT_NPC_FLAGS`: strip TRAINER if not trainer-of-target, STABLEMASTER
   if target not hunter, FLIGHTMASTER if target can see/reward a quest
   from this creature (`Object.cpp:643-691`).
2. `UNIT_FIELD_BASEATTACKTIME..RANGEDATTACKTIME`: stored float, sent as
   `uint32(max(f,0))` (`Object.cpp:693-697`).
3. Pos/neg stat and resist-buff ranges: stored float, sent `uint32(f)`
   (`Object.cpp:700-701`).
4. `UNIT_FIELD_FLAGS` to GMs: force `AURAS_VISIBLE`, clear
   `NOT_SELECTABLE` (`Object.cpp:703-704`).
5. `UNIT_DYNAMIC_FLAGS`: TRACK_UNIT only if target stalks it; TAPPED /
   TAPPED_BY_PLAYER from loot recipient; LOOTABLE only if
   `IsAllowedToLoot`; SPECIALINFO for debug cheat (`Object.cpp:706-742`).
6. `UNIT_FIELD_FACTIONTEMPLATE`: cross-faction raid ⇒ target's faction
   (`Object.cpp:744-764`); `PLAYER_FLAGS` FFA_PVP cleared for raidmates
   (`Object.cpp:766-773`) — this fork's inter-faction feature.
7. Health hiding (default on, §c): if target cannot see health, MAXHEALTH
   sent as 100, HEALTH as clamped percent (0 stays 0, nonzero rounds to
   at least 1) (`Object.cpp:775-797`).
8. `UNIT_FIELD_TARGET` (both words): creature `m_castingTargetGuid`
   overrides so the client faces the cast target; uses `continue` to
   emit one word per loop iteration (`Object.cpp:805-828`,
   `Creature.h:426-447`).
9. `UNIT_MOD_CAST_SPEED` (>1.11.2): values <0.001 sent as 0.0
   (`Object.cpp:829-837`). Type also flips INT→FLOAT across builds
   (§b). Otherwise raw word (`Object.cpp:838-842`).

GameObject: `GAMEOBJECT_DYN_FLAGS` is never the stored value — it is
`uint16 GO_DYNFLAG_LO_ACTIVATE + uint16 0` for questgiver/chest/generic/
spell-focus/goober when `ActivateToQuest(target)||GM`, else `uint32 0`
(`Object.cpp:853-874`); the bit (and on VALUES ≥1.12.1 also
`GAMEOBJECT_ANIMPROGRESS`) is force-set every block
(`Object.cpp:591-597,614-623`), and the per-target decision is cached in
`target->m_visibleGobjQuestActivated` under a mutex
(`Object.cpp:625-629`, `Player.h:1788-1789`).
Corpse: `CORPSE_FIELD_DYNAMIC_FLAGS` LOOTABLE cleared if looted or
friendly to target (`Object.cpp:886-898`). Item (≤1.6.1 only):
`ITEM_FIELD_FLAGS` = `uint16 proto.Flags + uint16 stored`
(`Object.cpp:906-927`). All other types/fields: raw word copy
(`Object.cpp:929-939`).

### A.8 Entity construction flows (what `Create` minimally writes)

- **Creature** (`Creature.cpp:1225-1269,1791-1799`): `SetMap` →
  `CreateFromProto` (`Object::_Create(guidlow, firstId, HIGHGUID_UNIT)` +
  `UpdateEntry`) → position select/relocate → corpse-delay by rank from
  config → linking-holder registration → `LoadCreatureAddon`.
  `UpdateEntry` (`Creature.cpp:473…`): `InitEntry`, sheath/bytes-2,
  `SelectLevel`, resistances and attack times from `CreatureInfo`.
  DB spawn path `LoadFromDB` (`Creature.cpp:1801…`): needs `creature`
  row (`CreatureData`) + `creature_template` (`CreatureInfo`), honours
  SPAWN_FLAG_DISABLED/DEAD/ACTIVE, respawn time from persistent state,
  health/mana percents. Template structs: `CreatureInfo`
  (`CreatureDefines.h:234-…`, ~65 fields incl. 4 display candidates,
  `CreatureDefines.h:221,244-246`), `CreatureData`
  (`CreatureDefines.h:370-…`, up to 5 candidate entries,
  `CreatureDefines.h:367`).
- **Player (new/bot)** `Create` (`Player.cpp:399-…`): `_Create(…,0,
  HIGHGUID_PLAYER)`, race/class `PlayerInfo` + ChrClasses DBC lookup,
  clears 19 visible/inventory slots, relocate to start pos, faction,
  BYTES_0 race/class/gender/powertype, `InitPlayerDisplayIds`, flags
  `UNIT_FLAG_PLAYER_CONTROLLED`, cast-speed init (float 1.0 >1.11.2 else
  int 0), appearance bytes, level/money from config,
  `InitStatsForLevel/InitTaxiNodes/InitTalentForLevel/
  InitPrimaryProfessions`, reputation load, max health/power, default
  spells. Sets `m_saveDisabled=true` (bots only; real chars use
  `SaveNewPlayer` + `LoadFromDB`).
- **Player (login)** `LoadFromDB(guid, holder)` (`Player.cpp:14470…`):
  consumes 22 pre-fetched queries (`Player.h:610-635`:
  LOADFROM/GROUP/BOUNDINSTANCES/AURAS/SPELLS/QUESTSTATUS/HONORCP/
  REPUTATION/INVENTORY/ITEMLOOT/ACTIONS/SOCIALLIST/HOMEBIND/
  SPELLCOOLDOWNS/GUILD/BGDATA/ACCOUNTDATA/SKILLS/MAILS/MAILEDITEMS/
  BATTLEGROUND_DATA/FORGOTTEN_SKILLS). The `characters` row is 58
  columns (comment `Player.cpp:14471-14481`): identity/appearance,
  position+transport, taxi mask/path, honor fields, health/power1-5,
  `explored_zones` blob (64 words → `PLAYER_EXPLORED_ZONES_1`,
  `Player.h:70`, loaded `Player.cpp:14544`), ammo, action-bar byte,
  phase mask, create time, instance. There is **no whole-values blob**:
  most update fields are recomputed (stats/items/auras) at login.
- **GameObject** (`GameObject.cpp:173-…`): relocate+stationary pos,
  template lookup or fail, `_Create` with `HIGHGUID_TRANSPORT` if type
  TRANSPORT else `HIGHGUID_GAMEOBJECT`, scale, pos/facing/rotation
  floats, faction/flags/entry/display/state/type/animprogress; transport
  adds update flag + level=pause. Templates: `GameObjectInfo`
  (`GameObjectDefines.h:211-…`, type-tagged union), spawn row
  `GameObjectData` (`GameObjectDefines.h:910-927`).
- **DynamicObject** (`DynamicObject.cpp:61-…`): `WorldObject::_Create`,
  caster's map, entry=spellId, caster GUID, bytes=type (spell 1543 flare
  hardcodes 0x10 and doubles radius), spell/radius/pos fields; requires
  the `SpellEntry` to exist; farsight focus is active object.
- **Corpse** (`Corpse.cpp:74-107`): bones form writes only the GUID;
  player form copies owner pos/map, scale, POS/FACING floats, owner GUID.
  Display/items/bytes/flags are filled later by `Player::CreateCorpse`
  (declared `Player.h:2121`; body not read — §f).
- **Item/Bag** (`Item.cpp:200-…`, `Bag.cpp:60-…`): `_Create` HIGHGUID_ITEM
  / HIGHGUID_CONTAINER (same 0x4000), entry, scale 1.0, owner GUID,
  contained (empty for Item, **owner** for Bag), stack=1, durability from
  prototype, spell charges, duration; Bag adds NUM_SLOTS and zeroes
  `MAX_BAG_SIZE` slot GUIDs. Both require an `ItemPrototype`.
  `NewItemOrBag` picks Bag iff `InventoryType==INVTYPE_BAG` (`Bag.h:72-78`).

## (b) Sizes / counts

### B.1 Source sizes (`wc -l`, this checkout)

| File | Lines | File | Lines |
|---|---|---|---|
| Object.h / .cpp | 831 / 3674 | Player.h / .cpp | 2565 / 22443 |
| Unit.h / .cpp | 1535 / 11073 | Creature.h / .cpp | 746 / 4118 |
| SpellCaster.h / .cpp | 473 / 2387 | GameObject.h / .cpp | 320 / 2596 |
| Item.h / .cpp | 162 / 1289 | Pet.h / .cpp | 308 / 2351 |
| Corpse.h / .cpp | 112 / 274 | DynamicObject.h / .cpp | 91 / 302 |
| Bag.h / .cpp | 80 / 216 | Totem + TemporarySummon (.h+.cpp) | 64+216 / 67+298 |
| ObjectGuid.h / .cpp | 336 / 217 | UpdateData.h/.cpp + UpdateMask.h | 112/243 + 119 |
| UpdateFields.h/.cpp | 155 / 121 | 11 variant pairs (.h+.cpp) | 258–352 + 244–338 each |

Objects directory total: 71,517 lines (headers + cpps incl. defines).
Player alone is 35% of it (25,008 lines); Unit+Player+Creature cpps are
37,634 lines. Any Rust estimate must treat Player.cpp (22,443) as many
modules, not one (§ Player clusters below).

Declared `m_*` data members per header (regex count of `m_name;`/`m_name[`):
Player.h 150, Unit.h 91, Creature.h 61, Object.h 29, Pet.h 23,
GameObject.h 21, SpellCaster.h 16, DynamicObject.h 8, Corpse.h 5,
Item.h 3 (+ `loot,uState,uQueuePos,mb_in_trade,generatedLoot` un-prefixed),
Bag.h 1. A live Player therefore carries ~150+91+16+29 ≈ 286 declared
members through its bases (plus containers' heap payloads), before the
values arrays.

### B.2 Field counts per object type — all 11 variants

True `*_END` = word count = `m_valuesCount`. Parsed from the `.cpp`
`g_updateFieldsData` END rows (authoritative; see §A.5 trap) and
cross-checked against enum arithmetic at 1.12.1
(`UpdateFields_1_12_1.h:12,30,37,115,301,321,334,351` reinterpreted
through `UpdateFields_1_12_1.cpp` END offsets 0x6/0x30/0x7A/0xBC/0x502/
0x1A/0x10/0x26).

| Variant (build) | Rows | OBJECT | ITEM | CONTAINER | UNIT | PLAYER | GO | DYN | CORPSE |
|---|---|---|---|---|---|---|---|---|---|
| 1.2.4 (4222) | 230 | 6 | 48 | 90 | 176 | 1050 | 22 | 16 | 36 |
| 1.3.1 (4297) | 269 | 6 | 48 | 90 | 176 | 1050 | 22 | 16 | 36 |
| 1.4.2 (4375) | 296 | 6 | 48 | 90 | 176 | 1128 | 22 | 16 | 36 |
| 1.5.1 (4449) | 299 | 6 | 48 | 90 | 176 | 1128 | 24 | 16 | 38 |
| 1.6.1 (4544) | 321 | 6 | 48 | 90 | 176 | 1150 | 24 | 16 | 38 |
| 1.7.1 (4695) | 324 | 6 | 48 | 90 | 204 | 1178 | 24 | 16 | 38 |
| 1.8.4 (4878) | 319 | 6 | 48 | 90 | 204 | 1216 | 24 | 16 | 38 |
| 1.9.4 (5086) | 321 | 6 | 48 | 122 | 206 | 1218 | 24 | 16 | 38 |
| 1.10.2 (5302) | 321 | 6 | 48 | 122 | 188 | 1200 | 24 | 16 | 38 |
| 1.11.2 (5464) | 322 | 6 | 48 | 122 | 188 | 1264 | 24 | 16 | 38 |
| 1.12.1 (5875) | 324 | 6 | 48 | 122 | 188 | 1282 | 26 | 16 | 38 |

Notes: PLAYER_END **shrinks** 1.9.4→1.10.2 (1218→1200) while UNIT_END also
shrinks (206→188) — layouts are not monotonic supersets; offsets move.
CONTAINER jumps 90→122 at 1.9.4 (keyring-era growth is in PLAYER, bag
slots 72 words constant at 1.12.1). DYNAMICOBJECT is 16 in every build.

### B.3 The 1.12.1 (5875) layout in detail

Verified against `UpdateFields_1_12_1.h` enums + `.cpp` rows (324 rows =
316 named fields + 8 END sentinels). Composition by mask row count:
PLAYER 183, UNIT 75, GAMEOBJECT 17, ITEM 15, CORPSE 14,
DYNAMICOBJECT 10, OBJECT 6, CONTAINER 4.
Flag distribution over rows: PUBLIC 176, PRIVATE 78, GROUP_ONLY 20,
PRIVATE+OWNER_ONLY 17, NONE 13, DYNAMIC 6, OWNER_ONLY+UNK2 5,
PRIVATE+OWNER_ONLY+SPECIAL_INFO 5, OWNER_ONLY 4.

- OBJECT 0..5 (`…_1_12_1.h:5-13`): GUID(2) @0, TYPE @2, ENTRY @3,
  SCALE_X float @4, PADDING @5.
- ITEM 6..47 (`:14-31`): OWNER/CONTAINED/CREATOR/GIFTCREATOR GUIDs,
  STACK_COUNT, DURATION, SPELL_CHARGES[5], FLAGS, ENCHANTMENT[21]
  (= 7 slots × 3 words, cf. `Item.h` enchant accessors), seed/random,
  durability pair.
- CONTAINER 48..121 (`:32-38`): NUM_SLOTS @48, PAD @49, SLOT_1[72]
  GUID words @50 ⇒ max 36 items; `MAX_BAG_SIZE=(122-50)/2=36`
  (`Bag.h:29-31`).
- UNIT 6..187 (`:39-116`): 8 link GUIDs, HEALTH(DYNAMIC)/POWER1-5/
  MAX*, LEVEL, FACTION, BYTES_0 (race/class/gender/powertype lanes),
  virtual items, FLAGS @46, AURA[48]+AURAFLAGS[6]+LEVELS[12]+APPS[12],
  attack times, bounding/combat floats, display ids, BYTES_1/2,
  DYNAMIC_FLAGS @143, NPC_FLAGS @147, STAT0-4, RESISTANCES[7] @155,
  attack-power block, power-cost blocks[7+7]. (Decimal offsets here are
  the 5875 indices also frozen in `UpdateFields.h:51-70`.)
- PLAYER 188..1281 (`:117-302`): duel/guild/bytes; QUEST_LOG 20 slots ×
  3 words (id GROUP_ONLY + 2 PRIVATE; stride `MAX_QUEST_OFFSET 3`,
  `Player.h:444,1107-1123`); VISIBLE_ITEM 19 × 12 words (creator GUID +
  8 + props + pad); inventory GUID fields INV[46 words]=23 slots,
  PACK[32]=16, BANK[48]=24, BANKBAG[12]=6, BUYBACK[24]=12,
  KEYRING[64]=32; FARSIGHT/COMBO GUIDs; XP; SKILL_INFO[384] TWO_SHORT;
  talent/prof points; combat percentages (floats); EXPLORED_ZONES[64];
  COINAGE (money); pos/neg stats; resist/damage buff blocks; honor
  block; COMBAT_RATING[20]; END 1282.
- GAMEOBJECT 6..25 (`:303-322`): CREATED_BY GUID, DISPLAYID, FLAGS @9,
  ROTATION[4] floats, STATE, POS_X/Y/Z + FACING floats (duplicated from
  `m_position` at Create), DYN_FLAGS(DYNAMIC) @19, FACTION, TYPE_ID,
  LEVEL, ARTKIT+ANIMPROGRESS (1.12.1-new, §B.4), PAD.
- DYNAMICOBJECT 6..15 (`:323-335`): CASTER GUID, BYTES, SPELLID,
  RADIUS float, POS/FACING floats, PAD.
- CORPSE 6..37 (`:336-352`): OWNER GUID, FACING/POS floats, DISPLAY_ID,
  ITEM[19] (visible items), BYTES_1/2, GUILD, FLAGS @35,
  DYNAMIC_FLAGS(DYNAMIC) @36, PAD.

Memory per entity for values+mirror (`count×8` B) and create-mask size
(`ceil(count/32)` blocks): Player 10,256 B + 164 B mask (41 blocks);
Creature/Unit 1,504 B + 24 B (6); Container 976 B + 16 B (4);
Item 384 B + 8 B (2); Corpse 304 B + 8 B (2); GameObject 208 B + 4 B (1);
DynamicObject 128 B + 4 B (1). Worst-case VALUES payload if every bit set
≈ mask + count×4 (Player ≈ 5,292 B) — before movement/header.

### B.4 Cross-variant deltas (sampled, verified by row diff)

- 1.11.2 → 1.12.1: +`PLAYER_FIELD_COMBAT_RATING_1[20]` (PRIVATE),
  −`PLAYER_SELECTION` (every later player field shifts −2), GO loses
  `GAMEOBJECT_TIMESTAMP`, gains ARTKIT/ANIMPROGRESS/PADDING (GO fields
  after POS shift −1, END 24→26); `UNIT_MOD_CAST_SPEED` INT→FLOAT.
  Code follows: timestamp written only `<1.12.1` in GO Create; artkit/
  animprogress accessors stubbed to 0 pre-1.12.1 (`GameObject.h:138-148`);
  animprogress force-sent only `>=1.12.1` (`Object.cpp:620-622`).
- 1.2.4 → 1.12.1 (name-level): 15 names only in 1.2.4 (single-slot
  buyback set, `PLAYER_CHAT_FILTERS`, `UNIT_FIELD_ATTACKPOWER` spelling,
  …) vs 40+ added by 1.12.1 (quest log 9→20 slots expansion, keyring,
  honor block, watched faction, corpse dynflags, …). A single-build Rust
  port avoids all of this; a progression server needs generated tables
  per build exactly like the C++ generator output.

### B.5 Player domain clusters (for by-domain Rust modules)

`Player` (class body `Player.h:746-2552`, 1,807 lines) is already
banner-partitioned into 28 named systems. Counts below: distinct `m_*`
data members attributed to the banner section + approximate declared
methods in the section (counted by script over the header; method counts
include inline bodies, treat as ±10%). Wire-field homes are update-field
indices, not members — listed because the module must own those setters.

| Rust module (proposed) | C++ section (Player.h) | Members | Methods~ | Key state / wire fields |
|---|---|---|---|---|
| `inventory` (+money,trade,loot UI) | STORAGE :858-1076 | 9 (+`m_itemDuration`) | 143 | `m_items[118]` (`Player.h:470-477,863`), update queue, enchant/item durations, `m_trade`, proficiencies; slots: equip 19, bags 4, pack 16, bank 24+6, buyback 12, keyring 16-used/32 (`:481-547`); money = `PLAYER_FIELD_COINAGE` (`:1033-1050`), max `0x7FFFFFFF-1` (`:655` region) |
| `quests` | QUEST :1090-1222 | 3 + `mQuestStatus` map | 89 | `m_timedquests`, share info, `m_ingametime`; wire: 20×3 quest-log words, counters packed 6-bit×4 + state byte (`:1114-1122`) |
| `spells` (+cooldown client sync) | SPELL :1338-1427 | 6 | 48 | `PlayerSpellMap m_spells`, `m_auraBaseMod`, `m_spellMods[]`, item-set effects; cooldowns themselves live in SpellCaster maps (§A.1), Player adds send/load/save + GCD overrides (`:1396-1425`) |
| `talents` | TALENT :1428-1449 | 3 (incl. mutable multiplier) | 10 | free points = `PLAYER_CHARACTER_POINTS1` (`:1443-1444`); talents are learned spells — no separate talent table in Player |
| `skills` | SKILLS :1569-1613 | 2 | 28 | `m_skillStatusMap`, forgotten-skills (>1.10.2); values in `PLAYER_SKILL_INFO[384]` TWO_SHORT (cur/max + bonuses); prof points = `PLAYER_CHARACTER_POINTS2` |
| `stats` | STAT :1450-1568 | 12 | 72 | regen/combo/ammo; writes STAT/resist/AP/damage/% player fields; virtuals shared with Creature (`Unit.h:462-471`) |
| `reputation` (+team/race-change) | FACTION :2212-2243 | 3 | 19 | `ReputationMgr m_reputationMgr` value member (own header 134 lines), `m_team`, temp at-war set; watched faction field; **no reputation update-fields** — ranks go by dedicated packets |
| `social/group/guild` | GROUP :2482-2534, GUILD :2536-2552, CHAT :2153-2211 | 6+1+3 | 33+9+27 | group refs + update masks, LFG area; guild id/rank are update fields (`:2543-2547`), only `m_guildIdInvited` is local; friends/ignore are **not** in Player: `PlayerSocial` via session/master (`:767-772`, `SocialMgr.h:117`, 168 lines) |
| `taxi` | TAXI :1935-1949 | 1 | 7 | `PlayerTaxi m_taxi` value member: known-node mask, destination deque, path, discount (`PlayerTaxi.h:29-94`); persisted as `known_taxi_mask`+`current_taxi_path` strings |
| `bind/teleport/location` | LOCATION :1614-1861 | ~41 section-attributed (incl. visibility/camera) | 74 | homebind = `m_homebind`+area (`:1691-1692,1838-1839`), loaded by LOADHOMEBIND query, `SetBindPoint/TeleportToHomebind` (`:1739,1783`); also teleport semaphores, recall/summon points, `m_visibleGUIDs`, camera, mover |
| `persistence` | LOAD :1223-1267, SAVE :1269-1297 | 1+2 | 28+18 | 22 login queries (§A.8); save timer/disabled; `_Save*` per domain above |
| rest of Player | GM 3/30, GOSSIP 0/5, PET 5/18, ENV 3/20, REST 5/9, CINEMATIC 4/5, COMBAT 3/23, SESSION+MISC 19/69, PVP 2/22 (+`HonorMgr` 219-line header), BG 2/18, INSTANCE 5+/19, ANTICHEAT 0/5, BROADCASTER 1/3 | — | — | honor/pvp fields are the PLAYER_FIELD_*KILLS/CONTRIBUTION block; instance binds `m_boundInstances`+mutex |

Decomposition warning: the clusters are **not** separable data-wise in
C++ — e.g. inventory owns `m_items` but visible-item fields, quest item
checks (`Player.h:1187-1188`), spell reagent checks, and stat application
all read it; money changes notify quests (`MoneyChanged`,
`Player.h:1193`); reputation changes notify quests (`:1194`). In Rust,
keep one `Player` aggregate with domain sub-structs + free functions per
module operating on `&mut Player` (or split borrows), rather than
independent owners. `WorldSession* m_session` (`Player.h:2014`) is
reached from everywhere (packet sends) — model as a session handle/id,
not a borrow stored in the entity.

Unit-level clusters (shared Player/Creature, `Unit.h:321-1457`):
stat system (create stats/resistances, aura-mod matrix
`m_auraModifiersGroup[UNIT_MOD_END][MODIFIER_TYPE_END]`, `Unit.h:347-354`),
state/death (`:491-503`), visibility (`:600-606`), auras
(`m_spellAuraHolders` multimap + `m_modAuras[TOTAL_AURAS]`, `:632-641`),
combat/threat (`ThreatManager`+`HostileRefManager` value members,
`:907-909`; Rust `mangos-threat` crate exists), relations/charm/pets/
totems (`:1174-1180`), movement (`MotionMaster`, speed rates, pending
movement-change queue for player-controlled units, `:1316-1326`;
`mangos-movement` exists). Creature adds: AI pointer, loot + recipients,
respawn/corpse timers, template/data/addon pointers (shared immutable —
`&'static`-like in Rust), home/summon positions, vendor counts
(`Creature.h:614-687`).

## (c) DB / config inputs

- **Build selection (config-equivalent):** `SUPPORTED_CLIENT_BUILD` CMake
  cache var, default `CLIENT_BUILD_1_12_1` (`vmangos/cmake/options.cmake:11`,
  generated header template `Progression.h.in:24-36`). Changing it
  recompiles field tables, opcodes, DBC formats, and content-patch cap.
- **Wire-affecting world config** (defaults in `World.cpp`):
  `ShowHealthValues=false` → health-as-percent rewrite (§A.7.7)
  (`World.cpp:617`, read per `BuildValuesUpdate` at `Object.cpp:581`);
  `Compression.Level=1 (1..9)`, `Compression.Update.Size=128`,
  `Compression.Movement.Count=300` (`World.cpp:557-559`, used
  `UpdateData.cpp:68,152`). Other `sWorld.getConfig` reads touch this
  surface for creation defaults (start level/money in `Player::Create`),
  corpse-decay-by-rank (`Creature.cpp:1237-1253`), save interval
  (`Player` ctor `m_nextSave`), visibility forcing for large GOs
  (`GameObject::Create`). Exact key list beyond the four above was not
  exhaustively enumerated (§f).
- **World DB (templates + spawns), via `ObjectMgr`:** `creature_template`
  → `CreatureInfo`, `creature` → `CreatureData`, creature addons/groups/
  linking; `gameobject_template` → `GameObjectInfo`, `gameobject` →
  `GameObjectData`; `item_template` → `ItemPrototype` (required by
  Item/Bag Create); player create info (`PlayerInfo` race/class start
  pos + items + actions, structs `Player.h:168-224`). Table/column-level
  loaders live in `ObjectMgr.cpp` (outside this surface; not read — §f).
- **DBC (client data):** ChrClasses (powertype), ChrRaces/faction
  templates, CreatureModelData (bounding/collision/scale via
  `Unit::UpdateModelData/InitPlayerDisplayIds`), Spell (DynamicObject
  requires the spell to exist), TaxiNodes/Path, SkillLine, Talent.
- **Character DB:** `characters` (58 cols, §A.8) + the 21 satellite
  login queries (inventory+item instances, spells, cooldowns, skills,
  quest status, reputation, honor, auras, actions, social, homebind,
  guild, group, bound instances, BG data, account data, mails);
  `corpse` table written by `Corpse::SaveToDB` (guid, player, pos, map,
  time, type, instance; bones never saved) (`Corpse.cpp:109-117`);
  item instance rows via `Item::SaveToDB/LoadFromDB` (bodies not fully
  read — §f); creature/gameobject respawn times in map persistent state.
  Only two update-field ranges persist as raw blobs: explored-zones
  (64 words) and taxi mask/path strings; everything else is recomputed
  or stored decomposed. **A Rust port can therefore keep the same char
  DB schema and rebuild the values array at login, exactly like C++.**
- **GUID seeding:** per-high generators max-loaded from DB at startup
  (`ObjectGuid.h:288-289`); freed GUIDs recycle via queue. Rust must
  seed identically or collide with persisted item/creature GUIDs.

## (d) Wire-visible semantics a Rust port must replicate

1. **Word-array fidelity.** The client sees raw `u32` words at build-5875
   offsets (§B.3), including float bit patterns, byte/short lanes, and
   two-word GUIDs. Rust model: `values: Box<[u32]>` (or per-type const
   arrays) + typed accessors with the same lane semantics; do not
   "clean up" into per-field structs unless every offset is preserved
   by construction (codegen from the 324-row table is the safe path).
2. **Identity on the wire.** Field 0..1 = full GUID, field 2 = TypeMask
   (not TypeId); CREATE header carries PackedGuid + `u8` TypeId;
   VALUES header carries PackedGuid only; MOVEMENT blocks and
   DESTROY carry the **unpacked** GUID; out-of-range lists carry
   PackedGuids (1.12.1). (§A.6 anchors.)
3. **Create = nonzero ∩ visible; update = changed-vs-mirror ∩ visible.**
   Including: forced GO dyn-flag/animprogress bits every block, empty
   VALUES blocks suppressed, `CREATE_OBJECT2` for new objects, SELF/
   MELEE_ATTACKING flag rules, movement section shapes per flag
   combination (LIVING speeds+spline, HAS_POSITION floats, HIGHGUID 0,
   ALL 1, transport progress/mstime). (§A.6.)
4. **Single global mirror.** Dirtiness is consumed once per tick for all
   viewers; late viewers depend on CREATE, not on replayed VALUES.
   `SetUInt64` poisons both mirror halves; `ForceValuesUpdateAtIndex`
   poisons one. Player self-updates are emitted even if the camera is
   elsewhere. (§A.4, §A.6.)
5. **Per-viewer rewrites** in §A.7, in order — especially health-as-percent
   (default), NPC-flag stripping, tapped/lootable dynflags, GO quest
   activation as `u16+u16`, casting-target override, float→uint
   conversions for attack times and stat-buff ranges, GM flag forcing.
   These are observable by addons/other clients and by sniff diffing.
6. **Flag-based field privacy** (§A.6 visibility): PRIVATE only to self
   (note: self means `target==this` Player; a possessed/charmed viewer
   does not automatically get PRIVATE), OWNER_ONLY to owner/charmer
   (items also need UNK2, granted together), GROUP_ONLY to raid of
   owner-or-self (quest-log ids), SPECIAL_INFO gated on dynflag +
   `CanSeeSpecialInfoOf`, DYNAMIC treated as public for inclusion.
   Getting this wrong leaks other players' stats/inventory GUIDs —
   the 1.12 client will render whatever it receives.
7. **Packet framing:** blockCount+hasTransport header, out-of-range
   section first, 32 KiB split-before-append, compression threshold 128
   (configurable) with `u32` raw-size prefix and zlib level from config,
   out-of-range GUIDs cleared after first sub-packet. Byte-identical
   zlib output is **not** required (any valid deflate stream works),
   but thresholds/opcode choice are sniff-visible. (§A.6.)
8. **Child-create ordering:** self-login sends owned items (equip →
   bags/bank → buyback → keyring; bags recursively contain items)
   **before** the player block; other viewers never receive item creates
   for someone else's inventory (only VISIBLE_ITEM fields). (§A.6.)
9. **Scale/type defaults:** `GetObjectScale` returns 1.0 when SCALE_X
   word is 0 (`Object.h:144`) — but the zero word itself is omitted from
   CREATE, so the client also defaults; keep both. `OBJECT_FIELD_TYPE`
   must equal the TypeMask (client dispatches on it).
10. **GUID algebra:** entry-in-high for creature/GO/pet/transport,
    24-bit counters there, player GUID = bare counter (high 0, GUID 0 is
    "none" and also an invalid player), TRANSPORT high maps to TypeId
    OBJECT via `GetTypeId` while real transports are GameObjects with
    TypeId GAMEOBJECT — code that round-trips `guid→TypeId→class` must
    special-case it exactly as C++ callers do (they mostly avoid it).
11. **What is NOT in update fields** (easy to over-port): names (queried
    via name-query packets; `GetName()` is server-side from template/
    `m_name`), reputation values, talents beyond free-points count,
    spellbook/cooldowns, action bars beyond one byte, social lists,
    guild beyond id/rank/timestamp, taxi known-mask. Position for units
    travels in MovementInfo/splines, not fields — except GO/DynObj/Corpse
    duplicate pos into fields at Create and never update them (static).

## (e) Smallest viable first slice + entry dependencies

**Slice goal:** a client logs in and sees its own player plus static
creatures/gameobjects around it, with health/nameplate-correct creates
and value updates — no combat, spells, items beyond empty inventory,
quests, or AI movement.

In-scope entities (minimal spawnables):
- `Player` (self only): values[1282], identity/appearance/level/faction/
  display/health/power fields from a `characters` row (or fixture),
  LIVING create with zero spline, empty `m_items[118]`.
- `Creature` (generic subtype only): values[188], from one
  `CreatureInfo`+`CreatureData` fixture: entry, display+scale, level,
  faction, npc flags, health/max, bytes, bounding/combat radii. No AI,
  no spells, `m_AI=None` equivalent; static position via MovementInfo.
- `GameObject` (non-transport, non-quest types first): values[26],
  HAS_POSITION create, dyn-flags rewrite returning 0.
- `Item`/`Bag` only as far as self-create with empty inventory emits
  nothing — defer both classes; add `Corpse` (38) and `DynamicObject`
  (16) in slice 2 (death + first AoE spell respectively).

Required machinery, in dependency order:
1. `mangos-guid` (exists, 549 lines — reuse): GUID/high/TypeId/packed.
2. **Generated 5875 field table** (new): codegen the 324 rows from
   `UpdateFields_1_12_1.cpp` into Rust consts (offsets, sizes, types,
   flags) + per-type flags arrays + END counts. Add a golden test:
   PLAYER_END==1282, CONTAINER_END==122 (catches the header-comment trap).
3. `Values` type (new): boxed `[u32]` + mirror, typed get/set
   (u32/i32/u64/f32/byte/short/guid), change-only dirty semantics,
   force-update poison, create/update mask builders parameterized by
   viewer relation (self/owner/charmer/raid/special-info/debug).
4. `UpdateData` packet builder (new, in/near `mangos-protocol`):
   block append with 32 KiB split, out-of-range section, zlib threshold
   logic; opcodes 169/170/502 already need to exist in the opcode table.
5. Movement-section writer for the two shapes used above (LIVING
   without spline; HAS_POSITION floats) — depends on `MovementInfo`
   wire format from the movement surface; stub speeds from template.
6. Entity structs `ObjectBase{type_id,mask,update_flags,values,in_world}`
   + `WorldPos{map,instance,pos,cell}` + `Player/Creature/GameObject`
   aggregates with only the members the slice touches; template stores
   (`CreatureInfo/GameObjectInfo` fixtures or `mangos-world-data` rows).
7. Visibility stub: "everyone sees everything in the test map" to drive
   CREATE/VALUES/OUT_OF_RANGE, replaced by the map-visibility surface
   later; but keep the C++ call shape (`BuildUpdateData(viewers)` then
   clear mirror) so semantics don't change when real visibility lands.
8. Per-viewer rewrite pass with health-percent + NPC-flag + dynflag
   rules (§A.7 items 1,5,7 and GO rule) — include from day one; sniff
   tests will flag their absence immediately.

Entry dependencies (must exist before this slice compiles/runs):
session/world packet send path, map instance to own the update list,
DBC display/model lookup for scale+bounds (or fixture constants),
character-row fixture or `mangos-db` read of `characters`, GUID
generators seeded above fixture maxima. Deferred safely: auras (fields
stay 0), threat/AI, spells/cooldowns, inventory logic, groups/guilds
(GROUP_ONLY never granted → quest ids hidden, acceptable for slice),
persistence writes (read-only login).

Verification for the slice: byte-compare CREATE blocks against C++
sniffs for the same fixture spawn (mask + words), then a two-client
test for health-percent hiding and private-field non-leakage.

## (f) Unresolved

- Full prod (`2bae5f9`) vs HEAD audit of `Object.cpp/Object.h/ObjectGuid.h`
  beyond `-w` stat + targeted grep: diffs exist (Object.cpp ~260 changed
  lines `-w`) but were not read line-by-line; field tables verified equal.
- `Player::CreateCorpse`, `Creature::InitEntry/SelectLevel` tails,
  `Item::SaveToDB/LoadFromDB` field↔column mapping, and `Pet::Create`
  bodies not read; corpse visible-item fill and pet GUID high
  (UNIT vs PET at runtime) therefore unverified here.
- `ObjectMgr` DB loaders (exact SQL/columns for templates/spawns),
  `MovementInfo` wire layout, and visibility/`Camera` fan-out owned by
  sibling surfaces — referenced, not inventoried here.
- Config keys beyond the four wire keys (§c) not exhaustively listed;
  `sWorld.getConfig` call sites in Player/Creature cpps not enumerated.
- Whether Rust must support pre-1.12 progression builds is a product
  decision: this digest prices 1.12.1-only as the slice; all-11-builds
  multiplies table/opcode/DBC work and reintroduces every `#if` branch
  cited above.
- No `sizeof` measurements taken (no compile run); per-entity memory
  beyond values+mirror is member-count-based, not byte-exact.
