# 06 — Static Data and File Formats

Scope: DBC loading, extracted client data (maps / vmaps / mmaps), extractor
tools, world-database content loaded at boot, configuration files, and
`scripts/*.py` data generators. All citations are `file:line` relative to the
workspace root `/Users/jeffarn/Programming/azeroth-server`. Read-only
inspection; no live system touched; no credential values reproduced.

## 0. Where the data lives at runtime

- One root knob: `DataDir` config key, read in
  `vmangos/src/game/World.cpp:904` (`sConfig.GetStringDefault("DataDir", "./")`),
  stored as `m_dataPath` (`World.cpp:920-921`). It cannot change on config
  reload (`World.cpp:915-916`).
- Sub-layout under `DataDir` (all derived by string concatenation, no manifest):
  - DBCs: `<DataDir><SUPPORTED_CLIENT_BUILD>/dbc/` — build-number directory
    (`World.cpp:1370-1376`, then `LoadDBCStores(dbcPath)` appends `dbc/` at
    `vmangos/src/game/Database/DBCStores.cpp:182`). For 1.12.1 that is
    `5875/dbc/` (default `SUPPORTED_CLIENT_BUILD "CLIENT_BUILD_1_12_1"` at
    `vmangos/cmake/options.cmake:11`; build 5875 noted at
    `vmangos/src/game/Objects/UpdateFields_1_12_1.h:3`).
  - Terrain maps: `<DataDir>maps/%03u%02u%02u.map` (mapId, gy, gx)
    (`vmangos/src/game/Maps/GridMap.cpp:631-633`, again `:1133-1135`).
  - VMaps: `<DataDir>vmaps/` (`World.cpp:937`; `GameObjectModel.cpp:43`).
  - MMaps: `<DataDir>mmaps/%03i.mmap` + `%03i%02i%02i.mmtile`
    (`vmangos/src/game/Maps/MoveMap.cpp:73-75,138-140`).
- Boot hard-fails if starter-area `.map`/vmap files are missing
  (`World.cpp:1296-1301`) and if DBCs are missing/incompatible
  (`DBCStores.cpp:422-437`).

### Locally present assets (this checkout)

`vmangos/run/bin/` contains exactly one file: `run-mangosd` (345 bytes, bash
autorestart loop). No `dbc/`, `maps/`, `vmaps/`, `mmaps/`, `5875/`, binaries,
or `Server.log` are present locally. They are gitignored deployment artifacts
(`.gitignore:8-14` ignores `run/bin/mangosd`, `realmd`, `Extractors/`,
`maps/`, `mmaps/`, `vmaps/`, `5875/`; `vmangos/.gitignore:44-46` whitelists
only `run-mangosd`). `vmangos/run/etc/` holds only the two `.example` configs
(mangosd 117,433 B; realmd 7,174 B). **Rust implication:** asset
compatibility cannot be byte-verified from this checkout — the formats below
are evidenced from loader/writer code only; a production `DataDir` snapshot
is needed for fixture-based tests.

## 1. DBC files (client database tables)

### 1.1 On-disk format (WDBC)

Loader: `vmangos/src/shared/Database/DBCFileLoader.cpp`, `.h`.

- Header, 20 bytes, five little-endian `uint32`: magic, recordCount,
  fieldCount, recordSize, stringSize (`DBCFileLoader.cpp:47-90`).
- Magic: `0x43424457` i.e. ASCII `WDBC` read as LE uint32
  (`DBCFileLoader.cpp:53-58`). No version field — compatibility is enforced
  only by field-count equality against the format string
  (`DBCFileLoader.cpp:187-188`, `strlen(format) != fieldCount` → reject).
- Body: `recordSize * recordCount` bytes of fixed-width records followed
  immediately by `stringSize` bytes of NUL-terminated string block; both are
  slurped in one `fread` (`DBCFileLoader.cpp:103-110`). String fields are
  uint32 offsets into that block (`DBCFileLoader.h:75-81`).
- Field widths on disk: 4 bytes for everything except format chars `b`/`X`
  (1 byte) (`DBCFileLoader.cpp:92-101`). There are no 16/64-bit on-disk
  fields in practice, despite `FT_64BITINT='L'` existing in the enum
  (`DBCFileLoader.h:28-42`); `GetFormatRecordSize` would `assert(false)` on
  it (`DBCFileLoader.cpp:161-163`).
- Endianness: explicitly little-endian. Every header/value read passes
  through `EndianConvert`, which byte-swaps only on big-endian hosts
  (`DBCFileLoader.cpp:53,66,74,82,90`; `DBCFileLoader.h:55-68`;
  `vmangos/src/framework/Utilities/ByteConverter.h:57-77`). This is the
  *only* one of the four binary formats with endian handling.
