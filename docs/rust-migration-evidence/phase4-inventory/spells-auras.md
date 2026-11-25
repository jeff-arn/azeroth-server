# Phase 4 evidence inventory — Spells / auras (§14 #7/#50)

Oracle: C++ at checkout HEAD `1bec69f` (`vmangos/src/`). Production binary
`2bae5f9` divergences flagged only where inspected (none found in bodies
read below; `git log 2bae5f9..HEAD` on Spells/ not diffed line-by-line —
see Unresolved). All anchors are `file:line` at HEAD. Rust state:
`vmangos-rs/crates/` (15 crates: mangos-assets/auth/bots/build-info/
config/db/dbc/detour/guid/log/movement/protocol/threat/world-data +
mangosd) contains **no spell/aura crate**; only opcode/DBC references
(`mangos-protocol/src/world.rs`, `mangos-dbc/src/formats.rs`).

## (a) Verified structure / flow

### Entry → prepare → cast → go

```
WorldSession::HandleCastSpellOpcode (Handlers/SpellHandler.cpp:286)
  -> SpellCaster::CastSpell(...) (Objects/SpellCaster.cpp:2108 prepare call)
  -> Spell::prepare(targets, triggeredByAura, chance)  Spell.cpp:3452
  -> Spell::prepare(Aura*, chance)                     Spell.cpp:3458
  -> [SpellEvent on caster, 1 ms] Spell::update(diff)  Spell.cpp:4101
  -> Spell::cast(skipCheck)                            Spell.cpp:3699
  -> handle_immediate() Spell.cpp:3917  |  handle_delayed() Spell.cpp:3985
  -> DoAllEffectOnTarget(TargetInfo*)                  Spell.cpp:1284
  -> DoSpellHitOnUnit(unit, effectMask)                Spell.cpp:1699
  -> HandleEffects(..., effIdx, dmgMult)               Spell.cpp:5407
  -> (*this.*SpellEffects[eff])(i)                     Spell.cpp:5429
```

- `prepare` (Spell.cpp:3458-3618): sets `SPELL_STATE_PREPARING`;
  `m_delayed = speed>0 || player delayable-effects` (3461); registers
  `SpellEvent` (3474); rejects concurrent non-melee cast
  (`SPELL_FAILED_SPELL_IN_PROGRESS`, 3478) and disabled spells (3487);
  `m_powerCost = CalculatePowerCost(...)` (3495); strict
  `CheckCast(true)` (3503); optional script `chance` roll (3519);
  `prepareDataForTriggerSystem()` (3529, def 826);
  `m_casttime = SpellEntry::GetCastTime` (3532);
  `m_duration = CalculateDuration` (3538); strips interrupt-flag auras
  (3555); `m_spellScript->OnSuccessfulStart` (3585); non-triggered:
  `SendSpellStart()` (3590) + `m_caster->AddGCD(*m_spellInfo)` (3593 —
  GCD starts at cast **start**, not completion); triggered+instant casts
  immediately via `cast(true)` (3603).
- `cast` (Spell.cpp:3699-3915): cast-depth counter guard (3703);
  `UpdatePointers()` (3722); recomputes power cost + `CheckPower` +
  `CheckCast(false)` (3749-3777); `FillTargetMap()` (3784);
  `SendSpellCooldown()` (3798, def 4088); `TakePower/TakeReagents/
  TakeAmmo` (3819-3821); `SendCastResult` (3823);
  `InitializeDamageMultipliers()` (3824); `m_spellScript->OnCast`
  (3827); caster cast-end procs `ProcDamageAndSpell(...PROC_EX_CAST_END)`
  (3847); delayed branch: `SendSpellGo()` (3857) then
  `HandleDelayedSpellLaunch` per target (3864), state DELAYED (3868);
  immediate branch: `handle_immediate()` (3895).
- `handle_immediate` (3917-3983): `SendSpellGo()` before effects except
  INSTAKILL/SPAWN (3920-3923); `_handle_immediate_phase`; channel start
  (3928); loops `m_UniqueTargetInfo` → `DoAllEffectOnTarget` (3949),
  then GO targets (3965), corpse (3969); `_handle_finish_phase`;
  `TakeCastItem`; `finish(true)` (3982, def 4409).
