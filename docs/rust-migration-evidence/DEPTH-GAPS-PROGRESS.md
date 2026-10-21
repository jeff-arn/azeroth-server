# Depth-gap closure — progress

Goal: port the gameplay systems Phases 0–6 left shallow,
by leverage: (1) XP/level-up runtime, (2) stat +
spell/aura depth, (3) pet save + item cooldowns,
(4) movement anticheat. Semantic oracle: production
fork `2bae5f9` (source at `/tmp/azeroth-prod` this
session; paths below are fork-relative).

## Slice 1 inventory — XP/level-up (2026-10-19)

Fork surface read:

- `game/Formulas.h` `XP::` — `GetGrayLevel`,
  `GetColorCode`, `GetZeroDifference`,
  `BaseGainLevelFactor`, `BaseGain` (45 base),
  `Gain(pUnit, pCreature)` (critter/totem/no-xp
  gates; tail incl. rate/elite handling still to
  read line-by-line).
- `Player::GiveXP` (`Objects/Player.cpp:3062`) —
  trial/personal-rate quirks to adjudicate (fork
  `GetPersonalXpRate` is custom), alive/max-level
  gates, rest bonus only when `victim` (kill XP),
  `SendLogXPGain` (`:3048`, packet shape to read),
  multi-level while loop.
- `Player::GiveLevel` (`:3115`) — fork-only logging
  and GM levelup alert, BG-queue bracket leaving
  (BG out of scope this goal), `SMSG_LEVELUP_INFO`
  (`:3230`: level, hp gain, mana-or-0, 4×0, 5 stat
  deltas vs create stats), `PLAYER_NEXT_LEVEL_XP =
  GetXPForLevel`, played-level-time reset,
  `UpdateSkillsForLevel`, create-stat/health/mana
  re-baseline, `InitTalentForLevel`,
  `UpdateAllStats`, full health/mana/energy refill,
  pet level sync, fork bot "gratz" hook.
- `Player::GetXPRestBonus` (`:8380`) and
  `RewardPlayerAndGroupAtKill` — bodies still to
  read (group split rules). Quest XP
  (`RewardQuest`) and exploration XP paths still to
  inventory.

Rust reachability (good news): the data layer is
already complete — `mangos-world-data/player_info.rs`
loads `player_xp_for_level`, per-race/class
`level_info`, and `class_level_info`
(`xp_for_level()` at `:201`). `PLAYER_XP` already
round-trips load/save. Creature death funnels
through `Map::kill_unit`
(`mangos-map/src/spells.rs:1116`) emitting
`MapEvent::UnitDied` — the XP hook point.

Still to pin before/at implementation: whether the
Rust `Player` tracks create stats/health/mana (the
levelup packet and re-baseline need them), the Rust
stat-recompute entry point to re-run on level-up,
talent-point field handling, skill-max update
(`level*5`), and whether `Rate.XP.*` config exists
in the Rust config surface (Phase 6 posture was
fork defaults at 1.0).

## Slice 1a — kill XP + level-up (done)

Implementation:

- `mangos-entities` `Player::give_xp` /
  `give_level` (`Player.cpp` `GiveXP:3062`,
  `GiveLevel:3115`): play-time-flag gates, alive /
  max-level gates, kill-only rest consumption,
  multi-level loop; level-up writes next-level XP,
  skill max `level*5`, create-stat/health/mana
  re-baseline, talent-point delta
  (`CalculateTalentsPoints`; used points untracked),
  derived-stat recompute and full refill. Outcome
  types `XpGainOutcome` / `LevelUpInfo`.
- `mangos-map/src/xp.rs` (new): fork `XP::`
  formulas (`Formulas.h`: `GetGrayLevel`,
  `GetZeroDifference`, `BaseGain`, `Gain`,
  `xp_in_group_rate`), solo + group distribution
  from `Map::kill_unit`, `SMSG_LOG_XPGAIN` /
  `SMSG_LEVELUP_INFO` builders.
- Actor fix: entity `Player.group_id` was
  login-time only; `sync_entity_group` in
  `world_actor.rs` now mirrors runtime group
  create/join/remove/destroy (bot joins route
  through accept), so runtime-formed groups earn
  group XP.
- Adjudications vs fork: damage-origin modifier =
  1.0 (untracked), `UNIT_STATE_NO_KILL_REWARD`
  never set, temp summons stand in for
  `UNIT_CREATED_BY_SPELL`, reward keys off the
  killing-blow player (fork `Unit::Kill` likewise),
  pet XP deferred to the pet slice, fork personal
  XP rate unmodeled (no reachable setter).

Verification:

- `mangos-entities/tests/xp_level_up.rs` 4/4;
  map formula unit tests; live harness
  `mangos-map/tests/live_xp.rs`
  (`LIVE-XP-PASS given=78 rest=78 level 19 -> 20`
  vs independent SQL oracle), wired into `ci.sh`.
- Bench DM group probe: `XP-DING-WIRE-PASS` /
  `XP-DING-PERSIST-PASS` (Ruikanis persisted
  level 20; fixture restored after).
- True C++ save differential (clone DBs
  `characters_cpp_xp` / `characters_clone_xp`
  from the Phase 6 diff base; C++ via GM `.die`,
  Rust via master melee; probe
  `/tmp/xp_diff2_probe.py`): wire `given=27` on
  both sides, `LEVELUP level=20` both. Save dump
  292 (C++) vs 291 (Rust) lines: skills, spells,
  talents, cooldowns, reputation, inventory shape
  byte-identical; position byte-exact; level 20
  both.

