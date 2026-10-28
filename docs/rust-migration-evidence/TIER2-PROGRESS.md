# Tier 2 correctness depth — progress

Goal: spell numbers and per-spell behaviors fork-exact
(coefficients, immunities, remaining aura consumers,
triggered-cast cooldowns, customs corpus, Tier 1
remainders), fork-exact against repo fork
`vmangos/src/game/`.

## Slice 1 inventory — spell bonus coefficients

Fork pipeline (`Objects/SpellCaster.cpp`,
`Objects/Unit.cpp`, `Spells/SpellEntry.cpp`,
`Spells/SpellEffects.cpp`, `Spells/SpellAuras.cpp`):

- Direct magic damage (`CalculateDamage`,
  SpellCaster.cpp:988): `SpellDamageBonusDone` →
  `SpellDamageBonusTaken` → crit bonus → armor
  (physical only) → `ditheru`. Same Done→Taken order
  in `EffectHeal`/`EffectHealMechanical` (HEAL),
  `EffectHealthLeech`, power drain, and proc-dummy
  damage points.
- `SpellDamageBonusDone`: gates `SPELL_CUSTOM_FIXED_DAMAGE`,
  EX3 ignore-caster-modifiers, Ignite family; creature
  (non-player-owned-pet) multiplier
  `Creature::_GetSpellDamageMod(rank)` from world
  `Rate.Creature.*.SpellDamage` configs;
  `MOD_DAMAGE_PERCENT_DONE` with equipped-item
  (wand-exclusion) gates + paladin-seal arm;
  `MOD_DAMAGE_DONE_VERSUS` multiplier and
  `MOD_DAMAGE_DONE_CREATURE` flat by victim creature
  type mask (`1 << (type-1)`); override-class-script
  flat cases 4418/4554/4555 (4555 Idol of the Moon
  vanilla-bug formula); hunter-pet happiness
  ×1.25/×0.75; advertised benefit =
  `MOD_DAMAGE_DONE` school-masked (equipped gates) +
  `MOD_SPELL_DAMAGE_OF_STAT_PERCENT` (spirit) +
  `MOD_FLAT_SPELL_DAMAGE_VERSUS` + pet
  `GetBonusDamage()`; then `SpellBonusWithCoeffs`;
  `(damage + DoneTotal*stack) * DoneTotalMod`; tail
  spellmod DAMAGE/DOT. Totems delegate to owner.
- `SpellBonusWithCoeffs`: coeff = template
  `EffectBonusCoefficient[i]` when >= 0 else
  `CalculateDefaultCoefficient` =
  `GetCastTimeForBonus/3500 * DotFactor` (DotFactor =
  duration/15000/ticks for non-channeled DoTs);
  level penalty only without template coeff:
  spellLevel<=0 or >20 → 1 else
  `1 - (20-level)*0.0375`; `CalculateCustomCoefficient`:
  Seal of Righteousness 0.10 (0.125 two-hand), Seal
  of Command 20424 0.20 done / 0.29 taken, shaman
  chain spells coeff *= DmgMultiplier^(targets-1);
  spellmod SPELL_BONUS_DAMAGE scales coeff×100;
  `total += benefit * coeff * lvlPenalty`.
- `GetCastTimeForBonus`: cast time (duration if
  channeled) clamped 1500..7000; non-channeled DoT
  pinned 3500; hybrid direct+overtime split by PtOT;
  area-target spells halved (`IsAreaEffectTarget` set
  in SpellEntry.h:374); leech effects halved;
  ×0.95 per extra effect (dummy/slow +1,
  confuse/stun/root +2).
- `SpellDamageBonusTaken`: EX4 ignore-taken gate;
  `MOD_DAMAGE_PERCENT_TAKEN` multiplier; flat
  `MOD_DAMAGE_TAKEN` through coeffs ×stack, negative
  capped at −damage/2.
- `SpellHealingBonusDone`: gates (DmgClass none +
  passive, fixed damage, EX3); `MOD_HEALING_DONE_PERCENT`
  multiplier; override scripts 4415 (amount/4), 3736;
  advertised = `MOD_HEALING_DONE` +
  `MOD_SPELL_HEALING_OF_STAT_PERCENT` (spirit);
  coeffs; spellmod tail. `SpellHealingBonusTaken`:
  `MOD_HEALING_PCT` max-negative then max-positive
  (not multiplied stack); DmgClass-none early return;
  Healing Way dummy 29203 for Healing Wave; flat
  `MOD_HEALING` + Blessing of Light dummy amounts
  through coeffs, −heal/2 cap.
- Periodic: Done(DOT) snapshots into the aura amount
  at apply (`CalculateDotDamage` at
  HandlePeriodicDamage; heal at HandlePeriodicHeal,
  1.11 rule); Taken(DOT) per tick in `PeriodicTick`
  (5742/5825/5918), then absorb/resist. Consecration
  re-runs Done per tick; Curse of Agony/Starshards
  tick ramps (customs, Slice 5 candidates).

Rust today (`mangos-map spells.rs`): header records
"no bonus coefficients". Direct damage
(`school_damage`): armor first (fork: last), spellmod,
then flat `MOD_DAMAGE_DONE`+`MOD_DAMAGE_TAKEN` at
coefficient 1 (direct only) and percent multipliers,
all skipped under EX3 (fork Taken exempt from EX3,
gated by EX4 instead). Direct heal: spellmod +
`MOD_HEALING_DONE` flat at coefficient 1 +
percent-done; no Taken side. Periodic ticks run
through `school_damage` (spellmod DOT per tick,
percent both sides, no flat) — fork snapshots Done
at apply and applies Taken per tick. `SpellEntry`
already carries `bonus_coefficient` (template),
`custom`, family name/flags, cast-time/duration DBC
lookups; no coefficient helpers, no versus-creature
helpers, no creature-rank spell-damage rates in map
config, pet `GetBonusDamage` unmodeled (Slice 2 pet
runtime has happiness power but no bonus-damage
field), totems don't exist as casters yet.

Slice 1 plan: coefficient math on `SpellEntry`
(mangos-spells, unit-tested against hand-computed
fork values); Done/Taken functions in mangos-map
replacing the coefficient-1 blocks with fork order
(bonus → crit → armor); periodic Done snapshot at
holder build + Taken per tick; creature rank rates
through map config; spellmod SPELL_BONUS_DAMAGE op.
Recorded exclusions for this slice: totem owner
delegation (no totem casters), shaman chain target
index (cast context lacks it), Seal of Righteousness
weapon-handedness (no weapon surface in map; base
0.10 used), Consecration/Agony/Starshards tick
customs (Slice 5), melee/ranged `MeleeDamageBonus`
arms (existing melee pipeline, separate audit).

## Slice 1 result — spell bonus coefficients

Implemented the fork bonus pipeline for magic-class
damage and heals: coefficient math on `SpellEntry`
(template coefficient, default formula with
cast-time-for-bonus incl. hybrid split/area halving/
leech halving/extra-effect 0.95 steps, Nostalrius
level penalty, custom coefficients for Seal of
Command + shaman chain shape); `SpellBonusWithCoeffs`
with the SPELL_BONUS_DAMAGE spellmod; Done/Taken for
damage and healing (creature rank spell-damage
rates, versus-creature flat/pct, override class
scripts 4418/4554/4555/4415/3736, stat-percent
benefits, hunter-pet happiness, Healing Way and
Blessing of Light taken arms, MOD_HEALING_PCT
max-neg/max-pos). `school_damage` reordered to fork
`CalculateDamage` (Done → Taken → crit → armor →
ditheru → resist); periodic Done snapshots at aura
apply with Taken per tick; SCALES_WITH_CREATURE_LEVEL
implemented via class-1 CLS ratio at all effect-value
call sites. 6118 audit: the Rust formula is
line-identical to the fork — the negative exemplar
amount is fork-faithful data behavior, pinned by unit
test. Unit tests `coefficient_math_matches_fork_formulas`
and `effect_value_scaling_matches_fork` pass.