- Alignment hazard: record field access is unaligned
  `reinterpret_cast<uint32*>(offset + ...)` (`DBCFileLoader.h:58,65`) —
  fine on x86-64, UB-ish elsewhere. A Rust port must use unaligned reads
  (`read_unaligned` / byteorder), not `#[repr(C)]` references into the buffer.

### 1.2 Format strings and record structs

- 59 format strings in `vmangos/src/game/Database/DBCfmt.h` (92 lines),
  e.g. `TalentEntryfmt[] = "niiiiiiiixxxxixxixxxi"` (`DBCfmt.h:81`). Alphabet
  (`DBCFileLoader.h:28-42`): `n`=index+stored, `d`=index-not-stored,
  `i`=uint32, `f`=float, `s`=string ptr, `b`=uint8, `x/X/F`=skip (4/1/4 B).
- Record structs: `vmangos/src/game/Database/DBCStructure.h`, 835 lines,
  ~55 `struct *Entry` types, all inside `#pragma pack(1)` …
  `#pragma pack(pop)` (`DBCStructure.h:35-42` and `:785-789`) — "required
  packing to portability" per the comment at `:35`.
- The in-memory record is *not* the on-disk record: `AutoProduceData`
  repacks selected fields into a packed C struct, widening strings from
  uint32 offsets to host `char*` pointers (8 bytes on 64-bit)
  (`DBCFileLoader.cpp:173-265`, esp. `:246-249`; size math at `:128-171`,
  `FT_STRING += sizeof(char*)` `:142-143`). `AutoProduceStrings` then
  patches those pointer slots into a copied string pool, first-non-empty
  locale wins (`DBCFileLoader.cpp:267-320`).
- Load-time invariant: `GetFormatRecordSize(fmt) == sizeof(T)` is asserted
  per DBC (`DBCStores.cpp:144-148`). Storage is a flat `char**` index table
  (id → packed record) plus string pools
  (`vmangos/src/shared/Database/DBCStore.h:27-66`).
- Several format strings are build-conditional (`DBCfmt.h:31-35`
  ChrClasses, `:53-59` ItemSet) — the DBC field count itself varies by
  client patch, so a Rust port needs per-build format variants or runtime
  format strings, not one fixed struct set.
- Orphaned formats: `DBCfmt.h` still defines formats no longer loaded as
  DBCs (`AreaTableEntryfmt:25`, `MapEntryfmt:63`, `TaxiNodesEntryfmt:83`,
  `SkillLineAbilityfmt:68`, `LiquidTypefmt:60`, `MailTemplateEntryfmt:62`,
  `CreatureSpellDatafmt:43`) because those tables moved to SQL (see §5).

### 1.3 Which DBCs load, and how many

`LoadDBCStores` (`DBCStores.cpp:180-441`):

- Declared count constant: `DBCFilesCount = 45` (`DBCStores.cpp:184`), used
  for the progress bar and the all-missing error threshold.
- Actual active `LoadDBC(...)` calls at 1.12.1: **46** (48 textual calls
  minus 2 commented out — ItemDisplayInfo `:226`, WorldMapOverlay `:413`).
  The 46th is the build-conditional `ItemBagFamily.dbc`
  (`DBCStores.cpp:223-225`, `#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_8_4`),
  which post-dates the hardcoded 45. Minor, but a port should not copy the
  constant as truth.
- The 46 files (in load order): AuctionHouse, BankBagSlotPrices,
  ChatChannels, CharacterFacialHairStyles, CharSections, ChrClasses,
  ChrRaces, CinematicSequences, CreatureDisplayInfo,
  CreatureDisplayInfoExtra, CreatureModelData, CreatureFamily,
  CreatureType, DurabilityCosts, DurabilityQuality, Emotes, EmotesText,
  GameObjectDisplayInfo, ItemBagFamily, ItemRandomProperties, ItemSet,
  Lock, NamesProfanity, NamesReserved, QuestSort, SkillLine,
  SkillRaceClassInfo, SkillTiers, SpellCategory, SpellCastTimes,
  SpellDuration, SpellFocusObject, SpellItemEnchantment, SpellRadius,
  SpellRange, SpellShapeshiftForm, SpellVisual, StableSlotPrices, Talent,
  TalentTab, TaxiPath, TaxiPathNode, TransportAnimation, WorldMapArea,
  WMOAreaTable, WorldSafeLocs (`DBCStores.cpp:193-414`).