`Rate.XP.*` finding: the differential exposed that
production runs `Rate.XP.Kill = 1.4` (also Quest /
Explore 1.4, Kill.Elite at 1.0 default) — C++ gave
27 where Rust's 1.0-default math gave 20. Rates
are now wired `ServerConfig` (`Rate.XP.Kill` /
`.Kill.Elite` / `.Quest` / `.Explore`,
`World.cpp:506-509` defaults 1.0) →
`MapConfig::xp_rates` → `creature_xp_gain` in fork
order (elite factor → `Kill.Elite` →
`xp_multiplier` → `Kill` → `nearbyint`), with the
`kill_rates_apply_in_fork_order` regression test.
Bench conf carries no `Rate.XP.*` keys, so bench
still runs 1.0.

Differential residuals, all adjudicated:

- XP remainder (C++ 23 vs Rust 49) and
  `rest_bonus`: Rust-side re-prime set
  `rest_bonus=15975` (and `health=100000`,
  played-time 0); the C++ prime did not. Both
  sides consume `min(given, pool)`: Rust saved
  `15975-27=15948`; C++ pool ≈1.18 added 1 rested
  point (total 28) and saved 0.178711. Prime
  asymmetry, rule-consistent both sides.
- `health` 480 vs 509: both refill to max at ding
  (fork `SetHealth(GetMaxHealth())`); oracle max
  by fork formula = `199 + StaminaBonus(43+6) =
  509` = Rust. C++ took 29 damage during its 45s
  logout camp (saved `power2=12` rage and armor
  durability wear corroborate combat).
- One C++-only row: `character_aura` spell 71
  (Defensive Stance form) present in the shared
  base — C++ preserved it, Rust drops form auras
  at load. Real one-row save divergence, carried
  to the Slice 2/3 inventory (aura application /
  save residuals). Aura 15007 (Resurrection
  Sickness) expired offline identically on both
  sides.
- `extra_flags` 4 vs 2: fork normalizes a
  session-flag bit at login that Rust carries
  verbatim; pre-existing, non-XP, noted for the
  save-residual inventory.
- Durability, quest timers, played/logout/death
  times, instance id: session-activity and
  temporal churn (Phase 6 classes).

Full `ci.sh` green (130 suites) with the rates
wiring; bench restored on `bench-lan.conf`
(listening on 18099), guid1 fixture verified
canonical, test GM `account_access` row removed.

Next: Slice 1b exploration XP
(`PLAYER_EXPLORED_ZONES` discovery on the
zone-change path, `Rate.XP.Explore`), then the
OrbStack level-up persistence gate. Quest XP waits
for a quest turn-in runtime (only
`CMSG_QUESTGIVER_STATUS_QUERY` is handled today).

## Slice 1b — exploration XP (done)

Implementation (fork `Player::CheckAreaExploreAndOutdoor`,
`Player.cpp:6250`, discovery arm):

- `mangos-world-data`: `AreaStore::by_flag_and_map`
  (`AreaEntry::GetByAreaFlagAndMap`) and
  `flag_by_map_id` (`GetFlagByMapId`); new
  `exploration.rs` store (`exploration_basexp`,
  `ObjectMgr::LoadExplorationBaseXP` / `GetBaseXP`),
  loaded in `mangosd` and carried in `WorldStores`.
- `mangos-entities`: `has_explored_flag` /
  `set_explored_flag` over the
  `PLAYER_EXPLORED_ZONES_1` value words (save rides
  the Phase 6 values diff).
- `mangos-map`: `Map::area_flag_at`
  (`TerrainInfo::GetAreaFlag` over gridmap areas;
  missing grid falls back to the map's linked-area
  flag — this also corrects `zone_and_area_id`'s
  fallback, which previously passed the linked zone
  *id* as a flag). `exploration_xp()` (pure,
  branch-tested) and `Map::check_area_explore`,
  hooked at both Rust relocation funnels matching
  the fork's `Player::SetPosition` site: client
  movement (`apply_movement_impl`) and server-side
  relocation (`relocate_unit`, covers bots and
  same-map teleports). Immediate cadence matches
  the fork default
  (`Movement.RelocationVmapsCheckDelay = 0`).
- XP semantics fork-exact: bit set before the area
  lookup (unknown areas set the bit, send nothing);
  XP only when `AreaLevel > 0` and below max level,
  via `give_xp(xp, is_kill=false)` (no rest
  consumption, `LOG_XPGAIN` type 1,
  `SMSG_LEVELUP_INFO` on ding); formulas
  `BaseXP(level+5)` below band, percent-decayed
  `BaseXP(area)` above band (integer math before the
  rate multiply), all scaled by `Rate.XP.Explore`;
  `SMSG_EXPLORATION_EXPERIENCE` sent after `GiveXP`
  even at 0 XP.
- Unported arms of the fork function, adjudicated:
  taxi-flight and cinematic gates are unreachable
  (both unmodeled); vmap WMO area overrides stay
  deferred with the existing zone-path adjudication
  (gridmap flags only, so WMO-only sub-areas do not
  discover yet); the outdoor-spell reapply/remove
  and tavern-exit arms belong to Slice 2 / the known
  rest residuals. The anticheat `OnExplore` hook is
  Slice 4.

Verification:

