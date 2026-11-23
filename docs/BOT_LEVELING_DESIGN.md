# Bot World Leveling Design: Zone Skills, Planner, and Hill-Climb Evals

Status: brainstorm design, pre-implementation. Captured 2026-11-22 for later
pickup. Decisions marked **Decided** were settled in discussion; the
[Open Questions](#open-questions) section lists what is deliberately not
settled yet.

## Goal

Bots should make up a large chunk of the playerbase and genuinely live in
the world: questing through zones at level-appropriate pace, grouping for
shared objectives, and clearing dungeons in assigned roles. A single AI
that understands "level from 1 to 60" across every zone is not tractable,
so competence is made *local*: bots are good at operating inside one zone
at a time, and zone transitions are cheated (bots are moved, equipped, and
handed the next zone's data) rather than navigated honestly.

The system is tuned by an eval bench running the real server. Zone evals
measure how effectively a bot works a sequence of quests from a zone's
lowest to highest recommended level. Dungeon evals measure how well a
trinity party (1 tank / 1 healer / 3 DPS) clears all bosses, optimizing
for as close to zero deaths as possible.

## Relationship to existing systems

This design covers the *world-level* bot layer (where bots go, what they
pursue, how the population is managed). It composes with, and does not
replace, the layers already described elsewhere:

- **Combat execution** — the behavior-layer system in
  [BOT_ARCHITECTURE_PROPOSAL.md](BOT_ARCHITECTURE_PROPOSAL.md) (situation /
  behavior / execution layers). In this design it backs the Combat skill.
- **Party follow/formation** — the Bot Formation & Movement System in
  [ARCHITECTURE.md](ARCHITECTURE.md). Ad-hoc parties and dungeon parties
  build on it.
- **Population placement** — `BotDirector` / `DynamicBotAI`
  ([ARCHITECTURE.md](ARCHITECTURE.md#bot-director)) currently places
  wandering passive/aggressive bots around human players. How the fidelity
  tiers below interact with or supersede DynamicBotAI wandering is an open
  question.
- **Persistent identities** — the Guild Bot System already proves
  persistent bot characters with saved gear. Leveling bots are persistent
  in the same sense.
- **Crowd bots** — static city scenery. Unaffected by this design.

## Approaches considered

Three directions were considered for the bot AI methodology:

- **A. Authored playbooks per zone and dungeon.** Hand-built quest order,
  grind spots, and pull-by-pull dungeon scripts. *Rejected as the primary
  approach:* a version of this has already been tried in bench testing and
  proved extremely brittle. Authoring also scales linearly with zones.
- **B. Reusable skills, zones as data.** Generic skills (accept quest,
  travel to hotspot, kill objective, vendor, recover) driven by
  per-zone data extracted from the world DB. Adding a zone is mostly new
  data, not new AI.
- **C. Learned layer on top of B.** The structure stays authored; tunable
  parameters (scoring weights, thresholds) are hill-climbed against eval
  results on the real server.

**Decided: C on top of B.** Generalization is the requirement A failed.
The learned layer tunes *how* generic skills execute; it must not
rediscover *what* to do per zone. If hill climbing starts encoding
zone-specific knowledge into global weights, A has been rebuilt with extra
steps. Discipline: parameters are global or per-class — never per-zone.
Zone differences live in extracted data, not weights.

## Standing assumptions

Settled early, and load-bearing for everything below:

- **Bot identity is real and persistent.** A bot is a real character that
  keeps level, gear, and quest history, so the world feels persistent.
  Population re-balancing (creating, retiring, or moving bots between
  zones) is allowed as needed.
- **Cheating may be visible.** The likely audience is a single human
  player, and most bots are unobserved at any moment. Teleporting between
  zones, swapping talent templates on dungeon entry, generating gear
  consistent with a bot's history — all acceptable. This permission does a
  lot of work in the design.
- **The eval bench never touches production data.** Bench characters live
  in a bench `characters` database on a bench server instance. This follows
  the production-phase rule that real character data is sacred
  (see [CLAUDE.md](../CLAUDE.md)); a learning loop writing thousands of
  deaths and quest completions into the live world is not acceptable.
  Bench *results* transfer; bench *characters* do not.

## Core architecture

Three layers. Only the middle one is the skill library proper.

```
┌─────────────────────────────────────────────────────────┐
│                       PLANNER                           │
│  Decides WHAT to do next: which quests, which trip,     │
│  when to vendor, grind, or leave the zone.              │
│  Scores candidates against extracted zone data and      │
│  bot state; re-plans whenever a skill reports back.     │
└────────────────────────┬────────────────────────────────┘
                         │ skill invocations
                         ▼
┌─────────────────────────────────────────────────────────┐
│                        SKILLS                           │
│  Decide HOW: Travel, Engage, Combat, Recover,           │
│  KillObjective, TownTrip, DeathRecovery, ...            │
│  Each has preconditions, a success condition, a         │
│  timeout, named failure reasons, and is interruptible.  │
└────────────────────────┬────────────────────────────────┘
                         │ primitives
                         ▼
┌─────────────────────────────────────────────────────────┐
│                     SERVER PRIMITIVES                   │
│  Pathfind to a point, cast, loot, interact with NPC.    │
│  Provided by the existing game server / bot stack.      │
└─────────────────────────────────────────────────────────┘
```

### Zone-as-data

A zone is data fed to the skills and planner, extracted automatically
from the world DB:

- **Quest graph** from quest templates: objectives, mob/item IDs, counts,
  chain prerequisites, accept/turn-in NPCs, level requirements, XP and
  item rewards.
- **Hotspots** by clustering creature and gameobject spawn coordinates
  for the mobs/objects a quest references.
- **Hubs**: quest-giver clusters, vendors, repair, flight paths, inns.
- **Unimplementable quests flagged at extraction** where possible:
  bespoke mechanics ("use item X on mob Y at low health") that no skill
  covers. The planner skips these. A bot that completes 90% of quests
  beats a bot wedged forever on one weird quest.

**Implemented 2026-11-23.** [`scripts/extract_zone_data.py`](../scripts/extract_zone_data.py)
reads the world DB and emits one JSON document per zone
([`data/zones/silverpine.json`](../data/zones/silverpine.json), schema
version 1): quests with chain edges, start/end NPCs and spawn points,
objectives classified into skill-shaped kinds (`kill`, `collect_drop`,
`collect_object`, `interact_go`, `deliver`, `talk`, `explore`) with
clustered hotspots, per-quest implementability with named skip reasons,
hub clusters with NPC services (vendor/repair/flightmaster/innkeeper/
trainer), grind-spot candidates from dense spawn cells, and a list of
out-of-zone quest references. Notable extraction rules: quest membership
is `ZoneOrSort`, spawn membership is an empirically fit coordinate bbox
per zone, item sources resolve through direct **and reference** loot
templates and then through `creature_template.loot_id` / chest `data1`
to spawnable entries, courier quests (`ReqItemId == SrcItemId`) classify
as `deliver` rather than collect, and quests whose starter or turn-in
only spawns outside the zone bbox are marked as needing review for a
zone-local run. Current results after those rules were tightened during
the Barrens pass: Silverpine has 42 quests, **22 zone-local
implementable / 20 need review**; the Barrens has 98 quests, **62
zone-local implementable / 36 need review**. The Barrens review bucket
is dominated by capital/cross-zone endpoints and genuinely scripted or
summoned objectives (Verog, Echeyakee/Ishamuhale, item-started beast
quests), while Silverpine loses several Undercity apothecary chains to
the endpoint rule. Known gaps carried forward: explore-trigger
coordinates live in `AreaTrigger.dbc`, not the world DB; escort quests
are flagged (`quest_start_scripts`) rather than positively identified;
each future zone needs its bbox fitted; and rectangular bboxes can
contaminate grind spots along non-rectangular borders (the Barrens box
clips parts of Durotar), so a polygon/DBC zone-membership source will
eventually be better than boxes alone.

Published player leveling/zone guides were considered as a supplement
(quest *ordering* priors). Two constraints apply if and when guides are
used: guides are copyrighted works — encoding a quest *sequence* as data
is acceptable, copying guide *text* into the repo is not — and project
source rules allow wowhead-classic / vanilla-wiki style sources, never
private-server forums (see [CLAUDE.md](../CLAUDE.md)). With the greedy
planner decision below, guide data drops out of the first version; DB
extraction alone feeds the planner. Guides remain an optional later input
if travel inefficiency shows up in metrics.

## The planner

The planner decides what to do next. It plans in **trips, not quests**:
the natural unit of vanilla questing is the hub round-trip — turn in,
accept everything suitable, execute a cluster of objectives, return. Its
main decision is "what is my next trip" (which hub, which objectives
batched, in what visit order). Within a trip, ordering hotspots is a
small routing problem. Between trips it re-evaluates from scratch. It
never commits to a full-zone sequence in advance; deaths, full bags, and
level-ups constantly invalidate long plans. Short horizon, frequent
re-planning.

### Candidate generation and filtering

On every re-plan (a skill finishes or fails, bags fill, the bot levels),
the planner builds candidates from current bot state: completed quests
awaiting turn-in, objectives for held quests, new pickups at nearby hubs,
maintenance needs (vendor, repair, training), grind spots, and the
zone-exit option. Hard filters remove the impossible first: wrong
faction, unmet prerequisites, quest log full (vanilla caps at 20),
elite/group quests the bot cannot solo, and quests flagged
unimplementable at extraction.

### Scoring

Surviving candidates are scored; the trip built is the one batching the
best-scoring nearby objectives. Scoring inputs:

- Travel **time** to the hotspot (not raw distance).
- XP value: quest reward plus expected kill XP en route.
- Level fit — legitimately per-class (a +2 quest is poor for a warrior,
  fine for a hunter).
- Overlap: several objectives in one hotspot beat one slightly better
  quest across the zone.
- Risk: mob level differential, elite density, and learned per-quest
  outcomes (below).
- Congestion: how many friendly bots already target this hotspot.

**Grind filler is a first-class candidate.** Vanilla has genuine XP gaps.
When no quest candidate scores above a threshold, the planner selects a
grind hotspot (scored by mob density, level, safety per class) and grinds
until new quests unlock or held quests become viable.

**Zone exit is just another candidate.** At the zone's exit level its
score rises until it wins: finish valuable turn-ins, abandon the dregs,
hand off to the next zone's graph via a cheated transition. No
special-case code path.

### Decided: greedy, with guardrails

The planner is **greedy**: it picks what scores best now and holds nothing
for future payoff. Guide-like lookahead (taking low-value quests because
a chain pays off later) was considered and deferred:

- Greedy decisions pay off immediately, so parameter changes show up in
  short evals. Lookahead pays off hours later — nearly impossible to
  attribute in noisy real-server hill climbing. Greedy is the more
  learnable design.
- Greedy sidesteps guide-like costs: dead-end chains terminating in
  elite/bespoke quests, quest-log pressure from held quests, chains that
  cross zone boundaries (the architecture is zone-local; chains are not),
  and harder debugging (justifications hidden several quests in the
  future).
- Greedy still follows most chains naturally: the next chain step is
  usually offered by the same NPC at turn-in, where accepting is free.
  What greedy refuses is *sacrificing* for a chain. The loss is narrow —
  a handful of big item-reward chains — and can be addressed later with a
  surgical whitelist (extraction can detect class-relevant item rewards)
  rather than a redesign.
- Lookahead remains reversible: it would arrive as one added scoring term
  (unlock value with bounded depth and decay) with a tunable weight.

Three cheap guardrails ship with greedy from the start:

1. **Anti-churn hysteresis.** A quest with partial progress gets a large
   keep-bonus. Abandoning 8/10 kills to chase a marginally better score
   is the classic greedy failure.
2. **Abandon blacklist.** A dropped quest goes on cooldown so the bot
   cannot loop accept–drop–accept at the same hub.
3. **Honest travel costing.** Greedy ping-ponging is mostly a
   travel-weight problem. With travel time costed correctly, overlap
   batching wins and ping-ponging largely disappears.

### Learned per-quest cost estimates

Every bench run records per-quest outcomes: completion time, deaths,
failures. Those statistics feed back into scoring as zone data — a second
kind of learning besides parameter tuning. A quest that kills priests
repeatedly gets its risk raised for priests until they are a level
higher. Failure handling uses the same data: a quest that fails N times
(tunable) is deferred, then abandoned, rather than becoming a permanent
wedge.

### Planner tunables

Roughly 12–15 numbers: the scoring weights themselves, quest level window
per class, abandon-after-N-failures, grind threshold, congestion penalty
strength, anti-churn keep-bonus. (Guide-adherence and lookahead weights
join this list only if those features return.)

**Reference slice implemented 2026-11-23.**
[`scripts/bot_sim.py`](../scripts/bot_sim.py) is a Python reference
implementation of the planner + skills over the extracted Silverpine
data with a deterministic simulated world (closed-form combat, euclidean
travel, per-hotspot respawn timers; XP from the world DB's
`player_xp_for_level` and quest `RewXP`). It emits the structured event
log the bench design calls for, including top-scored candidates on every
plan, and serves as the behavioral spec/oracle for the Rust port into
`mangos-bots` (production now runs the Rust server; the C++ tree is not
the target). Current results at seed 42 after the extractor's
zone-local endpoint tightening: Silverpine level 11 → 20 gives warrior
14 quests / 6 deaths / 12.2 sim-hours, hunter 13 / 2 / 11.4, priest
14 / 8 / 13.2. The same unmodified `bot_sim.py` on the Barrens data,
level 10 → 20, gives warrior 32 quests / 13 deaths / 12.4 sim-hours,
hunter 33 / 6 / 11.9, priest 32 / 16 / 13.8.
Two lessons were learned the hard way and are now part of the design:
**(1) the risk/survival term in scoring is not optional** — without it
the greedy planner repeatedly works quests whose mobs outlevel the bot
by 3–5 and dies on schedule; candidates whose expected per-pull HP cost
approaches lethal are deferred until the bot levels. **(2) grinding
needs a circuit breaker** — a grind that kills the bot twice aborts and
steps the target mob level down (floor −4), or the bot corpse-loops
forever (the pre-fix priest died 216 times, 209 of them grinding).
Kill counts remain grind-heavy (~1,000–1,100 in the Barrens and
~1,600–1,850 in Silverpine per run): each zone's zone-local implementable
quests only fund part of the run to 20, which is vanilla-accurate
pressure toward zone transitions, and an argument for measuring zone
traversal to a realistic exit level rather than 20. Trace audits also
show the current reference planner is greedy per quest with overlap
scoring; it usually holds one active quest at a time. True hub-trip
batch acceptance is therefore still an implementation gap against the
planner design above, to close in the Rust port or a later reference
revision.

## The skill library

Each skill is a goal with a contract: preconditions, a success condition,
a timeout, and **named failure reasons** (those reasons are what the bench
logs record, and what make hill-climb regressions diagnosable). Every
skill must be **interruptible with a clean handoff** — Travel interrupted
by combat, Recover interrupted by an add — because most bot deaths in
practice happen at skill boundaries, not inside skills.

First skill set, sufficient to quest a zone end to end:

- **Travel** — get from A to B over distance. Handles mounting, flight
  paths, hearthstone. Fails on stuck; interrupts when attacked.
- **Engage** — pick a target and start the fight on the bot's terms. Owns
  pull distance and adds-awareness.
- **Combat** — the class rotation. The one skill that is class-specific
  all the way down; treated as its own sub-library per class (backed by
  the behavior layers in
  [BOT_ARCHITECTURE_PROPOSAL.md](BOT_ARCHITECTURE_PROPOSAL.md)).
- **Recover** — eat, drink, bandage, rebuff, feed/summon pet. Must
  interrupt cleanly when attacked mid-drink.
- **KillObjective** — go to a hotspot, kill until count met. Owns roaming
  between spawn clusters and patience when spawns are sparse or camped.
- **CollectObjective (drops)** — kill-for-drops; drop-rate variance is
  the dominant failure mode.
- **CollectObjective (objects)** — ground-object gathering; navigation is
  the dominant failure mode. (Two skills wearing one name; keep separate.)
- **InteractObjective** — go somewhere, use an object or quest item,
  defend while doing it.
- **EscortObjective** — follow and protect an NPC. The Barrens has them;
  weird enough to be its own skill.
- **QuestAdmin** — accept and turn in at a hub, batching everything
  suitable in one stop.
- **TownTrip** — vendor, repair, restock food/water/ammo/reagents, train
  new spells on level-up.
- **DeathRecovery** — corpse run versus spirit healer; how many failed
  corpse attempts before giving up.
- **Disengage** — flee or reset a pull gone wrong instead of dying in
  place.

### Skill tunables

Candidate pool (all global or per-class, never per-zone):

- Engage: minimum health/mana to start a pull, max adds tolerated before
  disengaging, pull-back distance for ranged.
- Combat: cooldown-use thresholds, interrupt priority, mana conservation
  point. Per class/spec.
- Recover: drink-below X% mana, eat-below Y% health, skip recovery before
  trivial pulls.
- KillObjective: roam radius around a hotspot, respawn wait before moving
  clusters.
- TownTrip: bag-fullness % trigger, repair durability threshold,
  consumable stock levels.
- Travel: distance where mounting is worth it, distance where a flight
  path beats walking.
- DeathRecovery: max corpse attempts before spirit healer.

If a desired behavior cannot be expressed by moving one of these numbers,
it is a code change to a skill, not a tuning problem. That boundary is
worth policing: every added parameter makes the climb slower and noisier.
Total system budget (planner + skills) is roughly 40–55 tunables — a good
size for black-box hill climbing against a noisy real server. Dozens of
parameters are climbable this way; thousands are not.

## Classes and specs

**Decided: three classes first — hunter, warrior, priest** — expanding to
all classes as the hill climb matures.

Rationale: the trio covers pets, ranged kiting, mana, melee positioning,
rage, and self-healing. The warrior is the least forgiving vanilla
leveler; Engage and Recover that work for a warrior work for anyone,
so melee is de-risked while those skills are still being debugged.
The trio is also a dungeon skeleton (warrior tanks, priest heals, hunter
DPS), so the first 5-man bench party needs only two DPS slots filled, not
a new class raised from zero.

### Specs: leveling versus dungeon

**Decided: tune leveling specs for zone work; dungeons get a different
spec set.** Obvious pairings:

- Warrior: Arms leveling → Protection for later tanking.
- Priest: Shadow leveling → Holy for later healing.
- Hunter: Beast Mastery leveling (still fine for early dungeon DPS) →
  Marksmanship as the later divergence.

Only one leveling spec per class to start. Note how much specs diverge is
level-dependent: at Wailing Caverns range a shadow priest heals fine and
an arms warrior tanks with a weapon swap. The divergence is really a
level 40+ phenomenon, so early dungeon work may need only *role parameter
blocks* (tank stance/threat rotation, healing behavior) layered onto
leveling specs, with full dungeon specs arriving when content demands
them.

Vanilla has no dual spec, but the respec problem mostly evaporates:
identity is persistent and cheating is allowed, so a bot entering a
dungeon through Party Finder has its talent template swapped as part of
role assignment. In the bench, snapshots carry the correct template per
context (leveling spec for zone evals, dungeon spec for dungeon evals)
on the same character record.

### Class expansion discipline

Each new class adds its own Combat parameter block while shared skill
parameters stay frozen. If a new class seems to need a shared parameter
moved, that becomes either a per-class override of that one parameter, or
a re-climb gated by re-running existing classes' evals as regression
tests. Without that gate, class nine quietly breaks class one.

Expansion order roughly follows new-mechanic cost: warlock (reuses pet
machinery), mage (kiting, conjured consumables), rogue (energy, combo
points, stealth), druid (forms), shaman (totems). Paladin cannot exist in
Horde zones in vanilla, so full class coverage eventually forces an
Alliance zone pair too — worth knowing now, not worth solving now.

## Proving grounds

**Decided: prove on Silverpine Forest first, then the Barrens.**

- **Silverpine Forest** — compact: one hub (the Sepulcher), a north–south
  corridor, 10–20 band. Same faction as the Barrens, so generic skills
  carry over unchanged and failure differences isolate zone shape.
- **The Barrens** — the sprawl stress test: three hubs, enormous travel,
  escort quests, out-and-back quest design, notorious XP gaps (grind
  filler gets exercised).

The test of direction B: the Barrens should work with **only new data and
no new skill code**. If it needs new code, that is learned cheaply, with
Silverpine as the control that distinguishes "skill is broken" from
"hardest zone is hard." Starting in the Barrens would debug the skills
and the hardest zone simultaneously.

**Barrens result (2026-11-23): direction B held for the reference
slice.** With a fitted Kalimdor bbox (`x[-4300,1300] y[-4700,-900]`,
map 1) the extractor produced 98 Barrens quests (62 zone-local
implementable), and the unmodified `bot_sim.py` cleared level 10 → 20
for all three classes. The only code changes the Barrens forced were in
the extractor, and they were generic: resolve loot template ids through
`creature_template.loot_id` and chest `data1`, and classify quests whose
starter/turn-in is outside the zone box as cross-zone review items
instead of letting the simulator teleport to a missing endpoint. No
Barrens-specific planner or skill code was added.

## Eval bench infrastructure

**Decided: hill climbing runs against the real server** (a `mangosd`
instance), not a simulation — only the real server is truthful about
pathfinding, combat, respawns, and every other mechanic a simulation
would drift from.

**Decided: same DigitalOcean droplet as production** (headroom is
available), structured as follows:

- A second, **headless** `mangosd` instance. No human logs into the bench,
  so **no bench `realmd`** and no second realm entry. Logs and metrics
  are the observability surface.
- **Shared read-only**: the compiled binaries and extracted client data
  (maps, mmaps, vmaps, DBCs). The world content DB (`mangos`) is
  loaded into memory at startup and mostly read; a second instance can
  read the same one, with a separate bench copy if evals ever modify
  world state worth caring about.
- **Separate**: bench `characters` database, bench `logs` database, and
  ports (e.g. 3725/8086 versus production 3724/8085).
- **Resource guardrails from day one**: systemd CPU quota and lower
  scheduling priority for the bench. Headroom is an average; an eval
  batch pathfinding across the Barrens is a spike. The quota costs
  nothing when idle, guarantees production ticks win contention, and
  makes eval timings *more* comparable run-to-run (consistent CPU slice).

Fallback if co-tenancy ever hurts: an ephemeral second droplet spun up
from a snapshot per eval batch, results exported, droplet destroyed.

### Layered evals

A full zone traversal (lowest-to-highest recommended level) is the
*validation* test, but as an optimization target it is too coarse: hours
of runtime, and credit assignment is unanswerable (did pull size, route
order, or vendor timing cause those 40 deaths?). Evals therefore layer:

1. **Solo skill evals** (minutes). Kill 10 of mob X. Navigate hub →
   objective → hub. Vendor/repair/restock. Tune the generic skills with
   clean credit assignment.
2. **Solo quest episodes.** One quest or short chain, accept to turn-in.
   Tune quest execution and zone data (hotspot quality).
3. **Population eval.** A realistic mix — e.g. nine bots in Silverpine
   (three per class), staggered levels and starts approximating
   steady-state production. This is where the congestion weight is tuned;
   congestion cannot be tuned solo.
4. **Congestion stress.** Adversarial: all bots released at the same hub,
   same level, same moment. Steady state takes care of itself if this
   passes.
5. **Zone traversal.** The full lowest-to-highest path, run rarely as an
   integration/regression test. Proves the system end to end; does not
   drive the climb directly.

Dungeons layer the same way: single-pack pulls → individual bosses from
a checkpoint at the boss door (a full clear wiping on boss 3 wastes the
run as a learning episode) → full clears. Cheated teleports to boss
doors are free on the bench. Full-clear parties use the holy trinity:
1 tank / 1 healer / 3 DPS, appropriately leveled, optimizing for as close
to zero deaths as possible over all bosses.

### Metrics and logging

With no human watching, logs are the only eyes — so they must record
**decisions, not just outcomes**. A free-text log says a bot died; a
decision trace says it pulled three mobs because its health threshold
allowed it. When a parameter change regresses, the trace is how you find
out why.

- **Structured events** (one JSON record per event): run ID, bot,
  timestamp, event type, plus decision inputs — skill chosen, scores
  considered, target selected, pull size, failure reason, death cause,
  stuck detection with position samples.
- **Per-run summary**: parameter set used, quests completed, deaths,
  elapsed time, termination reason. **Early termination** on stuck/wedge
  detection is required, or failed runs burn hours.
- Solo metrics: completion time, deaths, quests completed, XP/hour.
- Population metrics add: hotspot occupancy distribution over time
  (spread versus stacked), time lost to mob-tag contention, named-mob
  wait times, and progression **variance** across bots (one bot starving
  on lost tag races is a failure solo metrics never show).

**Reward hacking warning.** XP/hour alone teaches bots to grind safe
green mobs and ignore quests. Zone metrics should be composite: quest
completions weighted heavily, deaths penalized, XP as a tiebreaker. What
is rewarded is what is learned, literally.

**Variance control.** The real server is stochastic (patrols, respawns,
crits). Candidates need repeated runs before a comparison means anything,
and the parameter count stays small for the same reason.

Observability tooling (a log viewer over the structured events:
run timelines, per-bot/event filtering, path drawing from position
samples, parameter-set comparison) is a later nicety. Emitting structured
records from the start keeps that door open at near-zero cost; parsing
free-text logs after the fact would not.

## Snapshot and reset machinery

Every eval needs a known starting state, so snapshot/reset machinery is
the bench foundation — everything else waits on it. A snapshot captures
more than level and gear: XP, talents (leveling or dungeon template per
eval type), spells, inventory down to ammo/reagents, gold, quest log
**and** completed-quest history, position, hearth binding, discovered
flight paths, weapon skills — and the hunter's **pet** (level, loyalty,
happiness), which is easy to forget and produces a hunter re-taming at
the start of every eval.

Three mechanisms were considered:

- **Whole-database restore.** Dump the bench `characters` DB in a
  known-good state, restore between batches. Guaranteed consistent
  (nothing remapped). Costs: requires the bench stopped (a running
  `mangosd` holds character state in memory; restoring under it
  corrupts), resets all bench characters at once, and each cycle pays a
  full server boot. Side benefit: world state resets too (fresh respawn
  timers, named mobs alive), which improves eval validity.
- **Scripted template setup.** Create the character through the server's
  own paths, then apply a template (level, gear, talents, quest flags)
  via GM-style operations while the server runs. Always valid, resets
  per-bot without a restart, templates are readable data that survives
  schema migrations, and gear granting uses the server's own GUID
  generation. Costs: slower per character, mid-chain quest seeding is
  fiddly, and world state is *not* reset (a named mob dead in run 1 may
  still be dead in run 2).
- **SQL template cloning.** Fast per clone, but requires owning GUID
  remapping across every character table forever, including after each
  append-only migration. Contained on a disposable bench DB, but a
  permanent maintenance burden. **Deferred**: only if reset speed is
  proven to be the bottleneck.

**Recommended shape: a hybrid.** Use scripted setup once to generate
golden characters (e.g. level-appropriate hunter/warrior/priest at
Silverpine entry), then dump the bench DB. The eval loop is stop →
restore → boot → run batch → collect metrics. Scripts remain the source
of truth; dumps are regenerable artifacts; a schema migration means
re-running the generator. In-server per-episode resets get added later
only if boot time dominates eval time. Batch size is a dial in the
harness for exactly this reason.

**Bootstrapped 2026-11-23 (decided mechanism: donor-morph).** Code
inspection settled how "scripted setup" actually works in this codebase:
fresh-character SQL would have to replicate
`Player::SaveNewPlayer`/`Player::Create` exactly (a fidelity trap), and
bots are in any case pre-created `characters` rows registered in the
`playerbot` table (`PlayerBotMgr` loads by GUID) whose level, spells,
and gear are completed at spawn by runtime morphing (`GiveLevel`, spell
learning in the bot AIs, gear templates) — donor rows in the DB are bare
shells with empty inventory/spell/skill rows. So goldens are built by
**donor-morph**: [`scripts/bot_bench_golden.py`](../scripts/bot_bench_golden.py)
copies an existing server-created bot character per fixture out of a
source characters DB (SELECTs only; the target must be a `*_bench` DB
and every write is target-qualified), leaving quest/social/group/pet
state behind, then sets name/account/level/money/position/homebind from
[`scripts/bot_bench_fixtures.json`](../scripts/bot_bench_fixtures.json)
and registers the `playerbot` row. Spawn and homebind coordinates were
taken from world-DB NPC spawns at the Sepulcher (High Executor Hadrec
outdoors; Innkeeper Bates for the bind).

First fixtures (all level 11 at Sepulcher entry): **Brannock** (Undead
warrior, donor Rotbane), **Selvara** (Troll hunter, donor Darkspear —
there are no Forsaken hunters in 1.12), **Morwyn** (Undead priest, donor
Deathweave). Validated end-to-end against the workstation's local MySQL
mirror of the characters DB: 61 target tables initialized, all three
fixtures applied and re-applied idempotently, quest/pet tables empty as
designed, and the source DB fingerprint (row counts, level/money sums)
unchanged across runs. The local `characters_bench` result is a
workstation artifact; producing the droplet golden still requires an
operator run there once the bench DB exists (credentials by environment,
as in `scripts/backup-db.sh`). Two consequences to design around:
fixture validity now depends on the bench bot spawn path actually
running the same runtime morphing (level/spells/gear) that existing bot
AIs do, and hunter pets are intentionally absent — the bench must
tame/summon at runtime.

**Cold start is measured, and it is fast.** On 2026-11-23, production
`azeroth-mangosd` started at 07:05:03 UTC (`ExecMainStartTimestamp`,
matching the first `Server.log` line) and logged "World server listening
on 0.0.0.0:8085" at 07:05:10 UTC — roughly **7 seconds** service-start to
world-ready, bots and all (222 bots loaded in the same second). At that
cost, restore-per-batch is effectively free and even restore-per-episode
is thinkable. Two caveats before designing around it: a bench boot under
its CPU quota will be somewhat slower, and some data (mmap tiles) loads
lazily on first use per area, so "listening" is not "all nav data
resident" — the first bot entering a zone may pay tile loads the boot
did not. Neither changes the conclusion that boot time will not
dominate eval time.

## Congestion and symmetry breaking

Bots are most of the playerbase, so the planner cannot ignore other bots.
Identical tuned parameters plus identical starting state produce
identical decisions: a crowd released at one hub would all score the same
hotspot highest and arrive as a single blob. Parameter sets trained only
in solo evals make clustering *worse*. Two mechanisms prevent it:

- **Per-bot symmetry breaking (not learned).** Each bot carries small
  persistent random offsets — seeded per character, stable for its life —
  applied to planner scoring and timing. One bot slightly prefers the
  northern hotspot, another leaves the hub 90 seconds later, a third
  vendors at 75% bags instead of 85%. The hill climb tunes shared
  parameters; the jitter sits outside the climb and also reads as
  personality to an observer.
- **Congestion penalty (learned in population evals only).** The planner
  scores hotspots down by how many friendly bots already target them.
  Signal source options: local observation only (believable but myopic)
  or a shared zone-level count of bots targeting each hotspot
  (omniscient, slightly unnatural perfect spacing). Leaning: shared
  count — the audience is one player, cheating is allowed, and perfect
  spacing looks mildly uncanny while no spacing looks broken.

## Production population: fidelity tiers

Not every bot needs full-fidelity decision-making at all times. If the
human is in the Wetlands, heavy computation in Searing Gorge is wasted —
but `/who` should still show plausible players there. Bots therefore run
in tiers (the bot analogue of the grid activation MaNGOS already uses
for creatures):

- **Paper tier** — no spawned character. A lightweight simulation
  advances the bot's level, zone, and rough gear on timers. Source of
  `/who` believability for far zones.
- **Ambient tier** — spawned, thinking slowly: simple wander/grind/vendor
  loops at a long tick. Adjacent zones, capital cities.
- **Active tier** — full planner and combat brain. The human's current
  zone, and any zone under eval.
- **Party tier** — active plus party coordination. Only when grouped
  with the human.

The hard part is transitions, not the tiers. Paper → active means
materializing position, quest log, and inventory consistent with the
paper history. The "visible cheating is fine" assumption keeps
"consistent enough" a low bar. Identity lives in the character record
and the paper simulation; re-balancing moves paper bots between zones or
retires/creates them, and nobody observes the seams. (This is also where
the DynamicBotAI interaction question sits.)

## Dungeons and Party Finder

A custom Party Finder assigns bots roles for dungeons, and bots carry a
learned per-role protocol for each dungeon. **Coordination is scripted,
owned by a party brain** — one coordinator holds the encounter state
machine (next pull, marks, CC assignments, boss phase) rather than five
bots coordinating emergently. Reliability over emergence.

- **The tank is the pace-setter and direction-setter**, per normal party
  mechanics:
  - *Bot tank*: the coordinator lives in the tank. It must telegraph its
    next move so a human in another role can follow: raid markers on
    targets, a short party-chat line before non-obvious pulls, pausing
    at pull boundaries, facing the next pack. Predictability *is* the
    telegraph.
  - *Human tank*: the coordinator goes reactive. It infers the human's
    pull from target, marks, movement, and first hostile action, then
    assigns everyone else around it. Bots respect human pauses (no DPS
    pulling while the healer is visibly drinking).
- **Hybrid roles via an emergency interrupt layer** above the role
  script, driven by a capability matrix (a DPS druid *can* heal, battle-
  res, and briefly off-tank regardless of assigned role). Triggers must
  be conservative and sticky — healer dead, tank below a threshold with
  the healer out of mana — and once triggered the bot commits until the
  emergency clearly ends, or it flaps between roles and does neither.
- Role assignment applies the dungeon talent template (see Classes and
  specs); early dungeons likely need only role parameter blocks.
- Dungeon instances isolate parties naturally, which makes parallel
  dungeon evals on one bench server non-interfering (unlike parallel
  zone evals, which compete for quest mobs and are only useful when
  congestion itself is under test).

Dungeon-internal protocol design beyond this sketch (pull scripts, boss
handling, how this meets the boss-pattern learning in
[BOT_ARCHITECTURE_PROPOSAL.md](BOT_ARCHITECTURE_PROPOSAL.md)) is future
work; see Open Questions.

## Ad-hoc parties

**Decided: bots form ad-hoc parties for shared open-world objectives.**
This turns congestion from a problem into the most human-looking
behavior in the system, and builds the dungeon substrate (invite
handling, leader-follow, assist targeting, split/merge) in a low-stakes
setting.

- **Formation.** When 2+ ungrouped bots pursue the same quest objective
  in the same hotspot, the congestion signal becomes a grouping
  opportunity. First arrival leads and invites the others. The party
  brain is a miniature of the dungeon one: follow the leader between
  objectives, assist the leader's target, nobody pulls alone.
- **Stay or disband by quest-log overlap.** After the shared objective,
  compare members' logs: high overlap (tunable threshold) → stay together
  for the next shared quest; low → split. For collect quests where each
  member needs their own drops, the party stays until *everyone* has
  their items, with a tunable cap on how long finished members keep
  helping.
- **Chat announcement with a join window.** When a stack of bots is about
  to do a quest or named mob, the leader announces in **zone chat** and
  waits a timeout for a human to join:
  - Gated on a human actually being in the zone. No human → no
    announcement, no wait. In bench evals this disables by configuration
    so timing metrics stay clean.
  - Leader only, with a per-quest cooldown. Message carries quest name,
    a location landmark from zone data ("at the Fenris Keep entrance"),
    and open spots (5-man cap). Phrasing varies across a template pool.
  - Joining is trivial: the human whispers the leader (e.g. anything
    containing "inv") during the open window, or invites any member
    directly and the bot accepts.
  - Timeout is tunable and bounded by travel reality: bots may quietly
    check the responding human's rough distance (cheating allowed) and
    hold longer for a nearby responder than for one a 5-minute ride
    away. A short countdown message precedes the pull. A human whispering
    mid-fight gets an immediate invite — party kill credit is usually
    still salvageable if they are in range at the death.
- **Leadership passes to the human on join (decided).** The rule
  generalizes the dungeon one: in any mixed party the human is the
  pace-setter, tank or not. Loot rules, invites, and direction become
  theirs; the party brain drops into reactive mode (follow the human,
  assist their target, pull nothing unprompted). Edge cases: if the
  human goes idle or heads to objectives no bot shares, after a tunable
  patience the bots say so politely in party chat, split off, and resume
  their own planning. If the human leaves, leadership reverts to a bot
  and the overlap check re-runs. (Human-join behaviors are
  production-only; the headless bench has no human to hand lead to.)

Ad-hoc parameters: minimum stack size to group, overlap threshold to
stay grouped, announcement wait duration, finished-member patience on
collect quests. Matching population-eval metrics: parties formed, shared
completions, time saved versus queueing at spawns, announcements made.

## Decision log

Settled in the 2026-11-22 brainstorm, in the order decided:

1. Methodology: **C (learned layer) on top of B (reusable skills,
   zones as data)**. A (per-zone authored playbooks) rejected — already
   proved brittle in bench testing.
2. **Bot identity is real and persistent**; population re-balancing is
   allowed.
3. **Visible cheating is acceptable** (teleported transitions, template
   swaps, generated state). Likely a single human observer.
4. **Hill climbing on the real server**, against a bench instance, never
   production data.
5. Bench on the **same droplet** as production, with systemd CPU quota +
   lower priority; separate bench `characters`/`logs` DBs and ports.
6. Bench is **headless**: no bench `realmd`, no human login. Structured
   logs + metrics are the observability surface.
7. **Greedy planner** with anti-churn hysteresis, abandon blacklist, and
   honest travel costing. Guide-like lookahead deferred (reversible).
8. **Silverpine Forest first, then the Barrens** as the generalization
   proof (new data only, no new skill code).
9. **First classes: hunter, warrior, priest**, leveling specs for zone
   work, different spec set for dungeons (applied as a template swap on
   role assignment; early dungeons may need role parameter blocks only).
   Expand to all classes under the frozen-shared-parameters discipline.
10. **Zone data by automatic DB extraction** (quest graph, clustered
    hotspots, hubs). Player guides deferred with the greedy choice;
    copyright/source constraints noted for if they return.
11. **Congestion modeled in evals** (population + stress tiers), because
    solo-trained parameter sets would cluster in production.
12. **Ad-hoc parties** for shared objectives, announced in zone chat with
    a human join window; **leadership passes to the human** on join.
13. **Dungeon coordination is scripted** under a party brain; tank sets
    pace; bot tank telegraphs; human tank makes the coordinator reactive;
    hybrid roles via a sticky emergency-interrupt layer.
14. **Golden characters are donor-morphs** (2026-11-23): existing
    server-created bot characters copied read-only into a `*_bench`
    characters DB and morphed per fixture by
    `scripts/bot_bench_golden.py`. Fresh-character SQL rejected (must
    replicate `Player::Create` exactly); level/spells/gear are completed
    at spawn by existing runtime morphing. First fixtures bootstrapped
    and validated locally (Brannock/Selvara/Morwyn, level 11 at the
    Sepulcher).
15. **Zone data is extracted, not authored** (2026-11-23):
    `scripts/extract_zone_data.py` generates the per-zone JSON
    (quest graph, classified objectives with hotspots, hubs, grind
    spots) from the world DB. Current zone-local counts are Silverpine
    22 of 42 implementable and the Barrens 62 of 98. Registry of
    extraction rules (ZoneOrSort membership, bbox spawns, loot-template
    to spawnable-entry resolution, courier detection, cross-zone
    endpoint flags) lives in the extractor.
16. **Vertical slice is a Python reference implementation first**
    (2026-11-23): `scripts/bot_sim.py` validates the greedy planner and
    skill contracts over extracted data and is the spec/oracle for the
    Rust port into `mangos-bots`. All three classes clear Silverpine
    11→20 and the Barrens 10→20 in simulation with the same planner/skill
    code. Risk-aware scoring and a grind circuit breaker (step mob level
    down after grind deaths) are required parts of the planner, learned
    from trace audits of the slice; true hub-trip batch acceptance
    remains unimplemented in the reference.

## Open questions

Deliberately not settled. Roughly in the order they will bite:

1. ~~**Bench cold-start time.**~~ Resolved 2026-11-23: production cold
   start measured at ~7 s (see Snapshot and reset machinery).
   Residual: re-measure under the bench CPU quota once the bench exists;
   not expected to change the design.
2. **Snapshot mechanism follow-ups.** Mechanism decided and bootstrapped
   (donor-morph via `scripts/bot_bench_golden.py`, see Snapshot section
   and decision 14). Remaining: stand up the bench DB on the droplet and
   run the generator there (operator credentials); which additional
   golden states the fixture library needs (per dungeon-entry party,
   other zones/classes); fixture versioning against schema migrations
   (generator introspects columns at runtime, which covers additive
   drift); how talent templates are produced per spec/level (can
   `scripts/generate_talent_builds.py` feed the spawn-time morph?); and
   hunter pet handling at bench spawn (pets are deliberately not copied).
3. **Extraction follow-ups.** Schema defined; Silverpine and the Barrens
   extracted (decision 15; grind spots come from spawn density).
   Remaining: explore-trigger coordinates need an `AreaTrigger.dbc`
   join; positive escort detection beyond the `quest_start_scripts`
   flag; chat-announcement landmarks (hub names) are not in the JSON yet;
   a repeatable bbox/zone-membership procedure for future zones
   (rectangles contaminate grind spots on non-rectangular borders);
   a policy for cross-zone endpoint quests (zone-local exclusion vs
   flight/capital trips vs seeded breadcrumbs); and where the generated
   JSON lives at runtime (`data/zones/` for now).
4. **Bench concurrency budget.** How many bots/parties run at once on
   the quota'd bench before tick fidelity degrades? Sizes the population
   and stress evals.
5. **DynamicBotAI / BotDirector interaction.** Do leveling bots replace
   DynamicBotAI wandering in zones where they operate? Does BotDirector
   become the fidelity-tier manager, or does a new population manager
   own tiers while BotDirector keeps its placement role?
6. **Paper-tier mechanics.** How exactly does `/who` believability work
   for unspawned paper bots (real sessions vs synthetic presence)? What
   does the paper simulation track, and what does materialization
   generate (gear, quest log, position) at each level band?
7. **Zone transition details.** Cheated, but what precisely happens:
   teleport destination, flight-path grants, hearth rebind, partial
   quest cleanup? Do cross-zone quests a bot already holds survive the
   move?
8. **Dungeon protocol internals.** Pull scripting format, boss handling,
   CC assignment, wipe recovery, and how this composes with the combat
   event ring buffer / pattern learning sketched in
   [BOT_ARCHITECTURE_PROPOSAL.md](BOT_ARCHITECTURE_PROPOSAL.md). Also:
   Party Finder UI/mechanics (queue, role selection, fill-with-bots
   semantics) are undesigned.
9. **Lookahead/guide revisit trigger.** What bench evidence (travel-time
   share, specific class pain such as warrior weapon chains) would
   justify adding bounded lookahead or guide ordering back in?
10. **Economy effects.** Persistent questing bots loot, vendor, and hold
    gold at population scale. Interactions with the Auction House bot
    and the wider economy are unexamined.
11. **Chat surface details.** Template pool tone/size for announcements;
    whether bots use `/say`/`/yell` near hotspots in addition to zone
    chat; any bot-to-bot party chat flavor.
12. **Alliance coverage.** Paladin (and Alliance zone pairs generally)
    is required for all-classes coverage but undesigned; the first
    Alliance proving zones are unpicked.
13. **Eval harness ownership.** What drives the stop → restore → boot →
    run → measure loop (script vs service), where metrics land, and how
    parameter sets are versioned alongside results.

## Suggested pickup order

For whoever resumes this (likely after reflection): goldens (14),
Silverpine+Barrens extraction (15), and the Python reference slice (16)
are done, including the Barrens generalization run. Next: stand the
bench DB up on the droplet, generate goldens there, and re-measure cold
start under the bench CPU quota; then the Rust port into `mangos-bots`
against the reference traces (closing the hub-trip batching gap), and
population evals before any production population is trusted. Dungeons
and fidelity tiers after the zone loop is proven.
