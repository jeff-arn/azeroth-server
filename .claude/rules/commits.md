# Commit message rules

This project uses Conventional Commits 1.0 (https://www.conventionalcommits.org/) with project-specific scopes. The `commit-msg` hook in `.githooks/commit-msg` enforces the format. The `scripts/release.sh` script derives version bumps from commit types.

## Format

```
<type>[(<scope>)][!]: <subject>

[optional body]

[optional footer]
```

### Type (required)

One of:

- `feat` — new feature
- `fix` — bug fix
- `perf` — performance improvement
- `refactor` — code change that neither fixes a bug nor adds a feature
- `docs` — documentation only
- `style` — formatting / whitespace / clang-format
- `test` — adding or fixing tests
- `build` — build system or external dependencies
- `ci` — CI configuration
- `chore` — maintenance, tooling, dependency bumps
- `revert` — reverts a previous commit

### Scope (recommended, optional)

One of: `vmangos`, `dashboard`, `discord-bot`, `scripts`, `db`, `bots`, `docs`.

The scope tells readers (and `release.sh`) which subsystem changed.

- `vmangos` — C++ game server source. The big one — covers PartyBot, GuildBot, CrowdBot, BotDirector, AHBot, and all the formation/social/services code unless you specifically scope `bots`.
- `bots` — bot AI changes specifically (use when the change is squarely in `PartyBotAI`, `BotBehavior*`, `BotFormation`, `BotChatManager`, `BotServiceMgr`). When in doubt, `vmangos` is fine.
- `dashboard` — Flask dashboard
- `discord-bot` — Discord bot
- `scripts` — `scripts/*.sh`, `scripts/*.py`
- `db` — SQL schema or migrations
- `docs` — `docs/`, `CLAUDE.md`, `.claude/rules/*`, `README.md`

### Subject (required)

- Imperative mood: `add ...`, `fix ...`, `remove ...`. Not "added", "adds", "adding".
- Lowercase first letter
- No trailing period
- Max 72 characters
- Concrete: `fix priest shield not casting on warrior below 15%` is better than `fix priest shield bug`

### Body (optional)

- Wrap at 72 columns
- Separated from subject by a blank line
- Explains **why**, not what. The diff shows what.
- Reference issues or PRs (`Refs #42`) or commits (`Reverts a1b2c3d`) where relevant.

### Footer (optional)

- `BREAKING CHANGE: <description>` — required when the commit breaks an existing API, packet shape, DB schema, or config key. Also goes with a `!` after the type/scope.
- `Refs #<issue>` or `Closes #<issue>` for issue tracking.
- No `Co-Authored-By:` trailers by default. AI assistance is implicit in this project.

## Breaking changes

A change is breaking when it requires consumers to do something they didn't have to do before. Examples:

- Renaming a dashboard API field
- Changing a config key in `mangosd.conf` or `realmd.conf`
- Removing a CLI command (`.partybot`, `.guildraid`, etc.)
- Schema changes that aren't backward-compatible (which in production phase shouldn't happen — see `.claude/rules/database.md`)

Format:

```
feat(dashboard)!: rename /api/players to /api/online-players

BREAKING CHANGE: clients calling /api/players must now use /api/online-players.
```

## Version bumps (used by `scripts/release.sh`)

- `fix` → patch bump (e.g. 0.4.3 → 0.4.4)
- `feat` → minor bump (e.g. 0.4.3 → 0.5.0)
- `feat!` or any `BREAKING CHANGE:` footer → major bump, **but only when explicitly requested** (`release.sh major`)
- All other types → no version bump

The release script tags, pushes to GitHub, creates a GitHub Release with notes, and posts release notes to the Discord #patch-notes channel.

## One logical change per commit

A commit should be a single atomic change. Not:

```
feat(bots): add stealth opener, fix priest shield, update CLAUDE.md
```

Yes:

```
feat(bots): add rogue stealth opener behavior
fix(bots): exclude warrior tank from PW:S above 15% HP
docs: document stealth opener behavior in ARCHITECTURE.md
```

Three commits, three reverts available, three lines in the changelog.

## Examples

### Good

```
feat(bots): add breadcrumb trail formation system for dungeon movement

Replaces random-angle MoveFollow with role-ordered slots behind the
leader. Two-layer safety on lateral offsets (navmesh + aggro scan).
Falls back to MoveFollow when trail is empty.

Refs #18
```

```
fix(bots): exclude warrior tank from PW:S above 15% HP

PW:S threat modifier was pulling threat off warrior tanks. Applied
the gate in three places: pre-pull, in-combat critical threshold,
SelectShieldTarget.
```

```
chore(db): add personality column to guild_bot_roster

Migration is idempotent (ADD COLUMN IF NOT EXISTS). Defaults to
'chill' for existing rows.
```

```
docs: document time_t fireTime rule in bots.md
```

### Bad

```
fixed bot bug                          ← no type, "fixed" past tense
feat: stuff                            ← non-concrete subject
fix(bot): big refactor of formation    ← "fix" but actually refactor; no scope match
WIP                                    ← rejected by hook
Co-authored-by: Claude <...>           ← unnecessary trailer
update                                 ← no type, no subject
```

## When the hook rejects you

- Bypassable auto-subjects (`Merge ...`, `Revert ...`, `fixup! ...`, `squash! ...`, `amend! ...`) are accepted. Don't fight these.
- A merge commit with a non-default message must still conform.
- `git commit --amend` re-runs the hook on the new message.
- `--no-verify` exists but the security hooks (gitleaks) run via `pre-commit`, not `commit-msg`. Bypassing `commit-msg` won't bypass secret scanning. Still, don't bypass `commit-msg` casually — it exists because malformed messages break the release script.