- Notable absences: **there is no Spell.dbc, Map.dbc, AreaTable.dbc,
  Faction.dbc, or TaxiNodes.dbc load** — spells come from the
  `spell_template` SQL table (`vmangos/src/game/Spells/SpellMgr.cpp:3535,3550`),
  maps/areas/factions/taxi-nodes from SQL too (§5).
- Locale overlay: after each base load, per-locale `<locale>/<file>` DBCs
  are probed and only their strings merged (`DBCStores.cpp:154-162`;
  `DBCStore.h:68-83`). Derived indexes are built inline during load:
  talent→rank map (`DBCStores.cpp:324-332`), taxi path graphs and node mask
  (`:337-402`), WMO area triple map (`:408-412`), name-validation regexes
  compiled from NamesProfanity/Reserved with emacs-regex translation
  (`:233-272`), facial-hair/char-section maps (`:199-208`).
- Failure mode: any bad/missing file → `exit(1)` (`DBCStores.cpp:428-437`).

**Rust implication (DBC):** files are reused **unchanged**. A Rust loader is
a small, exactly-specified parser (20-byte LE header, format-string-driven
field walk). The clean-room risk is low; the work is transcribing 46 format
strings + ~55 record structs and reproducing the derived indexes and the
locale first-non-empty string merge. `scripts/dbc.py` (§7) is an existing
independent LE reader usable as a test oracle. Do not mimic the
pointer-patched packed-struct memory model; use owned `Vec<Record>` +
string interning.

## 2. Terrain maps (`.map`)

Loader: `vmangos/src/game/Maps/GridMap.cpp`; header structs in
`vmangos/src/game/Maps/GridMapDefines.h`. Writer (extractor):
`vmangos/contrib/extractor/System.cpp`.

- Naming: `maps/%03u%02u%02u.map` = mapId, gy, gx (`GridMap.cpp:631-633`).
- File header: `GridMapFileHeader`, ten `uint32` (40 bytes): mapMagic,
  versionMagic, then offset/size pairs for area, height, liquid, holes
  sections (`GridMapDefines.h:5-17`), read with a single struct `fread`
  (`GridMap.cpp:78-85`).
- Magics: `MAP_MAGIC = "MAPS"`, `MAP_VERSION_MAGIC = "z1.4"`, compared as
  host-order uint32 casts of the string literals (`GridMap.cpp:34-35,85`).
  Section fourccs: `AREA`, `MHGT`, `MLIQ` (`GridMap.cpp:36-38`). The
  extractor writes identical constants (`contrib/extractor/System.cpp:239-243,289-290`)
  — loader and writer share `GridMapDefines.h` semantics.
- Sections are seek-loaded independently, each guarded by its fourcc
  (`GridMap.cpp:88-117` dispatch; `:146-250` section loaders).
  - Area: `GridMapAreaHeader{u32 fourcc; u16 flags; u16 gridArea}` then
    optionally 16×16 `uint16` (`GridMapDefines.h:21-26`; `GridMap.cpp:148-162`).
  - Height: header `{u32 fourcc; u32 flags; float gridHeight; float gridMaxHeight}`
    (`GridMapDefines.h:32-38`); payload is V9 129×129 + V8 128×128 grids in
    one of three encodings selected by flags `MAP_HEIGHT_AS_INT16` /
    `AS_INT8` / float (`GridMapDefines.h:28-30`; `GridMap.cpp:164-206`),
    dequantized via `(max-min)/65535` or `/255` (`GridMap.cpp:181,190`).
  - Holes: raw `uint16[16][16]` (`GridMap.cpp:208-216`).
  - Liquid: `GridMapLiquidHeader` (`GridMapDefines.h:43-54`) mixes
    u32/u8/u16/u8×4/float; then optional 16×16 entry+flags arrays and a
    width×height float level map (`GridMap.cpp:218-250`).
- Endianness/packing: **no endian conversion anywhere** and no
  `#pragma pack`; the format relies on little-endian host order and on the
  structs being padding-free under natural alignment (true for these
  layouts on x86-64: header is all-u32; liquid header's float lands at
  offset 12). This is implicit, not asserted — no `static_assert(sizeof…)`
  in `GridMapDefines.h`.

**Rust implication (.map):** existing files reusable **unchanged** on
little-endian targets. Parse with explicit LE reads (`u32::from_le_bytes`)
and hard-code the section layouts; replicate the magic-compare as byte
comparison against `b"MAPS"`/`b"z1.4"` (note C++ compares as uint32, so it
is byte-order-sensitive on a BE host — a Rust port should compare bytes).
The three height encodings and exact V9/V8 interpolation
(`GridMap.cpp:252+`, `getHeightFrom*`) must be ported bit-compatibly
(float32 arithmetic) or heights will drift vs. the C++ server.

