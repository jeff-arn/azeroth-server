//! Aura-script arms + dispatch (`OnProc`, `OnBeforeApply`, `OnAfterApply`,
//! `OnPeriodicDummy`, `OnPeriodicTrigger`, `OnPeriodicTickEnd`).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `AuraScript::OnProc` dispatch
    /// (`UnitAuraProcHandler.cpp` script protocol):
    /// `Some(true)` = `PROC_OK`, `Some(false)` =
    /// `PROC_FAILED`, `None` = default handler.
    pub(crate) fn aura_script_on_proc(&mut self, ctx: &ProcHandlerCtx) -> Option<bool> {
        match ctx.aura_spell.script_name.as_str() {
            "spell_mage_ignite" => {
                let victim = ctx.victim?;
                #[allow(clippy::cast_possible_wrap)]
                let original = ctx.original_amount as i32;
                Some(self.mage_ignite_on_proc(
                    ctx.owner,
                    victim,
                    original,
                    ctx.aura_spell.id,
                    ctx.cooldown,
                ))
            }
            "spell_mage_combustion_proc" => Some(self.mage_combustion_proc_on_proc(
                ctx.owner,
                ctx.victim,
                ctx.aura_spell.id,
                ctx.proc_ex & mangos_spells::defines::proc_ex::CRITICAL_HIT != 0,
                ctx.holder.proc_charges <= 1,
            )),
            _ => None,
        }
    }

    /// `AuraScript::OnPeriodicTickEnd` dispatch
    /// (fork `Aura::PeriodicTickEnd`,
    /// `SpellAuras.cpp` — after each periodic
    /// tick's effect).
    pub(crate) fn aura_script_on_periodic_tick_end(&mut self, tick: &PeriodicTick) {
        let Some(spell) = self.stores.spells.get(tick.spell_id) else {
            return;
        };
        if spell.script_name == "spell_nefarian_class_call_mage" {
            let holder = self
                .aura_holders(tick.target)
                .into_iter()
                .find(|h| h.spell_id == tick.spell_id && h.caster == tick.caster);
            let Some(holder) = holder else { return };
            self.nefarian_class_call_mage_on_periodic_tick_end(tick.target, &holder);
        }
    }

    /// `AuraScript::OnBeforeApply` (fork
    /// `SpellAuras.cpp:903`, called per effect aura
    /// before the apply/unapply handler). The Rust
    /// seam runs at holder level in `add_aura_holder`
    /// / `remove_aura_at`, iterating present effects;
    /// holder is mutable so arms can adjust aura
    /// state (e.g. `SetPeriodicTimer`).
    pub(crate) fn aura_script_on_before_apply(
        &mut self,
        victim: ObjectGuid,
        holder: &mut AuraHolder,
        apply: bool,
        mode: AuraRemoveMode,
    ) {
        let Some(name) = self
            .stores
            .spells
            .get(holder.spell_id)
            .map(|s| s.script_name.clone())
        else {
            return;
        };
        if name.is_empty() {
            return;
        }
        for eff in 0..3 {
            if holder.effects[eff].is_none() {
                continue;
            }
            if name == "spell_haunting_phantoms"
                && apply
                && eff == 0
                && let Some(e) = holder.effects[0].as_mut()
            {
                // `HauntingPhantomsScript::OnBeforeApply` (`stratholme.cpp:638-642`):
                // on apply of effect 0, first periodic dummy tick at 5s
                // (`Aura::SetPeriodicTimer`, which also makes the aura periodic).
                e.periodic_timer = 5_000;
                e.period_ms = 5_000;
            }
            if name == "spell_golemaggs_trust"
                && apply
                && eff == 0
                && let Some(e) = holder.effects[0].as_mut()
            {
                // `GolemaggsTrustScript::OnBeforeApply` (`boss_golemagg.cpp:354-358`):
                // on apply of effect 0, first periodic dummy tick at 1s.
                e.periodic_timer = 1_000;
                e.period_ms = 1_000;
            }
            // 6946 Curse of the Bleakheart
            // (felwood.cpp CurseOfTheBleakheartScript):
            // first periodic dummy tick at 5s.
            // Gargoyle Stoneform: stand state +
            // unselectable flag (idempotent).
            if name == "spell_gargoyle_stoneform" {
                self.gargoyle_stoneform_on_before_apply(victim, apply);
            }
            // Grobbulus Mutating Injection: on
            // removal the bearer casts Poison Cloud.
            if name == "spell_grobbulus_mutating_injection" && !apply && eff == 0 {
                self.grobbulus_mutating_injection_on_before_apply(victim, holder, mode);
            }
            // Combustion buff clicked off: drop the
            // invisible counter aura (starts the
            // cooldown).
            if name == "spell_mage_combustion_buff"
                && !apply
                && eff == 0
                && mode == AuraRemoveMode::Cancel
            {
                self.mage_combustion_buff_on_after_apply(victim);
            }
            // Shadowmeld: on removal (eff 0, player)
            // drop the Elusiveness visual.
            if name == "spell_shadowmeld" && !apply && eff == 0 && victim.is_player() {
                self.shadowmeld_on_before_apply(victim);
            }
            if name == "spell_curse_of_bleakheart"
                && apply
                && eff == 0
                && let Some(e) = holder.effects[0].as_mut()
            {
                // Aura::SetPeriodicTimer: also makes
                // the aura periodic with this period
                // (DBC amplitude 0).
                e.periodic_timer = 5_000;
                e.period_ms = 5_000;
            }
        }
    }

    /// `AuraScript::OnAfterApply` (fork
    /// `SpellAuras.cpp:907`, after the handler).
    /// Holder is already in the unit's aura list on
    /// apply (and gone on remove), so arms address
    /// it through the unit when needed. Arms below
    /// (wyvern sting onward) dispatch here.
    pub(crate) fn aura_script_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        holder_caster: ObjectGuid,
        apply: bool,
        remove_mode: AuraRemoveMode,
    ) {
        // Wyvern Sting ranks: on remove the holder
        // caster (captured pre-removal) casts the
        // rank sting dot on the victim.
        if !apply && matches!(spell_id, 19386 | 24132 | 24133) {
            self.hunter_wyvern_sting_on_after_apply(victim, holder_caster, spell_id);
        }
        // Item aura scripts keyed by spell id
        // (Discombobulator Ray / Ashbringer).
        if spell_id == 4060 {
            self.discombobulate_on_after_apply(victim, apply);
        }
        if spell_id == 28282 {
            self.ashbringer_on_after_apply(victim, apply);
        }
        let name = self
            .stores
            .spells
            .get(spell_id)
            .map(|s| s.script_name.as_str());
        match name {
            Some("spell_cannibalize_aura") => {
                self.cannibalize_aura_on_after_apply(victim, spell_id, apply);
            }
            Some("spell_shadowmeld") => {
                self.shadowmeld_on_after_apply(victim, spell_id, apply);
            }
            Some("spell_stoneform") => {
                self.stoneform_on_after_apply(victim, spell_id, apply);
            }
            Some("spell_silithyst") => {
                self.silithyst_on_after_apply(victim, spell_id, apply, remove_mode);
            }
            Some("spell_controlling_steam_tonk") => {
                self.controlling_steam_tonk_on_after_apply(victim, spell_id, apply);
            }
            Some("spell_nefarian_polymorph") => {
                if apply {
                    self.nefarian_polymorph_on_after_apply(victim);
                }
            }
            Some("spell_thaddius_positive_charge_aura") => {
                if !apply {
                    self.thaddius_positive_charge_aura_on_after_apply(victim);
                }
            }
            Some("spell_chains_of_kelthuzad") => {
                self.chains_of_kelthuzad_on_after_apply(victim, spell_id, apply);
            }
            Some("spell_thaddius_negative_charge_aura") if !apply => {
                self.thaddius_negative_charge_aura_on_after_apply(victim);
            }
            _ => {}
        }
    }

    /// `AuraScript::OnPeriodicDummy` (fork
    /// `Aura::PeriodicDummyTick` script dispatch,
    /// before the hardcoded family switch — the
    /// switch remainder is recorded in
    /// TIER3-PROGRESS Slice 2 step 4).
    pub(crate) fn aura_script_on_periodic_dummy(&mut self, tick: &PeriodicTick) {
        let Some(name) = self
            .stores
            .spells
            .get(tick.spell_id)
            .map(|s| s.script_name.clone())
        else {
            return;
        };
        match name.as_str() {
            // 6946: 5% per tick, target casts 6945
            // on itself triggered by the aura.
            "spell_curse_of_bleakheart" => {
                if self.unit_is_alive(tick.target) && self.spell_rng().irand(0, 99) < 5 {
                    self.cast_spell(
                        tick.target,
                        6945,
                        CastTarget::Unit(tick.target),
                        CastOptions {
                            triggered: true,
                            triggered_by_aura: true,
                            skip_known: true,
                            cast_item: None,
                            custom_base_points: [None; 3],
                            triggered_by_aura_spell: tick.spell_id,
                            triggered_by_spell: 0,
                        },
                    );
                }
            }
            // 7057 Haunting Spirits: 5% per tick to
            // summon via 7067 (moved here from the
            // inline periodic-dummy arm).
            "spell_haunting_spirits"
                if tick.effect_index == 0
                    && self.unit_is_alive(tick.target)
                    && self.spell_rng().irand(0, 99) < 5 =>
            {
                self.cast_spell(
                    tick.target,
                    7067,
                    CastTarget::Unit(tick.target),
                    CastOptions {
                        triggered: true,
                        triggered_by_aura: true,
                        skip_known: true,
                        cast_item: None,
                        custom_base_points: [None; 3],
                        triggered_by_aura_spell: tick.spell_id,
                        triggered_by_spell: 0,
                    },
                );
            }
            "spell_haunting_phantoms"
                if tick.effect_index == 0 && self.unit_is_alive(tick.target) =>
            {
                self.haunting_phantoms_on_periodic_dummy(tick.target, tick.spell_id);
            }
            "spell_golemaggs_trust"
                if tick.effect_index == 0 && self.unit_is_alive(tick.target) =>
            {
                self.golemaggs_trust_on_periodic_dummy(tick.target, tick.caster, tick.spell_id);
            }
            _ => {}
        }
    }

    /// `AuraScript::OnPeriodicTrigger` (fork
    /// `Aura::TriggerSpell`, `SpellAuras.cpp:1498`):
    /// runs before the default trigger cast; an arm
    /// may cast a custom spell itself and set
    /// `trigger` to `None` to suppress the default,
    /// or replace the spell id. The fork passes the
    /// wearer as trigger caster in the generic case
    /// (with the aura caster as original-caster
    /// guid); the Rust periodic-trigger arm resolves
    /// the caster by the wearer-casts attribute
    /// instead — arms receive that resolved caster.
    /// Arms below (curse of idiocy onward) dispatch here.
    pub(crate) fn aura_script_on_periodic_trigger(
        &mut self,
        tick: &PeriodicTick,
        spell: &SpellEntry,
        trigger_caster: ObjectGuid,
        trigger: &mut Option<u32>,
    ) {
        // 1010 Curse of Idiocy: suppress the trigger
        // cast once both stats have lost > 90 (and
        // for aura-triggered self casts).
        if spell.script_name == "spell_warlock_curse_of_idiocy"
            && self.warlock_curse_of_idiocy_on_periodic_trigger(trigger_caster, tick.target)
        {
            *trigger = None;
        }
        // 13810 Frost Trap Aura: caster proc hook
        // (Entrapment; proc pipeline UNMODELED in
        // the helper).
        if spell.script_name == "spell_hunter_frost_trap_aura" {
            self.hunter_frost_trap_aura_on_periodic_trigger(
                tick.caster,
                tick.target,
                tick.spell_id,
            );
        }
        // Darkmoon turret/flamethrower mana upkeep:
        // helper returns None to suppress (and it
        // removes the aura itself when out of mana).
        if spell.script_name == "spell_activate_mg_turret"
            && self
                .activate_mg_turret_on_periodic_trigger(tick.target, tick.spell_id)
                .is_none()
        {
            *trigger = None;
        }
        if spell.script_name == "spell_flamethrower"
            && self
                .flamethrower_on_periodic_trigger(tick.target, tick.spell_id)
                .is_none()
        {
            *trigger = None;
        }
        // Thaddius charge auras: maintain the amp
        // aura stack from nearby same-charge players.
        if spell.script_name == "spell_thaddius_positive_charge_aura" {
            self.thaddius_positive_charge_aura_on_periodic_trigger(tick.target);
        }
        if spell.script_name == "spell_thaddius_negative_charge_aura" {
            self.thaddius_negative_charge_aura_on_periodic_trigger(tick.target);
        }
        // 16191 Mana Tide: dithered custom cast,
        // default trigger suppressed.
        if spell.script_name == "spell_shaman_mana_tide" {
            let trigger_id = spell.effects[tick.effect_index as usize].trigger_spell;
            if self.shaman_mana_tide_on_periodic_trigger(
                tick.target,
                trigger_id,
                tick.amount,
                tick.spell_id,
            ) {
                *trigger = None;
            }
        }
    }

    // ===== spell_warrior_sweeping_strikes (spell_warrior.cpp:129) spells: 12292

    /// `WarriorSweepingStrikesAuraScript::OnHolderInit`
    /// (`spell_warrior.cpp:131-135`).
    #[allow(dead_code, clippy::unused_self)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warrior_sweeping_strikes_on_holder_init(&mut self, holder: &mut AuraHolder) {
        // Sweeping Strikes should not be removed on shapeshift.
        // UNMODELED: `AuraHolder` has no removed-on-shape-lost field;
        // add one (default true) and set it false here.
        let _ = holder;
    }

    // ===== spell_warrior_blood_fury (spell_warrior.cpp:141) spells: 23234
    // (holder-level arm in `aura_script_on_before_apply`, next to the bleakheart arm;
    // guest on `apply`, effect 0 present.)
    // "spell_warrior_blood_fury" if apply && holder.effects[0].is_some() => {

    // ===== spell_warrior_blood_fury (spell_warrior.cpp:141) spells: 23234

    /// `WarriorBloodFuryAuraScript::OnBeforeApply`
    /// (`spell_warrior.cpp:148-169`): on cancel/expire, debit 25% of
    /// AP via 23230.
    /// Fork body is inside `#if (SUPPORTED_CLIENT_BUILD > 1_3_1)
    /// && (<= 1_8_4)`: compiled OUT for a 1.12.1 target —
    /// transcribed for completeness, gate before wiring.
    /// `SPELL_BLOOD_FURY_DEBUFF = 23230`.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warrior_blood_fury_on_before_apply(&mut self, victim: ObjectGuid, holder: &AuraHolder) {
        // `aura->GetEffIndex() != EFFECT_INDEX_0` return: holder-level
        // equivalent is effect-0 presence (checked at the arm).
        // UNMODELED: remove-mode gate (`AURA_REMOVE_BY_CANCEL ||
        // AURA_REMOVE_BY_EXPIRE`) — holders carry no remove mode; thread
        // the fork `GetRemoveMode()` through `remove_aura_at`.
        // UNMODELED: fork defers 1ms via `m_Events.AddLambdaEventAtOffset`
        // (ExclusiveAuraUnapply workaround); no delayed-event seam, so the
        // debit below lands immediately.
        let Some(u) = self.unit_base(victim) else {
            return;
        };
        // Fork reads `UNIT_FIELD_ATTACK_POWER` int field; lane subset
        // uses base + mods like the bloodthirst arm.
        let attack_power = u.base_attack_power + u.attack_power_mods[0] + u.attack_power_mods[1];
        #[allow(clippy::cast_possible_truncation)]
        let debit = (-25.0 * attack_power / 100.0) as i32;
        if debit < 0 {
            self.cast_custom_spell(victim, victim, 23230, [Some(debit), None, None]);
        }
        let _ = holder;
    }

    // ===== spell_shaman_flametongue_proc_dummy (spell_shaman.cpp:20) spells: 8026, 8028, 8029, 8248, 8253, 10445, 10523, 16343, 16344, 16389 (Flametongue Weapon/Totem Proc)

    // ===== spell_shaman_mana_tide (spell_shaman.cpp:47) spells: 16191

    /// `ShamanManaTideAuraScript::OnPeriodicTrigger`
    /// (`spell_shaman.cpp:49-60`): custom dithered amount instead of
    /// the default trigger; prevents the default cast.
    fn shaman_mana_tide_on_periodic_trigger(
        &mut self,
        target: ObjectGuid,
        trigger_spell_id: u32,
        modifier_amount: i32,
        aura_spell_id: u32,
    ) -> bool {
        // `triggerSpellId = EffectTriggerSpell[effIdx]` read by caller.
        if trigger_spell_id == 0 {
            return false;
        }
        #[allow(clippy::cast_precision_loss)]
        let dithered = mangos_spells::entry::dither(modifier_amount as f32, self.spell_rng());
        self.cast_custom_spell_aura(
            target,
            target,
            trigger_spell_id,
            [Some(dithered), None, None],
            aura_spell_id,
        );
        // Prevent default cast (`spellInfo = nullptr`).
        true
    }

    // ===== spell_rogue_eviscerate (spell_rogue.cpp:20) spells: 2098, 6760, 6761, 6762, 8623, 8624, 11299, 11300, 31016

    // ===== spell_warlock_curse_of_idiocy (spell_warlock.cpp:382) spells: 1010

    // NEEDS-HOST: OnPeriodicTrigger seam: fork periodic-trigger dispatch hands the
    // aura's pending `spellInfo` by mutable reference so the script can suppress
    // it (`spellInfo = nullptr`). Host helper returns true = suppress.
    /// `WarlockCurseOfIdiocyAuraScript::OnPeriodicTrigger`
    /// (`spell_warlock.cpp:389-426`): stops the intellect/spirit drain once each
    /// has lost more than 90.
    fn warlock_curse_of_idiocy_on_periodic_trigger(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> bool {
        // Prevent cast by triggered auras
        if caster == victim {
            return true;
        }
        // Stop triggering after each affected stats lost > 90.
        // Fork `Stats`: STAT_INTELLECT = 3, STAT_SPIRIT = 4 (`SharedDefines.h:140-145`).
        let mut intellect_loss: i32 = 0;
        let mut spirit_loss: i32 = 0;
        if let Some(unit) = self.unit_base(victim) {
            for holder in &unit.auras {
                if holder.spell_id != 1010 {
                    continue;
                }
                for effect in holder.effects.iter().flatten() {
                    if effect.aura_type != mangos_spells::defines::aura::MOD_STAT {
                        continue;
                    }
                    match effect.misc_value {
                        3 => intellect_loss += effect.amount,
                        4 => spirit_loss += effect.amount,
                        _ => {}
                    }
                }
            }
        }
        intellect_loss <= -90 && spirit_loss <= -90
    }

    // ===== spell_hunter_wyvern_sting (spell_hunter.cpp:20) spells: 19386, 24132, 24133
    // Fork: `OnAfterApply`: on remove of effect 0, the (player) caster casts the
    // rank-matched sting dot on the aura target: 19386 -> 24131, 24132 -> 24134,
    // 24133 -> 24135. Unknown rank logs in the fork and returns.
    // The `!apply` arm below runs at holder level; the fork additionally gates on
    // `GetEffIndex() == EFFECT_INDEX_0` (single-effect aura: removal of the only
    // effect). `holder_caster` must be captured before holder removal
    // (fork `aura->GetCaster()`), and the caster must be a player
    // (fork `GetTypeId() != TYPEID_PLAYER` return; see UNMODELED note).
    // (in `aura_script_on_after_apply`; `victim`, `spell_id`, `apply` in scope;
    // thread `holder_caster: ObjectGuid` captured pre-removal into this call):
    // if spell_id == 19386 || spell_id == 24132 || spell_id == 24133 {
    //     if !apply {
    //     }
    /// `HunterWyvernStingScript::OnAfterApply` (`spell_hunter.cpp:22-50`).
    fn hunter_wyvern_sting_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        holder_caster: ObjectGuid,
        spell_id: u32,
    ) {
        if !holder_caster.is_player() {
            return;
        }
        let dot = match spell_id {
            19386 => 24131,
            24132 => 24134,
            24133 => 24135,
            _ => return, // fork logs "Unknown wyvern sting rank"; no log sink here
        };
        self.cast_spell(
            holder_caster,
            dot,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: spell_id,
                triggered_by_spell: 0,
            },
        );
    }

    // ===== spell_hunter_frost_trap_aura (spell_hunter.cpp:122) spells: 13810
    // Fork: `OnPeriodicTrigger`: the trap's caster procs off the entering target
    // (talent `Entrapment` root chance lives in the proc system):
    // `ProcDamageAndSpell(target, PROC_FLAG_ON_TRAP_ACTIVATION, PROC_FLAG_NONE,
    // PROC_EX_NORMAL_HIT, 1, 1, BASE_ATTACK, aura->GetSpellProto())`.
    // OnPeriodicTrigger seam: fork calls it from `Aura::PeriodicTrigger`
    // (`SpellAuras.cpp:1499`). Wired via `aura_script_on_periodic_trigger`.
    /// `HunterFrostTrapAuraScript::OnPeriodicTrigger` (`spell_hunter.cpp:125-133`).
    /// `caster` = aura holder caster (`aura->GetCaster()`, may be gone ->
    /// return), `target` = the unit that triggered the trap.
    fn hunter_frost_trap_aura_on_periodic_trigger(
        &mut self,
        caster: ObjectGuid,
        _target: ObjectGuid,
        spell_id: u32,
    ) {
        let _ = spell_id; // fork passes `aura->GetSpellProto()` as proc spell
        if !self.unit_is_alive(caster) {}
        // UNMODELED: `ProcDamageAndSpell` with PROC_FLAG_ON_TRAP_ACTIVATION —
        // no proc pipeline in the lane yet (this is where Entrapment roots).
    }

    // ===== spell_mage_ignite (spell_mage.cpp:48) spells: 11119, 11120, 12846, 12847, 12848
    // Fork: `OnProc` on the ignite talent aura. Rank from the triggering talent
    // aura: 11119 -> 4%, 11120 -> 8%, 12846 -> 12%, 12847 -> 16%, 12848 -> 20% of
    // `originalAmount` as ignite dot base points (spell 12654). If the victim
    // already has ignite (12654 eff 0): below 5 stacks and before its last tick,
    // add the new damage to the tick, stack +1 and refresh; at 5 stacks just
    // refresh; past its last tick, remove and reapply fresh. Unknown talent id
    // logs in the fork and fails the proc.
    // OnProc seam: fork calls it from the proc handler
    // (`UnitAuraProcHandler.cpp`, `SpellAuraProcResult` protocol).
    // Wired via `aura_script_on_proc`.
    /// `MageIgniteScript::OnProc` (`spell_mage.cpp:60-125`).
    /// Returns `Some(true)` = `PROC_OK`, `Some(false)` = `PROC_FAILED`,
    /// `None` = no handling (mirrors `optional<SpellAuraProcResult>`).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::items_after_statements, clippy::cast_precision_loss)]
    fn mage_ignite_on_proc(
        &mut self,
        owner: ObjectGuid,
        victim: ObjectGuid,
        original_amount: i32,
        triggered_by_aura_spell_id: u32,
        proc_charges_cooldown: u32,
    ) -> bool {
        const IGNITE_DOT: u32 = 12654;
        let _ = proc_charges_cooldown;
        let pct = match triggered_by_aura_spell_id {
            11119 => 0.04,
            11120 => 0.08,
            12846 => 0.12,
            12847 => 0.16,
            12848 => 0.20,
            _ => return false, // fork logs "non handled spell id"
        };
        let base_points = (pct * original_amount as f32) as i32;
        // Snapshot the victim's current ignite (spell 12654 eff 0): stack
        // count, tick damage (`m_modifier.m_amount`) and tick progress, so the
        // mutation below runs without holding the unit borrow.
        struct IgniteState {
            stack: u32,
            ticks_done: u32,
        }
        let mut current: Option<IgniteState> = None;
        if let Some(unit) = self.unit_base(victim) {
            for holder in &unit.auras {
                if holder.spell_id != IGNITE_DOT {
                    continue;
                }
                let Some(state) = holder.effects[0].as_ref() else {
                    continue;
                };
                current = Some(IgniteState {
                    stack: holder.stack,
                    ticks_done: state.periodic_ticks,
                });
                break;
            }
        }
        if let Some(dot) = current {
            // Fork stack branch: ticks exhausted ->
            // remove and reapply fresh below; else
            // accumulate tick damage (below 5
            // stacks), ModStackAmount(1), Refresh
            // (duration = max, periodicTick = 0,
            // timer = period; visible stack/duration
            // resend rides the generic aura update).
            let max_ticks = {
                let state = self.unit_base(victim).and_then(|u| {
                    u.auras
                        .iter()
                        .find(|h| h.spell_id == IGNITE_DOT)
                        .and_then(|h| {
                            h.effects[0]
                                .as_ref()
                                .map(|s| (h.max_duration_ms, s.period_ms))
                        })
                });
                match state {
                    Some((max_ms, period)) if max_ms > 0 && period > 0 => (max_ms as u32) / period,
                    _ => 0,
                }
            };
            if dot.ticks_done >= max_ticks && max_ticks != 0 {
                self.remove_auras_due_to_spell(victim, IGNITE_DOT);
            } else {
                if let Some(unit) = self.unit_base_mut(victim) {
                    if let Some(holder) = unit.auras.iter_mut().find(|h| h.spell_id == IGNITE_DOT) {
                        if dot.stack < 5 {
                            if let Some(state) = holder.effects[0].as_mut() {
                                state.amount += base_points;
                                state.periodic_ticks = 0;
                                #[allow(clippy::cast_possible_wrap)]
                                {
                                    state.periodic_timer = state.period_ms as i32;
                                }
                            }
                            holder.stack = dot.stack + 1;
                        } else {
                            holder.stack = 5;
                            if let Some(state) = holder.effects[0].as_mut() {
                                state.periodic_ticks = 0;
                                #[allow(clippy::cast_possible_wrap)]
                                {
                                    state.periodic_timer = state.period_ms as i32;
                                }
                            }
                        }
                        holder.duration_ms = holder.max_duration_ms;
                    }
                }
                return true;
            }
        }
        // No Ignite found: `TriggerProccedSpell(victim, basePoints, 12654,
        // nullptr, triggeredByAura, cooldown)` — custom-base-point triggered
        // cast carrying the talent aura id.
        self.cast_custom_spell_aura(
            owner,
            victim,
            IGNITE_DOT,
            [Some(base_points), None, None],
            triggered_by_aura_spell_id,
        );
        true
    }

    // ===== spell_mage_combustion_proc (spell_mage.cpp:139) spells: 11129
    // Fork (`#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_10_2`): `OnProc` on the
    // invisible combustion counter aura. No victim (aoe like flamestrike with no
    // target hit) -> fail. Counter dispelled/cancelled (no 28682 buff on owner)
    // -> remove the proc aura, fail. Last charge + crit -> remove the 28682 buff,
    // proc OK (charge counting removes the counter). Otherwise recast the 28682
    // buff triggered on self; proc OK only on crit (charge updates on crit hits).
    // OnProc seam (same as ignite above; wired via `aura_script_on_proc`).
    /// `MageCombustionProcScript::OnProc` (`spell_mage.cpp:141-163`).
    /// `victim`: `None` when no target was affected. `last_charge` mirrors
    /// `triggeredByAura->GetHolder()->GetAuraCharges() <= 1` (the future
    /// `OnProc` dispatcher supplies it). Returns `Some(true)` = `PROC_OK`,
    /// `Some(false)` = `PROC_FAILED`, `None` = no handling.
    fn mage_combustion_proc_on_proc(
        &mut self,
        owner: ObjectGuid,
        victim: Option<ObjectGuid>,
        triggered_by_aura_spell_id: u32,
        proc_ex_crit: bool,
        last_charge: bool,
    ) -> bool {
        const COMBUSTION_PROC_AURA: u32 = 11129;
        const COMBUSTION_CRIT_BUFF: u32 = 28682;
        let _ = COMBUSTION_PROC_AURA;
        let Some(_victim) = victim else {
            // does not proc if no target is affected (aoe like flamestrike)
            return false;
        };
        // combustion counter was dispelled or clicked off
        if !self.unit_has_aura_spell(owner, COMBUSTION_CRIT_BUFF) {
            self.remove_auras_due_to_spell(owner, triggered_by_aura_spell_id);
            return false;
        }
        // last charge and crit
        if last_charge && proc_ex_crit {
            self.remove_auras_due_to_spell(owner, COMBUSTION_CRIT_BUFF);
            // remove Combustion auras; charge counting (will be removed)
            return true;
        }
        self.cast_spell(
            owner,
            COMBUSTION_CRIT_BUFF,
            CastTarget::Unit(owner),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: triggered_by_aura_spell_id,
                triggered_by_spell: 0,
            },
        );
        // charge update only at crit hits, no hidden cooldowns
        proc_ex_crit
    }

    // ===== spell_mage_combustion_buff (spell_mage.cpp:169) spells: 28682
    // Fork (`#if ..._1_10_2`): `OnAfterApply`: when effect 0 is removed by cancel
    // (buff clicked off), the invisible proc aura 11129 is removed from the
    // target, starting the cooldown.
    // (same holder-caster note as wyvern sting; `victim`, `spell_id`, `apply` in
    // scope; remove-mode must be AURA_REMOVE_BY_CANCEL — the dispatcher must
    // thread the remove mode; eff gate is EFFECT_INDEX_0):
    // if spell_id == 28682 && !apply {
    /// `MageCombustionBuffScript::OnAfterApply` (`spell_mage.cpp:171-176`).
    fn mage_combustion_buff_on_after_apply(&mut self, victim: ObjectGuid) {
        // removing the invisible proc aura starts cooldown
        self.remove_auras_due_to_spell(victim, 11129);
    }

    // ===== spell_cannibalize_aura (spell_special.cpp:255) spells: 20578

    // NEEDS-HOST: OnPeriodicTickEnd seam: no `AuraScript::OnPeriodicTickEnd`
    // dispatch exists in the Rust host (fork calls it at the end of
    // `Aura::PeriodicTick`, `SpellAuras.cpp`). Wire a post-tick hook in the
    // periodic-tick completion path.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn cannibalize_aura_on_periodic_tick_end(
        &mut self,
        victim: ObjectGuid,
        misc_value: i32,
        target_alive: bool,
    ) {
        // Fork: while alive, if the target's power type equals the aura misc
        // value, hold the cannibalize emote (`EMOTE_STATE_CANNIBALIZE`).
        // UNMODELED: `HandleEmoteCommand` and the power-type accessor have no
        // Rust runtime; the alive + misc gate above is the reachable part.
        let _ = (victim, misc_value, target_alive);
    }

    // ===== spell_shadowmeld (spell_special.cpp:414) spells: 20580

    // `aura_script_on_before_apply`, mirroring the Bleakheart inline arm)

    // ===== spell_stoneform (spell_special.cpp:457) spells: 20594

    // ===== spell_silithyst (spell_special.cpp:286) spells: 29519

    // ===== spell_controlling_steam_tonk (spell_special.cpp:367) spells: 24937

    // ===== spell_activate_mg_turret (spell_special.cpp:339) spells: 25026

    // ===== spell_activate_mg_turret (spell_special.cpp:339) spells: 25026

    // OnPeriodicTrigger seam (fork calls it from `Aura::PeriodicTriggerSpell`,
    // `SpellAuras.cpp`, letting the script replace or null `spellInfo`).
    // Wired via `aura_script_on_periodic_trigger`; the `Option<u32>` return
    // vetoes the triggered spell. `spell_haunting_spirits` precedent covers
    // the periodic-dummy variant only, not this one.
    fn activate_mg_turret_on_periodic_trigger(
        &mut self,
        victim: ObjectGuid,
        aura_spell_id: u32,
    ) -> Option<u32> {
        // MANA_COST_PER_TICK = 10. Fork: while the target holds >= 10 mana,
        // pay 10 (`ModifyPower(POWER_MANA, -10)` + `SendEnergizeSpellLog`)
        // and let the trigger spell fire; else remove the aura and force
        // `spellInfo = nullptr` (return `None` here).
        // Energize log packet UNMODELED; the mana
        // debit + aura removal are live.
        let mana = self.unit_power(victim, mangos_spells::defines::power::MANA);
        if mana >= 10 {
            self.unit_set_power(victim, mangos_spells::defines::power::MANA, mana - 10);
            Some(aura_spell_id)
        } else {
            self.remove_auras_due_to_spell(victim, aura_spell_id);
            None
        }
    }

    // ===== spell_flamethrower (spell_special.cpp:339) spells: 25027

    // ===== spell_flamethrower (spell_special.cpp:339) spells: 25027

    // OnPeriodicTrigger seam: same seam as `spell_activate_mg_turret`
    // (shared `DarkmoonFaireManaConsumptionAuraScript` struct); wired via
    // `aura_script_on_periodic_trigger`, see above.
    fn flamethrower_on_periodic_trigger(
        &mut self,
        victim: ObjectGuid,
        aura_spell_id: u32,
    ) -> Option<u32> {
        // MANA_COST_PER_TICK = 10; identical body to the MG Turret.
        // Energize log packet UNMODELED; the mana
        // debit + aura removal are live.
        let mana = self.unit_power(victim, mangos_spells::defines::power::MANA);
        if mana >= 10 {
            self.unit_set_power(victim, mangos_spells::defines::power::MANA, mana - 10);
            Some(aura_spell_id)
        } else {
            self.remove_auras_due_to_spell(victim, aura_spell_id);
            None
        }
    }

    // ---------------------------------------------------------------------------
    // Helpers (`impl Map`, matching the Holy Shock / Bleakheart exemplar style).
    // `OnAfterApply` helpers take the dispatcher's `(victim, spell_id, apply)`:
    // the fork's `GetEffIndex()` gate is noted per helper; the host calls these
    // at holder level, so the eff-0-only behavior below assumes the eff-0 aura
    // is present (see the per-effect gate note in `unresolved`).
    // ---------------------------------------------------------------------------

    // ===== spell_cannibalize_aura (spell_special.cpp:255) spells: 20578

    /// `CannibalizeAuraScript::OnAfterApply` (`spell_special.cpp:257-269`):
    /// fork gates on `GetEffIndex() == EFFECT_INDEX_0`; on removal clears
    /// the cannibalize emote (`HandleEmoteCommand(EMOTE_STATE_NONE)`).
    /// UNMODELED: emote commands have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn cannibalize_aura_on_after_apply(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        let _ = (victim, spell_id, apply);
    }

    // ===== spell_shadowmeld (spell_special.cpp:414) spells: 20580

    /// `ShadowmeldAuraScript::OnAfterApply` (`spell_special.cpp:421-435`):
    /// fork gates on apply + `GetEffIndex() == EFFECT_INDEX_0` + player
    /// target; casts the Elusiveness visual 21009 triggered by the aura.
    fn shadowmeld_on_after_apply(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        if !apply || !victim.is_player() {
            return;
        }
        self.cast_spell(
            victim,
            21009,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: spell_id,
                triggered_by_spell: 0,
            },
        );
    }

    // ===== spell_shadowmeld (spell_special.cpp:414) spells: 20580

    /// `ShadowmeldAuraScript::OnBeforeApply` (`spell_special.cpp:437-452`):
    /// fork gates on removal + `GetEffIndex() == EFFECT_INDEX_0` + player
    /// target; removes the Elusiveness visual 21009.
    fn shadowmeld_on_before_apply(&mut self, victim: ObjectGuid) {
        self.remove_auras_due_to_spell(victim, 21009);
    }

    // ===== spell_stoneform (spell_special.cpp:457) spells: 20594

    /// `StoneformAuraScript::OnAfterApply` (`spell_special.cpp:459-476`):
    /// fork gates on `GetEffIndex() == EFFECT_INDEX_0` (the 1.12.1 effect
    /// index; the fork's `#if` keeps `EFFECT_INDEX_2` for
    /// `CLIENT_BUILD_1_6_1` and older) and grants Disease + Poison dispel
    /// immunity for the aura's lifetime.
    /// UNMODELED: `ApplySpellDispelImmunity` has no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn stoneform_on_after_apply(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        let _ = (victim, spell_id, apply);
    }

    // ===== spell_silithyst (spell_special.cpp:286) spells: 29519

    /// `SilithystAuraScript::OnAfterApply` (`spell_special.cpp:295-333`):
    /// fork gates on `GetEffIndex() == EFFECT_INDEX_1` + player target in
    /// zone 1377 (Silithus); on apply casts the team buff (29894 Alliance
    /// / 29895 Horde), on removal drops it, and on cancel/death/dispel
    /// removal notifies the zone script (`HandleDropFlag`). Body lives in
    /// `outdoor_pvp_si` (`si_silithyst_aura`); the effect-index gate is
    /// dropped (GO-hello precedent: spell-id presence only, recorded).
    fn silithyst_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        apply: bool,
        remove_mode: AuraRemoveMode,
    ) {
        self.si_silithyst_aura(victim, spell_id, apply, remove_mode);
    }

    // ===== spell_controlling_steam_tonk (spell_special.cpp:367) spells: 24937

    /// `ControllingSteamTonkAuraScript::OnAfterApply`
    /// (`spell_special.cpp:377-409`): fork gates on removal +
    /// `GetEffIndex() == EFFECT_INDEX_0` with a player caster; casts
    /// Damaged Tonk 27771 on the tonk, 3s Stun 9179 on the player,
    /// removes Unroot 24935, then resets the Tonk Control Console
    /// (GO 180524: `GO_STATE_READY`, loot state, clear `GO_FLAG_IN_USE`).
    /// UNMODELED: gameobject lookup/state (`FindNearestGameObject`,
    /// `SetGoState`, loot state, `GO_FLAG_IN_USE`) has no Rust runtime;
    /// the three spell casts below are the reachable part.
    fn controlling_steam_tonk_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        apply: bool,
    ) {
        if apply {
            return;
        }
        let Some(holder) = self
            .unit_base(victim)
            .and_then(|u| u.auras.iter().find(|h| h.spell_id == spell_id))
            .cloned()
        else {
            // Holder already gone on the remove path: fall back to the
            // casts that do not need holder state.
            self.cast_spell(
                victim,
                27771,
                CastTarget::Unit(victim),
                CastOptions {
                    triggered: true,
                    triggered_by_aura: true,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: spell_id,
                    triggered_by_spell: 0,
                },
            );
            return;
        };
        let caster = holder.caster;
        self.cast_spell(
            victim,
            27771,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: spell_id,
                triggered_by_spell: 0,
            },
        );
        if caster.is_player() {
            self.cast_spell(
                caster,
                9179,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    triggered_by_aura: true,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: spell_id,
                    triggered_by_spell: 0,
                },
            );
            self.remove_auras_due_to_spell(caster, 24935);
        }
        // UNMODELED: Tonk Control Console (GO 180524) reset.
    }

    // ===== spell_heavy_armor_kit (spell_item.cpp:20) spells: 2833
    // (drop into the `match spell.script_name.as_str()` in `spell_script_on_check_cast`)

    // ===== spell_gdr_periodic (spell_item.cpp:243) spells: 13493
    // (AuraScript: OnAuraValueCalculate + OnPeriodicCalculateAmount + OnAfterApply — none wired)
    // NEEDS-HOST: OnAuraValueCalculate seam: amount calculation during aura modifier
    // setup (holder `base_points` is written by the map there).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn gdr_periodic_damage_on_aura_value_calculate(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        eff_index: usize,
    ) -> i32 {
        let _ = (victim, holder, eff_index);
        self.spell_rng().irand(100, 500)
    }

    // ===== spell_gdr_periodic (spell_item.cpp:243) spells: 13493

    // NEEDS-HOST: OnPeriodicCalculateAmount seam: periodic amount tick calculation
    // (fork `Aura::PeriodicTick` amount path; no periodic-amount hook in the lane yet).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn gdr_periodic_damage_on_periodic_calculate_amount(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        amount: &mut f32,
    ) {
        let _ = (victim, holder);
        // UNMODELED: fork zeroes `amount` unless the victim's current channeled
        // spell is still the GDR channel (13278) — no current-channel lookup in
        // the lane — and accumulates `dmg += amount` in script-member state, for
        // which holders carry no per-script storage.
        *amount = 0.0;
    }

    // ===== spell_gdr_periodic (spell_item.cpp:243) spells: 13493

    // NEEDS-HOST: OnAfterApply seam: fork `SpellAuras.cpp:907` (holder level in
    // `add_aura_holder` / `remove_aura_at`); the lane `aura_script_on_after_apply`
    // passes `(victim, spell_id, apply)` without remove-mode or holder access.
    #[allow(dead_code, clippy::neg_cmp_op_on_partial_ord)] // NO-DISPATCHER-YET (step-4 remainder)
    fn gdr_periodic_damage_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        apply: bool,
        expired: bool,
        accumulated: f32,
    ) {
        if apply || !expired || !(accumulated > 0.0) {
            return;
        }
        // UNMODELED: fork re-targets the victim's current target
        // (`GetTargetGuid` + map lookup) and needs `AURA_REMOVE_BY_EXPIRE`.
        // `custom_base_points` carries the accumulated `dmg` (`CastCustomSpell`
        // bp0); target mapped to the victim here.
        self.cast_spell(
            victim,
            13279, // SPELL_GDR_DAMAGE_HIT
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                custom_base_points: [Some(accumulated as i32), None, None],
                ..CastOptions::default()
            },
        );
        let _ = holder;
    }

    // ===== spell_discombobulate (spell_item.cpp:627) spells: 4060 (Discombobulator Ray)
    // (the lane `aura_script_on_after_apply(victim, spell_id, apply)` has no arms
    // yet — first user; add `if spell_id == 4060 { self.discombobulate_on_after_apply(victim, apply); }`)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn discombobulate_on_after_apply(&mut self, victim: ObjectGuid, apply: bool) {
        if !apply {
            return;
        }
        // Fork additionally gates on `GetEffIndex() == EFFECT_INDEX_0`; the lane
        // seam is holder-level, so the per-effect gate cannot be evaluated here.
        // UNMODELED: `target->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED)` —
        // discombobulate removes mount auras on apply; the lane has no
        // remove-aura-by-type API yet.
        let _ = victim;
    }

    // ===== spell_ashbringer (spell_item.cpp:647) spells: 28282
    // (same `aura_script_on_after_apply` site: `if spell_id == 28282 { ... }`)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn ashbringer_on_after_apply(&mut self, victim: ObjectGuid, apply: bool) {
        let _ = (victim, apply);
        // UNMODELED: the whole body is reputation/faction state with no lane API —
        // fork gates on `GetEffIndex() == EFFECT_INDEX_0` and `TYPEID_PLAYER`,
        // then `ApplyForceReaction(56 SCARLET_CRUSADE, FRIENDLY rank 7, apply)` +
        // `SendForceReactions()`, and `StopAttackFaction(56)` when the forced rank
        // (apply) or the real rank (remove) is >= REP_FRIENDLY.
    }

    // ===== spell_nefarian_corrupted_totems (boss_nefarian.cpp:551) spells: 23424 -> {23419, 23420, 23422, 23423}

    // ===== spell_nefarian_shadow_flame_passive (boss_nefarian.cpp:571) spells: 22992 -> 22986
    // When Nefarian lands at the start of Phase 2 he uses an AoE
    // Shadowflame: ~1000 initial shadow damage plus a deadly DoT unless an
    // Onyxia Scale Cloak is equipped. Hiding behind Nefarian's throne
    // avoids the hit (geometry, handled by LoS, not here).

    // ===== spell_nefarian_class_call_warlock (boss_nefarian.cpp:587) spells: 23427 -> 23426 x2
    // Each Warlock summons 2 hostile Corrupted Infernals that stun and
    // damage nearby players.

    // ===== spell_nefarian_class_call_rogue (boss_nefarian.cpp:611) spells: 23414 (teleport)
    // Paralyze + teleport the player to a random position near Nefarian.

    // ===== spell_nefarian_class_call_mage (boss_nefarian.cpp:647) spells: 23410 (aura, 5s ticker)
    // Wild Magic: the aura ticks every 5s and forces a random raid member
    // polymorph (see NEEDS-HOST tick-end below).

    // ===== spell_nefarian_polymorph (boss_nefarian.cpp:704) spells: 23603 (aura, transform display)

    // ===== spell_nefarian_polymorph (boss_nefarian.cpp:704) spells: 23603 (aura, transform display)

    /// `NefarianPolymorphAuraScript::OnAfterApply`
    /// (`boss_nefarian.cpp:706-737`): on apply only (fork: effect 1;
    /// the holder-level seam carries no effect index, so this runs
    /// once per application); random transform display 1060/4473/7898
    /// plus `SetTransformScale(1.0)`.
    fn nefarian_polymorph_on_after_apply(&mut self, victim: ObjectGuid) {
        let _ = victim;
        // urand(0, 2) display table (boss_nefarian.cpp:718-731).
        let table = [1060_u32, 4473, 7898];
        let display_id = table[self.spell_rng().irand(0, 2) as usize];
        let _ = display_id;
        // UNMODELED: SetDisplayId + SetTransformScale: the Rust runtime
        // has no display-id seam, so the roll above is dropped.
    }

    // ===== spell_nefarian_class_call_mage (boss_nefarian.cpp:647) spells: 23410 (aura, 5s ticker)

    // OnPeriodicTickEnd seam (fork `Aura::PeriodicTickEnd` path).
    // Wired via `aura_script_on_periodic_tick_end` with the aura
    // bearer and holder.
    /// `NefarianClassCallMageAuraScript::OnPeriodicTickEnd`
    /// (`boss_nefarian.cpp:657-698`): every 5s the bearer (a living
    /// player mage without 23603) forces a random eligible raid member
    /// to be polymorphed via a non-triggered 23603 cast.
    fn nefarian_class_call_mage_on_periodic_tick_end(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
    ) {
        let _ = holder;
        if !victim.is_player() || !self.unit_is_alive(victim) {
            return;
        }
        if self.unit_has_aura(victim, 23_603) {
            return;
        }
        let candidates: Vec<ObjectGuid> = self.players.keys().copied().collect();
        let mut possible: Vec<ObjectGuid> = Vec::new();
        for guid in candidates {
            if !self.unit_is_alive(guid) || self.unit_has_aura(guid, 23_603) {
                continue;
            }
            // Fork IsWithinDist of the mage, 60.0yd 3d
            // (boss_nefarian.cpp:683); fork IsInWorld has no Rust
            // equivalent (a tracked position implies presence).
            // NOTE: the fork's self-skip is commented out
            // (boss_nefarian.cpp:682), so the bearer stays eligible.
            let in_range = match (self.unit_position(victim), self.unit_position(guid)) {
                (Some(a), Some(b)) => {
                    let dx = a.x - b.x;
                    let dy = a.y - b.y;
                    let dz = a.z - b.z;
                    (dx * dx + dy * dy + dz * dz).sqrt() < 60.0
                }
                _ => false,
            };
            if in_range {
                possible.push(guid);
            }
        }
        if possible.is_empty() {
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_possible_wrap
        )]
        let idx = self.spell_rng().irand(0, possible.len() as i32 - 1) as usize;
        let target = possible[idx];
        // `pMage->InterruptNonMeleeSpells(false)`
        // (boss_nefarian.cpp:695).
        self.interrupt_non_melee_spells(victim, false, 0);
        // Fork casts 23603 non-triggered (boss_nefarian.cpp:696); the
        // known-spell gate applies (skip_known unset, as in the fork).
        self.cast_spell(
            victim,
            23_603,
            CastTarget::Unit(target),
            CastOptions::default(),
        );
    }

    // ===== spell_thaddius_positive_charge (boss_thaddius.cpp:1191) spells: 28062 (nil damage if target has 28059)

    // ===== spell_thaddius_negative_charge (boss_thaddius.cpp:1209) spells: 28085 (nil damage if target has 28084)

    // ===== spell_thaddius_magnetic_pull (boss_thaddius.cpp:1225) spells: 28337 (knockback, skip effect)

    // ===== spell_thaddius_positive_charge_aura (boss_thaddius.cpp:1071) spells: 28059 apply, 29659 amp

    // ===== spell_thaddius_negative_charge_aura (boss_thaddius.cpp:1131) spells: 28084 apply, 29660 amp

    // ===== spell_thaddius_positive_charge_aura (boss_thaddius.cpp:1071) spells: 28059 apply, 29659 amp

    /// `ThaddiusPositiveChargeAuraScript::OnAfterApply`
    /// (`boss_thaddius.cpp:1113-1125`): on remove only (fork: effect 0;
    /// the holder-level seam carries no effect index, so this runs once
    /// per removal); drop the 29659 amplify aura.
    fn thaddius_positive_charge_aura_on_after_apply(&mut self, victim: ObjectGuid) {
        if self.unit_has_aura(victim, 29_659) {
            self.remove_auras_due_to_spell(victim, 29_659);
        }
    }

    // ===== spell_thaddius_negative_charge_aura (boss_thaddius.cpp:1131) spells: 28084 apply, 29660 amp

    /// `ThaddiusNegativeChargeAuraScript::OnAfterApply`
    /// (`boss_thaddius.cpp:1173-1185`): on remove only (fork: effect 0,
    /// same seam note as above); drop the 29660 amplify aura.
    fn thaddius_negative_charge_aura_on_after_apply(&mut self, victim: ObjectGuid) {
        if self.unit_has_aura(victim, 29_660) {
            self.remove_auras_due_to_spell(victim, 29_660);
        }
    }

    // ===== spell_thaddius_positive_charge_aura (boss_thaddius.cpp:1071) spells: 28059 apply, 29659 amp

    // OnPeriodicTrigger seam (fork `Aura::PeriodicTriggerSpell` path).
    // Wired via `aura_script_on_periodic_trigger` with the aura bearer.
    /// `ThaddiusPositiveChargeAuraScript::OnPeriodicTrigger`
    /// (`boss_thaddius.cpp:1079-1111`): Naxxramas only; count other
    /// living players within 13.0yd (2d) bearing 28059. If any, ensure
    /// the 29659 amp aura and set its stack to the count; else remove
    /// the amp aura.
    fn thaddius_positive_charge_aura_on_periodic_trigger(&mut self, victim: ObjectGuid) {
        // MAP_NAXXRAMAS = 533 (fork SharedDefines.h:1660).
        if self.map_id != 533 {
            return;
        }
        let others: Vec<ObjectGuid> = self.players.keys().copied().collect();
        let mut num_stacks = 0_u32;
        for guid in others {
            if guid == victim || !self.unit_is_alive(guid) {
                continue;
            }
            if !self.unit_has_aura(guid, 28_059) {
                continue;
            }
            // 2d distance is good enough (fork comment).
            let in_range = match (self.unit_position(victim), self.unit_position(guid)) {
                (Some(a), Some(b)) => {
                    let dx = a.x - b.x;
                    let dy = a.y - b.y;
                    dx * dx + dy * dy < 13.0 * 13.0
                }
                _ => false,
            };
            if in_range {
                num_stacks += 1;
            }
        }
        if num_stacks > 0 {
            // Fork Unit::AddAura: bearer casts the amp on itself.
            if !self.unit_has_aura(victim, 29_659)
                && let Some(amp) = self.stores.spells.get(29_659).cloned()
            {
                self.apply_spell_aura(victim, &amp, victim, None, [None; 3]);
            }
            if let Some(unit) = self.unit_base_mut(victim)
                && let Some(h) = unit.auras.iter_mut().find(|h| h.spell_id == 29_659)
            {
                // Aura::SetStackAmount(numStacks).
                h.stack = num_stacks;
            }
        } else {
            self.remove_auras_due_to_spell(victim, 29_659);
        }
    }

    // ===== spell_thaddius_negative_charge_aura (boss_thaddius.cpp:1131) spells: 28084 apply, 29660 amp

    // OnPeriodicTrigger seam, same as above (wired via
    // `aura_script_on_periodic_trigger`).
    /// `ThaddiusNegativeChargeAuraScript::OnPeriodicTrigger`
    /// (`boss_thaddius.cpp:1139-1171`): mirror of the positive charge
    /// with 28084 bearers and the 29660 amp aura.
    fn thaddius_negative_charge_aura_on_periodic_trigger(&mut self, victim: ObjectGuid) {
        // MAP_NAXXRAMAS = 533 (fork SharedDefines.h:1660).
        if self.map_id != 533 {
            return;
        }
        let others: Vec<ObjectGuid> = self.players.keys().copied().collect();
        let mut num_stacks = 0_u32;
        for guid in others {
            if guid == victim || !self.unit_is_alive(guid) {
                continue;
            }
            if !self.unit_has_aura(guid, 28_084) {
                continue;
            }
            let in_range = match (self.unit_position(victim), self.unit_position(guid)) {
                (Some(a), Some(b)) => {
                    let dx = a.x - b.x;
                    let dy = a.y - b.y;
                    dx * dx + dy * dy < 13.0 * 13.0
                }
                _ => false,
            };
            if in_range {
                num_stacks += 1;
            }
        }
        if num_stacks > 0 {
            if !self.unit_has_aura(victim, 29_660)
                && let Some(amp) = self.stores.spells.get(29_660).cloned()
            {
                self.apply_spell_aura(victim, &amp, victim, None, [None; 3]);
            }
            if let Some(unit) = self.unit_base_mut(victim)
                && let Some(h) = unit.auras.iter_mut().find(|h| h.spell_id == 29_660)
            {
                h.stack = num_stacks;
            }
        } else {
            self.remove_auras_due_to_spell(victim, 29_660);
        }
    }

    // ===== spell_grobbulus_mutating_injection (boss_grobbulus.cpp:227) spells: 28169 remove -> 28206 + 28240
    // (Fork calls this per effect aura; the seam iterates present
    // effects, so this runs once per present effect, as in the fork.)

    // ===== spell_grobbulus_cloud_poison (boss_grobbulus.cpp:256) spells: 28241 (radius from 28158 holder)
    // OnSetTargetMap seam (fork Spell targeting radius path,
    // `ScriptMgr.h:153`). Wired via `spell_script_on_set_target_map`
    // with the caster, the spell entry, the effect index and the
    // inout radius.

    // ===== spell_grobbulus_mutagen_explosion (boss_grobbulus.cpp:279) spells: 28206 (x1.5 expired / /1.5 dispelled)

    // ===== spell_grobbulus_mutating_injection (boss_grobbulus.cpp:227) spells: 28169 remove -> 28206 + 28240

    /// `GrobbulusMutatingInjectionScript::OnBeforeApply`
    /// (`boss_grobbulus.cpp:230-250`): remove path only. The bearer
    /// casts Poison Cloud 28240 on itself, triggered by the injection
    /// aura; the caster casts Mutagen Explosion 28206 on the bearer
    /// (see NEEDS-HOST below).
    fn grobbulus_mutating_injection_on_before_apply(
        &mut self,
        victim: ObjectGuid,
        holder: &mut AuraHolder,
        mode: AuraRemoveMode,
    ) {
        let spell_id = holder.spell_id;
        let caster = holder.caster;
        // Mutagen Explosion from the aura caster;
        // the dispelled cast carries no aura
        // context (the explosion script reads it
        // as its damage-scale gate).
        if self.unit_base(caster).is_some() {
            let by_dispel = mode == AuraRemoveMode::Dispel;
            self.cast_spell(
                caster,
                28_206,
                CastTarget::Unit(victim),
                CastOptions {
                    triggered: true,
                    triggered_by_aura: !by_dispel,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: if by_dispel { 0 } else { spell_id },
                    triggered_by_spell: if by_dispel { 0 } else { spell_id },
                },
            );
        }
        // Poison Cloud on the bearer, triggered by the aura.
        self.cast_spell(
            victim,
            28_240,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: spell_id,
                triggered_by_spell: 0,
            },
        );
    }

    // ===== spell_chains_of_kelthuzad (boss_kelthuzad.cpp:1042) spells: 28410

    /// `ChainsOfKelThuzadAuraScript::OnAfterApply`
    /// (`boss_kelthuzad.cpp:1044-1060`): effect 0 only; player
    /// victims only; on apply, charmed players may receive
    /// positive spells.
    fn chains_of_kelthuzad_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        apply: bool,
    ) {
        // `aura->GetEffIndex() != EFFECT_INDEX_0` return:
        // holder-level equivalent is effect-0 presence on the
        // holder (checked here through the unit, since the seam
        // passes no holder).
        let has_eff0 = self.unit_base(victim).is_some_and(|u| {
            u.auras
                .iter()
                .any(|h| h.spell_id == spell_id && h.effects[0].is_some())
        });
        if !has_eff0 {
            return;
        }
        // `target->ToPlayer()` null return: non-player victims skip.
        if !victim.is_player() {
            return;
        }
        if apply {
            // Enable positive spells for charmed players (allows healing/buffing while charmed).
            // UNMODELED: `player->m_AI->enablePositiveSpells` — the
            // Rust AI state has no charm/positive-spell override
            // field; thread one through the charm handling, then set
            // it here.
            let _ = victim;
        }
    }

    // ===== spell_gargoyle_stoneform (instance_naxxramas.cpp:2008) spells: 29153

    // ===== spell_gargoyle_stoneform (instance_naxxramas.cpp:2008) spells: 29153

    /// `GargoyleStoneformScript::OnBeforeApply`
    /// (`instance_naxxramas.cpp:2010-2023`): on apply the gargoyle
    /// takes the sniff stand state and becomes unselectable; on
    /// remove both revert. Idempotent, so the dispatcher's per-
    /// effect loop needs no eff gate.
    fn gargoyle_stoneform_on_before_apply(&mut self, victim: ObjectGuid, apply: bool) {
        // Using stand state 9 in sniff.
        const STONEFORM_STAND_STATE: u8 = 9;
        const STAND_STATE_STAND: u8 = 0;
        const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
        let Some(unit) = self.unit_base_mut(victim) else {
            return;
        };
        if apply {
            unit.values()
                .set_byte(offsets::UNIT_FIELD_BYTES_1, 0, STONEFORM_STAND_STATE);
            let flags = unit.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            unit.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, flags | UNIT_FLAG_NOT_SELECTABLE);
        } else {
            unit.values()
                .set_byte(offsets::UNIT_FIELD_BYTES_1, 0, STAND_STATE_STAND);
            let flags = unit.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            unit.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, flags & !UNIT_FLAG_NOT_SELECTABLE);
        }
    }

    // ===== spell_unrelenting_rider_shadow_bolt_volley (instance_naxxramas.cpp:2029) spells: 27831

    // ===== spell_five_fat_finger_exploding_heart_technique (blackrock_depths.cpp:2275) spells: 27673 (triggers 27676)

    // NEEDS-HOST: OnAuraInit seam: fork calls it from aura construction
    // (`Spells/SpellAuras.cpp:487`). Wire a dispatch at holder/aura creation that
    // stores per-holder state: the fork keeps `m_originalPosition` (the victim's
    // position at apply), which the periodic hooks below read.
    #[allow(dead_code, clippy::unused_self)] // NO-DISPATCHER-YET (step-4 remainder)
    fn five_fat_finger_exploding_heart_technique_on_aura_init(
        &mut self,
        victim: ObjectGuid,
        holder: &mut AuraHolder,
    ) {
        let _ = (victim, holder);
        // NEEDS-HOST state: victim position snapshot (`m_originalPosition`).
    }

    // ===== spell_five_fat_finger_exploding_heart_technique (blackrock_depths.cpp:2275) spells: 27673 (triggers 27676)

    // NEEDS-HOST: OnPeriodicTrigger seam: fork calls it from the periodic-trigger
    // pass (`Spells/SpellAuras.cpp:1499`) to swap the triggered spell. Wire a
    // dispatch that can replace the triggered spell id.
    #[allow(dead_code, clippy::unused_self)] // NO-DISPATCHER-YET (step-4 remainder)
    fn five_fat_finger_exploding_heart_technique_on_periodic_trigger(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        triggered_spell: &mut u32,
    ) {
        // Fork: below 5 stacks nothing happens; at 5 stacks, if the victim moved
        // >= 5.0 yards from the apply position ("5 steps 5 yards?"), the periodic
        // trigger becomes 27676 (Exploding Heart).
        if holder.stack < 5 {
            return;
        }
        let _ = victim;
        // NEEDS-HOST state: distance check against `m_originalPosition`.
        let _ = triggered_spell;
    }

    // ===== spell_five_fat_finger_exploding_heart_technique (blackrock_depths.cpp:2275) spells: 27673 (triggers 27676)

    // NEEDS-HOST: OnPeriodicTickEnd seam: fork calls it at the end of the periodic
    // tick (`Spells/SpellAuras.cpp:6257`). Wire a post-tick dispatch.
    #[allow(dead_code, clippy::unused_self)] // NO-DISPATCHER-YET (step-4 remainder)
    fn five_fat_finger_exploding_heart_technique_on_periodic_tick_end(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
    ) {
        // Fork: same gate as the trigger hook; at 5 stacks and >= 5.0 yards moved
        // the victim's own aura (`aura->GetId()`, i.e. 27673) is removed.
        if holder.stack < 5 {
            return;
        }
        let _ = victim;
        // NEEDS-HOST state: distance check against `m_originalPosition`, then a
        // remove-aura-by-spell-id runtime call.
    }

    // ===== spell_death_from_below_cannon_fire (arathi_highlands.cpp:438) spells: <id not stated in fork comments>

    // ===== spell_haunting_phantoms (stratholme.cpp:636) spells: 16336

    /// `HauntingPhantomsScript::OnPeriodicDummy` (`stratholme.cpp:644-657`):
    /// 5% chance every tick to summon a phantom on the victim, triggered:
    /// 16334 (Spiteful Phantom) or 16335 (Wrath Phantom) by coin flip.
    fn haunting_phantoms_on_periodic_dummy(&mut self, victim: ObjectGuid, aura_spell: u32) {
        if self.spell_rng().irand(0, 99) >= 5 {
            return;
        }
        let summon = if self.spell_rng().irand(0, 1) == 1 {
            16_334
        } else {
            16_335
        };
        self.cast_spell(
            victim,
            summon,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: aura_spell,
                triggered_by_spell: 0,
            },
        );
    }

    // ===== spell_golemaggs_trust (boss_golemagg.cpp:352) spells: 20556 (casts 20553)

    /// `GolemaggsTrustScript::OnPeriodicDummy` (`boss_golemagg.cpp:360-379`):
    /// while the caster (Golemagg) is alive and in combat, every Core Rager
    /// (entry 11672, `molten_core.h:71`) within 30.0 yards gets the
    /// Golemagg's Trust buff 20553, triggered by this aura — increased damage
    /// and 50% increased attack speed when tanked too close to Golemagg.
    fn golemaggs_trust_on_periodic_dummy(
        &mut self,
        _victim: ObjectGuid,
        caster: ObjectGuid,
        aura_spell: u32,
    ) {
        if !self.unit_is_alive(caster) || !self.ai_in_combat(caster) {
            return;
        }
        // UNMODELED: `GetCreatureListWithEntryInGrid(11672, 30.0f)` — the host
        // has no nearby-creature-by-entry query; each found rager would get:
        // `cast_spell(caster, 20553, Unit(rager), triggered + triggered_by_aura,
        // triggered_by_aura_spell = aura_spell)`.
        let _ = aura_spell;
    }
}