- Unit tests: `exploration_xp_branches` (in-band,
  below-band +5 read, percent decay, clamp-to-0),
  `ExplorationBaseXp` missing-level zero.
- Clone-DB probe (`/tmp/explore_probe.py`, clone
  conf at production rates 1.4): Ruikanis primed
  with zeroed `explored_zones` discovered Sentinel
  Hill on first movement — wire
  `LOG_XPGAIN total=147 type=1` then
  `EXPLORATION_EXPERIENCE area=108 xp=147`; SQL
  oracle `BaseXP(15)=105 × 1.4 = 147` exact;
  persisted `xp=147` and explored bit 196 (word 6 =
  16); relog + movement produced no repeat packets
  (`EXPLORE-PROBE-PASS`).
- Full `ci.sh` green (130 suites) including the
  motion-funnel hook; workspace clippy clean; bench
  restored on the rebuilt binary.

Next: OrbStack level-up persistence gate, then
Slice 2 (stat/aura depth).

## Slice 1 gate — OrbStack level-up persistence (passed)

Setup: stock 5875 client on `wow-rig-arm` against the
bench (Rust realmd + rebuilt mangosd, realmlist via
host Tailscale IP). Character creation is a Phase 3
stub in Rust (`CMSG_CHAR_CREATE` unhandled — found by
this gate, out of this goal's scope), so the gate char
"Probechar" (guid 900, TESTUSER) was cloned from
guid 1's bench rows and primed to level 19, XP 21250
(threshold 21300), zeroed explored zones, at Sentinel
Hill. Drive script `/tmp/orb_gate.py`, artifacts
`/tmp/wowrig/artifacts/gate-*.png` (pixel-inspected).

First run caught a real bug: the client printed the
full level-up chat train ("Congratulations, you have
reached level 20!", +26 HP, +1 talent point, Str/Agi/
Sta/Spi +1, "Discovered Sentinel Hill: 105 experience
gained") but the character sheet still showed
Level 19 with pre-ding stats and XP 21250/21300.
Root cause: `Map::send_object_updates` queued objects
on `ObjectBase::is_object_updated()`, which no
production writer ever sets — all value writes go
through the raw `Values` mirror-diff store and
`execute_delayed_actions` has no caller, so no values
block ever shipped after creates. Fix: the flush also
queues objects whose `Values::has_changes()` mirror
diff is non-empty (same outbound set the fork's
delayed marks produce). Regression coverage in
`live_xp.rs`: after the ding the gainer must receive
an update packet and the values mirror must be synced
(only the flush syncs it in-world).

Re-run on the fixed binary: discovery ding rendered,
character sheet showed Level 20 Human Warrior with
Str 47 / Agi 35 / Sta 43 / Int 22 / Spi 27 and
XP 55/23200; camp logout persisted `level=20 xp=55`
in the bench DB and character select listed
"Level 20 Warrior". Gate artifacts `gate-04` (ding),
`gate-05` (sheet), `gate-06` (char select after
logout). Probechar deleted afterwards; guid 1 fixture
verified canonical; full `ci.sh` (130 suites),
`two_client_smoke.py`, and `login_train_audit.py
--strict` all pass with the flush change.

**Slice 1 complete**: kill XP (solo + group + rest),
exploration XP, level-up side effects, `Rate.XP.*`
config wiring — differential- or oracle-proven per
surface above, plus stock-client persistence.
Quest XP remains blocked on a quest turn-in runtime
(unreachable today, formula lands with it).

Next: Slice 2 (stat/aura depth), starting with
`MOD_STAT` / stat-percent auras and the form-aura
persistence row from the Slice 1a differential.

### Correction (Slice 2 inventory): aura 71 is not dropped

Isolation on the clone (login/logout round-trip, then
a DM areatrigger transfer cycle) preserves spell 71
and 15007 rows on the current binary. The Slice 1a
differential loss is attributed to clone history
churn: several earlier failed/crashed Rust cycles ran
against `characters_clone_xp` between its prime and
the successful save, while the C++ side used a
pristine clone. Form-aura *persistence* needs no fix;
shapeshift *mechanics* (form byte, stance side
effects) remain Slice 2 depth.

## Slice 2 inventory — stat/aura depth

Fork model pinned: `m_auraModifiersGroup[unitMod]
[BASE_VALUE|BASE_PCT|TOTAL_VALUE|TOTAL_PCT]`
registers (`Unit.cpp:8022` `HandleStatModifier`,
percent via `ApplyPercentModFloatVar`, -100 clamps to
-99.99); `GetTotalStatValue` =
`((BASE_VALUE + create) * BASE_PCT + TOTAL_VALUE) *
TOTAL_PCT` (0 when TOTAL_PCT <= 0). Item stats enter
at `BASE_VALUE` plus `ApplyStatBuffMod` pos/neg
display fields (signed-float accumulators on
`PLAYER_FIELD_POSSTAT/NEGSTAT0`).
`SPELL_AURA_MOD_STAT` (29) = TOTAL_VALUE + buff mod
(misc -2/-1 = all stats; Scorpid-Sting override-class
-script branch deferred with spell mods).
`MOD_PERCENT_STAT` (80) = BASE_PCT, players only.
`MOD_TOTAL_STAT_PERCENTAGE` (137) = TOTAL_PCT + pos/
neg percent scaling + HP-ratio preservation for
stamina on `Attributes & 0x10` spells. Fork updates
dispatch from `HandleStatModifier` (UpdateStats ->
max health/power/armor/AP chains); `SetMaxHealth`
clamps current HP down to a shrunken max. Creature
max health = health-group value + (stamina -
create stamina) * 10.

Rust today: `aura_effect_apply` handles only
`MOD_ATTACK_POWER`; periodic ticks cover
damage/heal/energize/damage-percent; effect dispatch
covers school damage, heal, energize, trigger spell,
apply aura, summons — everything else is state-only
or a no-op. No procs, no channeled casts in the spell
engine. Player derived stats are item-only at
zero-aura defaults (`recompute_derived_stats`, called
from materialize and `give_level` only); equipping
mid-session refreshes weapon damage fields but not
stat totals. Saved-aura restore runs through
`add_aura_holder`, so register application there
covers load for free.

Slice 2a (first increment): aura stat registers on
`UnitBase`, handlers 29/80/137 with fork pos/neg
semantics, derived-stat refresh for players
(item aggregate retained on the entity) and
creatures, proven by the Resurrection Sickness
health differential on the clone pair. Spell mods,
procs, channeling/pushback, and remaining handlers
inventory-scoped after 2a lands.

## Slice 2a — stat-aura pipeline (done)

Landed: aura consts `MOD_PERCENT_STAT=80`,
`MOD_TOTAL_STAT_PERCENTAGE=137`; `UnitBase.stat_mods:
[StatModGroup; MAX_STATS]` with `handle_stat_modifier`
and fork composition
`total = ((create + item_base + aura_base) * base_pct
+ total) * total_pct`; `apply_percent_mod_float_var`
(-100 → -99.99); Player retains
`equipped: EquippedBonuses` with pos/neg seeded once
at materialize then incremental (display-only fork
semantics), `Player::refresh_derived_stats`; Creature
`spawn_max_health/mana` + `refresh_derived_stats`
(stamina/intellect deltas ×10/×15 over spawn maxima);
map `apply_stat_aura_mod` for 29/80/137 (80
players-only), `hp_ratio_preserve` for stamina 137 on
`Attributes & 0x10` spells, refresh dispatched from
the add/remove aura wrappers only for stat holders.
Unit test `stat_mod_registers_compose_like_fork`
passes.

Setters made fork-exact against `Unit.cpp`:
`SetHealth` caps at max health; `SetMaxHealth` and
`SetMaxPower` clamp the current value down to a
shrunken max in their tails. (`set_power` remains
uncapped vs the fork `SetPower` cap — pre-existing;
production callers self-clamp and test fixtures set
warrior mana above its 0 max. Recorded residual.)

Differential: clone primed with 15007 Resurrection
Sickness saved by Rust at `health=195` (max 195:
stamina 48 × 0.25 = 12 → StaminaBonus 12 → 183 + 12),
matching fork math — the Phase 6 health adjudication
is closed.

Regression found and root-caused: `live_spells`
Rend ticks 3 vs expected 4. Bisect + instrumentation
showed the new derived refresh firing when saved
15007 applies, recomputing player max health (323)
and fork-correctly clamping the test's raw-field
god pool (health 100000) down; the player then died
to the dummy before tick 4 cleared the aura. The
assert was not weakened — the fixture now expresses
the pool through `create_health`, which any refresh
preserves. Verification: `live_spells` passes; full
`./ci.sh` exit 0 (live DB suites env-skipped in that
run) plus all 7 ignored `mangos-map` live suites pass
with the bench env sourced.

## Slice 2b inventory — spell depth beyond stat auras

Fork surface: 96 `Spell::Effect*` handlers, 122
`Aura::Handle*` handlers, `Unit::ProcDamageAndSpell`
+ `IsTriggeredAtSpellProcEvent` (+ `spell_proc_event`
table), `Player::ApplySpellMod` over aura 107/108
(`ADD_FLAT/PCT_MODIFIER`) with charge consumption,
`Spell::Delayed`/`DelayedChannel` pushback from the
`DealDamage` current-spell loop, channeled casts
(`m_channeled`, `SPELL_STATE_CASTING`, channel
start/update packets). Rust today: effects = school
damage/heal/energize/trigger/apply-aura; aura apply =
attack power + stat family; periodic =
damage/heal/energize/damage-percent; `ActiveCast` is
a single countdown with no channel state; no procs,
no spell mods (`SpellEntry` proc fields parse but are
unused). Leverage order: pushback → channeling →
spell mods → procs → remaining handlers by corpus
reachability.

## Slice 2b increment 1 — cast pushback (done)

Fork-exact `Spell::Delayed` (`Spell.cpp:7686`) plus
the `DAMAGE_CANCELS` interrupt arm of the
`DealDamage` loop (`Unit.cpp` ~896). Players only;
non-DoT damage only (`deal_damage` gained the fork
`DOT` arm flag; melee and direct spells push,
periodic ticks do not). `SpellEntry` now carries
`interrupt_flags`/`aura_interrupt_flags`/
`channel_interrupt_flags` (loader fields 22-24;
oracle generator + regenerated `spell_math_oracle`
gained the same fields, +30 lines, no drift). Resist
roll is the fork sum: spellmod term identity until
spell mods land, so the `RESIST_PUSHBACK` (149) aura
total vs `irand(0,99)`. Delay schedule
1000/800/600/400/200→200 (`m_delayAtDamageCount` on
`ActiveCast`), remaining capped at total cast time,
`SMSG_SPELL_DELAYED` (packed guid + actual delay)
direct to the player. Fully-absorbed-hit cancels
ride the absent absorb system (unreachable,
recorded). Evidence: unit test
`pushback_delay_schedule_matches_fork`;
`live_spells` pushback phase — Holy Light self-cast
under dummy melee yields `PUSHBACK delays=[1000]`,
Rend line unchanged at 4 ticks; `./ci.sh` exit 0 and
7/7 live `mangos-map` suites pass.

## Slice 2b increment 2 — channeling (done)

Single-target channeled casts, fork-modeled on
`Spell::handle_immediate`/`SendChannelStart`/
`SendChannelUpdate`/`DelayedChannel` and the holder
cost/range arms. `SpellEntry::is_channeled`
(AttributesEx 0x04|0x40) also fixes the savable-aura
filter, which tested 0x40000000
(`SPELL_ATTR_EX_COMBO_ON_BLOCK`, Overpower) instead
of `IS_SELF_CHANNELED` 0x40. `ActiveCast` carries
channel state; completion starts the channel
(fields `UNIT_CHANNEL_SPELL` +
`UNIT_FIELD_CHANNEL_OBJECT`, `MSG_CHANNEL_START`
direct to player casters); the tick runs
target-validity, the holder range rule (hostile
×1.33, friendly +1.25), the per-second drain
(`manaPerSecond + perLevel × level`, health-cost
funnel guard, fizzle 29 on insufficient power), and
the countdown. Interrupted ends (cancel/movement via
`cancel_cast`, damage-cancel flag, drain failure,
range/death) strip holders immediately; natural
finish leaves holders to `update_auras` later in the
tick so the final periodic tick lands (subset tick
order runs casts before auras, the fork updates
holders inside the spell update). `DelayedChannel`
shares the pushback counter/resist, shortens channel
+ holders with `UpdatePeriodicTimer` realignment, and
refreshes via `MSG_CHANNEL_UPDATE`. Recorded
deviations: `MSG_CHANNEL_START` follows aura-apply
packets (fork sends it first); natural-finish field
reset is immediate (fork delays 1 s); drain is
per-channel, not per-holder (area channels pending);
jump/turning cancel arms ride the existing
movement-cancel path.

Evidence (`live_spells`, Mind Flay 15407 on the bench
dummy): `CHANNEL starts=[3000] updates=[1300, 0]
ticks=[25, 25]` — a dummy melee hit shortened the
channel by the first 1000 ms schedule step mid-flight
(update 1300), holder realignment kept exactly the
2 remaining ticks, health delta equals tick sum;
`CHANNEL-INTERRUPT updates=[0] post_ticks=0` after
`cancel_cast` at 0.5 s. `./ci.sh` exit 0, 7/7 live
`mangos-map` suites pass. Drain Life's
`PERIODIC_LEECH` (53) tick handler remains in the
remaining-handlers queue.

## Slice 2b increment 3 — spell mods (done)

`Player::ApplySpellMod` as a holder-derived query:
modifier state lives on holder effects (no separate
registry), so application/removal is inherently
reversible. Matching is fork-exact
(`IsAffectedOnSpell`): source family name equal and
target `SpellFamilyFlags & EffectItemType[eff] != 0`
— `EffectItemType` (fields 107-109) is now loaded
(loader + oracle generator/regen, 30 lines, no
drift). Composition `(base + flat) * (1 + pct/100)`
with the fork specials: percent mods skipped on a
zero base, instant-cast mods skipped on 10 s+ casts,
and a -100% casting-time mod zeroing flat and
breaking the loop. Charges seed from proc charges
(infinite when the source stacks; custom 1 for
Shadow Trance/Netherwind Focus), drop once per cast
via an `m_appliedMods`-shaped dedupe list on
`ActiveCast`, exhaust to -1, and are swept at cast
finish/channel start (`RemoveSpellMods`); a null
applied list mirrors the fork null spell pointer
(applies, never drops). Owners are players only
(`HandleAddModifier` gate), and
`SPELL_ATTR_EX3_IGNORE_CASTER_MODIFIERS` short-
circuits. Wired ops: DAMAGE/DOT (damage + heals,
direct + periodic), DURATION (aura + channel),
RANGE (cast check; channel range uses the fork
null-spell call), CRITICAL_CHANCE,
CRIT_DAMAGE_BONUS, CASTING_TIME, COST (check
no-drop, take drops), COOLDOWN (both recovery
fields), GLOBAL_COOLDOWN, NOT_LOSE_CASTING_TIME
(pushback/channel resist term, replacing the
identity placeholder). Remaining ops (THREAT,
RADIUS, ALL_EFFECTS, ATTACK_POWER, SPEED, HASTE,
SPELL_BONUS_DAMAGE, MULTIPLE_VALUE, proc-adjacent
CHANCE_OF_SUCCESS…) stay queued with their systems.

Evidence: formula unit test
`spell_mod_composition_matches_fork`; `live_spells`
Nature's Swiftness phase — control Healing Touch
does not land inside the cast call, with Swiftness
`SPELLMOD swift_heal=53` lands synchronously and the
one-charge buff is consumed. A regression was caught
and fixed in-slice: modded-duration suppression
initially ate permanent (-1) auras. `./ci.sh` exit 0,
7/7 live `mangos-map` suites pass, earlier phase
lines unchanged (Rend 4 ticks, pushback, channel).

## Slice 2b increment 4 — procs (done)

Fork-exact proc engine in `crates/mangos-map/src/procs.rs`:
`ProcDamageAndSpell` delay routing (`Spell.ProcDelay`,
fork code default 400 ms; KILL instant; instant-target
attacker pass immediate, remainder queued on the map),
`ProcDamageAndSpellReal/For` two-sided matching,
`IsSpellProcEventCanTriggeredBy` (`SpellMgr.cpp:441`)
incl. the CAST_END equality gate and the fork quirk
that `spellFamilyMask` is not checked there (the
per-effect mask check lives in `HandleTriggers` with
`CanProcFrom`), `IsTriggeredAtSpellProcEvent` general
path (DBC vs `spell_proc_event` flags, custom chance,
PPM = weapon_speed·ppm/600 on the dealer side,
`SPELLMOD_CHANCE_OF_SUCCESS`, grey-victim gate on
on-kill player procs via `xp::gray_level`, apply-time
guard via holder `apply_time_ms`), and `HandleTriggers`
execution: aura 42 → triggered cast (cooldown from the
proc event lands on the *triggered* spell), aura 43 →
rolled school damage, NULL handler = OK (generic charge
burn), `CantTrigger` only for types 85/149, charge
drop with `PROC_FAILURE_BURNS_CHARGE` inversion, holder
removal at zero. `spell_proc_event` loader
(`crates/mangos-world-data/src/proc_events.rs`) with
`spell_chain` higher-rank fill (1094 chain rows;
Talent/DBC-derived chains not modeled).

Raise sites: melee swings (all outcomes incl.
miss/dodge/parry, main-hand), direct school damage
(hit/crit/full-resist), periodic damage ticks
(DEAL/TAKE_HARMFUL_PERIODIC), direct heals, cast-end
(per main target, `CAST_END|NORMAL_HIT/MISS`), kills.
Recorded deviations: the fork's hard-coded per-spell
proc switches and dummy-handler corpus are not ported
except the Lightning Shield rank-trigger overwrite
(324→26364 … 10432→26363, family 11 mask 0x400
visual 37 — without it the DBC trigger 26545 is a
no-op dummy); periodic-heal proc raising, `PERIODIC_LEECH`
(53), off-hand swings, and base-amount override on
triggered casts remain unmodeled.

Evidence: `live_spells` proc phase — Lightning Shield
324 on the level-19 player vs the level-30 dummy:
`PROC ls_hits=[(172, 13)] ls_gos=[(12, 0, 1), (92, 0, 1), (172, 1, 0)]`
— exactly 3 charges cast (26364 base 12 + die 1 = 13),
spaced ≥70×50 ms (3500 ms proc-event cooldown),
charges burn on cast hit-or-miss (22% hit floor at
+11 levels, fork-correct), holder drops at zero, no
4th cast in a 6 s tail window. `./ci.sh` exit 0, 7/7
live `mangos-map` suites pass, earlier phase lines
unchanged.

Remaining in Slice 2: the effect/aura handler tail by
corpus reachability (96 fork effects / 122 aura
handlers vs the wired subset) and the queued spellmod
ops — picked up after Slices 3–4, ordered by
reachability from the current runtime (Deadmines
corpus + player spellbooks).

## Slice 3 — pet save + item/permanent cooldowns (done)

Inventory outcome: spell-cooldown save/load plumbing
already existed end-to-end; the residuals were
enforcement and round-trip fidelity.

- **Login seeding**: loaded `character_spell_cooldown`
  rows were only rendered into `SMSG_INITIAL_SPELLS`;
  the map never enforced them after relog. `Map::seed_cooldowns`
  now runs at login (after saved auras, fork
  `_LoadSpellCooldowns` order) from the same
  validated remaining-time rows.
- **Item cooldowns**: `Map.cooldowns` values became
  `CooldownEntry { expiry_ms, item_id, permanent }`
  (`CooldownData` subset). Seeded item rows keep
  `item_id` through enforcement and save; no item-use
  runtime exists yet to create new ones
  (`CMSG_USE_ITEM` unhandled — lands with its system).
- **Permanent cooldowns**: `SPELL_ATTR_COOLDOWN_ON_EVENT`
  casts enter a permanent hold at cast (fork
  `Spell.cpp:4173` `permanent=true`): never expires,
  excluded from `_SaveSpellCooldowns`
  (`IsPermanent()` skip), converted to the timed
  recovery when the holder is removed
  (`remove_aura_at` → `start_event_cooldown`,
  spellmod with the fork null spell pointer). The
  summon/gameobject fade arms stay with those
  systems; script/proc forced cooldowns are timed
  entries as in the fork.
- **Pet save**: no pet runtime exists in the Rust
  server (no summon-pet/tame/pet-action surface;
  `character_pet` is read only by the char-enum
  join). Fork `SaveToDB` writes pet state only when
  `GetPet()` is non-null, so row preservation is
  fork-identical for every reachable state;
  `SavePetToDB` lands with the pet runtime itself.

Evidence: `live_spells` cooldown phase —
`COOLDOWN-ON-EVENT expire=1000010 item=0` (4511:
hold blocks recast, absent from save rows, cancel
converts to the 10 s timed recovery),
`COOLDOWN-SEED expire=1000058 item=6948` (seeded row
blocks the cast and round-trips `item_id`). C++
save differential (`/tmp/cd_diff.py`, clone DBs
`characters_clone4`/`characters_cpp4`, seeded rows
2136 cat-19 + 635 item-6948, scripted login/idle/
logout on both servers): `CD-DIFF-PASS` — rows
byte-identical including expiry seconds, with
`logout_time` confirming both saves fired.
`./ci.sh` exit 0, 7/7 live suites. Bench `mangosd`
rebuilt and restarted on `bench-lan.conf` after the
differential.

## Slice 4 — movement anticheat (done)

`crates/mangosd/src/anticheat.rs` ports the fork
`MovementAnticheat` reachable subset with fork
config defaults (`Anticheat.Enable` default false;
per-cheat Enable/Reject/Threshold/Penalty from
`World.cpp`):

- **Active-mover gate** (always on, as in the fork
  handler): `CMSG_SET_ACTIVE_MOVER` sets the session
  client mover (routed with the movement stream);
  movement packets drop until confirmed, and during
  the reject window (`server_ms + 100 +
  min(1000, latency)`; fork's per-tick `diff` term
  is not tracked at intake). No possession runtime
  exists, so the mover is always the player.
- **Flag tests**: self-root, root-move, fixed-Z,
  levitating, swim+fly, water-walk, slow-fall,
  hover — strip-on-accept, reject with forced
  heartbeat (`Map::send_forced_heartbeat`).
- **Position tests**: null client time, jump speed
  change, overspeed jump, multi-jump, stop-swim,
  time desync (+reverse-time/num-desync counting),
  teleport (3D allowance + axis-without-flags),
  fake transport, teleport-to-transport, wall
  climb (high-slope arm), no-fall-time, bad fall
  reset/stop, bad move start, speed hack
  (extrapolation with ground snap; LOS term and
  the wall-climb vmap mid arm are not ported — no
  static/vmap height split in the Rust terrain
  layer), skipped heartbeats.
- **Counting/actions**: fork 4 s finalize with
  tick/total thresholds; penalties log and kick
  (ban bits degrade to kick; account-ban writes,
  GM report delivery, packet-log dump, botting
  heuristics, forbidden-area (BG scope), explore
  and unreachable hooks are not ported).
  Wrong-ack counting wired at the existing
  `HandleMove*Ack` reject sites.

Evidence (`/tmp/anticheat_probe.py`, clone DB,
`Anticheat.Enable = 1`): `AC-GATE moved=0.00`
(movement before active-mover dropped),
`AC-LEGAL after=(-10636.1) expect_x~-10636.1`
(legal walk persists), `AC-TELEPORT
forced_heartbeats=1` and position unchanged
(+500 yd packet rejected), `AC-WATERWALK
forced_heartbeats=1`, `AC-NULLTIME-KICK
kicked=True`, `ANTICHEAT-PROBE-PASS`. Scripted
clients now send `CMSG_SET_ACTIVE_MOVER` at login
(`two_client_smoke.login_client`, real-client
behavior); `TWO-CLIENT-SMOKE-PASS` on the bench
server with the gate live. `./ci.sh` exit 0, 7/7
live suites. Bench `mangosd` rebuilt/restarted on
`bench-lan.conf`.

## Slice 2b tail — tranche 1 (damage/heal/resist/absorb auras)

Read-at-use aura folds in `school_damage`/heal paths:
`total_aura_modifier`/`total_aura_multiplier` helpers;
MOD_DAMAGE_DONE (13) / MOD_DAMAGE_TAKEN (14) flat at
coefficient 1 on direct damage (spell-power
coefficients unmodeled — recorded), MOD_DAMAGE_PERCENT
_DONE (79) / _TAKEN (87); MOD_HEALING_DONE (135) +
MOD_HEALING_DONE_PERCENT (136) on direct heals;
MOD_RESISTANCE (22) folded at resistance read (client
field not refreshed — recorded); MOD_CRIT_PERCENT
(52); SCHOOL_ABSORB (69) pools consumed inside
`deal_damage_school`, holder drops when spent.

Evidence (`live_spells`): `ABSORB hp0=100000
min_while_up=100000 dropped_at=Some(71)
landed_after=true` (exemplar 11835); `./ci.sh`
exit 0. Note: exemplar 6118 scales to a negative
amount at caster 19 (spellLevel-99 scaling quirk) —
flagged for a later scaling audit, not fixed here.

## Slice 2b tail — tranche 2 (periodic trigger)

PERIODIC_TRIGGER_SPELL (23) tick arm: triggered
`cast_spell` of the effect's trigger spell (wearer
branch for attr 0x800), added to `is_periodic()` and
to apply-time period seeding (amplitude). Fork note:
aura-triggered casts still respect the triggered
spell's own cooldown in the fork (Spell.cpp:5584,
silent fail); Rust triggered casts skip the cooldown
check — recorded divergence, visible only for
triggers with recoveryTime > period.