## 3. VMaps (collision / line-of-sight geometry)

Loaders: `vmangos/src/game/vmap/MapTree.cpp`, `WorldModel.cpp`,
`ModelInstance.cpp`, `BIH.cpp`, `GameObjectModel.cpp`. Definitions:
`vmangos/src/game/vmap/VMapDefinitions.h`. Writer:
`vmangos/src/game/vmap/TileAssembler.cpp` (linked into the assembler tool,
§6), fed by raw files from the vmap extractor.

### 3.1 File family

- Magics: final files start with the 8-byte ASCII `VMAP_7.0`; intermediate
  raw extractor files with `VMAPs05` (`VMapDefinitions.h:26-27`). Versioning
  is entirely in the magic string — no numeric version field.
- `<mapId:03>.vmtree` — per-map tree (`MAP_FILENAME_EXTENSION2 ".vmtree"`,
  `vmangos/src/game/vmap/VMapManager2.h:32`; writer at
  `TileAssembler.cpp:106,116`). Parse: 8-byte magic, 1-byte `tiled` flag,
  chunk `NODE` then a serialized BIH tree, chunk `GOBJ`, then (non-tiled
  maps only) a stream of `ModelSpawn` + uint32 tree-reference pairs until
  EOF (`MapTree.cpp:326-407`).
- `<mapId:03>_<x:02>_<y:02>.vmtile` — per-tile spawns for tiled maps
  (writer `TileAssembler.cpp:163,166`). Parse: magic, `uint32 numSpawns`,
  then per spawn a `ModelSpawn` record + uint32 referenced node
  (`MapTree.cpp:419-491`). Unload re-reads the whole tile file to decrement
  reference counts (`MapTree.cpp:495-544`) — the file is its own index.
- World-model files (one per WMO/M2 model name, referenced by name from
  spawns): magic, chunk `WMOD` + chunkSize + RootWMOID, chunk `GMOD` +
  group count + per-group `GroupModel` records, chunk `GBIH` + group BIH
  (`WorldModel.cpp:587-670`, write `:587-626`, read `:628-670`).
- `temp_gameobject_models` — no magic/header at all: repeated
  `{u32 displayId, u32 nameLen, char[nameLen], Vector3 low, Vector3 high}`
  until EOF (`VMapDefinitions.h:28`; loader `GameObjectModel.cpp:41-68`;
  name buffer capped at 500, `GameObjectModel.cpp:48,54`). Boot-loaded from
  `World.cpp:1772`.

### 3.2 Record-level serialization

- `ModelSpawn` is written **field by field**, not as a struct dump:
  u32 flags, u16 adtId, u32 ID, 3×float pos, 3×float rot, float scale,
  conditional 6×float bounds (only if `MOD_HAS_BOUND`), u32 nameLen,
  name bytes (max 500) (`ModelInstance.cpp:178-222` read, `:224-245`
  write). The conditional bounds make records variable-length — a parser
  must branch on flags exactly as C++ does.
- BIH (bounding-interval hierarchy) serialization: bounds low/high
  (3+3 floats), u32 treeSize, tree u32 array, u32 object count, object u32
  array (`BIH.cpp:255-284`). Tree nodes pack axis/leaf info into the top 2
  bits of a u32 (`BIH.h:400-405`; traversal `BIH.h:330-337`) — the on-disk
  tree is the in-memory traversal structure verbatim.
- `GroupModel` read/write at `WorldModel.cpp:271` (write) and `:330` (read)
  (per-group mesh: bounds, vertices, triangles, mesh BIH, liquid data).
- Endianness/packing: **no endian conversion, no struct packing pragma**;
  everything is raw `fread`/`fwrite` of host-order u32/u16/float scalars
  and G3D `Vector3`s (3×f32). Files are therefore little-endian de facto
  and assume IEEE-754 f32. Chunk tags (`NODE`/`GOBJ`/`WMOD`/`GMOD`/`GBIH`)
  are compared as raw bytes via `readChunk` (`MapTree.cpp:345,355`;
  declared `VMapDefinitions.h:31`), which is endian-safe.
- Raw intermediate format: `TileAssembler.cpp:527` checks
  `RAW_VMAP_MAGIC` when consuming extractor output — a second, internal
  format that only the toolchain (not the server) reads.

