# C++ rules

The vmangos source is C++14 (declared in `vmangos/CMakeLists.txt`). It's a fork of MaNGOS — a long-running codebase with its own conventions that often predate modern C++ idioms. **Match the surrounding code first**, then refer to these rules for greenfield decisions.

## Standard and style

- C++14. Do not raise the standard without coordinating across the whole tree — VMaNGOS uses some older patterns (raw new/delete, manual containers in places) that need a project-wide pass to migrate.
- `clang-format` is authoritative for whitespace. The config (`vmangos/.clang-format`) sets:
  - 4-space indent
  - `BraceWrapping: AfterFunction: true` — Allman-style braces for functions, classes, namespaces
  - `PointerAlignment: Left` — `Type* ptr`, not `Type *ptr` or `Type * ptr`
  - `ColumnLimit: 999` — effectively no hard column limit, but keep lines readable
  - `NamespaceIndentation: All` — contents of `namespace MaNGOS { ... }` are indented
- `.editorconfig` sets tab width to 16. This is deliberately wide so accidental tabs are visually obvious — never commit tab indentation, always spaces.

## Naming

- Types: `PascalCase` (`PartyBotAI`, `BotBehavior`, `SituationContext`)
- Functions and methods: `PascalCase` matches existing MaNGOS style (`UpdateAI`, `FindPartyTank`, `LearnTalent`). Avoid lowerCamelCase for new code in vmangos — it clashes with the rest of the tree.
- Member variables: `m_camelCase` (`m_spells`, `m_resetSpellData`, `m_lastTaughtPetGuid`)
- Static members: `s_` prefix (`s_playerRoleAssignments`)
- Constants and enums: `SCREAMING_SNAKE_CASE` (`ACT_ENABLED`, `ROLE_TANK`, `SEC_MODERATOR`, `MOVE_PATHFINDING`)
- Files: match the primary type (`PartyBotAI.cpp` / `PartyBotAI.h`, `BotFormation.cpp` / `BotFormation.h`)

## Headers and includes

- `#pragma once` is acceptable in this fork (already used in `BotBehavior.h`, etc.). Don't introduce traditional include guards for new headers — match the surrounding files.
- Include order follows `.clang-format`'s `IncludeCategories`: system `<...>` first, then quoted `"..."` headers, then everything else. clang-format reorders these for you; don't fight it.
- Forward-declare in headers where possible. Include the full type only in `.cpp`.
- A header should compile standalone — if `Foo.h` uses `Bar`, include `Bar.h` from `Foo.h` (or forward-declare). Don't rely on transitive includes from `mangosd`-side .cpps.

## Namespaces

- Top-level vmangos code uses `MaNGOS::` for utilities (e.g. `MaNGOS::AnyHostileUnitInObjectRangeCheck`). For new helpers, follow what the neighboring file does — many bot files live in the global namespace and that's fine for now.
- Anonymous namespaces > `static` for file-local helpers in new C++14 code. Don't refactor existing `static` helpers as part of an unrelated change.

## Ownership and lifetimes

- Raw pointers are the norm in this codebase (`Player*`, `Unit*`, `Map*`). Most of these are non-owning — they refer to objects whose lifetime is managed by the World/Map/Object hierarchies.
- Do **not** sprinkle `std::unique_ptr` / `std::shared_ptr` retroactively. The existing object lifetimes are managed by VMaNGOS's own systems (object containers, sessions, map updates). Mixing smart pointers in without understanding ownership invites double-frees.
- New, self-contained data structures (e.g. a new ring buffer, an internal cache) can use `std::unique_ptr` and STL containers freely. Use modern idioms in greenfield code; preserve existing idioms in surrounding code.
- Avoid raw `new` / `delete` in new code. If you need heap allocation, prefer `std::make_unique` or container `emplace_back`.
- GUIDs (`ObjectGuid`) are values, not pointers. Use them when you need to refer to an object across ticks — pointers can dangle when sessions log out.

## Exceptions and errors

- VMaNGOS does not throw exceptions across the world tick. Don't introduce `throw` in code reachable from `World::Update`, `Map::Update`, or session packet handlers. An exception there kills the world server.
- Use return values, status enums, or `nullptr` for failure. Log meaningful diagnostics via `sLog` (see below).
- `assert` / `MANGOS_ASSERT` is for invariants that should never fail — do not use it for input validation or runtime conditions. A failed assertion crashes the server.

## Logging

- Use `sLog.outX(...)` for server logs (`outError`, `outBasic`, `outDebug`, `outString`). Don't use `printf`, `std::cout`, or `fprintf(stderr, ...)` — they bypass the log subsystem and won't appear in `Server.log`.
- Bot AI debug uses the `[BotAI]` prefix convention (see `.partybot debug` in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md)). Match the prefix style of the subsystem you're touching.
- Don't log per-tick spam. The world tick runs many times per second. If you must log something inside `UpdateAI` or similar, gate it behind a debug flag or sample it.

## Const correctness

- Methods that don't modify object state should be `const`. The existing code is inconsistent — improve const-correctness in code you touch, but don't do a project-wide pass as a side effect of an unrelated change.
- Pass non-trivially-copyable parameters by `const&` unless you specifically need a copy.

## Concurrency

- The world tick is single-threaded. Most game logic runs on the world thread. Do **not** spawn threads from inside bot AI, behaviors, or packet handlers.
- The async I/O layer (network, DB) is threaded — don't access game objects from async DB callbacks. Use the existing patterns (e.g. `CharacterDatabase.AsyncQuery` with completion handlers that the world thread picks up).

## What "good" looks like

- A new behavior file looks like the existing `BotBehaviors_Shared.cpp` entries: subclasses `BotBehavior`, implements `Score(ctx)` checking preconditions, implements `Execute()` doing the action and returning `true` on success / `false` to fall through.
- A new packet handler looks like the existing handlers in `WorldSession::HandleXxx` — read the packet, validate, mutate game state, send a response.
- A new SQL query goes through `CharacterDatabase` / `WorldDatabase` / `LoginDatabase`, prepared statements where available. No string concatenation with user-supplied data.

## What to avoid

- Don't import C++17/20 features. STL features added after C++14 won't compile.
- Don't reorganize existing files as a side effect of a small change. PR review is much easier when the diff is the actual change, not the change plus a reformat.
- Don't add abstract base classes "for future flexibility" — see `.claude/rules/clean-code.md`. The bot AI used to have per-class polymorphism; the v0.9.0 refactor replaced it with scored composition for a reason.
- Don't `using namespace std;` in headers. In .cpp files, scoped `using std::vector;` is fine if it improves readability.