Evidence (`live_spells`, 1009 → 25595):
`PERIODIC-TRIGGER gos=2 hits=[23]`; `./ci.sh` exit 0.

## Slice 2b tail — tranche 3 (weapon damage effects)

`EffectWeaponDmg` (fork SpellEffects.cpp:3321) for
WEAPON_DAMAGE (58), WEAPON_DAMAGE_NOSCHOOL (17),
WEAPON_PERCENT_DAMAGE (31), NORMALIZED_WEAPON_DMG
(121): bonus = sum of flat effect values, percent
effects multiply swing+bonus, swing rolled from
UNIT_FIELD_MINDAMAGE/MAXDAMAGE, first weapon effect
only, routed through `school_damage` (NORMAL-school
armor applies). Deviations recorded: main-hand only,
AP normalization unmodeled (fields bake AP/speed),
avoidance rides the spell hit roll instead of the
melee table.

Evidence (`live_spells`, weapon range pinned to
100): `WEAPON-DAMAGE hs=70 pct=222` (Heroic Strike
78: 111 pre-armor; 26613: 350% → 350 pre-armor);
`./ci.sh` exit 0.

## Slice 2b tail — tranche 4 (speed + loss-of-control)

- `Unit::UpdateSpeed` (fork Unit.cpp:7195) run+swim
  subset: max-positive MOD_INCREASE_SPEED (31),
  bonus = max(MOD_SPEED_NOT_STACK 130,
  MOD_SPEED_ALWAYS 129 multiplier), strongest
  MOD_DECREASE_SPEED (33) slow multiplies last;
  swim via MOD_INCREASE_SWIM_SPEED (107). Applied
  as `base_speed_rate × factor` on aura apply/remove
  (new `UnitBase::base_speed_rate`, creature template
  rates mirrored at spawn). Player rates flow through
  the fork pending-change/ACK path. Mounted,
  normalization (191) and ghost arms unmodeled (no
  mount runtime).