- `DoAllEffectOnTarget` (1284): per-target `effectMask` (≤3 effects,
  `MAX_EFFECT_INDEX`); strips PERSISTENT_AREA_AURA (1304); hit →
  `DoSpellHitOnUnit` (1378), reflect (1384), miss/resist/dodge/parry/
  block/immune branches (1388); damage applied **after** procs (1579
  comment: victim killing-blow auras must see caster alive).
- `DoSpellHitOnUnit` (1699): mechanic-resist per effect (1715),
  delayed immunity recheck (1724), combat/threat entry (1766-1787),
  aura holder creation `CreateSpellAuraHolder` (1846), per-effect
  `HandleEffects` (1860), `unit->AddSpellAuraHolder` (1910).
- `update` (4101): movement-interrupt check vs `m_castPosition`
  >0.5 units (4128); drives cast timer → `cast()`, delayed impacts,
  channel ticks. `cancel()` (3620) refunds GCD in PREPARING (3632).
- `CheckCast(strict)` def Spell.cpp:5472; `CheckCasterAuras` 6646.
  Not read in full (Unresolved).

### Target selection (SpellCastTargets / SpellTargets)

- Client-supplied targets: `class SpellCastTargets` Spell.h:83
  (unit/GO/item/corpse GUIDs + destination + target mask).
- Server expansion: `FillTargetMap()` dispatches per effect index on
  `EffectImplicitTargetA/B[i]` (Spell.cpp:442-593, ~30 `SetTargetMap`
  call sites) into `m_UniqueTargetInfo` (deduped, carries
  `effectMask`, `missCondition`, `isCrit`, `timeDelay`, `damage`).
- `SetTargetMap(effIndex, targetMode, list)` Spell.cpp:2138: radius =
  `sSpellRadiusStore[EffectRadiusIndex]` else max range from
  `sSpellRangeStore` (2141-2144); spell-mods radius/jump-targets
  (2152-2153); leeway bonus for caster-centred AoE (2163);
  `m_spellScript->OnSetTargetMap` may mutate radius/max-targets
  (2172-2173); giant `switch(targetMode)` (2175+, TARGET_* enum,
  ~1100 lines to ~3300) incl. chain targets (2183), cone, area via
  `FillAreaTargets` (def 8209) + `SpellNotifierCreatureAndPlayer`
  grid search (7994) filtered by `enum SpellTargets` Spell.h:206
  (FRIENDLY/NOT_FRIENDLY/AOE_DAMAGE/...).

### SpellEffects dispatch

- Table `pEffect SpellEffects[TOTAL_SPELL_EFFECTS]` SpellEffects.cpp:55,
  `TOTAL_SPELL_EFFECTS = 134` SpellDefines.h:347. Initializers present
  for indices **0-131 only (132 entries)**; 132 `APPLY_AREA_AURA_RAID`
  and 133 `APPLY_AREA_AURA_OWNER` (SpellDefines.h:345-346) have **no
  initializer → null fn-ptr** (verified: table ends :187-188). Port
  must decide: replicate inert/crash-risk gap or route to
  `EffectApplyAreaAura` (used by 35/119/128/129).
- 96 distinct `void Spell::Effect*` method definitions in
  SpellEffects.cpp (5805 lines); several table slots alias one method
  (e.g. `EffectWeaponDmg` ×4: 17/31/58/121; `EffectSummonTotem` ×5;
  `EffectApplyAreaAura` ×4); stubs `EffectEmpty/EffectNULL/
  EffectUnused` (190-200) cover ~20 slots.
