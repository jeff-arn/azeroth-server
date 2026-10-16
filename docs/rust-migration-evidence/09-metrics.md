# 09 — Quantitative codebase metrics (migration sizing)

Workspace: `/Users/jeffarn/Programming/azeroth-server` (VMaNGOS WoW 1.12.1, C++14, production).
Method date: this session. Tool availability: `command -v tokei cloc wc find` → only `/usr/bin/wc`
and `/usr/bin/find` present (tokei/cloc NOT installed). All numbers below come from executed
Python-3 `os.walk` + per-file line/regex counts and `wc -l` spot-checks; exact script logic is
summarised in §10 so every figure is reproducible. Exclusions in all walks: `.git`, `build`,
`venv`/`.venv`, `__pycache__`, `target`, `node_modules`, `vmangos-database`. "LOC" = physical
lines (`count("\n")` per file, cross-checked with `wc -l`, e.g. `Player.cpp` = 22443 by both).
Blank/comment split is heuristic (empty or `//`/`*`/`/*`-led line) — treat as approximate.

## 1. Total LOC by language (whole repo)

| Ext | Files | Physical LOC | Notes |
|---|---:|---:|---|
| .sql | 2519 | 847566 | world/char schema + 1038 migration files under `vmangos/sql/migrations/` + `old_migrations/`, `custom/`, `translations/` — data, not code to port line-for-line |
| .cpp | 717 | 452723 | incl. `dep/` (146 files, 82331 LOC) and `tests/` (6 files) |
| .h | 722 | 209067 | incl. `dep/` headers (329 files) |
| .hpp | 34 | 45952 | incl. vendored `src/shared/json.hpp` (14722) and `nonstd/{expected,optional}.hpp` (2445+2065) |
| .c | 31 | 18391 | all under `vmangos/dep/` (vendored) |
| .py | 22 | 4429 | dashboard + discord-bot + scripts (incl. tests); §8 breaks this down |
| .md | 32 | 4427 | docs |
| .html | 7 | 3848 | dashboard templates |
| .cmake/.txt/CMakeLists | 19+ | ~1354 | build glue (`.`txt count includes non-CMake text) |
| .sh | 7 | 312 | `scripts/backup-db.sh`, `scripts/release.sh`, hooks |
| .rs | 0 | 0 | no Rust source yet; `vmangos-rs/` contains only `target/debug/` (empty of sources) |

Rust implication: the porting surface is the C++/C code, ~680k physical lines gross, but roughly
250k of that is vendored `dep/` (§3) that should be *replaced by crates/FFI*, not transliterated.
SQL (848k lines) is schema+content: a Rust server keeps the MySQL databases; only the ~1–2k query
call sites (§6) are code work. Python (4.4k) can stay as-is (§8).

## 2. C++ LOC by `vmangos/src` subtree — the real porting surface

`vmangos/src` total (.cpp+.h+.hpp): **461753 physical lines, 535 .cpp + 404 .h/.hpp files**.
Heuristic split: ~64137 blank, ~40662 comment-led, ~356954 code. So budget on ~357k code lines.

| Subtree | .cpp | .h/.hpp | LOC .cpp | LOC .h | Total LOC | Share of src |
|---|---:|---:|---:|---:|---:|---:|
| `src/game` (incl. PlayerBots) | 236 | 256 | 240775 | 67770 | **308545** | 66.8% |
| └ `src/game/PlayerBots` | 18 | 18 | 21275 | 2979 | 24254 | 5.3% (7.9% of game) |
| `src/scripts` (instance/boss scripts) | 223 | 34 | 103742 | 3671 | **107413** | 23.3% |
| `src/shared` (DB, IO, crypto, config) | 63 | 78 | 10440 | 27617 | 38057 | 8.2% |
| `src/framework` | 3 | 26 | 302 | 2879 | 3181 | 0.7% |
| `src/realmd` (auth server) | 4 | 5 | 2291 | 591 | 2882 | 0.6% |
| `src/mangosd` (world entry point) | 6 | 5 | 1465 | 210 | 1675 | 0.4% |