Live evidence (`/tmp/coeff_probe.py`): Mind Blast
8092 unbuffed 43-45, with +840 spell damage (24243)
268-269 — shift exactly 225 = 840 × 0.268 template
coefficient; Corruption 172 ticks 10 → 77 = 10 + 840
× 0.08 snapshot. `COEFF-PROBE-PASS` (one flake rerun:
tick-collection timing), oracles pass,
`TWO-CLIENT-SMOKE-PASS`, `./ci.sh CI_EXIT=0`
(`/tmp/ci_t2s1.log`), bench restarted (`PORT_OK`).

Recorded exclusions: melee/ranged `MeleeDamageBonus`
arms keep pre-slice math; totem owner delegation;
wand equipped-item gates (no entry fields); pet
`GetBonusDamage`; Seal of Righteousness (needs
SpellIconID on the entry subset); shaman chain
target-count scaling (cast context lacks target
index); Consecration/Agony/Starshards per-tick
customs (Slice 5); creature spell-damage conf rates
parse (defaults 1.0 like the other creature rates).

Slice 1 closes Tier 2 item (1). Next: Slice 2
immunities.

## Slice 2 inventory — immunities

Fork model: aura handlers maintain per-unit
`m_spellImmune[op]` lists of (spellId, type) —
MECHANIC_IMMUNITY (77)→IMMUNITY_MECHANIC,
SCHOOL_IMMUNITY (39)→SCHOOL, EFFECT_IMMUNITY (37)→
EFFECT, STATE_IMMUNITY (38)→STATE (aura type),
DAMAGE_IMMUNITY (40)→DAMAGE, DISPEL_IMMUNITY (41)→
DISPEL; MECHANIC_IMMUNITY_MASK (147) is read live
from holders. Resolution functions
(`Unit.cpp:5570-5835`): `IsImmuneToDamage` (damage
list by school mask + school list with the
positivity-mismatch / EX-immunity-to-hostile-and-
friendly rule), `IsImmuneToSpell` (dispel + school +
spell mechanic + mask auras; gates NO_IMMUNITIES
0x20000000, passive skips dispel, PURGES/EX2 skip
school), `IsImmuneToSpellEffect` (effect id,
effect mechanic + mask, aura state), `IsImmuneToSchool`.
Call sites: `AddUnitTarget` strips immune effects
from the per-target effectMask; spell-level immune
→ `SPELL_MISS_IMMUNE` (7) in hit resolution;
positive-spell CheckTarget → TARGET_AURASTATE;
periodic ticks skip with `SendSpellOrDamageImmune`;
melee damage calc checks damage immunity.
Creature overrides read `mechanic_immune_mask` /
`school_immune_mask` template columns (not on self
casts); permanent type/statics entries (spellId 0)
for Mechanical/Elemental, immunity_flags
(taunt/stat/cast-speed/disease/poison/curse),
SESSILE. Purge side effects on apply when the
immunity spell has EX_IMMUNITY_PURGES_EFFECT
(0x8000): mechanic aura removal, negative
school-aura removal + `UNIT_FLAG_IMMUNE`, state
aura removal, dispel-type removal; school immunity
also strips INVULNERABILITY_BUFF_CANCELS auras and
Banish sets UNIT_STATE_ISOLATED.

Rust today: no immunity state or checks anywhere in
hit resolution (whole-spell per-victim hits, no
per-effect masks); template immune masks already
loaded on `CreatureInfo`. Plan: derive immunity
entries from holders on demand (recompute idiom) +
permanent creature entries from template data;
spell-level immune → miss 7 in the finish-cast hit
loop; per-effect skip in `apply_effects`; damage
immunity in ticks and the melee damage funnel;
purge side effects after holder add; positive
CheckTarget gate where the cast-target validation
lives if reachable. The flag-carrier/BG arms and
IsIgnoringCasterAndTargetRestrictions are Tier 3 /
unmodeled, recorded.

## Slice 2 result — immunities

Implemented fork immunity resolution as holder-derived
recomputes: immunity entry lists from holders
(MECHANIC 77, SCHOOL 39, EFFECT 37, STATE 38, DAMAGE
40, DISPEL 41, MASK 147) plus permanent creature
entries (Mechanical/Elemental types, immunity_flags,
SESSILE) and template mechanic/school immune mask
overrides. `IsImmuneToDamage`, `IsImmuneToSpell`,
`IsImmuneToSpellEffect`, and `IsImmuneToSchool`
ported with the shared positivity rule. Wiring:
spell-level immune → `SPELL_MISS_IMMUNE` (7) in the
finish-cast hit loop; per-effect strip in
`apply_effects`; positive-spell CheckTarget →
FAILED_TARGET_AURASTATE (103); periodic damage/heal
ticks test immunity and send
`SMSG_SPELLORDAMAGE_IMMUNE`; damage-immunity backstop
in `deal_damage_school` for melee/script funnels.
Apply-time purge side effects ported
(EX_IMMUNITY_PURGES_EFFECT gates): mechanic aura
removal, state aura removal, negative school-aura
removal, dispel-type removal, invulnerability-buff
strip; `UNIT_FLAG_IMMUNE` and Banish
`UNIT_STATE_ISOLATED` as holder recomputes.

Live evidence (`/tmp/immune_probe.py`): Fear 5782 at
Harvest Golem (mechanic_immune_mask FEAR) →
SPELL_GO miss condition 7, Mind Blast control hits;
Earth Shock 8042 at Rock Elemental
(school_immune_mask NATURE) → miss 7, control hits.
`IMMUNE-PROBE-PASS`, oracles pass (one fixture-drift
incident: a late logout save raced the probe
restore; oracle columns re-restored and the restore
script now also rewrites class/level/rest_bonus),
`TWO-CLIENT-SMOKE-PASS`, `./ci.sh CI_EXIT=0`
(`/tmp/ci_t2s2.log`), bench restarted (`PORT_OK`).

Recorded exclusions: WSG flag-carrier immunity arms
(Tier 3), `IsIgnoringCasterAndTargetRestrictions`
script gates, ReflectSpell/DamageSplit immune-skip
arms inside DealDamage (unmodeled in the damage
funnel), evade-miss precedence over immune (no evade
miss state), spell-steal dispel-immunity branch in
dispel resolution (listed for Slice 2 follow-up if
dispel paths grow), player race-table creature type
beyond the humanoid default (shapeshift types).

Slice 2 closes Tier 2 item (2). Next: Slice 3
remaining aura consumers.

## Slice 3 inventory — remaining aura consumers

Fork: PERIODIC_LEECH (53) ticks like periodic
damage (Taken(DOT), absorb/resist, non-melee log)
then heals the caster for actual damage dealt ×
EffectMultipleValue (SPELLMOD_MULTIPLE_VALUE),
0.5× threat assist. MOD_SKILL (30) /
MOD_SKILL_TALENT (98) write the packed
PLAYER_SKILL_BONUS temp/perm halves via
ModifySkillBonus (known skills only; defense
refreshes defense bonuses). Attack-speed auras
(MOD_ATTACK_SPEED, MOD_MELEE_HASTE 138,
MOD_RANGED_HASTE, ranged-ammo haste) percent-mod
both `m_modAttackSpeedPct[att]` and the
BASEATTACKTIME field itself (haste divides:
factor 100/(100+val) for positive amounts,
(100-val)/100 for negative at 5875);
SetAttackTime writes raw × pct; swing reset =
field value. SPELLMOD_HASTE scales the aura amount
once at apply.