- Key handlers: `EffectSchoolDMG` SpellEffects.cpp:295 — **does no
  damage itself**, only `m_damage += damage` (300); real damage is
  dealt centrally in `DoAllEffectOnTarget` (Spell.cpp:1519-1579).
  `EffectApplyAura` SpellEffects.cpp:1602: requires pre-created
  `m_spellAuraHolder`, rejects dead targets (1611) and
  `HasMorePowerfulSpellActive` (1614), `CreateAura(...)` +
  `holder->AddAura(aur, effIdx)` (1630-1631). `EffectDummy` (304) is a
  multi-thousand-line per-family/per-spell-id switch (hardcoded spell
  behaviours live here + in scripts, not in data).

### Auras: lifecycle, periodic ticks, modifiers

- Types: `Aura` (one effect), `SpellAuraHolder` (one spell cast on one
  target, ≤3 `Aura* m_auras[MAX_EFFECT_INDEX]` SpellAuras.h:99),
  subclasses `AreaAura`, `PersistentAreaAura`, `SingleEnemyTargetAura`
  (SpellAuras.cpp:406/458/469); factories `CreateAura` (479),
  `CreateSpellAuraHolder` (491).
- Handler table `pAuraHandler AuraHandler[TOTAL_AURAS]` SpellAuras.cpp:63,
  `TOTAL_AURAS = 193` SpellAuraDefines.h:332, all 193 slots initialized;
  122 distinct `void Aura::Handle*` definitions (rest alias
  `HandleNoImmediateEffect` — meaning is read at damage/stat compute
  time, e.g. :78, :123, :151 comments).
- Apply path: `Aura::ApplyModifier(apply, Real)` SpellAuras.cpp:884 —
  exclusive-aura gate (894), `AuraScript::OnBeforeApply/OnAfterApply`
  (902/906), dispatch `(this->*AuraHandler[aura])(apply, Real)` (905).
  Handlers mutate Unit state/update fields directly, e.g.
  `HandleAurasVisible` → `UNIT_FIELD_FLAGS` (4450),
  `HandleAuraModStat` (4507), `HandleModDamageDone` (5083);
  `Real=false` = re-apply without side effects (shapeshift/stat
  recalc). Holder-level: `SpellAuraHolder::ApplyAuraModifiers` (6587),
  `_AddSpellAuraHolder` (6594) / `_RemoveSpellAuraHolder` (6673).
- Visible slots: `MAX_AURAS 48`, `MAX_POSITIVE_AURAS 32`
  (SpellAuraDefines.h:25-26); slot alloc scans `UNIT_FIELD_AURA+i`
  (6613/6624); writes `UNIT_FIELD_AURA` (7339), `UNIT_FIELD_AURAFLAGS`
  (7341), `UNIT_FIELD_AURALEVELS` (7370) — these update-field writes
  are the wire-visible aura state.
- Periodic: `Aura::CalculatePeriodic` (7883) sets `m_isPeriodic` for
  PERIODIC_DAMAGE/HEAL/ENERGIZE/LEECH/... (7890-7900),
  `m_periodicTimer = periodictime` (= `EffectAmplitude`, 7939, with
  hardcoded totem/trap exceptions 7920-7937). `Aura::Update(diff)`
  (528): `m_periodicTimer -= diff`; on ≤0 re-arms `+= periodictime`
  (541, no drift clamp beyond -period) and calls `PeriodicTick()`
  (544). `SpellAuraHolder::Update(diff)` (7095): PvP heartbeat break
  (7099-7114, logistic 12 s avg/15 s max), PvE heartbeat 5 s
  (7117-7138), duration expiry → `RemoveSpellAuraHolder` (7112).
- `PeriodicTick` SpellAuras.cpp:5667 (~590 lines, switch on aura type):
  PERIODIC_DAMAGE branch 5675-5784 (see Oracles). `PeriodicDummyTick`
  6260; `TriggerSpell` 1076 (periodic-trigger casts).
- Stacking/refresh/exclusive: `Aura::Refresh` (308),
  `SpellAuraHolder::Refresh/CanBeRefreshedBy` (362/379),
  `ModStackAmount/SetStackAmount` (6789/6812), `ComputeExclusive/
  ExclusiveAuraUnapply` (8166/8240), spell-group stack rules via
  SpellMgr (below). Removal modes (`AURA_REMOVE_BY_*`) thread through
  `Unit::RemoveSpellAuraHolder` (not read; Unresolved).