**Rust implication (vmaps):** existing `.vmtree/.vmtile`/model files are
reusable **unchanged** on LE. The port surface is larger than `.map`:
a field-wise spawn parser, the BIH wire format (must reproduce the packed
u32 node encoding if reusing files with a native Rust BIH — recommended:
parse into the same `Vec<u32>` tree/objects layout and port the traversal
in `BIH.h`), `GroupModel` mesh data, and G3D-compatible f32 ray/AABox math
for identical LoS/height answers. There is no checksumming or length
prefixing beyond counts, so a Rust parser should add bounds checks the C++
lacks (several `fread` results are unchecked, e.g. `MapTree.cpp:379,462`).

## 4. MMaps (Recast/Detour navigation meshes)

Loader: `vmangos/src/game/Maps/MoveMap.cpp`; shared header:
`vmangos/src/game/Maps/MoveMapSharedDefines.h`. Generator: §6.

- `%03i.mmap` (per map): the file is **one raw `dtNavMeshParams` C struct**
  dumped to disk — loaded by `fread(&params, sizeof(dtNavMeshParams), …)`
  straight into Detour's `init` (`MoveMap.cpp:73-98`, struct read at `:85-87`).
  No magic, no version. Its layout is whatever the vendored
  `recastnavigation` headers define (`MoveMapSharedDefines.h:22` includes
  `DetourNavMesh.h`).
- `%03i%02i%02i.mmtile` (per tile; note filename order is mapId, **y, x** —
  `MoveMap.cpp:140`): 20-byte header `MmapTileHeader` = 5×u32
  {mmapMagic, dtVersion, mmapVersion, size, usesLiquids}
  (`MoveMapSharedDefines.h:28-37`), followed by `size` bytes of opaque
  Detour tile data handed to `dtNavMesh::addTile` with ownership transfer
  (`MoveMap.cpp:155-210`; alloc `:173`, `addTile` `:195`).
- Magics/versions: `MMAP_MAGIC 0x4d4d4150` ('MMAP' as LE u32),
  `MMAP_VERSION 6` (`MoveMapSharedDefines.h:25-26`). The loader checks
  magic (`MoveMap.cpp:159-164`) and `mmapVersion` (`:166-171`) but **does
  not check the header's `dtVersion` field** — that check exists only in
  the generator's incremental path (`vmangos/contrib/mmap/src/TileWorker.cpp:364`).
  Detour-internal tile versioning is inside the opaque payload.
- GameObject navmeshes reuse the same container: `mmaps/go%04i.mmtile`,
  same header checks (`MoveMap.cpp:369-433`, esp. `:388-403`), but the
  payload is a whole navmesh (`mesh->init(data, …)`, `:419`) rather than a
  tile — same extension, two payload kinds.
- Area/terrain enums baked into generated tiles: `NavTerrain`
  (`MoveMapSharedDefines.h:39-51`) and `NavMeshAreas`
  (`GridMapDefines.h:85-96`) — shared between generator and core.
- Endianness/packing: header is all-u32 (padding-free); **no endian
  conversion**; the Detour payload is a serialized C++ object graph
  (Detour explicitly byte-swaps only if built to — stock Detour tile data
  is host-endian). Struct-packing risk concentrates in the raw
  `dtNavMeshParams` `.mmap` file.
- Loading is lazy/per-grid and reference-counted via `MMapData`
  (`MoveMap.cpp:119-210` load, `:212-253` unload), gated by config
  `CONFIG_BOOL_MMAP_ENABLED` (`MoveMap.cpp:69-70`).

**Rust implication (mmaps):** this is the **least portable** format. The
20-byte container header is trivial, but the payload is only meaningful to
a compatible Detour build. Realistic Rust options, in order: (a) FFI to the
same vendored Recast/Detour C++ (keeps existing `.mmtile` files valid);
(b) regenerate all mmaps with a Rust navmesh stack and accept pathing
differences; (c) reimplement Detour's tile format + query in Rust
(large). A pure-Rust port that reuses existing `.mmtile` files unchanged
implicitly chooses (a) or (c). The `.mmap` params file should be parsed
field-wise (orig[3], tileWidth/Height, maxTiles, maxPolys per Detour
headers) rather than via `#[repr(C)]` transmute.

## 5. World-database content loaded at boot

Boot sequence: `World::SetInitialWorldSettings()` (`World.cpp:1284` to
~`:1800`), a linear, dependency-ordered list of ~120 `Load*` calls. Order
matters and is documented inline (e.g. quests after DBC+creature+item+GO
tables, `World.cpp:1532`).

