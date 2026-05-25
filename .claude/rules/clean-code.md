# Clean code rules

General code quality principles, adapted to a C++/Python game server context. The Chainsmith rules apply broadly here too — what's specific to this project is at the bottom.

## Functions

- One job per function. If the function name needs an "and", split it.
- Function depth over surface. A function that calls three well-named helpers reads better than a 200-line behemoth that does everything inline.
- Pure where possible. A function that only reads its inputs and returns a value is easier to test, easier to reason about, and easier to call from anywhere. Side effects (DB writes, packet sends, log lines, mutation of `m_` members) cluster at the boundary.
- Avoid out-parameters in C++ when a return value or struct would be clearer. The exception is multi-return where C++14 lacks structured bindings without `std::tie` boilerplate — match the surrounding code.
- Long `if/else` ladders in C++ usually want to be a table-driven dispatch. The bot AI v0.9.0 refactor replaced exactly this kind of ladder with scored behaviors — don't reintroduce ladders.

## Naming

- Names reveal intent. `BehaviorPriestShield` is good. `DoStuff` is not. `PWS_v2` is not.
- Domain language. Use WoW terms (`tank`, `taunt`, `kite`, `aggro`, `mob`, `pull`) for game logic. Use the project's terms (`PartyBot`, `CrowdBot`, `GuildBot`, `BotDirector`) for bot categories.
- Length scales with scope. A loop index inside a 5-line block can be `i`. A field on a singleton accessed from across the codebase should be fully spelled out.
- Don't encode types in names (`pCount`, `strName`, `i_value`). The exception is the established `m_` / `s_` prefixes in vmangos — keep them.

## Comments

- Comments explain why. The code already explains what.
- Keep comments accurate. A wrong comment is worse than no comment — readers trust it and act on it.
- Update comments in the same commit as the code they describe. A comment that drifted three commits ago is going to mislead someone.
- "TODO" without an owner and a real plan is a future bug report no one filed. Either do the work or open an issue.
- Don't restate the code in a comment. `// Increment the counter` next to `++counter;` is noise.

## Avoiding over-engineering

This codebase has a history of careful, minimal additions and a few specific places where over-engineering hurt:

- **No speculative abstractions.** The behavior layer is composable on purpose. You don't need a `BehaviorFactory<T>` template. Add a concrete `Behavior` subclass.
- **No premature builders.** `SituationContext` is a plain struct. It doesn't need a builder pattern. If a context becomes painful to construct, that's a sign to split the context, not to add infrastructure.
- **No wrapper types without a reason.** `BotSpellKit` is a wrapper — it pays for itself because behaviors no longer dispatch on class. If you can't articulate the payoff in one sentence, the wrapper isn't worth it.
- **No "framework for future needs".** YAGNI. If the future arrives, write it then.
- **No generalization on a sample of one.** Three concrete implementations is the earliest you should consider extracting a shared abstraction.
- **C++-specific**: prefer free functions in anonymous namespaces over `static` member functions for file-local helpers. Anonymous-namespace functions can't be accidentally referenced from another translation unit; `static` members can.

## When to break a rule

- Name the rule you're breaking
- Explain why the alternative is worse for this specific case
- Be willing to defend the choice in review

"It's faster" is a valid reason if profiled. "It looks cleaner" is not a valid reason. "It matches the surrounding code" *is* a valid reason in this codebase — VMaNGOS has a style, and consistency beats local cleanness.

## Project-specific anti-patterns

The bot stack has cost time on these:

- **Re-introducing per-class polymorphism inside behaviors.** Use `BotSpellKit`. See `.claude/rules/bots.md`.
- **Scoring a "free action" as a behavior.** Free actions (dispel, pet attack, Auto Shot) are pre-steps in the dispatcher, not scored behaviors.
- **Returning score >0 from `Score()` without checking `Execute()` preconditions.** Blocks lower-scoring behaviors. The single most common bug in the v0.9.0 refactor.
- **Generic "Manager" classes that wrap one or two methods.** A free function in a namespace is usually fine.
- **Per-class virtual methods replacing scored behaviors.** v0.8 had this. v0.9 doesn't. Don't go backwards.

## What good looks like (examples from this repo)

- `BotBehavior` — small interface, concrete subclasses do real work, no factories or registries beyond the simple list in `BotRoleConfig`.
- `BreadcrumbTrail` — single-responsibility ring buffer with `RecordPoint` and `WalkBack`. Doesn't try to also handle navmesh validation or aggro scanning; those are layered on top.
- `make_migration.py` — one job (create a timestamped SQL file), readable in under 50 lines.

## Dead code

- Commented-out blocks of code are not "in case we need it later". Git has it. Delete it.
- "Removed because X" comments rot. The git log is the canonical record.
- `// TODO` without a date or an owner is dead-code-in-progress. Either do it now or file an issue.
- Empty methods left as `{}` placeholders for `BattleBotAI` compatibility (per the v0.9.0 refactor) are an exception — the comment explaining why is in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md). Don't add more "empty for compatibility" methods without that justification.