### Procs, cooldown/GCD, scripts

- Proc entry: `SpellCaster::ProcDamageAndSpell(ProcSystemArguments&&)`
  Objects/SpellCaster.cpp:255 → `_delayed` (280) / `_real` (292) →
  `Unit::ProcDamageAndSpellFor` Objects/Unit.cpp:8979 → per-aura-type
  proc table in `UnitAuraProcHandler.cpp` (2000 lines; e.g.
  `HandleProcTriggerSpellAuraProc` :1129). Call sites in spell flow:
  cast-end (Spell.cpp:3847), direct damage (1575), heal (1507),
  reflect (1130), DoT tick (SpellAuras.cpp:5775 with
  `PROC_FLAG_DEAL/TAKE_HARMFUL_PERIODIC` 5760-5761). Eligibility data:
  DBC proc flags on SpellEntry overridden by `spell_proc_event` /
  `spell_proc_item_enchant` ppm (SpellMgr.cpp:303/362).
- GCD: per-`StartRecoveryCategory` map `m_GCDCatMap` (SpellCaster.h:437);
  `AddGCD` Objects/SpellCaster.cpp:2111 (`StartRecoveryTime` ms from
  spell_template; 0 = no GCD), `HasGCD` 2120, expiry in
  `UpdateCooldowns` 2138, `ResetGCD` 2273. Server-enforced only;
  client shows its own GCD (Spell.cpp:3591 comment).
- Cooldown: `Spell::SendSpellCooldown` Spell.cpp:4088 →
  `SpellCaster::AddCooldown` SpellCaster.cpp:2131 (`RecoveryTime` +
  `Category`/`CategoryRecoveryTime` into `m_cooldownMap`); PASSIVE
  spells skip (4091); `SPELL_ATTR_COOLDOWN_ON_EVENT` = permanent
  until event (4098). Wire: `Player::SendSpellCooldown(spellId,
  cooldown, target)` Objects/Player.cpp:20743 (SMSG_SPELL_COOLDOWN;
  body not read).
- Spell mods (talents/items): `SpellModifier/SpellModMgr`
  (SpellModMgr.cpp 281 lines) applied via `Player::ApplySpellMod`
  at cost/cast-time/damage/radius/duration points (e.g.
  SpellCaster.cpp:924 SPELLMOD_ALL_EFFECTS; Spell.cpp:2152).
- Scripts: `struct SpellScript` ScriptMgr.h:138 (14 hooks:
  OnInit/OnSuccessfulStart/OnSuccessfulFinish/OnCheckCast/
  OnEffectExecute/OnSetTargetMap/OnCheckTarget×2/OnCast/OnHit/
  OnAfterHit/OnSummon×2/OnSuccessfulDispel) and `struct AuraScript`
  ScriptMgr.h:172 (13 hooks incl. OnCheckProc/OnProc/OnAbsorb/
  OnPeriodicCalculateAmount/OnPeriodicTickEnd). Bound per-spell via
  DB-loaded script bindings (`SpellMgr::LoadSpellScripts`-family at
  SpellMgr.cpp:2676 reads `spell_script`/`spell_aura_script`-style
  tables by family/icon/visual/category/effect match — exact table
  names at call site not verified, Unresolved) +
  `Script::GetSpellScript/GetAuraScript` fn-ptrs (ScriptMgr.h:247-248).
  `m_spellScript` on Spell (Spell.h:654); holder caches aura script at
  construction (SpellAuras.cpp:6501).

## (b) Sizes / counts (HEAD, `wc -l`)