Caveat: `src/shared` header LOC is inflated by vendored single-header libs living in-tree:
`vmangos/src/shared/json.hpp:1` (14722 lines, nlohmann/json), `shared/nonstd/expected.hpp` (2445)
+ `optional.hpp` (2065) = 19232 of shared's 27617 header lines. First-party shared code is closer
to ~19k lines. Similarly `framework` is mostly headers (2879 of 3181).

### 2a. `src/game` by directory (physical LOC, .cpp+.h)

Largest first — this is where a Rust rewrite spends its life:

| Dir | Files | LOC | | Dir | Files | LOC |
|---|---:|---:|---|---|---:|---:|
| game/*.cpp/h (top level) | 71 | 47155 | | Handlers | 27 | 16212 |
| Objects | 67 | 71031 | | Server | 17 | 10512 |
| Spells | 18 | 33375 | | Movement | 45 | 8812 |
| PlayerBots | 36 | 24290* | | Battlegrounds | 11 | 8678 |
| Commands | 11 | 23602 | | Chat | 14 | 7995 |
| Maps | 38 | 21013 | | AI | 41 | 7633 |

Smaller: Anticheat 6122 (19), vmap 5182 (23), Group 4024 (7), Database 2584 (11), Guild 1975 (4),
AuctionHouse 1933 (4), OutdoorPvP 1885 (5), Transports 1266 (4), LFG 1162 (6), Mail 1085 (4),
Threat 1046 (4), PacketBroadcast 465 (5).
(*PlayerBots 24290 via dir-walk vs 24254 via subtree-walk: 36-line delta from a `.hpp` counted in
one pass only — immaterial; use ~24.3k.)

`src/scripts` by region: eastern_kingdoms 54135 (146 files), kalimdor 34696 (70), world 8541 (20),
battlegrounds 5211 (2), spells 3250 (11), custom 1289 (3), PrecompiledHeaders 41 (2).

Rust implication: `game`+`scripts` = 416k lines (90% of src). `scripts/` alone (107k, 223 .cpp of
boss/instance scripts) is highly repetitive event-handler code — the strongest candidate for a
data-driven or macro-generated Rust representation rather than 1:1 translation, or for deferral
behind a scripting FFI. `realmd` (2.9k) + `mangosd` (1.7k) are trivially portable entry points;
`shared` first-party (~19k) maps onto mature crates (tokio, sqlx/mysql, serde_json replaces the
14.7k-line vendored json.hpp outright).

## 3. Vendored code — `vmangos/dep/` (EXCLUDE from porting scope)

**250245 physical lines: 146 .cpp (82331) + 329 .h/.hpp (149523) + 31 .c (18391).**

| dep dir | Files (.cpp/.h/.c) | LOC | Rust replacement strategy |
|---|---:|---:|---|
| dep/src (g3dlite etc.) | 123 | 62060 | glam/nalgebra + own collision code; audit before FFI |
| dep/include | 128 | 58498 | headers for the above (ACE-era) — drop with tokio/std |
| dep/windows | 130 | 57997 | Windows-only OpenSSL/MySQL headers — drop entirely |
| dep/gsoap | 6 | 30800 | SOAP (remote admin) — drop or tiny HTTP replacement |
| dep/recastnavigation | 30 | 19398 | pathfinding mesh — FFI candidate (`recast` crates immature) |
| dep/cpptrace | 62 | 9251 | stack traces — `backtrace` crate |
| dep/doctest | 1 | 7107 | test framework — `cargo test` |
| dep/libmpq | 20 | 4899 | MPQ archive reading — no mature crate; FFI or pre-extract |
| dep/loadlib | 6 | 741 | DBC/map loaders — rewrite (small) |

Rust implication: **do not budget 250k lines of porting here.** Real first-party C++ is
461753 (src) + ~340 (tests, §7) ≈ **462k physical / ~357k code lines**. Of `dep/`, only
recastnavigation (19k) and libmpq (5k) plausibly need FFI bindings or reimplementation;
gsoap/windows/doctest/cpptrace (~105k) vanish. The same applies in-tree: vendored
`json.hpp`/`nonstd` (~19k in shared) → serde/std `Option`/`Result`.

## 4. Largest files (concentration risk)

25 largest `.cpp` in `vmangos/src` (physical lines):

| LOC | File | LOC | File |
|---:|---|---:|---|
| 22443 | game/Objects/Player.cpp | 3479 | game/Chat/Chat.cpp |
| 12038 | game/ObjectMgr.cpp | 3201 | game/World.cpp |
| 11073 | game/Objects/Unit.cpp | 3026 | game/PlayerBots/BattleBotAI.cpp |
| 8413 | game/Spells/Spell.cpp | 2930 | game/ScriptMgr.cpp |
| 8261 | game/Spells/SpellAuras.cpp | 2902 | game/Commands/DebugCommands.cpp |
| 5806 | game/Commands/CharacterCommands.cpp | 2886 | scripts/kalimdor/silithus/silithus.cpp |
| 5805 | game/Spells/SpellEffects.cpp | 2756 | game/Commands/UnitCommands.cpp |
| 5025 | scripts/battlegrounds/battleground_alterac.cpp | 2750 | game/Commands/CreatureCommands.cpp |
| 4118 | game/Objects/Creature.cpp | 2596 | game/Objects/GameObject.cpp |
| 3888 | game/Spells/SpellMgr.cpp | 2526 | game/Maps/ScriptCommands.cpp |
| 3829 | game/PlayerBots/CombatBotBaseAI.cpp | 2485 | game/Group/Group.cpp |
| 3728 | game/Maps/Map.cpp | 2427 | game/PlayerBots/PlayerBotMgr.cpp |
| 3674 | game/Objects/Object.cpp | | |

15 largest `.h/.hpp` in `vmangos/src`: shared/json.hpp 14722 (vendored, drop),
game/Objects/Player.h 2565, shared/nonstd/expected.hpp 2445 (drop), shared/nonstd/optional.hpp
2065 (drop), game/SharedDefines.h 1841, game/ObjectMgr.h 1596, game/Maps/GridNotifiers.h 1585,
game/Objects/Unit.h 1535, game/Spells/SpellDefines.h 1268, game/Maps/ScriptCommands.h 1209,
game/Chat/Chat.h 1200, game/Language.h 1154, game/Spells/SpellEntry.h 1108, game/Maps/Map.h 1049,
game/World.h 1034.

Counts: `vmangos/src` = 535 .cpp / 404 .h+.hpp; repo-wide 717 .cpp / 722 .h / 34 .hpp (delta =
dep/ + tests/). Approx. member-function definitions in src .cpp: **8642** (regex `^…::name(`).

Rust implication: risk is extremely concentrated. `Player.cpp`+`Player.h` alone ≈ 25k lines
(5.4% of src); the top-5 .cpp (Player/ObjectMgr/Unit/Spell/SpellAuras) = 62228 lines = 13.5%.
These god-files must be *decomposed* (Player → inventory/quests/talents/combat modules), not
translated file-for-file, or the Rust crates will reproduce the same coupling. Conversely
`Commands/*` (~14k across Character/Unit/Creature/DebugCommands) is GM tooling that can be
deferred or minimally ported. `GridNotifiers.h` (1585, header-only visitor templates) and
`SharedDefines.h` (1841, enum soup) are compile-time metaprogramming/constant dumps — in Rust
they become traits/generics and `const`/`enum` modules, likely much smaller.

## 5. Unsafe / C-idiom density (regex counts over `vmangos/src`, .cpp+.h+.hpp)

Each cell: occurrences (files containing ≥1). "game" = subset under `src/game/`.

| Pattern (regex) | src total | game only | Rust reading |
|---|---:|---:|---|
| raw `new ` (`\bnew\s`) | 2332 (413) | 659 (145) | manual heap ownership everywhere; → Box/Arc/arena redesign, not syntax swap |
| `delete ` | 467 (114) | 419 (92) | paired with above; destructor-order bugs are the migration's top correctness risk |
| `delete[]` | 66 (27) | 46 (16) | → Vec |
| `new T[` array-new | 61 (29) | — | → Vec/Box<[T]> |
| `malloc` / `free(` | 1 / 1 (1 file) | 0 / 0 | excellent: essentially no C allocation in first-party code |
| `reinterpret_cast<` | 110 (25) | 76 (11) | packet/DBC byte-punning — each becomes an `unsafe` cast or a bytemuck/zerocopy type; audit all 110 |
| `static_cast<` | 747 (121) | 445 (69) | mostly benign; `as`/From |
| `dynamic_cast<` | 226 (76) | 52 (15) | RTTI downcasts — no direct Rust equivalent; forces enum/Any redesign (most in scripts/shared, only 52 in game) |
| `const_cast<` | 155 (23) | 146 (19) | const-correctness violations, 94% in game — borrow-checker friction points; each needs a mutability redesign |
| C-style cast sample* | 861 (138) | 711 (87) | `(uint32)x`, `(Player*)p` style; undercounts true total (see note) |
| `memcpy` / `memset` | 40 (17) / 62 (44) | 13 / 24 | small; slice copy / default-init |
| `printf`-family | 163 (29) | — | logging mostly via sLog instead (§6) |
| `goto` | 26 (4) | 4 (3) | negligible |
| `#define` | 1552 (471) | — | macro surface incl. 55 singleton macros (§6) |
| `#include` | 4812 total | — | ~5.1 includes/file; header graph → crate/module graph redesign |
| `extern` globals (`^extern`) | 110 (23) | — | global mutable state beyond singletons |
| raw-pointer members sample† | 319 (144) | — | `T* m_x;` fields — lifetime web the borrow checker must untangle |
| `std::unique_ptr` | 635 (134) | — | modern ownership present; maps cleanly to Box |
| `std::shared_ptr` (+boost) | 191 (41) | — | → Arc/Rc; check for cycles |
| `make_shared` | 57 (15) | — | |

\* C-style-cast regex only matched casts to a fixed type list (`uintN/int/float/double/char/void/
Unit/Player/Creature/Object/Item/SpellEntry/GameObject` with optional `*`) — 861 is a **lower
bound sample**, not a total; a clang-based count would be higher. † Regex
`^\s+\w+(::\w+)*\s*\*\s*m_\w+` — member fields only, also a lower bound.

Rust implication: allocation hygiene is better than typical legacy C++ (malloc≈0, unique_ptr
already at 635), so ownership translation is feasible; the hard 10% is (a) 2332 raw `new` sites
whose ownership must be *inferred*, (b) the cast cluster — 110 reinterpret_cast + 155 const_cast
+ 226 dynamic_cast ≈ 490 sites that each demand a design decision (unsafe block, enum, or
interior mutability), and (c) ≥319 raw-pointer member fields implying cross-object lifetimes
(Player↔Group↔Map) that will force `Arc<RwLock<…>>`, ECS-style handles (GUID lookup, which the
codebase already has via ObjectAccessor), or arena indices.

## 6. Singletons, globals, locks, virtuals (architecture-shape metrics)

**Singletons.** 55 `#define sX Singleton<X>::Instance()` macros in 52 files, 58 `Singleton<`
uses (47 files). Representative: `vmangos/src/game/World.h:1032`
`#define sWorld MaNGOS::Singleton<World>::Instance()`;
`vmangos/src/game/ObjectMgr.h:1594` (sObjectMgr); `vmangos/src/shared/Log.h` (sLog);
`vmangos/src/shared/Config/Config.h` (sConfig). Use-site census of all `\bs[A-Z]\w*` tokens in
src: **166 distinct tokens, 9539 total uses** — but this overcounts: it includes DBC store
globals (`sSpellRangeStore` etc.) and a few locals (`sName`, `sDest`). The 55 macro singletons
are the true global-service count. Top use sites:

| Token | Uses | Files | | Token | Uses | Files |
|---|---:|---:|---|---|---:|---:|
| sLog | 4054 | 239 | | sGameEventMgr | 193 | 24 |
| sObjectMgr | 1458 | 110 | | sConfig | 167 | 23 |
| sWorld | 1063 | 135 | | sBattleGroundMgr | 114 | 20 |
| sSpellMgr | 467 | 60 | | sScriptMgr | 102 | 22 |

Next tier: sMapMgr 98, sMapStorage 87, sPoolMgr 80, sObjectAccessor 72, sGuildMgr 71,
sTicketMgr 61, sAccountMgr 59; bot singletons: sGuildBotMgr 40, sPlayerBotMgr 36,
sBotServiceMgr 19, sBotChatMgr 16, sBotPatternMgr 10 (plus sBotDirector/sBotFormationMgr
defines in `game/PlayerBots/BotDirector.h`, `BotFormation.h`).

**Locks/threads.** `\w*Mutex` 94 (31 files; 91 in game), concrete mutex types
(`std::mutex|recursive_mutex|ACE_*Mutex|CRITICAL_SECTION`) 136 (55 files), lock operations
(`LockGuard|lock_guard|unique_lock|.lock(|EnterCriticalSection`) 147 (45 files; 115 in game),
thread types (`std::thread|boost::thread|ACE_Thread|pthread`) 39 (18 files). Concurrency is
real but bounded: ~150 lock sites, not thousands.

**Virtual dispatch.** `virtual` keyword: 553 (80 files; 470 in 56 game files). Only **14 files
have >10 virtuals** — the polymorphic cores: game/Objects/Object.h 40, game/Maps/ZoneScript.h 40,
game/AI/CreatureAI.h 37, game/Objects/Unit.h 36, game/ScriptMgr.h 31,
game/Battlegrounds/BattleGround.h 27, game/PlayerBots/CombatBotBaseAI.h 22,
game/Chat/AbstractPlayer.h 21, shared/Database/Database.h 17, game/vmap/IVMapManager.h 14,
game/Movement/MovementGenerator.h 12, game/Objects/{SpellCaster.h,Creature.h} 11,
game/Maps/InstanceData.h 11. Class census: 2758 `class X` occurrences (incl. forwards),
269 `class X : public …` inheritance decls.

Rust implication: (1) 55 singletons with 9539 token uses (5422 excluding sLog) are the single
largest structural blocker — Rust has no safe mutable-singleton idiom; each `sX` must become an
explicit `WorldState`/`Services` context threaded through ~135–239 files' worth of call sites
(sWorld in 135 files, sObjectMgr in 110), or `once_cell`/`LazyLock<RwLock<…>>` as a transitional
crutch. sLog (4054 uses) is the exception: it maps 1:1 onto `tracing`/`log` macros.
(2) Lock sites are few enough (147) to audit by hand, but ACE mutexes in shared/IO mean the
network layer's threading model must be re-thought around tokio tasks rather than locks.
(3) Polymorphism is concentrated, not diffuse: 14 heavy classes (Object/Unit/CreatureAI/
ZoneScript/BattleGround/MovementGenerator…) carry the vtable design — these 14 trait/enum
decisions determine the Rust object model; the other ~255 inheritance decls are mostly shallow
script subclasses that collapse into data + closures.

## 7. Cross-checks: opcode handlers, SQL call sites

**Opcodes.** `vmangos/src/game/Server/WorldSession.h`: 286 `void Handle…` declarations.
Definitions of `WorldSession::Handle…` across src: 1157 occurrences / **288 distinct names**
(includes declarations+definitions+table refs). Dispatch table in
`vmangos/src/game/Server/Protocol/Opcodes.cpp` (916 lines): 827 rows referencing
`WorldSession::Handle…` / **276 distinct handlers** — i.e. ~276 implemented client-packet
handlers behind an 827-row table (many opcodes share/null handlers). Protocol headers keep
per-patch opcode enums: `Opcodes_1_12_1.h` 831 lines / 827 `CMSG|SMSG|MSG_` tokens;
1_11_2: 815; 1_10_2: 797; 1_9_4: 783; 1_8_4: 766; 1_7_1: 758; 1_6_1: 749; 1_5_1: 746 — only
1.12.1 matters (server is 1.12.1-only per CLAUDE.md); the other 7 headers (~5400 lines) are
dead weight a Rust port drops. Handler interface types live in
`vmangos/src/game/Server/Protocol/Opcodes.h` (`OpcodeHandler` ×8, `opcodeTable` ×3).

**SQL.** Broad census over src: `Execute(` 735 (108 files), `Query(` 574 (94), `PExecute` 405
(48), `PQuery` 266 (51) — broad regexes overcount (any `.Execute`/local `Query`). Anchored to
DB handles: `CharacterDatabase.` 653 (47 files), `WorldDatabase.` 324 (35), `LoginDatabase.`
143 (18); strict `(Character|World|Login|Logs)Database.(Execute|Query|PExecute|PQuery|Async|
Direct)` = **831 call sites**. True SQL surface is ~800–1100 sites depending on whether async/
prepared wrappers are counted; quote **~831 anchored** as the defensible floor and ~1300
(735+574 broad) as ceiling. All string-formatted (`PExecute` = printf-style — injection-shaped,
needs parameterisation in any port).

Rust implication: ~276 packet handlers is a *finite, enumerable* work list — a Rust
`match opcode` dispatcher with 276 arms, each handler a function; the 827-row C++ table becomes
compile-time exhaustive. SQL at ~831–1300 sites across only ~100 files is similarly enumerable:
sqlx with compile-time-checked queries is viable, but every `PExecute` printf-format string
(405+266 sites) must be converted to bound parameters by hand — budget it as ~670 manual
conversions, and note production-phase rules (append-only migrations, sacred player data) mean
the schema itself does not change.

## 8. Tests and Python

**C++ tests exist but are tiny.** `vmangos/tests/`: 6 .cpp totalling ~340 lines —
test_breadcrumb_trail.cpp 135, test_behavior_scores.cpp 123 (122 by `wc -l`),
test_chat_cooldown.cpp 48, test_stubs.cpp 17, test_canary.cpp 13, test_main.cpp 5 — plus
CMakeLists.txt (47) declaring `vmangos_tests` (doctest, `vmangos/tests/CMakeLists.txt:1`:
"vmangos_tests — doctest-based unit tests for pure C++ logic") which links the **whole `game`
static lib** ("bot AI sources are entangled with Player/Map/World headers"). TEST_CASE files: 4
(+ doctest.h itself). No gtest/Boost.Test. Test:code ratio ≈ 340 : 461753 ≈ **0.07%**.

**Python: 22 files, 4429 lines total.**
- dashboard: 7 files / 1768 LOC — dashboard/dashboard.py 936, auth.py 64; tests 768
  (test_api_shapes.py 419, test_integration.py 187, test_auth.py 76, test_helpers.py 59, conftest 27).
- discord-bot: 6 files / 962 LOC — bot.py 461, post_patch_notes.py 55; tests 446
  (test_commands 195, test_helpers 152, test_levelup_parser 68, conftest 31).
- scripts: 6 files / 1180 LOC — generate_talent_builds.py 643, dump_talents.py 166, dbc.py 32;
  tests 339 (test_migrations 219, test_dbc 120).
- Remainder 518 LOC: `vmangos/dep/libmpq/bindings/python/mpq.py` 322 (vendored), `vmangos/contrib/mmap/mmap_extract.py` 165, `vmangos/sql/make_migration.py` 31. (`find -name '*.py' | xargs wc -l` totals 4428 vs walker's 4429 — 1-line trailing-newline delta.)

Rust implication: there is **no meaningful C++ test safety net** (0.07%) — a port cannot lean on
existing tests for behavioural parity; the budget must include building golden tests (packet
captures, spell-calc fixtures) *before* translating Player/Spell/Unit. The doctest setup's own
comment (tests link all of `game` due to entanglement) is direct evidence of the coupling a Rust
crate layout must break. Python services (4.4k LOC, ~35% tests — the best-tested part of the
repo) should be left alone; they talk to MySQL/HTTP and are unaffected by a C++→Rust move except
via the shape-stable dashboard JSON API.

## 9. Debt markers and sizing summary

TODO 231 (124 files; 153 in game), FIXME 15 (10), HACK 24 (9; 18 in game) — 270 markers total,
~0.6 per 1000 lines: low for legacy code, suggests markers are not the team's debt-tracking
mechanism (don't use them as a completeness oracle for the port).

**Sizing rollup for the guide (physical → planning numbers):**

| Bucket | Physical LOC | Porting treatment |
|---|---:|---|
| First-party C++ (`vmangos/src`) | 461753 (~357k code) | translate/redesign |
| └ vendored headers inside src (json/nonstd) | −19232 | replace with serde/std |
| └ dead patch opcode headers (pre-1.12.1) | −~5400 | drop |
| **Net first-party to port** | **≈437k physical / ~335k code** | |
| Vendored `dep/` | 250245 | crates/FFI/drop — only recast (19k) + libmpq (5k) need binding work |
| C++ tests | ~340 | rewrite as cargo tests + new golden fixtures |
| Python | 4429 | keep |
| SQL | 847566 (2519 files) | keep DB; port ~831–1300 query sites only |

Effort concentration: top-5 files 13.5% of src; game+scripts 90%; 55 singletons / 9539 uses;
~490 hard cast sites; 2332 raw-new ownership inferences; 14 virtual-heavy classes deciding the
object model; 276 opcode handlers and ~670 printf-style SQL conversions as enumerable checklists.
A line-for-line estimate at typical Rust:C++ ratios (0.7–1.0× for this template/macro-heavy
style, given 1552 `#define`s and header-template code like GridNotifiers collapse in Rust)
suggests a Rust codebase of roughly **250–340k lines** — but the binding constraints are the
singleton-threading refactor and the missing test net, not raw LOC.

## 10. Reproduction (commands actually run)

```bash
command -v tokei cloc wc find          # → wc, find only
# LOC/file census: python3 os.walk over repo (skip .git/build/venv/__pycache__/target),
#   per-file sum(1 for _ in open(p)), grouped by ext and by vmangos/src subtree
# Largest files: same walk, sort (lines, path) desc, take 25 .cpp / 15 .h in vmangos/src
# Pattern counts: python3 re.findall over every src .cpp/.h/.hpp body, patterns as tabled in §5–7
# Singleton census: re \bs[A-Z][A-Za-z0-9_]*\b Counter + #define\s+s[A-Z] + Singleton<
# Opcode cross-check: distinct WorldSession::Handle\w+ in src / in Opcodes.cpp; void Handle in WorldSession.h
# SQL cross-check: broad Execute(/Query(/PExecute/PQuery vs (Character|World|Login|Logs)Database\.
wc -l vmangos/src/game/Objects/Player.cpp dashboard/dashboard.py vmangos/tests/test_behavior_scores.cpp
# → 22443 / 936 / 122 (validates walker line counts)
ls vmangos/sql/migrations | wc -l    # → 1038
```

Unresolved/approximations: blank/comment split is line-prefix heuristic (no tokei/cloc available
to install-free verify); C-style-cast and raw-pointer-member counts are regex lower bounds (a
libclang pass would raise both); per-class virtual counts are per-file (Object.h=40 may span
>1 class — files are the unit, stated as such); SQL "broad" regexes include non-DB Execute/Query
methods, hence the 831 anchored floor is the cited figure.