- Holder-derived `UNIT_FIELD_FLAGS` CC bits
  (STUNNED/CONFUSED/FLEEING/SILENCED/PACIFIED from
  auras 12/5/7/27/25/60) recomputed on aura
  apply/remove, plus the fork cast-prevention gate
  (Spell.cpp:6826): FAILED_STUNNED 100, CONFUSED 22,
  FLEEING 30, SILENCED 96 (prevention-type 1),
  PACIFIED 90 (type 2). Mechanic-immunity and
  stun-interrupt-flag nuances unmodeled; fear/confuse
  movement generators not modeled. MOD_POWER_REGEN
  (85) was already folded in `regen.rs`.

Evidence (`live_spells`): `SPEED-RATIO before=0.514
sprint_off=0.446 cheetah_off=0.343` (tick-free cancel
ratios prove max-positive-wins under an ambient
combat Dazed), restored rate exact after expiry;
`CC-STUN flags=0xc0000 cast=Failed(100)`,
`CC-SILENCE cast=Failed(96)`; `./ci.sh` exit 0,
7/7 live suites.

## Slice 2b tail — singles

- THREAT (63): flat effect value onto the victim's
  threat list. `THREAT before=420 after=488`
  (Growl 2649).
- DISPEL (38): fork candidate filter (dispel-type
  mask from effect misc; MAGIC/POISON positivity vs
  friendliness), removes up to effect-value count.
  Fork random pick + level resist roll not ported
  (stored order). `DISPEL fortitude_gone=true`
  (527 strips 1243). Landing this also required
  TARGET_UNIT (25) support in `check_target` and
  `collect_targets`, and target mode 0 treated as
  caster in `check_target` — both were silent
  FAILED_UNKNOWN rejections for real corpus spells.