| File | Lines |
|---|---|
| Spells/Spell.cpp | 8413 |
| Spells/SpellAuras.cpp | 8261 |
| Spells/SpellEffects.cpp | 5805 |
| Spells/SpellMgr.cpp | 3888 |
| Spells/SpellEntry.cpp / .h | 1112 / 1108 |
| Spells/Spell.h / SpellAuras.h / SpellMgr.h | 808 / 655 / 711 |
| Spells/SpellDefines.h / SpellAuraDefines.h / SpellClassMask.h | 1268 / 346 / 352 |
| Spells/SpellModMgr.cpp, AuraTracker.cpp, SpellModifier.cpp | 281 / 205 / 33 |
| **Spells/ total** | **33357** |
| Objects/SpellCaster.cpp (damage/cooldown/proc glue) | not counted here |
| Objects/Unit.cpp / UnitAuraProcHandler.cpp | 11073 / 2000 |

Counts: 134 spell-effect types (132 dispatched), 96 effect methods;
193 aura types, 122 aura handlers; 3 effects/spell max
(`MAX_EFFECT_INDEX`); 48 visible aura slots (32 positive + 16
negative); 20 `SpellMgr::Load*` functions (SpellMgr.cpp:43-3625);
~30 `SetTargetMap` call sites in FillTargetMap.

## (c) DB / config inputs

Primary spell data is **world-DB, not Spell.dbc** in this fork:
`SpellMgr::LoadSpells` SpellMgr.cpp:3530 reads `spell_template`
(build-gated `max(build) <= SUPPORTED_CLIENT_BUILD`, 3550) into
`mSpellEntryMap` via `LoadSpell(fields)` (3625); `locales_spell`
names (3578, 1.12 only). A Rust loader must read `spell_template`
columns in `LoadSpell` order, not the DBC layout.

World-DB tables loaded by SpellMgr (function → table):
`LoadSpellTargetPositions` 43 → `spell_target_position`;
`LoadSpellProcEvents` 303 → `spell_proc_event`;
`LoadSpellProcItemEnchant` 362 → `spell_proc_item_enchant`;
`LoadSpellGroups` 488 → `spell_group`;
`LoadSpellGroupStackRules` 569 → `spell_group_stack_rules`;
`LoadSpellElixirs` 700 → `spell_elixir`;
`LoadSpellThreats` 801 → `spell_threat`;
`LoadSpellChains` 1411 → `spell_chain` (+ DBC talents/abilities);
`LoadSpellEnchantCharges` 1770 → `spell_enchant_charges`;
`LoadSpellLearnSpells` 1818 → `spell_learn_spell`;
`LoadSpellScriptTarget` 1926 → `spell_script_target` (+ `conditions`);
`LoadSpellPetAuras` 2089 → `spell_pet_auras`;
`LoadSpellCones` 2237 → `spell_cone`;
`LoadSpellAreas` 2293 → `spell_area`;
`LoadExistingSpellIds` 2969 → `spell_template` (distinct entry);
script bindings ~2676 (table name unverified). Most are
`build_min/build_max`-gated on `SUPPORTED_CLIENT_BUILD`.
DBC (Database/DBCStores.cpp:311-316+): SpellCategory, SpellCastTimes,
SpellDuration, SpellFocusObject, SpellItemEnchantment, SpellRadius,
(+ SpellRange used Spell.cpp:364). Character-DB: known spells,
cooldown persistence — not inspected (Unresolved). Config: none found
in bodies read except `CONFIG_UINT32_MAX_PLAYER_LEVEL` fallback
(SpellAuras.cpp:6640).

## (d) Wire-visible semantics a Rust port must replicate

- Packet order: SMSG_SPELL_START at prepare (Spell.cpp:4593, packet
  4609) → SMSG_SPELL_GO (4630, packet 4643) **before** effects for
  immediate spells except INSTAKILL/SPAWN where GO follows effects
  (3920/3977); delayed: GO at cast, effects at impact
  (`handle_delayed` 3985). SMSG_SPELLLOGEXECUTE (4788),
  SMSG_SPELL_FAILED_OTHER (4910), SMSG_SPELLLOGMISS (4932),
  SMSG_SPELL_UPDATE_CHAIN_TARGETS (5098), SMSG_SPELL_DELAYED pushback
  (7566), cast-fail via `SendCastResult` (4522/4536).