Rust today: PERIODIC_LEECH has no tick arm (holder
sits inert); skill bonus fields are written 0 at
load and never updated (melee outcome uses
level×5 both sides — no real skill values);
BASEATTACKTIME is written raw by creature spawn
and player weapon state, swing timers read it
directly, no pct state exists. Plan: leech tick
arm reusing the school-damage pipeline with
health-delta heal-back; UnitBase attack-speed pct
array + holder recompute rewriting the three
attack-time fields (weapon state multiplies by
stored pct, fork SetAttackTime shape); skill
bonus field recompute for players. Defense-skill
combat read and actor skill-value checks stay
as-is (recorded).

### Slice 3 result

Implemented as planned. `PERIODIC_LEECH` ticks
through the PERIODIC_DAMAGE arm: apply-time
Done(DOT) snapshot extended to leech, Taken per
tick, caster healed by victim health delta across
the tick's `school_damage` (the fork DealDamage
result proxy in this sequential pipeline) times
effect multiple value after
`SPELLMOD_MULTIPLE_VALUE`. Two periodic-list
omissions found by the probe and fixed: the
state-creation periodic list and
`AuraEffectState::is_periodic` both lacked 53, so
the holder applied with period 0 and never ticked.
Attack speed: `UnitBase.mod_attack_speed_pct[3]`
recomputed from holders on aura add/remove
(MOD_ATTACKSPEED/MOD_MELEE_HASTE/MOD_RANGED_HASTE/
MOD_RANGED_AMMO_HASTE, `SPELLMOD_HASTE` applied per
holder amount at recompute); the three attack-time
fields rewrite by the old→new pct ratio (fork
scales the field itself) and player weapon state
writes raw × pct (`SetAttackTime`). Skill bonus:
`refresh_skill_bonuses` rewrites the packed
temp/perm halves over the 127 skill slots from
MOD_SKILL (temp) and MOD_SKILL_TALENT (perm)
holders for known skills.

`CONSUMERS-PROBE-PASS` (/tmp/consumers_probe.py):
Siphon Life 18881 ticks 45/45 on a trogg with
caster health 1451 -> 1496 (exactly one tick of
heal-back at multiple value 1); haste spell 3547
(+100) takes UNIT_FIELD_BASEATTACKTIME 2900 ->
1450 (fixture mainhand delay 2900 halved); skill
spell 30920 writes +5 temp bonus fields for
Swords and Daggers (base 4 + die 1). Siphon Life
deterministically never ticks on the Harvest
Golem: the fork grants mechanicals permanent
IMMUNITY_STATE PERIODIC_LEECH (Creature.cpp:370),
which the Slice 2 immune entries already model —
fork-exact, verified by diagnostic trace
(effect stripped at apply, Corruption ticks on
the same golem).

Collateral fork-exact bug fixed: the update-field
viewer's `can_see_health` was hardcoded false, so
even the owner received percent health (fork
`Object.cpp:775`: real values when
`Player::CanSeeHealthOf` — same-raid owner-player
or Beast Lore empathy). `Map::viewer_for` now
computes the fork relations (owner/charmer from
SUMMONEDBY/CHARMEDBY, same-raid, empathy aura)
and also feeds OWNER_ONLY/GROUP_ONLY/SPECIAL_INFO
grants and FFA-PvP strip from the same inputs,
which were previously never set. Found because
probe health reads were percentages (500 seeded
health read 24 on the wire). Unmodeled and
recorded: PLAYER_CHEAT_DEBUG_TARGET_INFO and the
game-master viewer bit (no cheat/GM state in the
runtime), item-owner viewer arm (no world-item
create path).

Recorded remainders: SPELLMOD_HASTE recompute vs
fork apply-once can drift if caster spellmods
change while the aura is active; leech heal uses
health-delta proxy (absorb shields on the victim
would over/under-count exactly as the delta sees
it); skill bonus is fields-only (melee outcome
still level×5, no defense-skill combat read);
stack>1 Done snapshot passes stack 1 at holder
build (later stacks may diverge).

Oracles `live_character_load` + `live_char_enum`
pass; `TWO-CLIENT-SMOKE-PASS`; `./ci.sh`
CI_EXIT=0 (/tmp/ci_t2s3.log). CI live_spells
dummy level pinned in-test (bench world DB dummy
is level 30 vs the 22% hit floor made
cast-landing sections RNG-sequence-dependent;
Lightning Shield damage band re-derived for fork
partial resists 3..=20).

## Slice 4 — triggered-cast cooldown semantics

### Inventory (fork vs Rust)

Fork (`Spell.cpp` CheckCast 5482/5498, prepare
3587-3608, cast() 3798/3819, TakePower 5226,
SendSpellCooldown 4088, CheckPower 7102;
`Player::AddCooldown` Player.cpp:21871,
`SpellCaster::AddCooldown` 2131): a triggered
cast (`m_IsTriggeredSpell`) skips the
cooldown-ready check, the GCD check, the power
check (`CheckPower`), GCD addition, and the
cast-in-progress gate; it keeps its DBC cast
time (timer 0 executes in the call point,
timer > 0 executes on later ticks without
occupying the current-spell slot; channeled
triggered casts do take the slot). At cast()
every non-passive spell — triggered included —
runs `SendSpellCooldown` -> `AddCooldown`
(spell + category, SPELLMOD_COOLDOWN, permanent
hold for COOLDOWN_ON_EVENT, already-existing
guard: an unexpired non-permanent entry is left
untouched). `TakePower` skips only cast-item and
**aura-triggered** casts (`m_triggeredByAuraSpell`);
a plain triggered cast (SPELL_EFFECT_TRIGGER_SPELL,
script/EventAI triggered flags) still pays power,
saturating, despite never being power-checked.
Proc execution adds the proc-event cooldown as a
forced-duration `AddCooldown` on the triggered
spell after the cast (UnitAuraProcHandler.cpp:518),
subject to the same already-existing guard.