- Pre-DBC SQL loads already cover former-DBC data: spells from
  `spell_template` (`World.cpp:1342`; `SpellMgr.cpp:3535,3550` — build-gated
  `WHERE build = max(build ≤ SUPPORTED_CLIENT_BUILD)`), factions
  (`World.cpp:1344`), sounds (`:1347`), taxi nodes (`:1350`), area triggers
  (`:1353`), skill-line abilities (`:1356`), mail templates (`:1359`), pet
  spell data (`:1362`). Then DBCs (`:1376`), then `map_template` /
  `area_template` (`:1381-1384`).
- Bulk content via `ObjectMgr` (`vmangos/src/game/ObjectMgr.cpp`, 12,038
  lines): ~98 distinct tables in `FROM` clauses in that file alone
  (grep-counted), including `creature_template`, `creature` (spawns),
  `gameobject_template`, `gameobject`, `item_template`, `quest_template`,
  `playercreateinfo*`, `*_levelstats`, `locales_*`, `areatrigger_*`
  (representative queries: `ObjectMgr.cpp:165,1177,2278,3777,4480,5299,7763`).
  Plus SpellMgr (spell chains/proc/learn/target tables, `World.cpp:1429-1593`),
  loot tables (`:1624`), pools, game events, waypoints (`:1652`), vendors/
  trainers (`:1640-1645`), guilds/auctions (characters DB, `:1672-1682`).
- Nine tables go through the generic `SQLStorage` mechanism (format-string
  driven, same `i/f/s/b` alphabet as DBC fmt):
  `creature_addon`, `creature_display_info_addon`,
  `gameobject_display_info_addon`, `page_text`, `map_template`,
  `conditions`, `area_template`, `mail_text_template`, `pet_spell_data`
  (`vmangos/src/game/Database/SQLStorages.cpp:27-47`; externs
  `SQLStorages.h:28-36`; load sites e.g. `ObjectMgr.cpp:6287,8092,11634,11650`).
- Patch/build gating is pervasive: rows carry `patch`/`build` /
  `patch_min..patch_max` and queries select the newest row ≤ current patch
  (e.g. `ObjectMgr.cpp:1702,4634,6720,7763`; `SpellMgr.cpp:3550`). The
  "static data" is therefore *versioned by SQL row*, not by file.
- What is in-repo vs not: `vmangos/sql/` contains the characters/logon/logs
  schemas (55 / 11 / 15 `CREATE TABLE` in `characters.sql` / `logon.sql` /
  `logs.sql`), append-only migrations (`sql/migrations/`, timestamped
  `*_world.sql`), and optional `sql/custom/*.sql` content patches. The
  world content dump itself is **not in this repo** (per `CLAUDE.md`,
  pulled separately into `vmangos-database/`, gitignored) — so table
  *shapes* are evidenced by the C++ queries above, not by a schema file
  for `mangos` (world) DB.

**Rust implication (world DB):** no file format to port — the contract is
~100 SQL query shapes + the patch-selection idiom + load ordering. A Rust
core can read the same MySQL databases unchanged (production rule:
append-only migrations, player data sacred — `CLAUDE.md`). The migration
cost is transcribing field-by-field row mappers (ObjectMgr uses positional
`fields[n]` access throughout) and reproducing the cross-validation the
loaders perform (DBError logging for dangling references into DBC ids,
e.g. `ObjectMgr.cpp:5760,6824,8475`).

## 6. Extractor / generator toolchain (in-repo)

Wired in `vmangos/contrib/CMakeLists.txt:20-23`
(extractor, vmap_assembler, vmap_extractor, mmap):

- `contrib/extractor/` — "ad" map extractor: reads client ADT/WDT via its
  own `loadlib` (`contrib/extractor/loadlib/adt.cpp,wdt.cpp`), writes
  `.map` files with the same magics/headers as the loader
  (`contrib/extractor/System.cpp:239-243,288-290,328-343`) and extracts
  DBCs (`dbcfile.cpp/h`). Height quantization choice (u8/u16/float) is
  made here (`System.cpp:245-253`).
- `contrib/vmap_extractor/vmapextract/` — extracts raw `VMAPs05` model
  geometry from MPQs (ADT/WMO/M2 parsers: `adtfile.cpp`, `wmo.cpp`,
  `model.cpp`, `vmapexport.cpp`, gameobject model list writer
  `gameobject_extract.cpp`).
- `contrib/vmap_assembler/vmap_assembler.cpp` — thin driver over the
  core's own `vmangos/src/game/vmap/TileAssembler.cpp`, which consumes raw
  files (`TileAssembler.cpp:527`) and writes final `VMAP_7.0`
  `.vmtree`/`.vmtile` + copied `temp_gameobject_models`
  (`TileAssembler.cpp:106,116,163,166,190,343-347`). Because writer and
  reader are the same source tree, format drift is impossible until
  someone edits one side.