- Damage log: `SendSpellNonMeleeDamageLog(&damageInfo)` **before**
  procs and before `DealSpellDamage` (Spell.cpp:1561 vs 1575/1579);
  DoT log via `SendPeriodicAuraLog` + `AuraTracker::SendPeriodicTick`
  (SpellAuras.cpp:5768-5770) before `DealDamage` (5779).
- Aura update fields: slot allocation order (positive 0-31, negative
  32-47), `UNIT_FIELD_AURA/AURAFLAGS/AURALEVELS` writes (7339-7377),
  stack count in aura flags byte, duration via aura application
  packets (holder `_Add/_Remove` 6594/6673 — packet bodies not read).
- GCD/cooldown: GCD categories independent per
  `StartRecoveryCategory` (not one global timer); cooldown starts at
  `cast()` (after cast time) while GCD starts at `prepare()` —
  cancelling during PREPARING refunds GCD but a completed cast keeps
  both (3632 vs 3593/4098). `SMSG_SPELL_COOLDOWN` values come from
  RecoveryTime/CategoryRecoveryTime, spell-mods may alter.
- Cast-bar/interrupt: movement >0.5u cancels if spell has movement
  interrupt flags (4128); pushback sends SMSG_SPELL_DELAYED (7566).
- Miss semantics: `SpellMissInfo` per target computed in FillTargetMap
  (hit/crit rolled **before** GO for delayed spells —
  `HandleDelayedSpellLaunch` 3864 pre-fills `target->damage/HitInfo`,
  consumed 1526-1530); binary/resist behaviour in
  `CalculateDamageAbsorbAndResist` (not read).
- Threat/combat side effects on hit/miss (1766-1787, 1398-1403) are
  observable via creature aggro timing — replicate ordering vs damage.

## Differential-fixture oracles (exact damage-computation points)

Pure-ish functions suitable for golden input→output fixtures (feed
SpellEntry + caster/target stats, capture result):

1. Base effect value (dice roll excluded by DieSides∈{0,1}):
   `SpellCaster::CalculateSpellEffectValue` Objects/SpellCaster.cpp:876
   — level clamp (885-889), `value = basePoints - baseDice + level*
   RealPointsPerLevel + rand` (894-914), combo points (917), spell-mod
   (924), creature-level scaling (940-947). Called via
   `Spell::CalculateDamage` Spell.h:365 ← `HandleEffects`
   Spell.cpp:5419 (`damage = CalculateDamage(i) * DamageMultiplier`).
2. Direct spell damage: `SpellCaster::CalculateSpellDamage`
   SpellCaster.cpp:951 — `SpellDamageBonusDone` (:1286) →
   victim `SpellDamageBonusTaken` (Unit.cpp:5344) → crit
   `SpellCriticalDamageBonus` (995) → armor for physical (1005) →
   `ditheru` (1010). Bonus coefficient: `SpellEntry::
   GetSpellBonusCoefficient`-family SpellEntry.cpp:633
   (`coeff = GetCastTimeForBonus/3500 * DotFactor`, DotFactor =
   duration/15000/ticks) + `CalculateCustomCoefficient` (638+,
   per-family hardcoded coefficients).
3. DoT tick: `Aura::PeriodicTick` PERIODIC_DAMAGE SpellAuras.cpp:5675 —
   `fdamage = m_modifier.m_amount` (5719, >1.9.4 builds; amount was
   bonus-baked at apply time in `HandlePeriodicDamage` :4287 via
   `SpellDamageBonusDone(...,DOT,stack)` :4275) →
   `AuraScript::OnPeriodicCalculateAmount` (5728) → family specials
   (Consecration recalc 5732, Curse of Agony ramp 5735, Starshards
   5738) → `SpellDamageBonusTaken(...,DOT)` (5742) → `ditheru` (5750)
   → `CalculateDamageAbsorbAndResist` (5753) → absorb/resist split
   (5763-5766). Snapshot semantics: caster +damage is frozen at
   application, victim taken-mods re-evaluated per tick — fixture must
   cover both moments.