The depth-gaps tranche-2 note ("aura-triggered
casts respect the triggered spell's own cooldown,
Spell.cpp:5584, silent fail") does not hold
against the current fork source: the only
ready checks are CheckCast (skipped when
triggered) and CheckPetCast. Periodic-trigger
casts fire every tick regardless of the triggered
spell's recovery; what the cooldown they add
gates is later manual casts and proc-event
ready checks. Recorded as a correction here.

Rust today: ready/GCD/power checks match (skipped
when triggered), but at cast completion one
`!cast.triggered` guard skips TakePower, GCD add,
and cooldown add together — so triggered casts
never start cooldowns (deviation A) and never pay
power even when not aura-triggered (deviation B).
Cast time is forced to 0 for triggered casts
(deviation C), and the single ActiveCast slot
fails triggered casts with SPELL_IN_PROGRESS
while any cast runs (deviation D). Proc forced
cooldown exists as a bare spell-key insert
(no category, no already-existing guard).
Plan: split the completion guard per fork
(TakePower unless aura-triggered/cast-item via a
new `triggered_by_aura` cast flag; GCD add only
non-triggered; cooldown add always except
passive, with the already-existing guard);
real cast times for triggered casts with a
pending-triggered list outside the current-cast
slot (channeled still takes the slot); proc
forced cooldown through the same guarded add.
Adjacent recorded non-goals: fork GCD is
per-StartRecoveryCategory (Rust: one timer),
PLAYER_CHEAT_NO_COOLDOWN/NO_POWER (no cheat
state), SMSG_SPELL_COOLDOWN login dump (client
predicts from SPELL_GO today).

## Slice 4 result — triggered-cast cooldown semantics

Implemented per inventory. `CastOptions` and
`ActiveCast` carry `triggered_by_aura` (set only
by periodic-trigger and proc-trigger cast sites;
effect-trigger, script, AI, and world-actor sites
pass false). Cast completion is split per fork:
`TakePower` runs unless the cast is from an item
or aura-triggered; the GCD starts only for
non-triggered casts; the cooldown add runs for
every non-passive cast. `add_cooldowns` and
`add_cooldowns_item` gained the passive gate and
the already-existing non-permanent-entry guard,
and the proc internal cooldown now routes through
the same guarded `add_cooldowns` with a forced
duration instead of a bare insert.

Triggered casts keep their real DBC cast times
(deviations C and D): non-channeled triggered
casts no longer occupy `Map.casts`. Zero-time
triggered casts execute inline through the
extracted `execute_cast`; timed triggered casts
sit in `Map.pending_triggered` and tick in
`update_casts`; channeled triggered casts still
take the current-cast slot.

Collateral fix found by the probe: the effect
loop's first `APPLY_AURA` arm used `break`, which
silently skipped every later non-aura effect of
the spell (e.g. a trailing `TRIGGER_SPELL`). The
holder now builds once via a `holder_applied`
flag and later effects execute.

Wire evidence (`/tmp/trigcd_probe.py`,
`TRIGCD-PROBE-PASS`): Low Swipe 8716 triggers
Knockdown 5164, and an immediate manual 5164
returns reason 60 (`FAILED_NOT_READY`) — the
triggered cast started the spell's real cooldown;
Trigger Test 5103 triggers Freeze III PROC 2647
and mana falls exactly 2572 → 2504 (68), proving
plain triggered casts pay power while
aura-triggered casts do not.

Test fallout from the effect-loop fix: in
`bench_spells_and_auras` the dummy now takes the
trailing-effect damage the fork also deals and
dies to reflected shield damage before the
finisher sections; the test revives and tops up
the dummy after the damage-shield section (test
setup only, library behavior unchanged and
fork-faithful).

Verification: `./ci.sh` green (`CI_EXIT=0`,
`/tmp/ci_t2s4.log`); oracle byte-compare tests
pass after guid 1 fixture restore;
`TWO-CLIENT-SMOKE-PASS` on the final binary;
bench `mangosd` rebuilt and restarted, port
18099 listening.

Recorded remainders: fork GCD is
per-StartRecoveryCategory (Rust keeps one timer);
no `PLAYER_CHEAT_NO_COOLDOWN`/`NO_POWER` state;
no `SMSG_SPELL_COOLDOWN` login dump (client
predicts from `SPELL_GO`); pending triggered
casts are not pushed back or interrupted by
movement (event-spell model).

## Slice 5 inventory — customs corpus

Three fork surfaces carry per-spell hardcoded
behavior, all currently silent no-ops in Rust
(`apply_effects` has no DUMMY/SCRIPT_EFFECT arm;
`procs.rs` returns OK for dummy-aura holders
like `HandleNULLProc`):

- `Spell::EffectDummy`
  (`vmangos/src/game/Spells/SpellEffects.cpp:304-1451`):
  outer target-type branches, then per-family
  switches (9 `SPELLFAMILY_*` cases) with 90
  unique spell-id cases, plus family-flag
  branches keyed by `SpellFamilyFlags`. Falls
  back to pet-aura registration and
  `sScriptMgr.OnEffectDummy` (content scripts,
  Tier 3).
- `Spell::EffectScriptEffect` (same file,
  3578–4467): 5 family cases, 82 unique spell-id
  cases. Falls back to the DB script system
  (`ScriptsStart(sSpellScripts, …)`); bench
  `spell_scripts` has 40 rows and Rust does not
  execute them (recorded remainder).
- `Unit::HandleDummyAuraProc`
  (`vmangos/src/game/UnitAuraProcHandler.cpp:532-1127`):
  family switch (GENERIC, MAGE, WARRIOR,
  WARLOCK, PRIEST, DRUID, ROGUE, HUNTER,
  PALADIN, SHAMAN) with 32 unique spell-id
  cases; the only non-NULL dummy proc handler
  in the fork's `AuraProcHandler` table besides
  fear/invisibility/mod-damage/mod-resistance
  handlers (already covered or recorded with
  the proc system).

Extraction is mechanical and reproducible:
`/tmp/extract_customs.py` collects `case` ids at
each `switch (m_spellInfo->Id)` /
`switch (dummySpell->Id)` brace depth;
`/tmp/customs_inventory.py` joins the 204 unique
ids against bench `spell_template` (max build
≤ 5875) and reachability tables
(`playercreateinfo_spell`, `npc_trainer`,
`spell_learn_spell`, `item_template` spells,
`creature_template` spell ids, `creature_spells`
lists). All 204 exist in `spell_template`.
DUMMY: 90 ids (38 with a reachability hit, all
family 0). SCRIPT: 82 ids (23 reachable, 6
family ≠ 0). PROC: 32 ids (8 reachable by table
join, 19 family ≠ 0 — class talents/set bonuses
learned via talent/item paths the joins do not
see, so family is the better player-reachability
signal there).

Tranche plan, by reachable priority:

1. Player combat. PROC: Seal of Righteousness
   (20154, 20287–20293, 21084), Eye for an Eye
   (9799, 25988), Sweeping Strikes (12292,
   18765), Retaliation (20230), Vampiric Embrace
   (15286), Blade Flurry extra-attack (13877),
   Clean Escape (23582), set-bonus mana refunds
   (28719, 28789, 28809, 28823, 28847, 28849,
   26169), Zandalari trinkets (24658, 24661),
   Obsidian Armor (27539), Adaptive Warding
   (28764). DUMMY: Deep Wounds (12162, 12850,
   12868), Last Stand (12975), Preparation
   (14185), Blood Fury (20572), Tame Beast
   (13535). SCRIPT: Rockbiter Weapon Proc threat
   (20865–20871).
2. Items and engineering (reach `I`): Universal
   Remote (8344), Shrink Ray (13006),
   Net-o-Matic (13120), Gnomish Mind Control Cap
   (13180), Six Demon Bag (14537), Symbol of Life
   (8593), dragonlings/battle chicken/goblin bomb
   summons (23074–23134), Snowball (21343),
   costumes and holiday items.
3. Creature and raid spells (reach `C`/`S`):
   Shadow Flame cloak check (22539, 22972–22985),
   Knock Away (10101), Lava Bomb (19411, 20474),
   Blood Siphon (24324), Drain Mana family
   (25671, 25755…), Twisted Reflection (21063),
   Viscidus set (25926–26003), Breath of Sargeras
   (28352), Widow's Embrace (28732).
4. Recorded remainder: quest traps and one-off
   event spells, battleground flag clicks (Tier 3
   scope), test/DND spells, and the content-script
   fallbacks (`OnEffectDummy`, `spell_scripts` DB
   execution, `scripts/spells/*.cpp` SpellScripts)
   — named here so the slice does not silently
   claim them.

Implementation shape (fork-faithful, first-of-kind
by the driver): a `customs` module in
`mangos-map` with `effect_dummy`,
`effect_script_effect`, and `dummy_aura_proc`
entry points mirroring the fork switch structure,
called from the `apply_effects` DUMMY (3) /
SCRIPT_EFFECT (77) arms and from the proc
handler for aura type 4. Unhandled ids keep the
fork's no-op/default behavior; each ported case
cites its fork lines.

## Slice 5 tranche 1 result — proc table + player-combat customs

Plumbing first, fork-shaped: `CastCustomSpell`
base points now exist end to end
(`SpellEntry::calculate_effect_value_bp` with
the fork's `bp - baseDice` start, `CastOptions`/
`ActiveCast.custom_base_points`, threaded through
`apply_effects`/`apply_spell_aura`/summon/create-
item), including the fork `TriggerProccedSpell`
quirk that engages zero-valued optionals for
unset slots (unit-tested). `SpellEntry` gained
`spell_icon_id` (`spell_template` field 121) for
the icon-keyed proc gates, and `CooldownEntry`
gained the originating `spell_id` so category
cooldowns carry their fork entry identity.

The full non-NULL `AuraProcHandler` table is
ported in `procs.rs`: dummy (4) with the whole
family switch (Seal of Righteousness weapon-speed
formula, Eye for an Eye, Sweeping Strikes incl.
the Execute branch, Retaliation, Blade Flurry,
Vampiric Embrace, set-bonus refunds, Zandalari
stack burns, Adaptive Warding, Obsidian Armor,
Viscidus counter, Illusion Passive despawn,
Twisted Reflection), fear/root damage-break
(7/26/60), mod-damage school gate + 24659 stack
burn (13), invisibility (18), Inner Fire gate
(22), casting-speed (65), power-cost schools
(72/73), reflect school (74), mechanic gates
(77/117), add-target trigger (109), override
class scripts (112: Improved Blizzard chill,
Shadowguard, Rejuvenation/Regrowth set bonuses,
heal-cast armor), and Flurry's haste guard (138).
`EffectDummy` tranche 1 and `EffectScriptEffect`
Rockbiter threat live in `customs.rs`: Deep
Wounds, Last Stand, Preparation (with
`SMSG_CLEAR_COOLDOWN`), Blood Fury, Tame Beast.

Collateral consumer fix the probe exposed:
`MOD_INCREASE_HEALTH` (34), `MOD_INCREASE_ENERGY`
(35), and their percent forms (133/132) were
never consumed — max health/power recomputes ran
at zero-aura modifier defaults, so e.g. Last
Stand's buff did nothing. The `TOTAL_VALUE` /
`TOTAL_PCT` groups now feed player and creature
`UpdateMaxHealth`/`UpdateMaxPower` in fork order,
with the fork handlers' current-value adjustments
on apply/remove (Last Stand follow/floor-at-1,
bear-form ceil ratio, percent preservation,
energy `ModifyPower`).

Wire evidence (`/tmp/customs_probe.py`,
`CUSTOMS-PROBE-PASS`): Last Stand max health
2221 → 2887 (exactly +30% base points through
the new consumer); Sprint gated at reason 60,
Preparation emits `SMSG_CLEAR_COOLDOWN` 2983 and
Sprint recasts; Seal of Righteousness melee-procs
spell 25742 (`SMSG_SPELL_GO` + damage log).
`./ci.sh` green (`CI_EXIT=0`, `/tmp/ci_t2s5.log`),
oracle byte-compare tests pass, `TWO-CLIENT-
SMOKE-PASS`, bench restarted on the final binary.

Recorded remainders for later tranches/slices:
weapon-enchant `CastItemCombatSpell` on the SoR
proc; holder cast-item identity on proc casts;
`SelectRandomUnfriendlyTarget` `LoS` and
valid-attack-target filters; Viscidus text
emotes; the Lifegiving Gem (23782) remove-cast
special in the health-percent handler; pet-power
maxima on creatures (mana only recomputed);
stack-scaled TOTAL amounts inherit the Slice 1
stack-amount convention.

## Slice 5 tranche 2 result — items/engineering customs

`Map::effect_dummy` / `effect_script_effect` in
`vmangos-rs/crates/mangos-map/src/customs.rs` now
take the fork's per-cast context the tranche-1
signature lacked: whether the cast carried an
item (`m_CastItem`) and the cast counter byte
(from `ActiveCast`), threaded through
`apply_effects`. Ported against
`Spell::EffectDummy` (`SpellEffects.cpp:304-1451`)
and `Spell::EffectScriptEffect` (`3578-4467`):

- EffectDummy: Universal Remote 8344 (mechanical
  gate, 8345/8346/8347 roll, 99999 threat on the
  aggro roll), Symbol of Life 8593 (JUST_ALIVED
  promotion subset), Polly 9976, capture spells
  11885-11889/12699 (dead-creature despawn),
  Shrink Ray 13006 (direct `CastSpell` aura apply
  + explicit `SendSpellGo` re-send, new
  `Map::send_spell_go_simple`), Net-o-Matic
  13120, Mind Control Cap 13180, Six Demon Bag
  14537, Test Fetid Skull 17271, dragonlings/
  chicken/bomb dispenser 23074-23076/23133/23134
  (cast-item gate, fork's 23134 self cast),
  Transporter Arrival 23448, Ultrasafe
  Transporter 23453 (incl. the 5% hardcoded
  teleport), Hourglass Sand 23645, Gift of Life
  23725, Predator's Midnight 24930, Summon of
  the Dawn 25860, Ribbon of Souls 26899,
  Dire Brew 27662, Greatfather's Trumpet 28414
  (`PlayDirectSound` now `pub(crate)`), Snowball
  21343 (same-raid gate), and the spell-icon
  1661 Berserking arm (26635 with health-scaled
  base points in all three slots).
- EffectScriptEffect: Bending Shinbone 8856,
  Jubling Cooldown 23853 (item cooldown on spell
  23851 via `add_cooldowns_item`), Uther's/Grom's
  Tribute 24194/24195 (race switch), stack burns
  24590/24660/26465, Trick 24714 and the hallowed
  wand costumes 24717-24720/24737 (gendered,
  not-in-combat), Cannon Fire/Magic Wings
  24731/24742, Trick or Treat 24751, Mistletoe
  26004/26218, Rotate Trigger 26137, Winter
  Wondervolt 26275 (≤1y gate, replace same-set
  aura), Black Qiraji 26656 (outdoor spell; see
  remainder), Bag of Candies 26678. The tranche-1
  Rockbiter threat arms are unchanged.

Random-selection helpers follow the fork's
`urand`/`roll_chance_i` draws on the spell RNG.
`MOD_SHAPESHIFT` presence stands in for
`GetShapeshiftForm()` (no separate form-byte
model), and raid membership is self-or-group.

Wire evidence (`/tmp/customs_probe.py` extended,
`CUSTOMS-PROBE-PASS`): Berserking 26296 →
`SMSG_SPELL_GO` 26635; Shrink Ray produces the
fork's double `SMSG_SPELL_GO` for 13006
(original + effect re-send); tranche-1 legs
re-pass (Last Stand +654 on a 2181 base = 30%
exactly, tracked through the adjacent trogg's
3148 debuff refresh; Preparation clears Sprint;
SoR procs 25742). `./ci.sh` green (`CI_EXIT=0`,
`/tmp/ci_t2s5b.log`), oracle byte-compare tests
pass after `ORACLE-FIXTURE-RESTORED`,
`TWO-CLIENT-SMOKE-PASS` (smoke reset spot moved
to Orgrimmar: the Valley of Trials spot sits 9y
from the Kaltunk spawn, which kills the smoke
bots on a fresh bench), bench restarted on the
final binary and listening on 18099.

Recorded tranche-2 remainders (unmodeled
runtime, not silently dropped): Symbol of Life
loot-recipient/addon side effects; Transporter
quest credit and Evil Twin aura chain (23445/
23446 casts land, credit unmodeled); Qiraji
indoor/outdoor selection needs the Map.dbc
mount flag (outdoor 26655 always) and its
`spell_scripts` teleport stays with tranche 4;
Mind Control Cap / Universal Remote charm runs
through the existing charm aura path with no
possess-session model; Greatfather's Trumpet
sound is caster-only here (fork `PlayDirectSound`
packet has no range filter either); Winter Veil
minipet summons (26532 family) proceed through
the generic summon effect — the fork's
same-minipet early-out has no minipet model to
check; aura-state mask writes (Berserking) have
no aura-state store.

## Slice 5 tranche 3 result — creature/raid customs + family arms; tranche 2 corrections

Tranche 3 re-read every fork case in
`Spell::EffectDummy`/`EffectScriptEffect`
against the Rust arms. That audit found several
tranche-2 arms ported from faulty case notes;
they are corrected here (the tranche-2 result
above stands for its wire-observed legs —
Berserking, Last Stand, Preparation, SoR — but
the per-case claims for the items below were
wrong and are superseded):

- 8344 Universal Remote: no mechanical gate and
  no threat add; roll is 8345 / 8346 / victim
  self-cast 8599.
- 13006 Shrink Ray: full roll table (13003
  normal case, direct `AddAura` + re-sent GO for
  13004/13010 on caster or victim, party-wide
  fallthrough casts); no shapeshift gate.
- 13120 Net-o-Matic: 16566 / 13119 / 13099, all
  on the victim.
- 13180 Mind Control Cap: `urand(0,5)` with a
  no-op outcome and a shapeshift-gated reversal.
- 14537 Six Demon Bag: 11921/13322/21179/13323/
  13323(self)/25189/14642(self) table.
- 21343 Snowball: knockdown requires same raid
  (not absence of it), victim self-casts 21167,
  gated on 21354 absence.
- 23134 Bomb Dispenser: cast-item gated like
  the others; victim casts 13261 on the caster
  on the backfire roll, else 13258 (it never
  recasts itself — the earlier self-cast could
  recurse through the dummy arm).
- 23448/23453 Transporters: fork roll structure
  and the Kalimdor failure teleport.
- 23645/23725 act on the caster, not the victim.
- 24930 is Hallow's End Treat (24924-24927);
  25860 is Reindeer Transformation (mounted
  gate, speed-rate split 25859/25858); 26899 is
  Friendship Bracelet (26898 removal + 26921/
  26664); 27662 is Silver Shafted Arrow
  (player victim self-cast 27570); 28414 is Call
  of the Ashbringer (random sound to the
  caster). 8856/17271 gates read the cast's item
  *target* in the fork; the cast-item flag
  stands in (identical on the item-use path).

New in tranche 3 (`customs.rs`): the paladin
family arms (Flash of Light icon-242 carrier →
19993 custom heal with libram adds; Judgement
seal consume → the seal's effect-3 judgement
spell; Seal of Fury threat term shared with
Rockbiter), druid 9033 shapeshift root/snare
strip, threat-percent arms (10101, 9204, 17950,
21147), Shadow Flame family, Hakkar blood pair
24320/24324, Drain Mana web (25671/25676/25754/
25755/26457/26559), Valentine guards/citizens/
amorous/end-check, Razelikh group teleport,
Bone summons, Breath of Sargeras, Chains of
Kel'Thuzad, Widow's Embrace, Improved Sprint
(via `RemoveAurasAtMechanicImmunity` with the
fork's non-positive flag), EPL 31929/32061 kill
credit, and the dummy-side creature/raid set
(3360, 4050, 6655, 7671 entry swap, 8897, 9009,
13567, 15998, 16032, 17009, 17190, 18350,
18954/18955/18993, 19411/20474, 19593 Buru egg,
19614, 23138, 23185/25044, 23845, 24150, 24781,
25716, 25938/26003 Viscidus, 26399, 26626,
26879, 27798, 28006, 28091, 28098/28110 Tesla,
28345, 28697, 29518). Supporting plumbing:
`triggered_by_aura_spell`/`triggered_by_spell`
threaded through `CastOptions`→`ActiveCast`
(fork `m_triggeredByAuraSpell` /
`m_triggeredBySpellInfo`, consumed by
13567/18350), set at periodic-trigger ticks,
proc casts, and `TRIGGER_SPELL` effects.

Cone targeting landed as tranche-3 collateral
(the Shadow Flame arm was unreachable without
it, as is player Cone of Cold): target mode 24
in `check_target` + enumeration
(`FillAreaTargets(PUSH_IN_CONE)` shape — effect
radius else max range, `HasInArc` in atan2
space, negative arc = back cone), backed by a
new `spell_cone` loader (102 bench rows; fork's
60-degree default). Cone victim search uses the
existing 2D radius convention and skips the
fork's moving-caster leeway bonus.

Evidence: `./ci.sh` green (`CI_EXIT=0`,
`/tmp/ci_t2s5c.log`); wire probe
`CUSTOMS-PROBE-PASS` with new legs — Judgement
consumes Seal of Light (20184+1 → GO 20185),
FoL carrier 19750 → GO 19993 with a +70 heal
jump, Shrink Ray fork outcomes (13003/13010
GOs), SoR proc across victim spawns; in-process
`live_spells` SHADOWFLAME section (cone hit,
cloak gate blocks 22682, uncloaked cast lands
it) — `LIVE-SPELLS-PASS`. Oracle byte-compares
pass after `ORACLE-FIXTURE-RESTORED`;
`TWO-CLIENT-SMOKE-PASS`; bench restarted on
the final binary (18099).

Discovered cross-slice divergence (recorded,
not fixed here): the fork accumulates spell
damage during the per-target effect loop and
deals it after all effects resolve; Rust deals
school/weapon damage immediately inside the
loop. When direct damage is lethal, later
effects of the same cast (e.g. Shadow Flame's
script arm with its `IsAlive` gate) therefore
fire in the fork but not in Rust. Deferred
damage is a damage-funnel restructure and
belongs to its own slice.

Mechanical coverage after tranche 3 (fork case
id → Rust handler presence): EffectDummy 79/90,
EffectScriptEffect 74/82 (plus all family
arms), HandleDummyAuraProc 31/32. The gaps are
the tranche-4 record below.

## Slice 5 tranche 4 — recorded remainder (slice closeout)

Deliberately unported, with the blocking
runtime named; each currently no-ops, matching
the fork when its script/DbScript hooks are
unregistered:

- Instance-script data (`InstanceData::SetData`
  / script calls): 6700, 27517, 24693, plus
  20863 (EventAI phase + gameobject flag).
- Battleground state (Tier 3 scope): 23383,
  23384 flag carriers; 30882 EPL `OutdoorPvP`
  activity/friendliness (the 31929/32061 arms
  themselves are ported).
- Gameobject-caster identity and trap runtime:
  19395 Gordunni Trap, 14813 (also needs
  dynamic gameobject summon), 23019 Crystal
  Prison (dynamic GO creation by entry).
- Spirit-healer resurrection flow: 17251 (the
  confirm opcode exists; the
  gossip→confirm→resurrect chain is unmodeled).
- Legacy creature `m_spells[4]` slot runtime:
  24936 RC Tank invalid-target setter (spell
  lists here are list-driven; no per-creature
  legacy slot array or caster for it).
- Minipet presence: Winter Veil 26469/26528/
  26532/26541 early-outs (summon proceeds —
  fork behavior when no minipet is out).
- `EffectDummy` pet-aura tail (`GetPetAura` /
  owner pet auras): no `spell_pet_auras`
  equivalent loaded (already recorded in
  `pets.rs`).
- Fork-dead cases (no-ops upstream too, correct
  by omission): 26074, 28441, 28374 Decimate
  (damage block commented out), proc 25926
  (commented out, "handled on Viscidus script").
- Unchanged from earlier tranches: fork
  non-triggered `CastSpell(.., false)` calls
  inside customs ride the triggered cast path;
  original-caster guid threading on victim
  self-casts (24320, 28560) is unmodeled;
  `spell_scripts` DB fallthrough (40 bench
  rows) and `OnEffectDummy` content scripts
  remain Tier 3.

Slice 5 is closed with tranches 1–4.

## Slice 6 inventory: Tier 1 recorded remainders

Fork-vs-Rust reachability audit of the five
remainders carried out of Tier 1
(TIER1-PROGRESS Slices 4–7). Per item: fork
semantics, Rust backing, scope decision.

### 6.1 Blink vmap LoS trim

Fork `WorldObject::MovePositionToFirstCollision`
(`Objects/Object.cpp:1999`): navmesh path
src→dest (+`halfHeight` on dest Z), path back
point, `UpdateAllowedPositionZ` (GridMap +
vmap half), then `Map::GetLosHitPosition`
(`Maps/Map.cpp:3596`) at collision-height
offsets −1.0f and −0.5f — nearest vmap
intersection trims x/y on each pass — then a
10-step Z-safety probe (`step = dist/10`,
`ATTACK_DISTANCE = 5.0f`): each iteration
first accepts the candidate if its terrain Z
is within `ATTACK_DISTANCE` of the source Z,
else steps back a tenth toward the source
snapping Z via `UpdateAllowedPositionZ`; any
step-to-step cliff (`> ATTACK_DISTANCE/2`)
stickily marks the destination unsafe. If the
probe ends safe, the *full* trimmed
destination is used (the fork overwrites even
an accepted stepped-back point with
`tempPos`); if unsafe and no stepped point
was accepted, the caster does not move. Final
`NormalizeMapCoord` + `UpdateAllowedPositionZ`. `GetCollisionHeight`
(`Object.cpp:2069`): players 1.85, creatures
model collision height (fallback 2.0).

Rust: `Heights` (`mangos-map/src/mapfiles.rs`)
loads GridMap `.map` + mmaps only;
`first_collision_dest` (`spells.rs:2774`) is
navmesh path-end only. The vmap *parser*
exists (`mangos-assets/src/vmap.rs`, BIH/
WorldModel/MapTreeFile/ModelSpawn, production
parity test) and bench data exists
(`datadir/vmaps`), but there is no runtime
vmap manager: no (map,gx,gy) tile cache, no
world-model instance transforms, no ray query
wired into map state, no `DataDir/vmaps`
config threading.

Decision: implement the GridMap-only subset
now — 10-step Z-safety walkback + normalize —
which is fork-exact wherever no vmap geometry
intervenes (open terrain). Record the runtime
vmap manager (both LoS trims + vmap half of
`UpdateAllowedPositionZ`) as a named future
slice; it is a first-of-kind asset system, not
a remainder-sized fix. Other Rust features
that would consume it (spell LoS via vmap
instead of current navmesh approximation,
creature evade LoS) gate on the same manager.

### 6.2 Pet at mount

Fork `Player::Mount` (`Player.cpp:18023`):
non-spell mount → `UnsummonPetTemporaryIfAny`;
aura mount → any `GetPet()` pet: temporary
unsummon only when the pet is permanent AND
`CONFIG_BOOL_PET_UNSUMMON_AT_MOUNT` (config
`PetUnsummonAtMount`, default false,
`World.cpp:801`) is true, else
`pet->SetEnabled(false)` (permanence gates
only the unsummon arm, not the disable).
`Player::Unmount`: pet present →
`SetEnabled(true)`, else
`ResummonPetTemporaryUnSummonedIfAny`.
`Pet::SetEnabled` (`Pet.cpp:2335`) sets
`m_enabled` and sends `SMSG_PET_MODE`
(guid, react, command, 0, enabled?0:0x8) to
the owner.

Rust: `PetState.enabled` exists and gates pet
AI/casts; `SMSG_PET_SPELLS` carries the flag
byte; `SMSG_PET_MODE` opcode (378) defined but
no standalone sender; per-player pet lookup
`pet_of(owner)` exists; `refresh_display`
(`spells.rs:5437`) performs mount/unmount
transitions with no pet hook. No
temporary-unsummon/resummon state exists and
`PetUnsummonAtMount` is not plumbed into Rust
config.

Decision: implement the default-config path —
on player mount transition, permanent pet →
`enabled=false` + `SMSG_PET_MODE`; on unmount
→ `enabled=true` + `SMSG_PET_MODE`. Record:
the config-true temporary-unsummon/resummon
branch (needs temporary-pet persistence
state + config plumbing), and the non-spell
mount caller (GM `.mount` path does not exist
in Rust).

### 6.3 Shapeshift display + form state

Fork `Aura::HandleAuraModShapeshift`
(`Spells/SpellAuras.cpp:2321`):
`SetShapeshiftForm(form)` writes
`UNIT_FIELD_BYTES_1` byte 2
(`UNIT_BYTES_1_OFFSET_SHAPESHIFT_FORM`) on
apply, 0 on remove. `GetShapeshiftDisplayInfo`
(`:2217`) maps form→display per class/race
(druid bear/cat/aquatic/travel/flight-less
vanilla set, shaman ghost wolf, priest
shadowform has none, warrior stances none,
Ghoul form model-id switch). Apply: if
display>0 and `!target->GetTransForm()`, set
display + `SetTransformScale(mod)`. Remove:
same gate restores native +
`ResetTransformScale`. Apply also casts 9033
(Slice 5: root/snare strip) for druid forms on
shift-in. Handler tail beyond display —
power-type switch, Furor rage table,
`HandleShapeshiftBoosts` (form-scaling AP/
armor from `SpellShapeshiftForm` DBC),
`InitDataForForm` (weapon/attack-time reset) —
feeds consumers Rust does not yet drive from
form state at all (power type is per-unit
static today).

Rust: `MOD_SHAPESHIFT` (36) holders exist
(aura application generic) but nothing reads
them except a customs presence check
(`customs.rs:89`) and a bot removal
(`spells.rs:926`). No form byte, no display,
no scale. `refresh_display` already models
TRANSFORM with transition state
(`transform_spell`, `transform_base_scale`),
and its removal-restore-native matches the
fork's `HandleAuraTransform` remove — so the
recorded "display fallback after transform"
quirk (transform expires while shifted → fork
shows native until re-shift) falls out of an
event-faithful port.

Decision: implement the holder-derived form
byte + display table + transform-scale +
transform interaction in `refresh_display`,
plus the druid 9033 shift-in cast (already
implemented as a spell in Slice 5). Record:
power-type switching, Furor, ShapeshiftBoosts/
`SpellShapeshiftForm` DBC attack scaling,
`InitDataForForm` — these need form-driven
stat recompute plumbing that does not exist;
they belong with the deferred stat-system
work, not this remainder.

### 6.4 Dynobj remainders

- Single-target replacement: fork
  `SpellEntry::HasSingleTargetAura` reads
  `Custom & SPELL_CUSTOM_SINGLE_TARGET_AURA`
  (0x100); no writer sets that bit anywhere
  in the fork (only
  `SetIsSingleTarget(false)` exists), so
  `m_isSingleTarget` is always false and the
  `SingleCastSpellTargetMap` swap in
  `Unit::AddAuraHolder` is dead code. Rust's
  absence is fork-exact. Closed as fork-dead.
- Channeled lifetime: fork
  `DynamicObject::Update` deletes when
  duration hits 0 AND (not channeled OR
  caster's channel object ≠ this); a running
  channel keeps its dynobj past nominal
  duration, and a broken channel frees it
  next tick. Rust `update_dynobjects` deletes
  on `alive_ms <= 0` and relies on
  `end_channel` for break cleanup. Implement
  the fork condition (needs `channeled` on
  the dynobj + caster channel-object read).
- `SPELLMOD_RADIUS`: fork
  `EffectPersistentAA` applies the player
  mod-owner radius mod to the dynobj radius.
  Rust dynobj creation uses the raw spell
  radius. Implement via existing spellmod.
- Affected-pass gates: fork uses
  valid-attack/assist-target checks (PvP,
  sanctuary, immunity via those helpers);
  Rust pass already gates hostility through
  the same target-validity helpers used by
  area spells. No vmap LoS in fork for
  persistent areas. No change; creature-
  caster dynobj remains covered by generic
  cast path (proven in live_ai at Tier 1).

### 6.5 Knockback / teleport remainders

- Knockback two-client visibility: no fork
  logic gap identified at Tier 1; the
  remainder was verification-only (observer
  sees `MSG_MOVE_FALL_LAND`/spline of the
  victim). Covered by adding a two-client
  probe leg; no code change expected.
- Wrong-ack counter: fork movement ack
  handling tolerates stale counters without
  anticheat strike in the knockback-pending
  window. Rust ack anticheat is the Tier 1
  movement slice's; audit during probe, fix
  only if the probe shows a strike.
- `TELEPORT_UNITS_FACE_CASTER` (43): fork
  `EffectTeleUnitsFaceCaster` — taxi gate,
  dest = explicit dest-location target else
  caster close-point at effect radius,
  `NearTeleportTo(..., -caster orientation,
  TELE_TO_NOT_LEAVE_COMBAT |
  TELE_TO_NOT_UNSUMMON_PET | (self ?
  TELE_TO_SPELL : 0))`. Rust: effect 43 has
  no dispatch arm. Implement (Charge-family
  destinations reach it through effect
  tables; verify bench template corpus).
- Hearthstone/same-map teleport side effects:
  audit `TELE_TO_*` flag consumers against
  Rust teleport helper during implementation;
  creature motion reset on teleport is
  handled by the generic relocate path.

Implementation order: 6.1 (Z-step), 6.2,
6.3, 6.4, 6.5. Each lands with CI + targeted
probe before the next.

## Slice 6 results

All five remainders landed; verification:
`./ci.sh` `CI_EXIT=0` (`/tmp/ci_t2s6.log`),
`live_spells` → `SHAPESHIFT form byte +
display + 9033 ok`, `FACECASTER close point
ok`, `CHANNEL-DYNOBJ break cleanup ok`,
`LIVE-SPELLS-PASS`; wire probe
`/tmp/slice6_probe.py` → `SLICE6-PROBE-PASS`
(SHAPESHIFT-WIRE, PETMOUNT, KNOCKBACK-VIS);
`ORACLE-FIXTURE-RESTORED` + oracle tests;
`TWO-CLIENT-SMOKE-PASS` on the rebuilt bench
(PORT_OK 18099).

- 6.1 Blink: `first_collision_dest` is now a
  literal transcription of the GridMap subset
  of `MovePositionToFirstCollision` (navmesh
  back point, `UpdateAllowedPositionZ` via
  `ground_z`, 10-step Z-safety probe with the
  sticky-unsafe + `tempPos` overwrite quirks,
  `NormalizeMapCoord`). Flat-terrain behavior
  unchanged (existing Blink legs pass); no
  cliff-specific wire leg was constructed —
  stepping correctness rests on the literal
  transcription. The runtime vmap manager
  (both `GetLosHitPosition` trims, vmap half
  of `UpdateAllowedPositionZ`, plus the
  `ObjectPosSelector`/`IsWithinLOS` walks in
  `GetNearPoint`) remains the named unbuilt
  system gating further LoS exactness; the
  water-surface arm of
  `UpdateAllowedPositionZ` rides the same
  `ground_z` approximation as all Rust motion.
- 6.2 Pet-at-mount: `set_pet_enabled` +
  `SMSG_PET_MODE` sender; mount/unmount
  transitions in `refresh_display` disable /
  re-enable the player's pet. Probe-observed:
  imp → Felsteed → `PET_MODE flag 0x8`,
  dismount → `0x0`. Recorded remainder:
  config-true temporary unsummon/resummon
  (`PetUnsummonAtMount` unplumbed, no
  temporary-pet state), non-spell `.mount`
  caller, and adjacent `Player::Mount` gates
  Rust lacks (already-mounted result,
  disallowed-mount-form, looting).
- 6.3 Shapeshift: holder-derived form state
  in `refresh_display` (`refresh_shapeshift`):
  `UNIT_FIELD_BYTES_1` byte 2, form
  exclusivity, fork display table incl.
  Ghoul/CREATUREBEAR model switch (via new
  `ModelData.model_id` threaded from
  `CreatureDisplayInfo.dbc`), transform-scale
  sharing with TRANSFORM, transform-expiry
  native fallback quirk preserved, 9033 cast
  on druid-form transitions,
  `SHAPESHIFTING_CANCELS` strip gated on real
  `SpellShapeshiftForm.dbc` `flags1` (new DBC
  load into `WorldStores.shapeshift_forms`;
  bench DBC stance bits: 17/18/19/28/30/31),
  unknown-form early return, 23397→2458
  removal cast. Verified in-process (bear→cat
  byte/display/exclusivity, root strip, cancel
  restore) and on the wire (byte 5 / display
  2281 / restore 49 for human guid 1).
  Recorded remainder: power-type switching
  (cat energy / bear rage), Furor procs
  (17099/17057), tactical-mastery rage caps,
  `HandleShapeshiftBoosts` form AP/armor
  scaling, `InitDataForForm` attack-time
  resets — all need form-driven stat/power
  plumbing that does not exist yet. Note the
  in-process test world has no create-info
  display (native seeded in-test as
  production materialize does).
- 6.4 Dynobj: single-target replacement
  closed as fork-dead (flag never set).
  Channel lifetime now fork-shaped: dynobj
  stores `channeled`; `update_dynobjects`
  applies the fork expiry condition (alive==0
  unless channel object) with the affected
  pass reordered before deletion (fork
  `Update` order); `end_channel` calls
  `remove_dynobjects_by_spell` (fork
  `RemoveDynObject` on cancel + finish), so
  early channel breaks despawn immediately —
  probe-observed via `SMSG_GAMEOBJECT_
  DESPAWN_ANIM` right after `cancel_cast`.
  `SPELLMOD_RADIUS` (new `spellmod::RADIUS`)
  applies at `spawn_dynobject` with charge
  tracking through `cast.applied_mods`.
  Farsight focus dynobjs (the one fork case
  that sets the channel object to a dynobj)
  remain unmodeled with the camera system.
- 6.5 Knockback/telephony:
  `TELEPORT_UNITS_FACE_CASTER` (43)
  implemented (dest-location arm +
  `GetClosePoint` primary point +
  `-orientation` quirk) and verified
  in-process against spell 514 (radius index
  13 = 10.0y from `SpellRadius.dbc`): victim
  lands at caster bounds + victim bounds +
  10.0 in front, facing `-caster.o`.
  Knockback two-client visibility verified
  on the wire: observer receives
  `MSG_MOVE_KNOCK_BACK` for the victim after
  a well-formed ack. Incidental probe finding
  (server is fork-correct): Tier 1-era
  scripted probes sent movement acks with a
  *packed* GUID; the fork reads a raw u64
  (`operator>>` on `ObjectGuid`), so those
  acks were silently dropped and Tier 1's
  ack-relocation evidence actually rode the
  fall-land relocation. `HandleMoveKnock
  BackAck` parity in Rust: pending match on
  counter + jump fields, wrong-ack log +
  anticheat strike on mismatch; the fork's
  additional ack-side flag/position anticheat
  tests are not run in Rust (precedent noted
  in `world_actor.rs`) — recorded nuance.
  `TELE_TO_*` flag audit: `near_teleport_unit`
  neither leaves combat nor unsummons pets,
  matching `NOT_LEAVE_COMBAT |
  NOT_UNSUMMON_PET`; `TELE_TO_SPELL` fall
  handling rides the existing near-teleport
  path. Taxi-flight gates on teleport effects
  remain unmodeled (no taxi system state
  behind them).

Slice 6 is closed. Remaining Tier 2 recorded
items: the runtime vmap manager, form-driven
power/stat recompute, pet temporary-unsummon
config arm, Farsight/camera dynobjs, taxi
gates, ack-side anticheat position tests,
plus the Slice 5 tranche-4 corpus and the
deferred-damage divergence (fork accumulates
spell damage across the effect loop; Rust
deals immediately).