- `contrib/mmap/` — navmesh generator (`src/MapBuilder.cpp`,
  `TerrainBuilder.cpp`, `TileWorker.cpp`, `generator.cpp`) plus
  `mmap_extract.py` driver, `config.json`, `offmesh.txt`; CLI documented in
  `contrib/mmap/readme` (`--offMeshInput`, `--maxAngle`, `--tile`, …).
  It writes the `MMAP` v6 tiles (§4) and validates
  `dtVersion == DT_NAVMESH_VERSION` on existing tiles
  (`contrib/mmap/src/TileWorker.cpp:364`).
- Legacy/unused: `contrib/dbcEditer/` (Delphi/VCL sources + committed
  `pjDbcEditer.exe`), `contrib/dbcformat/` (`DBC Store.exe` + `dbc.desc`
  field descriptions), `contrib/vmap_reader/`, `contrib/model_reader/` —
  Windows-era utilities, not in the CMake build list above.

**Rust implication (toolchain):** extractors do **not** need porting for a
Rust server to run — they are offline, one-way producers, and their outputs
are reusable (§1–§4). Port them only if the goal is a Rust-only toolchain;
if so, `TileAssembler` + BIH + `ModelSpawn` serialization are shared with
the runtime and should be one Rust crate used by both. The mmap generator
is the expensive one (full Recast pipeline).

## 7. Configuration files

Parser: `vmangos/src/shared/Config/Config.{h,cpp}` (singleton `sConfig`,
`Config.h:57`; used by both `mangosd` and `realmd`).

- Format: hand-rolled line parser, **not** INI. `Name = value` per line;
  `#` starts a comment (and aborts the line if hit while reading the name);
  `[` (section) aborts the line; values may be `"`-quoted; a second `"`
  terminates the value; names/values with embedded spaces before `=` are
  mangled (space ends name accumulation silently — spaces are skipped, not
  trimmed into the token) (`Config.cpp:69-153`). Duplicate keys: first
  wins, second prints a warning (`Config.cpp:146-150`). Lines capped at
  1024 bytes (`Config.cpp:51-55`). Empty value → line rejected
  (`Config.cpp:143-144`), so "set to empty" is indistinguishable from unset.
- Storage/typing: `unordered_map<string,string>` behind a
  `shared_timed_mutex` (`Config.h:50-54`); typed getters with caller-side
  defaults — `GetStringDefault/GetBoolDefault/GetIntDefault/GetFloatDefault`
  (`Config.cpp:175-200`). Bool accepts `true/TRUE/yes/YES/1`
  (`Config.cpp:187`). Int/float use `std::stoi/std::stof` — malformed
  values throw (uncaught in getters). There is **no central registry of
  option names/types/defaults**; every default lives at its call site
  (mangosd funnels most through `World::LoadConfigSettings`,
  `World.cpp:426`, into a `WorldConfig` enum-indexed array).
- Option counts (from the committed examples, uncommented
  `^[A-Za-z][\w.]*\s*=` assignments): `mangosd.conf.example` — 3,244 lines,
  **648 options**; `realmd.conf.example` — 204 lines, **33 options**
  (`vmangos/run/etc/`). Live `*.conf` files are gitignored and were not
  read.
- Reload: `Config::Reload()` re-reads the same filename
  (`Config.cpp:42-59`); some keys (e.g. `DataDir`) explicitly refuse to
  change on reload (`World.cpp:915-916`).

**Rust implication (config):** keep the file format byte-compatible
(operators have tuned production `mangosd.conf`s). The quirks to replicate
or consciously fix: first-wins duplicates, `#`/`[` line abortion, quote
termination, empty-value rejection, 1024-byte line cap. Defaults must be
harvested from ~hundreds of C++ call sites — the `.example` files (648+33
keys) are the practical checklist, cross-checked against
`LoadConfigSettings`.

## 8. `scripts/*.py` data generators

- `scripts/dbc.py` (32 lines): standalone LE DBC reader — asserts `WDBC`,
  unpacks header as `<4I`, treats every field as u32
  (`scripts/dbc.py:6-26`), string-block accessor (`:29-32`). Independent of
  the C++ fmt system (no byte/float field support — sufficient for the
  talent/spell tables it reads).