- INSTAKILL (1): direct damage equal to current
  health, no armor. `INSTAKILL died=true`
  (exemplar 5; death/respawn events verified).
- LEARN_SPELL (36): teaches `EffectTriggerSpell`
  to player victims via `Player::learn_spell`
  (runtime set = `deferred.spells`; the existing
  save diff persists it as a `character_spell`
  insert) + `SMSG_LEARNED_SPELL`. Pet branch
  unmodeled. `LEARN before=false after=true
  packet=true` (5210 → 5209).
- DAMAGE_SHIELD (15): `TriggerDamageShields`
  subset in the melee path (no hit/immunity roll,
  no damage-done bonuses). `DAMAGE-SHIELD dummy
  688 -> 107` (23272).
- MOD_SCALE (61): fork multiplicative object-scale
  apply/unapply. `SCALE 1 -> 2 -> 1` (24244).

Final verification: `./ci.sh` exit 0, 7/7 live
suites, bench `mangosd` rebuilt/restarted
(pid 33422, `bench-lan.conf`),
`TWO-CLIENT-SMOKE-PASS`, fixture guid 1 intact
(Ruikanis, level 19).

## Slice 2b tail — recorded residuals (not ported)

- CREATE_ITEM (24): no inventory-grant surface in
  the map layer; needs a `MapEvent` grant request
  plus a world-actor inventory handler.
- TELEPORT_UNITS (5): needs a `spell_target_position`
  world-data store and a cross-map player transfer
  event; Blink's front-leap destination fill is
  also unmodeled.
- KNOCK_BACK (98): needs a knockback movement
  generator (spline arc + player ack path).
- PERSISTENT_AREA_AURA (27) / dynobj-anchored
  effects: no dynamic-object runtime.
- SCRIPT_EFFECT (77) / DUMMY (3) customs: per-spell
  fork switches, skipped by design (same stance as
  proc dummy switches).
- Auras without reachable consumers in the current
  runtime: MOD_SKILL (30), MOD_MELEE_HASTE (138,
  swing timers don't read it), TRANSFORM (56) /
  MOUNTED (78) display-model runtime,
  MECHANIC_IMMUNITY (77) / SCHOOL_IMMUNITY (39) in
  hit resolution, PERIODIC_LEECH (53).
- Earlier recorded deviations still stand: quest
  turn-in XP (no quest runtime), pet save (no pet
  runtime), spell-power coefficients on flat
  damage/heal terms, 6118-style level-scaling audit,
  triggered-cast cooldown semantics (tranche 2).