4. Power/cast-time/duration: `CalculatePowerCost` (called
   Spell.cpp:3495/3749), `SpellEntry::GetCastTime` SpellEntry.cpp:454,
   `CalculateDuration` SpellEntry.cpp:720 — deterministic given
   spell-mods; good first fixtures before damage exists.

## (e) Smallest viable first slice

One direct-damage spell + one DoT, single explicit unit target,
no procs/scripts/spell-mods:

1. Data: load 2 rows of `spell_template` (+ SpellCastTimes/
   SpellDuration/SpellRadius/SpellRange DBC already in mangos-dbc)
   into a Rust `SpellEntry` (fields used above only: Effect[3],
   EffectBasePoints/DieSides/BaseDice/RealPointsPerLevel,
   EffectApplyAuraName/Amplitude/ImplicitTargetA, ManaCost/power type,
   RecoveryTime/StartRecovery*, School/DmgClass, Attributes).
   Candidates by shape (ids to confirm against DB): a SCHOOL_DAMAGE
   nuke (e.g. Frostbolt rank 1 — also exercises `speed>0` delayed
   path; choose an instant nuke if delay is deferred) and a
   PERIODIC_DAMAGE DoT (e.g. Corruption rank 1).
2. Pipeline: `prepare` (CheckCast subset: known spell, power, GCD,
   range/LoS hooks stubbed) → cast timer in map update → `cast` →
   FillTargetMap only for `TARGET_UNIT_ENEMY`-style explicit target →
   hit roll stub (always hit, no crit) → `HandleEffects` with 2 effect
   handlers: SCHOOL_DAMAGE (accumulate) and APPLY_AURA (create holder).
3. Damage: port oracles 1+2 minus bonus auras (flat `SpellBaseDamage
   BonusDone=0`) → `SendSpellNonMeleeDamageLog` equivalent +
   health subtract + threat hook into existing mangos-threat.
4. Aura: holder with duration/amplitude, `UNIT_FIELD_AURA*` slot
   write (negative range), tick loop replicating Aura::Update
   re-arm rule (541), tick damage = oracle 3 minus taken-mods,
   expiry removal. One aura handler: PERIODIC_DAMAGE (no-op apply,
   `m_isPeriodic` only, cf. HandlePeriodicDamage 4287 minus bonus).
5. Wire: CMSG_CAST_SPELL in, SMSG_SPELL_START/GO/FAILED + cooldown
   packet out, reusing mangos-protocol opcodes.

Entry dependencies (must exist first): Unit/Object update-field store
+ visibility broadcast (for aura slots & health), map tick driving
cast timers and aura updates, power/health pools on Unit, GUID target
resolution in map, `spell_template` DB loader in mangos-db/world-data,
RNG compatible `irand/ditheru` semantics (values, not bit-identical
stream), threat/combat-state hooks (`SetInCombatWithVictim`).
Deferred safely: procs, scripts, spell-mods, resist/absorb, crit,
AoE/chain targeting, channels, diminishing returns/heartbeat,
exclusive/stack rules (single DoT refresh = replace duration).

## (f) Unresolved

- `CheckCast` (Spell.cpp:5472, ~1100 lines), `FillTargetMap` head,
  `HandleDelayedSpellLaunch`, `CalculateDamageAbsorbAndResist`,
  `SpellDamageBonusDone/Taken` full bodies, `Unit::AddSpellAuraHolder/
  RemoveSpellAuraHolder` stacking decision, aura apply/remove packet
  bodies: located, not read in full.
- Script-binding table names at SpellMgr.cpp:2676 and script counts
  (how many spells have SpellScript/AuraScript) not verified.
- Character-DB spell/cooldown persistence and `playercreateinfo_spell`
  -style initial spells not inspected.
- Effects 132/133 null dispatch: whether any spell_template row uses
  them (needs DB query) unknown.
- Production `2bae5f9` vs HEAD `1bec69f` diff for Spells/ not
  inspected; all claims are HEAD-only.
- `AuraTracker` (205 lines, bot/observability side-channel at
  PeriodicTick) not read; likely non-wire, verify before omitting.