- `scripts/dump_talents.py` (166 lines): duplicates the reader inline
  (`scripts/dump_talents.py:31-49`, slightly different signature —
  `field_count`-based unpack vs `dbc.py`'s `record_size//4`), dumps
  TalentTab/Talent/Spell DBCs as a text table (`:60-166`). Hardcoded
  production DBC path `/home/wow/…/run/bin/5875/dbc` (`:8`) — confirms the
  deployed layout of §0.
- `scripts/generate_talent_builds.py` (643 lines): reads TalentTab, Talent,
  Spell DBCs via `scripts/dbc.py` (`:9-15,45,69`), maps researched talent
  builds to DBC talent IDs (`:111-120+`) and emits C++ talent-build
  definitions (docstring `:2-5`) — i.e., its output is *compiled into* the
  bot code; it is a code generator, not a runtime data source. Same
  hardcoded DBC path (`:11`). Field indexes are hand-derived (e.g. spell
  name at record field 120, `:74`).
- Tests: `scripts/tests/test_dbc.py` (120 lines) builds synthetic WDBC
  blobs (`:9-25`) and verifies header/string behavior — a ready-made
  cross-check corpus for a Rust DBC parser.

**Rust implication (scripts):** none are on the server's runtime path.
`dbc.py`/`test_dbc.py` serve as a second implementation/oracle for the
DBC format. `generate_talent_builds.py` would need retargeting (emit Rust
or a neutral data file) only when the bot talent tables are regenerated.

## 9. Summary: format-by-format Rust reuse verdict

| Format | Magic / version | Endian & packing | Reuse existing files in Rust? |
|---|---|---|---|
| DBC (`.dbc`) | `WDBC`, no version; field-count check | Explicit LE (`EndianConvert`); packed repack in memory only | **Yes, unchanged.** Parser is small; port 46 fmt strings + locale string merge. |
| Terrain (`.map`) | `MAPS` + `z1.4`; section fourccs | No conversion; LE + natural alignment assumed | **Yes, unchanged** (LE targets). Port 3 height encodings bit-exactly. |
| VMap (`.vmtree/.vmtile`, models) | `VMAP_7.0` (raw: `VMAPs05`); chunk tags | No conversion; field-wise f32/u32 stream | **Yes, unchanged** (LE). Must port BIH wire tree + spawn format + G3D-style math. |
| `temp_gameobject_models` | none | No conversion; EOF-terminated records | **Yes, unchanged.** Trivial parser; add EOF/bounds checks. |
| MMap params (`.mmap`) | none | Raw `dtNavMeshParams` struct dump | **Fragile.** Parse field-wise; layout tied to vendored Detour headers. |
| MMap tiles (`.mmtile`, `go*.mmtile`) | `MMAP` u32 + version 6 (20-B header) | No conversion; opaque Detour payload | **Only via compatible Detour** (FFI) or a Detour-format reimplementation; otherwise regenerate. |
| Config (`.conf`) | n/a (text) | Hand-rolled parser quirks | **Yes, unchanged** if parser quirks replicated; 648+33 keys. |
| World DB (MySQL) | row-level `patch/build` gating | SQL, positional field access | **Yes, unchanged DB.** Port = query/row-mapper transcription + load order. |

Cross-cutting notes:

- Only DBC loading does endian conversion; every other binary format is
  little-endian/native by construction. A Rust port should make LE
  explicit everywhere and compare magics as bytes.
- No format uses compression, checksums, or alignment padding *by design*;
  where C++ dumps whole structs (`.mmap` params, map section headers) the
  absence of padding is incidental and unverified by static asserts.
- Versioning is weakest where it matters most: `.mmap` has none, DBC has
  only field-count, `temp_gameobject_models` has none; `.map` (`z1.4`) and
  vmap (`VMAP_7.0`) embed the version in the magic itself.
- Constants disagree with reality in one place (`DBCFilesCount 45` vs 46
  active loads, `DBCStores.cpp:184,223-225`) — harvest behavior from code
  paths, not count constants.
- Nothing in this scope requires regenerating client-derived assets for a
  Rust core except navmeshes if Detour is not carried over via FFI.

## 10. Evidence gaps

- No extracted assets exist in this checkout (§0), so no real
  `.dbc/.map/.vmtile/.mmtile` bytes were inspected; all format claims rest
  on paired loader+writer source. Fixture files from a production
  `DataDir` are the missing verification step.
- `GroupModel::readFromFile/writeToFile` bodies (`WorldModel.cpp:271,330`)
  were located but not line-audited here; per-field mesh/liquid layout
  inside `GMOD` groups is the one vmap sub-format not fully transcribed.
- The world (`mangos`) DB schema file is not in-repo; table shapes are
  evidenced from C++ queries in `ObjectMgr.cpp`/`SpellMgr.cpp` only.
- Detour's internal tile payload layout (vendored `recastnavigation`
  version, `DT_NAVMESH_VERSION` value) was not read; §4 treats the payload
  as opaque, which matches how `MoveMap.cpp` treats it.

