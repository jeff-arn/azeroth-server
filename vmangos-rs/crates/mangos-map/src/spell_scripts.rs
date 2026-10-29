//! Fork `SpellScript` / `AuraScript` host (`ScriptMgr.h:138-215`,
//! Tier 3 Slice 2 step 4): dispatch by
//! `spell_template.script_name` at the cast-pipeline seams the
//! fork calls the virtuals from. Wired hooks: `OnCheckCast`
//! (end of `cast_spell` validation = `Spell::CheckCast` tail),
//! `OnEffectExecute` (per effect inside `apply_effects` =
//! `DoAllEffectOnTarget`; `false` skips the effect, the
//! `spell->damage` mutation rides the `script_damage` channel
//! into the school/weapon damage arms), `AuraScript::
//! OnBeforeApply`/`OnAfterApply` (holder-level in
//! `add_aura_holder`/`remove_aura_at`, fork `ApplyModifier`
//! order) and `OnPeriodicDummy` (in `periodic_dummy_tick`,
//! before the hardcoded family switch). Remaining hooks are
//! wired when a ported script first reaches them (recorded in
//! the evidence file).

use crate::map::Map;
use mangos_entities::aura::AuraHolder;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;
use mangos_spells::entry::SpellEntry;

use crate::spells::{CastOptions, CastTarget, PeriodicTick};

impl Map {
    /// `SpellScript::OnCheckCast` dispatch: `Some(reason)` fails
    /// the cast with that `SpellCastResult` byte.
    pub(crate) fn spell_script_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        match spell.script_name.as_str() {
            "spell_paladin_holy_shock" => self.holy_shock_on_check_cast(caster, target),
            "spell_warlock_conflagrate" => {
                self.warlock_conflagrate_on_check_cast(caster, spell, target)
            }
            "spell_warlock_life_tap" => self.warlock_life_tap_on_check_cast(caster, spell, target),
            "spell_warlock_create_healthstone" => {
                self.warlock_create_healthstone_on_check_cast(caster, spell, target)
            }
            "spell_warlock_ritual_of_summoning" => {
                self.warlock_ritual_of_summoning_on_check_cast(caster, spell, target)
            }
            "spell_hunter_mongoose_bite" => self.hunter_mongoose_bite_on_check_cast(caster, target),
            "spell_hunter_counterattack" => self.hunter_counterattack_on_check_cast(caster, target),
            "spell_druid_swiftmend" => self.druid_swiftmend_on_check_cast(caster, target),
            "spell_heavy_armor_kit" => self.heavy_armor_kit_on_check_cast(caster, target),
            "spell_reindeer_transformation" => {
                self.reindeer_transformation_on_check_cast(caster, target)
            }
            "spell_bag_of_gold" => self.bag_of_gold_on_check_cast(caster, target),
            "spell_kodo_kombobulator" => self.kodo_kombobulator_on_check_cast(caster, target),
            "spell_melodius_rapture" => self.melodius_rapture_on_check_cast(caster, target),
            "spell_purify_and_place_food" => {
                self.purify_and_place_food_on_check_cast(caster, target)
            }
            "spell_summon_black_qiraji_battle_tank" => {
                self.summon_black_qiraji_battle_tank_on_check_cast(caster, target)
            }
            _ => None,
        }
    }

    /// `SpellScript::OnEffectExecute` dispatch: `false` skips
    /// the effect. `script_damage` is the mutable
    /// `Spell::damage` equivalent for this victim's effect pass.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // fork hook context + arm table
    pub(crate) fn spell_script_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
    ) -> bool {
        let _ = triggered_by_spell;
        match spell.script_name.as_str() {
            "spell_paladin_holy_shock" => {
                self.holy_shock_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_paladin_hammer_of_wrath" => self.paladin_hammer_of_wrath_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_paladin_judgement_of_command_damage" => self
                .paladin_judgement_of_command_damage_on_effect_execute(
                    caster,
                    spell,
                    victim,
                    eff_index,
                    script_damage,
                ),
            "spell_paladin_judgement_of_command_dummy" => self
                .paladin_judgement_of_command_dummy_on_effect_execute(
                    caster, spell, victim, eff_index,
                ),
            "spell_paladin_judgement_of_light_proc_aura" => self
                .paladin_judgement_of_light_proc_aura_on_effect_execute(
                    caster, spell, victim, eff_index,
                ),
            "spell_paladin_judgement_of_light_heal" => self
                .paladin_judgement_of_light_heal_on_effect_execute(
                    caster, spell, victim, eff_index,
                ),
            "spell_paladin_reckoning" => {
                self.paladin_reckoning_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_warrior_bloodthirst" => self.warrior_bloodthirst_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_warrior_shield_slam" => self.warrior_shield_slam_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_warrior_execute_damage" => self.warrior_execute_damage_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_warrior_wrath" => self.warrior_wrath_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_warrior_bloodrage" => self.warrior_bloodrage_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_rogue_eviscerate" => self.rogue_eviscerate_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_rogue_vanish" => {
                self.rogue_vanish_on_effect_execute(caster, spell, victim, eff_index, script_damage)
            }
            "spell_warlock_demonic_sacrifice" => {
                self.warlock_demonic_sacrifice_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_warlock_conflagrate" => {
                self.warlock_conflagrate_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_warlock_life_tap" => {
                self.warlock_life_tap_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_warlock_create_healthstone" => {
                self.warlock_create_healthstone_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_hunter_readiness" => {
                self.hunter_readiness_on_effect_execute(caster, spell, eff_index)
            }
            "spell_hunter_refocus" => {
                self.hunter_refocus_on_effect_execute(caster, spell, eff_index)
            }
            "spell_mage_cold_snap" => {
                self.mage_cold_snap_on_effect_execute(caster, spell, eff_index)
            }
            "spell_priest_touch_of_weakness" => self.priest_touch_of_weakness_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                triggered_by_aura_spell,
            ),
            "spell_druid_ferocious_bite" => self.druid_ferocious_bite_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_druid_enrage" => {
                self.druid_enrage_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_druid_swiftmend" => self.druid_swiftmend_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_meteor" if eff_index == 0 => self.meteor_on_effect_execute(script_damage),
            "spell_darkmoon_steam_tonk_cannon" if eff_index == 0 => {
                self.darkmoon_steam_tonk_cannon_on_effect_execute(caster, victim)
            }
            "spell_showlabel_off" if eff_index == 0 => self.showlabel_off_on_effect_execute(caster),
            "spell_showlabel_on" if eff_index == 0 => self.showlabel_on_on_effect_execute(caster),
            "spell_gm_off" if eff_index == 0 => self.gm_off_on_effect_execute(caster),
            "spell_gm_on" if eff_index == 0 => self.gm_on_on_effect_execute(caster),
            "spell_invis_off" if eff_index == 0 => self.invis_off_on_effect_execute(caster),
            "spell_invis_on" if eff_index == 0 => self.invis_on_on_effect_execute(caster),
            "spell_bm_off" if eff_index == 0 => self.bm_off_on_effect_execute(caster),
            "spell_bm_on" if eff_index == 0 => self.bm_on_on_effect_execute(caster),
            "spell_clear_all_cooldowns" if eff_index == 0 => {
                self.clear_all_cooldowns_on_effect_execute(caster)
            }
            "spell_deviate_fish" => {
                self.deviate_fish_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_cooked_deviate_fish" => {
                self.cooked_deviate_fish_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_noggenfogger_elixir" => {
                self.noggenfogger_elixir_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_linkens_boomerang" => {
                self.linkens_boomerang_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_scorpid_surprise" => {
                self.scorpid_surprise_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_brittle_armor_dummy" => {
                self.brittle_armor_dummy_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_mercurial_shield_dummy" => {
                self.mercurial_shield_dummy_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_everlook_transporter" => {
                self.everlook_transporter_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_gdr_channel" => {
                self.gdr_channel_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_goblin_jumper_cables" => {
                self.goblin_jumper_cables_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_goblin_jumper_cables_xl" => {
                self.goblin_jumper_cables_xl_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_digging_claw" => {
                self.digging_claw_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_tanaris_field_sampling" => {
                self.tanaris_field_sampling_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_nefarian_corrupted_totems" => {
                self.nefarian_corrupted_totems_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_nefarian_shadow_flame_passive" => self
                .nefarian_shadow_flame_passive_on_effect_execute(caster, spell, victim, eff_index),
            "spell_nefarian_class_call_warlock" => {
                self.nefarian_class_call_warlock_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_nefarian_class_call_rogue" => {
                self.nefarian_class_call_rogue_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_thaddius_positive_charge" => self.thaddius_positive_charge_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_thaddius_negative_charge" => self.thaddius_negative_charge_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_thaddius_magnetic_pull" => {
                self.thaddius_magnetic_pull_on_effect_execute(caster, spell, victim, eff_index)
            }
            "spell_grobbulus_mutagen_explosion" => self
                .grobbulus_mutagen_explosion_on_effect_execute(
                    caster,
                    spell,
                    victim,
                    eff_index,
                    script_damage,
                    triggered_by_spell,
                ),
            "spell_maexxna_spider_web" if eff_index == 0 => {
                self.maexxna_spider_web_on_effect_execute(caster, victim)
            }
            "spell_kelthuzad_void_blast" => self.kelthuzad_void_blast_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_loatheb_corrupted_mind_aoe" if eff_index == 0 => {
                self.loatheb_corrupted_mind_aoe_on_effect_execute(caster, victim)
            }
            "spell_gluth_decimate" => self.gluth_decimate_on_effect_execute(
                caster,
                spell,
                victim,
                eff_index,
                script_damage,
            ),
            "spell_rajaxx_thundercrash" if eff_index == 0 => self
                .rajaxx_thundercrash_on_effect_execute(
                    caster,
                    spell,
                    victim,
                    eff_index,
                    script_damage,
                ),
            "spell_simone_seductress_chain_lightning" if eff_index == 0 => self
                .simone_seductress_chain_lightning_on_effect_execute(
                    caster,
                    spell,
                    victim,
                    eff_index,
                    script_damage,
                ),
            "spell_ubrs_freeze_rookery_egg" if eff_index == 0 => {
                self.ubrs_freeze_rookery_egg_on_effect_execute(caster, victim)
            }
            _ => true,
        }
    }

    /// `SpellScript::OnAfterHit` dispatch (fork
    /// `Spell.cpp` `DoAllEffectOnTarget`, after the
    /// effect pass for this unit target).
    pub(crate) fn spell_script_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
    ) {
        if spell.script_name == "spell_paladin_bubble" {
            self.paladin_bubble_on_after_hit(caster, spell, victim);
        }
        if spell.script_name == "spell_first_aid" {
            self.first_aid_on_after_hit(caster, victim, spell);
        }
    }

    /// `SpellScript::OnSetTargetMap` dispatch (fork
    /// `ScriptMgr.h:153`, called from
    /// `Spell::SetTargetMap` before the target
    /// switch): arms mutate mode/radius/max-targets/
    /// closest-selection for `collect_targets`.
    #[allow(clippy::too_many_arguments)] // fork hook signature
    pub(crate) fn spell_script_on_set_target_map(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        target_mode: &mut u32,
        radius: &mut f32,
        max_targets: &mut u32,
        select_closest: &mut bool,
    ) {
        match spell.script_name.as_str() {
            "spell_sapphiron_life_drain" => {
                *max_targets = self.sapphiron_life_drain_max_targets();
            }
            "spell_noth_curse_of_the_plaguebringer" => {
                *select_closest = Self::noth_curse_of_the_plaguebringer_select_closest_targets();
            }
            "spell_heigan_mana_burn" => {
                *radius = Self::heigan_mana_burn_radius();
            }
            "spell_faerlina_poison_bolt_volley" => {
                *max_targets = Self::faerlina_poison_bolt_volley_max_targets();
            }
            "spell_emperor_mutate_bug" => {
                self.emperor_mutate_bug_on_set_target_map(max_targets);
            }
            "spell_emperor_explode_bug" => {
                self.emperor_explode_bug_on_set_target_map(max_targets);
            }
            "spell_huhuran_wyvern_sting" => {
                self.huhuran_wyvern_sting_on_set_target_map(select_closest);
            }
            "spell_huhuran_poison_bolt_volley" => {
                self.huhuran_poison_bolt_volley_on_set_target_map(select_closest);
            }
            "spell_aq20_drain_mana" => {
                self.aq20_drain_mana_on_set_target_map(max_targets);
            }
            "spell_aq40_drain_mana" => {
                self.aq40_drain_mana_on_set_target_map(max_targets);
            }
            "spell_emerald_dragons_dream_fog" => {
                self.emerald_dragons_dream_fog_on_set_target_map(max_targets);
            }
            "spell_uldaman_awaken_vault_warder" => {
                self.uldaman_awaken_vault_warder_on_set_target_map(max_targets);
            }
            "spell_shazzrah_gate" => {
                self.shazzrah_gate_on_set_target_map(max_targets);
            }
            "spell_grobbulus_cloud_poison" => {
                self.grobbulus_cloud_poison_on_set_target_map(caster, spell, eff_index, radius);
            }
            _ => {
                let _ = (caster, eff_index, target_mode);
            }
        }
    }

    /// `SpellScript::OnCheckTarget` (Unit) dispatch
    /// (fork `ScriptMgr.h:157`, called during
    /// target-map fill): `false` drops the
    /// candidate. `main_target` is the cast's
    /// explicit unit target.
    pub(crate) fn spell_script_on_check_target(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        main_target: Option<ObjectGuid>,
        target: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = caster;
        match spell.script_name.as_str() {
            "spell_warrior_intimidating_shout" => {
                self.warrior_intimidating_shout_on_check_target(main_target, target, eff_index)
            }
            "spell_maexxna_web_spray" => self.maexxna_web_spray_on_check_target(target),
            "spell_unrelenting_rider_shadow_bolt_volley" => {
                self.unrelenting_rider_shadow_bolt_volley_on_check_target(target)
            }
            "spell_aq20_drain_mana" => self.aq20_drain_mana_on_check_target(target),
            "spell_aq40_drain_mana" => self.aq40_drain_mana_on_check_target(target),
            "spell_death_from_below_cannon_fire" => {
                self.death_from_below_cannon_fire_on_check_target(target)
            }
            _ => true,
        }
    }

    /// `Unit::IsFriendlyTo` subset (no faction/reaction system:
    /// the same player-vs-creature split the spell core uses in
    /// `is_hostile_pair`).
    pub(crate) fn script_is_friendly(a: ObjectGuid, b: ObjectGuid) -> bool {
        !Self::is_hostile_pair(a, b)
    }

    /// `WorldObject::IsFacingTarget` (`Object.cpp:1849`):
    /// under `NO_FACING_CHECKS_DISTANCE` (1.4 to center;
    /// positions here are centers) always facing, else the
    /// PI front arc.
    pub(crate) fn is_facing_target(&self, owner: ObjectGuid, target: ObjectGuid) -> bool {
        let (Some(a), Some(b)) = (self.unit_position(owner), self.unit_position(target)) else {
            return false;
        };
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        if dx * dx + dy * dy < 1.4 * 1.4 {
            return true;
        }
        self.has_in_arc(owner, target, std::f32::consts::PI)
    }

    /// Unit target of a cast (`SpellTargets::getUnitTarget`)
    /// for script hooks; non-unit cast targets have none.
    pub(crate) fn script_unit_target(target: CastTarget) -> Option<ObjectGuid> {
        match target {
            CastTarget::Unit(guid) => Some(guid),
            _ => None,
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
                self.grobbulus_mutating_injection_on_before_apply(victim, holder);
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
    /// it through the unit when needed. No arms yet;
    /// first user lands with its script lane.
    pub(crate) fn aura_script_on_after_apply(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        holder_caster: ObjectGuid,
        apply: bool,
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
                self.silithyst_on_after_apply(victim, spell_id, apply);
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
    /// No arms yet; first users land with the
    /// spell-script lanes.
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

    /// `PaladinHolyShockScript::OnCheckCast`
    /// (`spell_paladin.cpp:79-85`): hostile target must be in
    /// front of the caster.
    fn holy_shock_on_check_cast(&mut self, caster: ObjectGuid, target: CastTarget) -> Option<u8> {
        let unit_target = Self::script_unit_target(target)?;
        if !Self::script_is_friendly(caster, unit_target)
            && !self.is_facing_target(caster, unit_target)
        {
            return Some(mangos_spells::defines::cast_result::FAILED_UNIT_NOT_INFRONT);
        }
        None
    }

    /// `PaladinHolyShockScript::OnEffectExecute`
    /// (`spell_paladin.cpp:87-117`): triggered heal/hurt
    /// sub-spell by rank and friendliness; unknown rank logs in
    /// the fork and returns false.
    fn holy_shock_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        let (hurt, heal) = match spell.id {
            20_473 => (25_912, 25_914),
            20_929 => (25_911, 25_913),
            20_930 => (25_902, 25_903),
            _ => return false,
        };
        let sub = if Self::script_is_friendly(caster, victim) {
            heal
        } else {
            hurt
        };
        self.cast_spell(
            caster,
            sub,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_paladin_hammer_of_wrath (spell_paladin.cpp:20) spells: 24239, 24274, 24275

    /// `PaladinHammerOfWrathScript::OnEffectExecute`
    /// (`spell_paladin.cpp:22-31`): melee-crit attack-type override plus
    /// spell-damage bonuses on effect 0.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn paladin_hammer_of_wrath_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        let _ = (caster, spell, victim, script_damage);
        if eff_index != 0 {
            return true;
        }
        // `spell->m_attackType = BASE_ATTACK` (benefit from melee crit):
        // UNMODELED, see below. The Rust melee math keys off
        // `spell.dmg_class` (`spell_meleedamage`, `spells.rs:4401`) with no
        // per-cast attack-type override.
        // `SpellDamageBonusDone/Taken(..., SPELL_DIRECT_DAMAGE)`: subsumed by
        // the host — `school_damage` applies both bonuses itself
        // (`spells.rs:4443-4461`), so the arm performs no bonus math and
        // returns true to let the effect execute.
        true
    }

    // ===== spell_paladin_judgement_of_command_damage (spell_paladin.cpp:37) spells: 20467, 20963, 20964, 20965, 20966

    /// `PaladinJudgementOfCommandDamageScript::OnEffectExecute`
    /// (`spell_paladin.cpp:39-51`): base damage halved if the target is not
    /// stunned, then spell-damage bonuses, on effect 0.
    fn paladin_judgement_of_command_damage_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // Base damage halved if target not stunned. Fork tests
        // `UNIT_STATE_STUNNED | UNIT_STATE_PENDING_STUNNED` (fork
        // `UnitDefines.h:421`); this runtime has no pending-stun bit, so only
        // the applied-stun half is tested (`unit_base` + `has_unit_state`, the
        // same pattern content.rs lanes use).
        let stunned = self
            .unit_base(victim)
            .is_some_and(|u| u.has_unit_state(mangos_entities::unit::UNIT_STATE_STUNNED));
        if !stunned {
            // Fork halves the already-rolled `spell->damage`. The dispatcher
            // channel is additive (`value + script_damage`, `spells.rs:2441`),
            // so subtract half of the recomputed base. Caveat: `effect_value`
            // re-rolls the dice component (`calculate_effect_value_bp`), while
            // the fork halves the rolled value; exact when the effect has no
            // dice variance.
            let base = self.effect_value(spell, eff_index, self.unit_level(caster));
            *script_damage -= base * 0.5;
        }
        // `SpellDamageBonusDone/Taken(..., SPELL_DIRECT_DAMAGE)`: subsumed by
        // the host (`school_damage`, `spells.rs:4443-4461`).
        let _ = caster;
        true
    }

    // ===== spell_paladin_judgement_of_command_dummy (spell_paladin.cpp:57) spells: 20425, 20961, 20962, 20967, 20968

    /// `PaladinJudgementOfCommandDummyScript::OnEffectExecute`
    /// (`spell_paladin.cpp:59-71`): effect 0 base points carry the damage
    /// sub-spell id; triggered-cast it on the target, else fail the effect.
    fn paladin_judgement_of_command_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // `uint32 spellId = spell->m_currentBasePoints[effIdx]`: the payload
        // effect has no dice/level variance, so the deterministic
        // `simple_value` (base + base dice, `entry.rs`) equals the runtime
        // current base points without consuming the RNG stream.
        let sub_id = spell.simple_value(eff_index);
        // `sSpellMgr.GetSpellEntry(spellId)` null check: unknown id fails the
        // effect (`return false` skips it in the dispatcher).
        if sub_id <= 0 || self.stores.spells.get(sub_id as u32).is_none() {
            return false;
        }
        // `m_caster->CastSpell(target, entry, true, nullptr)`: triggered cast,
        // mirroring the holy-shock exemplar options.
        self.cast_spell(
            caster,
            sub_id as u32,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_paladin_holy_shock (spell_paladin.cpp:78) spells: 20473, 20929, 20930 — SKIPPED (already ported in spell_scripts.rs)

    // ===== spell_paladin_judgement_of_light_proc_aura (spell_paladin.cpp:125) spells: 20185, 20344, 20345, 20346, 25751

    /// `PaladinJudgementOfLightProcAuraScript::OnEffectExecute`
    /// (`spell_paladin.cpp:127-136`): Paladin T3 Judgement of Light — effect 0
    /// base points become 20 while the caster holds aura 28775.
    fn paladin_judgement_of_light_proc_aura_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index != 0 {
            return true;
        }
        // `spell->m_casterUnit`: every caster guid is a unit here.
        if self.bot_has_aura(caster, 28_775) {
            // `spell->m_currentBasePoints[effIdx] = 20`.
            // UNMODELED, see below: no base-point mutation channel reaches the
            // aura-apply arms (`script_damage` only feeds SCHOOL_DAMAGE /
            // WEAPON_DAMAGE, `spells.rs:2441,2476`).
        }
        true
    }

    // ===== spell_paladin_judgement_of_light_heal (spell_paladin.cpp:142) spells: 20267, 20341, 20342, 20343

    /// `PaladinJudgementOfLightHealScript::OnEffectExecute`
    /// (`spell_paladin.cpp:144-153`): effect 0 adds the extra heal stored in
    /// `m_triggeredByAuraBasePoints`.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn paladin_judgement_of_light_heal_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        if eff_index != 0 {
            return true;
        }
        // Extra heal stored in `m_triggeredByAuraBasePoints`.
        // UNMODELED, see below: `ActiveCast` carries only the triggering aura
        // spell id (`triggered_by_aura_spell`), not its base points, and the
        // `HEAL` arm (`spells.rs:2521`) ignores `script_damage`, so there is no
        // channel for the addition either.
        true
    }

    // ===== spell_paladin_reckoning (spell_paladin.cpp:159) spells: 20178

    /// `PaladinReckoningScript::OnEffectExecute` (`spell_paladin.cpp:161-183`):
    /// effect 0 grants an extra attack and skips the effect.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn paladin_reckoning_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        if eff_index != 0 {
            return true;
        }
        // World of Warcraft Client Patch 1.3.0 (2005-03-22)
        // - Fixed a bug where abilities that give extra attacks, like the
        //   paladin Reckoning talent, could cause the following swing to take
        //   longer than it should.
        // `#if SUPPORTED_CLIENT_BUILD <= CLIENT_BUILD_1_2_4` ResetAttackTimer
        // arm: compiled out for this port's 1.12.1 target.
        // It was possible to stack infinite extra attacks in early vanilla.
        // https://www.youtube.com/watch?v=TqPQ4SNmx2c
        // `#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_4_2` live arm:
        // `GetExtraAttacks() < 4` then `AddExtraAttack()`.
        // UNMODELED, see below: no extra-attack counter or attack-timer reset
        // in the runtime.
        // Fork returns false (skips the dummy effect) once the attack is
        // granted; transcribed as-is so the host skips the effect arm.
        false
    }

    // ===== spell_paladin_bubble (spell_paladin.cpp:196) spells: 642, 1020, 498, 5573, 1022, 5599, 10278
    // NEEDS-HOST: OnAfterHit seam: fork `Spells/Spell.cpp:1657` (`DoAllEffectOnTarget` unit case, after damage/heal is dealt and procs happen; `ScriptMgr.h:163`). The host wires OnCheckCast/OnEffectExecute/OnBeforeApply/OnAfterApply/OnPeriodicDummy only.

    /// `PaladinBubbleScript::OnAfterHit` (`spell_paladin.cpp:198-202`,
    /// `SPELL_FORBEARANCE = 25771`, fork line 190): the bubble target gains
    /// Forbearance via a triggered cast.
    fn paladin_bubble_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
    ) {
        let _ = spell;
        // `m_caster->CastSpell(GetUnitTarget(), SPELL_FORBEARANCE, true)`.
        self.cast_spell(
            caster,
            25_771,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_warrior_intimidating_shout (spell_warrior.cpp:20) spells: 5246
    // NEEDS-HOST: OnCheckTarget seam: `Unit::CheckTarget` / `Spell::AddUnitTarget`
    // (ScriptMgr.h:155-156). The host has no target-check dispatch yet; wire as
    // `spell_script_on_check_target(caster, spell, main_target, target, eff_index) -> bool`
    // where `main_target` is `spell->m_targets.getUnitTarget()`.

    /// `WarriorIntimidatingShoutScript::OnCheckTarget`
    /// (`spell_warrior.cpp:22-29`): the `AoE` fear does not apply to
    /// the spell main target (stunned by another aura).
    #[allow(clippy::unused_self)]
    fn warrior_intimidating_shout_on_check_target(
        &self,
        main_target: Option<ObjectGuid>,
        target: ObjectGuid,
        _eff_index: usize,
    ) -> bool {
        // Exception: Intimidating Shout — main target exempt.
        Some(target) != main_target
    }

    // ===== spell_warrior_bloodthirst (spell_warrior.cpp:35) spells: 23881, 23892, 23893, 23894

    /// `WarriorBloodthirstScript::OnEffectExecute`
    /// (`spell_warrior.cpp:37-47`): `damage = damage * attackPower / 100`.
    fn warrior_bloodthirst_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        let Some(u) = self.unit_base(caster) else {
            return true;
        };
        // `GetTotalAttackPowerValue(BASE_ATTACK)` reachable subset
        // (base + mods; exact aura-multiplier folding unmodeled, see below).
        let attack_power = u.base_attack_power + u.attack_power_mods[0] + u.attack_power_mods[1];
        // UNMODELED: `GetTotalAuraModifierByMiscMask(
        //   SPELL_AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
        //   target->GetCreatureTypeMask())` — the versus-aura type id is
        // unconfirmed in `mangos-spells` defines; add via
        // `total_aura_modifier(caster, ID, Some(self.creature_type_mask(victim)))`.
        let _ = victim;
        // Host channel is an additive delta over the
        // rolled effect value; fork multiplies the
        // rolled damage, so add base * AP/100 - base
        // (re-roll caveat as in the judgement arm;
        // Bloodthirst effect value is a flat percent).
        let base = self.effect_value(spell, eff_index, self.unit_level(caster));
        *script_damage += base * attack_power / 100.0 - base;
        true
    }

    // ===== spell_warrior_shield_slam (spell_warrior.cpp:53) spells: 23922, 23923, 23924, 23925

    /// `WarriorShieldSlamScript::OnEffectExecute`
    /// (`spell_warrior.cpp:55-62`): `damage += GetShieldBlockValue()`.
    fn warrior_shield_slam_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        _spell: &SpellEntry,
        _victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 1 {
            return true;
        }
        // UNMODELED: `unit_block_value` (ai.rs) is private at lane time;
        // expose a `pub(crate)` seam, then this call stands as written.
        let block = self.unit_block_value(caster);
        #[allow(clippy::cast_precision_loss)]
        {
            *script_damage += block as f32;
        }
        true
    }

    // ===== spell_warrior_execute_damage (spell_warrior.cpp:68) spells: 20647

    /// `WarriorExecuteDamageScript::OnEffectExecute`
    /// (`spell_warrior.cpp:70-77`): drains all rage on effect 0.
    fn warrior_execute_damage_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        _spell: &SpellEntry,
        _victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // UNMODELED: `unit_set_power` (spells.rs) is private at lane time;
        // expose a `pub(crate)` seam, then this call stands as written.
        self.unit_set_power(caster, mangos_spells::defines::power::RAGE, 0);
        true
    }

    // ===== spell_warrior_execute_dummy (spell_warrior.cpp:83) spells: 5308, 20658, 20660, 20661, 20662
    // NEEDS-HOST: OnCast seam: `Spell::cast` after all checks + reagents
    // (ScriptMgr.h:159). Wire as
    // `spell_script_on_cast(caster, spell, victim: Option<ObjectGuid>, current_base_points: [i32; 3])`.
    // fn warrior_execute_dummy_on_cast(
    //     &mut self,
    //     caster: ObjectGuid,
    //     spell: &SpellEntry,
    //     victim: ObjectGuid,
    //     current_base_points0: i32,
    // ) { ... } // body below

    /// `WarriorExecuteDummyScript::OnCast` (`spell_warrior.cpp:85-93`):
    /// forwards rage-scaled base points into 20647.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warrior_execute_dummy_on_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        current_base_points0: i32,
    ) {
        // UNMODELED: rage read — `unit_power` (spells.rs) is private at
        // lane time; expose a `pub(crate)` seam for
        // `GetPower(POWER_RAGE)`.
        let rage = self.unit_power(caster, mangos_spells::defines::power::RAGE);
        #[allow(clippy::cast_precision_loss)]
        let rage_f = rage as f32;
        let base_points0 = current_base_points0
            + mangos_spells::entry::dither(
                rage_f * spell.effects[0].dmg_multiplier,
                self.spell_rng(),
            );
        // m_casterUnit->SetPower(POWER_RAGE, 0); // Done in spell 20647
        self.cast_custom_spell(caster, victim, 20647, [Some(base_points0), None, None]);
    }

    // ===== spell_warrior_wrath (spell_warrior.cpp:99) spells: 21977 (Warrior's Wrath)

    /// `WarriorWrathScript::OnEffectExecute`
    /// (`spell_warrior.cpp:101-108`): triggered 21887 spell mod on
    /// effect 0.
    fn warrior_wrath_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        _spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        self.cast_spell(
            caster,
            21887,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_warrior_bloodrage (spell_warrior.cpp:114) spells: 2687

    /// `WarriorBloodrageScript::OnEffectExecute`
    /// (`spell_warrior.cpp:117-123`): puts the target in combat.
    /// Fork `SetInCombatState` also pulses the combat timer; the
    /// flag below is the unit-state subset the lane models.
    fn warrior_bloodrage_on_effect_execute(
        &mut self,
        _caster: ObjectGuid,
        _spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        self.set_in_combat_flag(victim, true);
        true
    }

    // ===== spell_warrior_sweeping_strikes (spell_warrior.cpp:129) spells: 12292
    // NEEDS-HOST: OnHolderInit seam: `SpellAuraHolder` creation, caster may be null
    // (ScriptMgr.h:178). Wire at holder insert before shapeshift-strip checks.
    // fn warrior_sweeping_strikes_on_holder_init(&mut self, holder: &mut AuraHolder) { ... }

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

    /// `ShamanFlametongueProcDummyScript::OnEffectExecute`
    /// (`spell_shaman.cpp:22-41`): weapon-speed-scaled 10444 proc.
    /// Fork quirk comments kept: damage coefficients 0.381% per 0.1
    /// speed / 15.244 per 4.0 speed observed; 3.85 factor used (at most
    /// one point off published values).
    /// `weapon_delay_ms` = `m_CastItem->GetProto()->Delay`;
    /// `spell_damage` = `SpellBaseDamageBonusDone(GetSpellSchoolMask())`.
    /// Missing cast item logs `Spell::EffectDummy: spell %i requires
    /// cast Item` and returns false — the caller (a future cast-item
    /// threading) owns that gate.
    #[allow(dead_code, clippy::too_many_arguments, clippy::cast_precision_loss)] // NO-DISPATCHER-YET (step-4 remainder)
    fn shaman_flametongue_proc_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
        weapon_delay_ms: u32,
        spell_damage: i32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // UNMODELED: `m_CastItem` (item `Delay`, missing-item `false`,
        // cast-item identity on the 10444 cast) is not threaded through
        // the effect dispatcher (`CastItemCtx` carries no delay); the
        // params above are the seam until it is.
        // UNMODELED: `spell_base_damage_bonus_done` (spells.rs) is
        // private at lane time; `spell.school_mask` is the school-mask
        // input (`GetSpellSchoolMask`).
        let _ = 1u32 << spell.school; // GetSpellSchoolMask seam
        let weapon_speed = weapon_delay_ms as f32 / 1_000.0;
        let total = (*script_damage + 3.85 * spell_damage as f32) * 0.01 * weapon_speed;
        let bp = mangos_spells::entry::dither(total, self.spell_rng());
        self.cast_custom_spell(caster, victim, 10444, [Some(bp), None, None]);
        true
    }

    // ===== spell_shaman_mana_tide (spell_shaman.cpp:47) spells: 16191
    // NEEDS-HOST: OnPeriodicTrigger seam: periodic-trigger tick before the
    // default `EffectTriggerSpell` cast (ScriptMgr.h:207). Wire as
    // `aura_script_on_periodic_trigger(target, trigger_spell_id, modifier_amount, aura_spell_id) -> bool`
    // where `true` suppresses the default cast (`spellInfo = nullptr`).
    //     suppress = self.shaman_mana_tide_on_periodic_trigger(tick.target, trigger_spell_id, modifier_amount, tick.spell_id);

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

    /// `RogueEviscerateScript::OnEffectExecute`
    /// (`spell_rogue.cpp:22-40`): AP-scaled bonus per combo point.
    /// Fork gate `#if SUPPORTED_CLIENT_BUILD > 1_11_2` (patch 1.12.0
    /// note: Eviscerate scales with AP; rank 9 manual off Blackhand
    /// Assassins) is ACTIVE for this 1.12.1 target — transcribed
    /// unconditionally.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn rogue_eviscerate_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        _spell: &SpellEntry,
        _victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // UNMODELED: combo points — the lane models no combo state
        // (`spells.rs`: "no combos"); needs a `GetComboPoints` seam.
        // UNMODELED: `IsFitToFamilyMask<CF_ROGUE_EVISCERATE>` family gate
        // plus `damage += GetTotalAttackPowerValue(BASE_ATTACK) * combo
        // * 0.03f` — needs the family-mask check and the AP seam from
        // the bloodthirst arm.
        let _ = caster;
        true
    }

    // ===== spell_rogue_vanish (spell_rogue.cpp:46) spells: 1856, 1857, 27617

    /// `RogueVanishScript::OnEffectExecute` (`spell_rogue.cpp:48-68`):
    /// strips root/snare (plus stalked at 1.12), re-stealths, and
    /// returns `false` so the default effect is skipped.
    /// Fork 1.12.0 note: Vanish now also removes Hunter's Mark / Mind
    /// Vision awareness (`MOD_STALKED`); gate `#if > 1_11_2` ACTIVE.
    fn rogue_vanish_on_effect_execute(
        &mut self,
        _caster: ObjectGuid,
        _spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        if eff_index != 1 {
            return true;
        }
        self.remove_spells_causing_aura(victim, mangos_spells::defines::aura::MOD_ROOT);
        self.remove_spells_causing_aura(victim, mangos_spells::defines::aura::MOD_DECREASE_SPEED);
        // UNMODELED: `RemoveSpellsCausingAura(MOD_STALKED)` — no
        // `MOD_STALKED` id in `mangos-spells` defines; confirm the id,
        // then add the third removal here.
        // UNMODELED: `Player::CastHighestStealthRank()` — no stealth-rank
        // seam; players only (`ToPlayer` gate in the fork).
        // `false`: skip the default effect handling.
        false
    }

    // ===== spell_warlock_demonic_sacrifice (spell_warlock.cpp:20) spells: 18788
    // (no override in fork; no arm)

    /// `WarlockDemonicSacrificeScript::OnEffectExecute` (`spell_warlock.cpp:22-50`):
    /// rank-1 pet entry picks the sacrifice buff the caster gains.
    fn warlock_demonic_sacrifice_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index == 0 {
            // `spell->GetUnitTarget()->GetEntry()`; players have no entry and
            // fall into the default arm, as in the fork's switch.
            let entry = self.creatures.get(&victim).map(|c| c.info.entry);
            let buff = match entry {
                Some(416) => 18_789,  // imp
                Some(417) => 18_792,  // fellhunter
                Some(1860) => 18_790, // voidwalker ("void")
                Some(1863) => 18_791, // succubus
                _ => {
                    // `sLog.Out(LOG_BASIC, LOG_LVL_ERROR, ...)`; no logger at
                    // this seam, effect still counts as executed.
                    return true;
                }
            };
            self.cast_spell(
                caster,
                buff,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

    // ===== spell_warlock_conflagrate (spell_warlock.cpp:56) spells: 17962, 18930, 18931, 18932

    /// Shared Immolate-from-caster scan (`spell_warlock.cpp:65-74,87-96`):
    /// victim holder with a `SPELL_AURA_PERIODIC_DAMAGE` effect whose spell
    /// `IsFitToFamily<SPELLFAMILY_WARLOCK, CF_WARLOCK_IMMOLATE>()`
    /// (`SPELLFAMILY_WARLOCK=5`, `DBCEnums.h:199`; `CF_WARLOCK_IMMOLATE` bit 2 =
    /// mask `0x4`, `SpellClassMask.h:97`) and whose caster is `caster`.
    /// Returns the holder's spell id (the `RemoveAurasByCasterSpell` key).
    fn warlock_conflagrate_immolate_id(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> Option<u32> {
        let victim_unit = self.unit_base(victim)?;
        for holder in &victim_unit.auras {
            if holder.caster != caster {
                continue;
            }
            let aura_spell = self.stores.spells.get(holder.spell_id)?;
            if aura_spell.spell_family_name != 5 || (aura_spell.spell_family_flags & 0x4) == 0 {
                continue;
            }
            let has_periodic_damage = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_DAMAGE);
            if has_periodic_damage {
                return Some(holder.spell_id);
            }
        }
        None
    }

    /// `WarlockConflagrateScript::OnCheckCast` (`spell_warlock.cpp:58-80`):
    /// requires the caster's own Immolate on the target.
    fn warlock_conflagrate_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = spell;
        let Some(unit_target) = Self::script_unit_target(target) else {
            return Some(mangos_spells::defines::cast_result::FAILED_BAD_IMPLICIT_TARGETS);
        };
        // for caster applied auras only
        if self
            .warlock_conflagrate_immolate_id(caster, unit_target)
            .is_none()
        {
            // Host cast-result constant (5875 table).
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        }
        None
    }

    /// `WarlockConflagrateScript::OnEffectExecute` (`spell_warlock.cpp:82-99`):
    /// consumes the caster's Immolate on the target.
    fn warlock_conflagrate_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index == 0
            && let Some(immolate_id) = self.warlock_conflagrate_immolate_id(caster, victim)
        {
            // for caster applied auras only
            self.remove_auras_by_caster_spell(victim, immolate_id, caster);
        }
        true
    }

    // ===== spell_warlock_life_tap (spell_warlock.cpp:105) spells: 1454, 1455, 1456, 11687, 11688, 11689

    /// `WarlockLifeTapScript::OnCheckCast` (`spell_warlock.cpp:107-122`):
    /// fizzles when the health cost would kill the caster.
    fn warlock_life_tap_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // UNMODELED: `ApplySpellMod(SPELLMOD_COST)` (no spell-mod owner access in
        // lane scope) and `SpellDamageBonusDone/Taken` (identity in this slice,
        // `mangos-spells/src/entry.rs`), so cost == base points.
        #[allow(clippy::cast_precision_loss)]
        let cost = spell.effects[0].base_points as f32;
        let health = self
            .unit_base(caster)
            .map(mangos_entities::unit::UnitBase::health)?;
        // use ceil as dithering might round up later.
        if i64::from(health) <= cost.ceil() as i64 {
            // Host cast-result constant (5875 table).
            return Some(mangos_spells::defines::cast_result::FAILED_FIZZLE);
        }
        None
    }

    /// `WarlockLifeTapScript::OnEffectExecute` (`spell_warlock.cpp:124-157`):
    /// converts health into the 31818 mana energize, scaled by Improved Life Tap.
    fn warlock_life_tap_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = victim;
        if eff_index != 0 {
            return true;
        }
        // UNMODELED: `CalculateSpellEffectValue` level scaling,
        // `ApplySpellMod(SPELLMOD_COST)`, `SpellDamageBonusDone/Taken`.
        #[allow(clippy::cast_precision_loss)]
        let dmg = spell.effects[0].base_points as f32;
        let idmg = mangos_spells::dither(dmg, self.spell_rng());
        let Some(health) = self
            .unit_base(caster)
            .map(mangos_entities::unit::UnitBase::health)
        else {
            return true;
        };
        if i64::from(health) > i64::from(idmg) {
            // Shouldn't Appear in Combat Log: direct field write, not damage.
            if let Some(unit) = self.unit_base_mut(caster) {
                unit.set_health(health.saturating_sub(idmg as u32));
            }
            let mut mana = idmg;
            // only Imp. Life Tap have this in combination with dummy aura:
            // family warlock + icon 208 (`spell_warlock.cpp:147`).
            if let Some(unit) = self.unit_base(caster) {
                for holder in &unit.auras {
                    let Some(aura_spell) = self.stores.spells.get(holder.spell_id) else {
                        continue;
                    };
                    if aura_spell.spell_family_name == 5 && aura_spell.spell_icon_id == 208 {
                        for effect in holder.effects.iter().flatten() {
                            if effect.aura_type == mangos_spells::defines::aura::DUMMY {
                                mana = (effect.amount + 100) * mana / 100;
                            }
                        }
                    }
                }
            }
            self.cast_spell(
                caster,
                31_818,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    custom_base_points: [Some(mana), None, None],
                    ..CastOptions::default()
                },
            );
        } else {
            // UNMODELED: `spell->SendCastResult(SPELL_FAILED_FIZZLE)`;
            // `send_cast_result` is private to `spells.rs`, no lane seam.
        }
        true
    }

    // ===== spell_warlock_curse_of_agony_dummy (spell_warlock.cpp:163) spells: 18280

    // NEEDS-HOST: OnEffectExecute seam: effect dispatch (`Spell::DoAllEffectOnTarget`
    // path) must carry the triggering aura's spell + base points for dummy 18280.
    // Fork body is also `#if SUPPORTED_CLIENT_BUILD <= CLIENT_BUILD_1_10_2`
    // (5302); at the default 1.12.1 build (5875) it compiles out entirely.
    /// `WarlockCurseOfAgonyDummyScript::OnEffectExecute`
    /// (`spell_warlock.cpp:165-184`): re-applies the Agony tick damage as 18277
    /// with the aura caster's bonuses.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warlock_curse_of_agony_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        trigger_spell: &SpellEntry,
        trigger_base_points: i32,
    ) -> bool {
        // UNMODELED at this seam: `SpellDamageBonusDone` (identity in this slice).
        let damage_point = trigger_base_points;
        // `GetSpellAuraHolder(proto->Id)` caster, else the spell caster.
        let aura_caster = self
            .unit_base(victim)
            .and_then(|u| {
                u.auras
                    .iter()
                    .find(|h| h.spell_id == trigger_spell.id)
                    .map(|h| h.caster)
            })
            .unwrap_or(caster);
        self.cast_spell(
            aura_caster,
            18_277,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                custom_base_points: [Some(damage_point), None, None],
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_warlock_devour_magic (spell_warlock.cpp:190) spells: 19505, 19731, 19734, 19736
    // (OnSuccessfulDispel has no dispatcher; no arm yet.)

    // NEEDS-HOST: OnSuccessfulDispel seam: fork `Spell::EffectDispel` success path
    // (`spell->m_casterUnit` + `m_spellInfo->Id` in scope).
    /// `WarlockDevourMagicScript::OnSuccessfulDispel`
    /// (`spell_warlock.cpp:192-217`): a dispelled debuff heals the felhunter.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warlock_devour_magic_on_successful_dispel(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
    ) {
        if eff_index != 0 {
            return;
        }
        let heal_spell = match spell_id {
            19_505 => 19_658,
            19_731 => 19_732,
            19_734 => 19_733,
            19_736 => 19_735,
            // `sLog.Out(LOG_SCRIPTS, LOG_LVL_DEBUG, ...)`; no logger at this seam.
            _ => return,
        };
        self.cast_spell(
            caster,
            heal_spell,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_warlock_inferno (spell_warlock.cpp:223) spells: 1122, 24670
    // (OnSummon has no dispatcher; no arm yet.)

    // NEEDS-HOST: OnSummon(Creature) seam: fork summon-effect path hands the
    // freshly summoned creature (`spell->m_caster` in scope).
    /// `WarlockInfernoScript::OnSummon` (`spell_warlock.cpp:225-235`): binds the
    /// Infernal with a free Enslave plus its root and burn-in visuals.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warlock_inferno_on_summon(&mut self, caster: ObjectGuid, summon: ObjectGuid) {
        // Enslave demon effect, without mana cost and cooldown
        self.cast_spell(
            caster,
            20_882,
            CastTarget::Unit(summon),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        // Short root spell on infernal from sniffs
        self.cast_spell(
            summon,
            22_707,
            CastTarget::Unit(summon),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        // Inferno effect
        self.cast_spell(
            summon,
            22_703,
            CastTarget::Unit(summon),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_warlock_create_healthstone (spell_warlock.cpp:245) spells: 5699, 6201, 6202, 11729, 11730

    /// Improved Healthstone ranks (`spell_warlock.cpp:246-251`).
    const WARLOCK_IMPROVED_HEALTHSTONE_R1: u32 = 18_692;
    const WARLOCK_IMPROVED_HEALTHSTONE_R2: u32 = 18_693;

    /// `WarlockCreateHealthstoneScript::GetItemId` (`spell_warlock.cpp:253-304`):
    /// rank-0/1/2 stone item per create spell, via the Improved Healthstone dummy.
    #[allow(clippy::items_after_statements)] // fork table order
    fn warlock_create_healthstone_item_id(&self, caster: ObjectGuid, spell_id: u32) -> u32 {
        let mut rank = 0;
        if let Some(unit) = self.unit_base(caster) {
            for holder in &unit.auras {
                if holder.spell_id == Self::WARLOCK_IMPROVED_HEALTHSTONE_R1 {
                    rank = 1;
                    break;
                } else if holder.spell_id == Self::WARLOCK_IMPROVED_HEALTHSTONE_R2 {
                    rank = 2;
                    break;
                }
            }
        }
        // rows: Minor, Lesser, Healthstone, Greater, Major.
        const ITEMS: [[u32; 3]; 5] = [
            [5512, 19_004, 19_005], // Minor Healthstone
            [5511, 19_006, 19_007], // Lesser Healthstone
            [5509, 19_008, 19_009], // Healthstone
            [5510, 19_010, 19_011], // Greater Healthstone
            [9421, 19_012, 19_013], // Major Healthstone
        ];
        let row = match spell_id {
            6201 => 0,  // Minor Healthstone
            6202 => 1,  // Lesser Healthstone
            5699 => 2,  // Healthstone
            11729 => 3, // Greater Healthstone
            11730 => 4, // Major Healthstone
            // `sLog.Out(LOG_SCRIPTS, LOG_LVL_ERROR, ...)`; 0 = no item.
            _ => return 0,
        };
        ITEMS[row][rank]
    }

    /// `WarlockCreateHealthstoneScript::OnCheckCast` (`spell_warlock.cpp:306-325`).
    /// NOTE fork quirk (`spell_warlock.cpp:309-310`): `if (!pCaster) SPELL_CAST_OK;`
    /// is a discarded expression, not a return — a null caster falls through.
    #[allow(clippy::unused_self)] // UNMODELED gates
    fn warlock_create_healthstone_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, spell, target);
        // UNMODELED: player-only gate (`ToPlayer`), `CanStoreNewItem` bag-space
        // check, `SendEquipError` + `SPELL_FAILED_DONT_REPORT` (fork byte 27, no
        // host constant). Rank lookup itself is modeled in the helper above.
        None
    }

    /// `WarlockCreateHealthstoneScript::OnEffectExecute`
    /// (`spell_warlock.cpp:327-341`).
    fn warlock_create_healthstone_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = caster;
        if eff_index != 0 {
            return true;
        }
        let item_id = self.warlock_create_healthstone_item_id(victim, spell.id);
        if item_id == 0 {
            return true;
        }
        // UNMODELED: `spell->DoCreateItem(effIdx, itemId)`; the host
        // `create_item_effect` reads the item id from effect data and takes no
        // explicit-id override, so there is no seam for the computed `item_id`.
        let _ = item_id;
        true
    }

    // ===== spell_warlock_ritual_of_summoning (spell_warlock.cpp:348) spells: 698

    /// `WarlockRitualOfSummoningScript::OnCheckCast` (`spell_warlock.cpp:349-376`).
    #[allow(clippy::unused_self)] // UNMODELED gates
    fn warlock_ritual_of_summoning_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, spell, target);
        // UNMODELED (no player/raid/map seams in lane scope): caster must be a
        // player (`ToPlayer`, else fork byte 10 BAD_TARGETS); must have a
        // selection (`GetSelectionGuid`, else 10); selection must be a different
        // player in the same raid (`IsInSameRaidWith`, else 10); target in combat
        // (fork byte 122, no host constant); dungeon maps require the same map
        // (fork byte 141, no host constant); battleground caster fails (fork byte
        // 61 NOT_HERE, no host constant).
        None
    }

    // ===== spell_warlock_curse_of_idiocy (spell_warlock.cpp:382) spells: 1010
    // (AuraScript::OnPeriodicTrigger has no dispatcher; no arm yet.)

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

    // ===== spell_hunter_readiness (spell_hunter.cpp:56) spells: 23989
    // Fork: `OnEffectExecute` eff 0, caster must be a player: clears cooldowns of
    // all `SPELLFAMILY_HUNTER` spells with `GetRecoveryTime() > 0` except 23989.
    // (in `spell_script_on_effect_execute`; needs the `if spell.script_name ==`
    // chain; `caster`, `spell`, `victim`, `eff_index`, `script_damage` in scope):
    // if spell.script_name == "spell_hunter_readiness" {
    //     return self.hunter_readiness_on_effect_execute(caster, spell, eff_index);
    /// `HunterReadinessScript::OnEffectExecute` (`spell_hunter.cpp:58-70`).
    fn hunter_readiness_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index != 0 {
            return true;
        }
        if !caster.is_player() {
            return true;
        }
        // immediately finishes the cooldown for hunter abilities
        // NEEDS-HOST: `remove_some_cooldowns` is private to `customs.rs:1571`;
        // make it `pub(crate)` for lane use (same call shape as the
        // Preparation arm at `customs.rs:263`).
        self.remove_some_cooldowns(caster, |entry| {
            entry.spell_family_name == 9 // SPELLFAMILY_HUNTER (DBCEnums.h:201)
                && entry.id != 23989
                && entry.recovery_time.max(entry.category_recovery_time) > 0
        });
        true
    }

    // ===== spell_hunter_refocus (spell_hunter.cpp:75) spells: 24531
    // Fork: `OnEffectExecute` eff 0, caster must be a player: clears cooldowns of
    // hunter spells fitting family `CF_HUNTER_ARCANE_SHOT, CF_HUNTER_MULTI_SHOT,
    // CF_HUNTER_VOLLEY, CF_HUNTER_AIMED_SHOT` except the refocus spell itself.
    // if spell.script_name == "spell_hunter_refocus" {
    //     return self.hunter_refocus_on_effect_execute(caster, spell, eff_index);
    /// `HunterRefocusScript::OnEffectExecute` (`spell_hunter.cpp:78-91`).
    fn hunter_refocus_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        if !caster.is_player() {
            return true;
        }
        // Refocus: "Instantly clears the cooldowns of Aimed Shot, Multishot,
        // Volley, and Arcane Shot."
        // Family mask union from `SpellClassMask.h`: CF_HUNTER_ARCANE_SHOT
        // bit 11 (0x800) | CF_HUNTER_MULTI_SHOT bit 12 (0x1000) |
        // CF_HUNTER_VOLLEY bit 13 (0x2000) | CF_HUNTER_AIMED_SHOT bit 17
        // (0x20000) = 0x23800.
        // NEEDS-HOST: `remove_some_cooldowns` visibility (see readiness).
        let self_spell = spell.id;
        self.remove_some_cooldowns(caster, |entry| {
            entry.spell_family_name == 9 // SPELLFAMILY_HUNTER
                && (entry.spell_family_flags & 0x23800) != 0
                && entry.id != self_spell
                && entry.recovery_time.max(entry.category_recovery_time) > 0
        });
        true
    }

    // ===== spell_hunter_mongoose_bite (spell_hunter.cpp:96) spells: 1495, 14269, 14270, 14271
    // Fork: `OnCheckCast`: fails with `SPELL_FAILED_BAD_TARGETS` unless the unit
    // target is the caster's current `REACTIVE_DEFENSE` target (dodge proc).
    // (in `spell_script_on_check_cast` match on `spell.script_name.as_str()`;
    // `caster`, `target` in scope):
    /// `HunterMongooseBiteScript::OnCheckCast` (`spell_hunter.cpp:99-104`).
    #[allow(clippy::unused_self)] // UNMODELED reactive-target gate
    fn hunter_mongoose_bite_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: `GetReactiveTarget(REACTIVE_DEFENSE)` — no reactive
        // target store in the lane yet, so the fork's BAD_TARGETS gate cannot
        // be evaluated; returning None (allow) until it exists.
        None
    }

    // ===== spell_hunter_counterattack (spell_hunter.cpp:109) spells: 19306, 20909, 20910
    // Fork: `OnCheckCast`: fails with `SPELL_FAILED_BAD_TARGETS` unless the unit
    // target is the caster's current `REACTIVE_HUNTER_PARRY` target (parry proc).
    /// `HunterCounterAttackScript::OnCheckCast` (`spell_hunter.cpp:112-116`).
    #[allow(clippy::unused_self)] // UNMODELED reactive-target gate
    fn hunter_counterattack_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: `GetReactiveTarget(REACTIVE_HUNTER_PARRY)` — no reactive
        // target store in the lane yet; returning None (allow) until it exists.
        None
    }

    // ===== spell_hunter_frost_trap_aura (spell_hunter.cpp:122) spells: 13810
    // Fork: `OnPeriodicTrigger`: the trap's caster procs off the entering target
    // (talent `Entrapment` root chance lives in the proc system):
    // `ProcDamageAndSpell(target, PROC_FLAG_ON_TRAP_ACTIVATION, PROC_FLAG_NONE,
    // PROC_EX_NORMAL_HIT, 1, 1, BASE_ATTACK, aura->GetSpellProto())`.
    // NEEDS-HOST: OnPeriodicTrigger seam: no dispatcher wires it yet; fork calls it
    // from `Aura::PeriodicTrigger` (`SpellAuras.cpp:1499`).
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

    // ===== spell_mage_cold_snap (spell_mage.cpp:21) spells: 12472
    // Fork: `OnEffectExecute` eff 0, caster must be a player: clears cooldowns of
    // all `SPELLFAMILY_MAGE` spells whose school mask includes FROST and whose
    // `GetRecoveryTime() > 0`.
    // if spell.script_name == "spell_mage_cold_snap" {
    //     return self.mage_cold_snap_on_effect_execute(caster, spell, eff_index);
    /// `MageColdSnapScript::OnEffectExecute` (`spell_mage.cpp:23-42`).
    fn mage_cold_snap_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index != 0 {
            return true;
        }
        if !caster.is_player() {
            return false;
        }
        // immediately finishes the cooldown on Frost spells
        // NEEDS-HOST: `remove_some_cooldowns` visibility (see readiness).
        self.remove_some_cooldowns(caster, |entry| {
            entry.spell_family_name == 3 // SPELLFAMILY_MAGE
                && (mangos_spells::defines::school::mask(entry.school)
                    & mangos_spells::defines::school::mask(mangos_spells::defines::school::FROST))
                    != 0
                && entry.recovery_time.max(entry.category_recovery_time) > 0
        });
        true
    }

    // ===== spell_mage_ignite (spell_mage.cpp:48) spells: 11119, 11120, 12846, 12847, 12848
    // Fork: `OnProc` on the ignite talent aura. Rank from the triggering talent
    // aura: 11119 -> 4%, 11120 -> 8%, 12846 -> 12%, 12847 -> 16%, 12848 -> 20% of
    // `originalAmount` as ignite dot base points (spell 12654). If the victim
    // already has ignite (12654 eff 0): below 5 stacks and before its last tick,
    // add the new damage to the tick, stack +1 and refresh; at 5 stacks just
    // refresh; past its last tick, remove and reapply fresh. Unknown talent id
    // logs in the fork and fails the proc.
    // NEEDS-HOST: OnProc seam: no dispatcher wires `AuraScript::OnProc` yet;
    // fork calls it from the proc handler (`UnitAuraProcHandler.cpp`,
    // `SpellAuraProcResult` protocol).
    /// `MageIgniteScript::OnProc` (`spell_mage.cpp:60-125`).
    /// Returns `Some(true)` = `PROC_OK`, `Some(false)` = `PROC_FAILED`,
    /// `None` = no handling (mirrors `optional<SpellAuraProcResult>`).
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code, clippy::items_after_statements, clippy::cast_precision_loss)] // NO-DISPATCHER-YET (step-4 remainder)
    fn mage_ignite_on_proc(
        &mut self,
        owner: ObjectGuid,
        victim: ObjectGuid,
        original_amount: i32,
        triggered_by_aura_spell_id: u32,
        proc_charges_cooldown: u32,
    ) -> Option<bool> {
        const IGNITE_DOT: u32 = 12654;
        let _ = proc_charges_cooldown;
        let pct = match triggered_by_aura_spell_id {
            11119 => 0.04,
            11120 => 0.08,
            12846 => 0.12,
            12847 => 0.16,
            12848 => 0.20,
            _ => return Some(false), // fork logs "non handled spell id"
        };
        let base_points = (pct * original_amount as f32) as i32;
        // Snapshot the victim's current ignite (spell 12654 eff 0): stack
        // count, tick damage (`m_modifier.m_amount`) and tick progress, so the
        // mutation below runs without holding the unit borrow.
        struct IgniteState {
            stack: u32,
            tick_damage: i32,
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
                    tick_damage: state.amount,
                    ticks_done: state.periodic_ticks,
                });
                break;
            }
        }
        if let Some(dot) = current {
            // UNMODELED: stack-1 tick-damage accumulate + holder refresh need
            // an aura-mutation seam (`ApplyModifier` re-run, `ModStackAmount`,
            // `Refresh`, tick-count reapply gate) the lane does not wire yet.
            // Fork-exact remainder: if ticks exhausted, remove 12654 and fall
            // through to the fresh apply; else if stack < 5 add base_points to
            // the tick and stack +1 (at 5 stacks just refresh), then PROC_OK.
            let _ = dot;
            return None;
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
        Some(true)
    }

    // ===== spell_mage_combustion_proc (spell_mage.cpp:139) spells: 11129
    // Fork (`#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_10_2`): `OnProc` on the
    // invisible combustion counter aura. No victim (aoe like flamestrike with no
    // target hit) -> fail. Counter dispelled/cancelled (no 28682 buff on owner)
    // -> remove the proc aura, fail. Last charge + crit -> remove the 28682 buff,
    // proc OK (charge counting removes the counter). Otherwise recast the 28682
    // buff triggered on self; proc OK only on crit (charge updates on crit hits).
    // NEEDS-HOST: OnProc seam (same as ignite above).
    /// `MageCombustionProcScript::OnProc` (`spell_mage.cpp:141-163`).
    /// `victim`: `None` when no target was affected. `last_charge` mirrors
    /// `triggeredByAura->GetHolder()->GetAuraCharges() <= 1` (the future
    /// `OnProc` dispatcher supplies it). Returns `Some(true)` = `PROC_OK`,
    /// `Some(false)` = `PROC_FAILED`, `None` = no handling.
    #[allow(dead_code, clippy::unnecessary_wraps)] // NO-DISPATCHER-YET (step-4 remainder)
    fn mage_combustion_proc_on_proc(
        &mut self,
        owner: ObjectGuid,
        victim: Option<ObjectGuid>,
        triggered_by_aura_spell_id: u32,
        proc_ex_crit: bool,
        last_charge: bool,
    ) -> Option<bool> {
        const COMBUSTION_PROC_AURA: u32 = 11129;
        const COMBUSTION_CRIT_BUFF: u32 = 28682;
        let _ = COMBUSTION_PROC_AURA;
        let Some(_victim) = victim else {
            // does not proc if no target is affected (aoe like flamestrike)
            return Some(false);
        };
        // combustion counter was dispelled or clicked off
        if !self.unit_has_aura_spell(owner, COMBUSTION_CRIT_BUFF) {
            self.remove_auras_due_to_spell(owner, triggered_by_aura_spell_id);
            return Some(false);
        }
        // last charge and crit
        if last_charge && proc_ex_crit {
            self.remove_auras_due_to_spell(owner, COMBUSTION_CRIT_BUFF);
            // remove Combustion auras; charge counting (will be removed)
            return Some(true);
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
        Some(proc_ex_crit)
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
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn mage_combustion_buff_on_after_apply(&mut self, victim: ObjectGuid) {
        // removing the invisible proc aura starts cooldown
        self.remove_auras_due_to_spell(victim, 11129);
    }

    // ===== spell_priest_touch_of_weakness (spell_priest.cpp:20) spells: 28598
    // Fork: `OnEffectExecute` eff 0, only when there is a unit target and the cast
    // was triggered by the Touch of Weakness aura (`m_triggeredByAuraSpell`):
    // rank map 2652 -> 2943, 19261 -> 19249, 19262 -> 19251, 19264 -> 19252,
    // 19265 -> 19253, 19266 -> 19254, cast triggered on the unit target.
    // Unknown aura id logs in the fork and returns false.
    // (`triggered_by_aura_spell` = the active cast's `m_triggeredByAuraSpell->Id`,
    // 0 = none; the dispatcher must thread it — see NEEDS-HOST note):
    // if spell.script_name == "spell_priest_touch_of_weakness" {
    //     return self.priest_touch_of_weakness_on_effect_execute(
    //         caster, spell, victim, eff_index, triggered_by_aura_spell,
    //     );
    // NEEDS-HOST: `m_triggeredByAuraSpell` context: the effect-execute dispatcher
    // has no trigger-origin id in scope; the cast pipeline must thread the active
    // cast's `triggered_by_aura_spell` (a `CastOptions` field on the way in) to
    // this arm. Seam: `Spell::EffectDummy` path that fires spell 28598.
    /// `PriestTouchOfWeaknessScript::OnEffectExecute` (`spell_priest.cpp:22-54`).
    fn priest_touch_of_weakness_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        triggered_by_aura_spell: u32,
    ) -> bool {
        let _ = spell;
        if eff_index != 0 || triggered_by_aura_spell == 0 {
            return true;
        }
        let sub = match triggered_by_aura_spell {
            2652 => 2943,      // Rank 1
            19261 => 19249,    // Rank 2
            19262 => 19251,    // Rank 3
            19264 => 19252,    // Rank 4
            19265 => 19253,    // Rank 5
            19266 => 19254,    // Rank 6
            _ => return false, // fork logs "triggered by unhandeled spell"
        };
        self.cast_spell(
            caster,
            sub,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: true,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 28598,
                triggered_by_spell: 0,
            },
        );
        true
    }

    // ===== spell_priest_power_word_shield (spell_priest.cpp:60) spells: 17, 592, 600, 3747, 6065, 6066, 10898, 10899, 10900, 10901, 27607
    // Fork: `OnHit`: when the hit lands (`missInfo == SPELL_MISS_NONE`) on a unit
    // target, the caster applies Weakened Soul (6788) triggered on that target.
    // NEEDS-HOST: OnHit seam: no dispatcher wires `SpellScript::OnHit` yet; fork
    // calls it from `Spell::DoAllEffectOnTarget` (`Spell.cpp:1489,1558,1584` for
    // the unit case, `:1956,1970` for gameobject targets).
    /// `PriestPowerWordShieldScript::OnHit` (`spell_priest.cpp:67-73`).
    /// `miss_none` mirrors `missInfo == SPELL_MISS_NONE`.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn priest_power_word_shield_on_hit(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        miss_none: bool,
    ) {
        const WEAKENED_SOUL: u32 = 6788;
        if !miss_none {
            return;
        }
        self.cast_spell(
            caster,
            WEAKENED_SOUL,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
    }

    // ===== spell_priest_holy_nova (spell_priest.cpp:79) spells: 15237, 15430, 15431, 27799, 27800, 27801
    // Fork: `OnSuccessfulFinish`: fires the rank-matched heal sub-spell on the
    // original spell targets: 15237 -> 23455, 15430 -> 23458, 15431 -> 23459,
    // 27799 -> 27803, 27800 -> 27804, 27801 -> 27805. Unknown id logs and
    // returns; the fork builds a raw `new Spell(caster, entry, triggered,
    // originalCasterGuid)` and `prepare`s it with the original targets.
    // NEEDS-HOST: OnSuccessfulFinish seam: no dispatcher wires
    // `SpellScript::OnSuccessfulFinish` yet; fork calls it from `Spell::finish`
    // (`Spell.cpp:4519`).
    /// `PriestHolyNovaScript::OnSuccessfulFinish` (`spell_priest.cpp:81-118`).
    /// `caster` = `spell->m_casterUnit`; the heal must reuse the original
    /// targets (see UNMODELED note).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn priest_holy_nova_on_successful_finish(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        victim: ObjectGuid,
    ) {
        let sub = match spell_id {
            15237 => 23455, // Holy Nova, rank 1
            15430 => 23458, // Holy Nova, rank 2
            15431 => 23459, // Holy Nova, rank 3
            27799 => 27803, // Holy Nova, rank 4
            27800 => 27804, // Holy Nova, rank 5
            27801 => 27805, // Holy Nova, rank 6
            _ => return,    // fork logs "assigned to unhandled spell id"
        };
        // UNMODELED: fork re-prepares a raw Spell on ALL original targets;
        // the lane can only emit a single-target triggered cast, so only the
        // primary victim is covered here; multi-target re-emit needs a cast
        // seam that takes an explicit target list.
        self.cast_spell(
            caster,
            sub,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: spell_id,
            },
        );
    }

    // ===== spell_druid_ferocious_bite (spell_druid.cpp:20) spells: 22568, 22827, 22828, 22829, 31018
    // Fork (1.12.1 path, `SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_11_2`): eff 0
    // with a unit target, caster must be a player. Damage rides `spell->damage`:
    // `damage += GetTotalAttackPowerValue(BASE_ATTACK) * combo * 0.03` when combo
    // points are up (1.12 "greater potency with attack power" note), then
    // `damage += GetPower(POWER_ENERGY) * m_spellInfo.DmgMultiplier[effIdx]`;
    // energy is then set to 0. The `#else` (pre-1.7.1) branch casting per-rank
    // damage spells 22851/22853/22861/22862 is NOT transcribed (dead fork for the
    // 1.12.1 client). `spell->damage` rides the `script_damage` channel.
    // if spell.script_name == "spell_druid_ferocious_bite" {
    //     return self.druid_ferocious_bite_on_effect_execute(
    //         caster, spell, victim, eff_index, script_damage,
    //     );
    /// `DruidFerociousBiteScript::OnEffectExecute` (`spell_druid.cpp:22-71`).
    #[allow(clippy::unused_self)] // UNMODELED combo-point body
    fn druid_ferocious_bite_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        _script_damage: &mut f32,
    ) -> bool {
        let _ = (caster, victim);
        if eff_index != 0 {
            return true;
        }
        // UNMODELED: combo-point count (no combo store in the lane) and
        // `POWER_ENERGY` read/reset (`unit_power` is private to `spells.rs`,
        // and there is no power setter). The attack-power read itself exists
        // (`unit_value_u32(caster, offsets::UNIT_FIELD_ATTACK_POWER)`, as in
        // `customs.rs:274`). Fork-exact math for the host seam once combo +
        // energy lands:
        //   if !caster.is_player() { return true; }
        //   if combo > 0 { *script_damage += ap * combo as f32 * 0.03; }
        //   *script_damage += energy as f32 * spell.effects[eff_index].dmg_multiplier;
        //   set energy to 0.
        let _ = spell;
        true
    }

    // ===== spell_druid_enrage (spell_druid.cpp:77) spells: 5229
    // Fork (1.12.1 path, `> CLIENT_BUILD_1_8_4`): `OnEffectExecute` eff 1 with a
    // unit target: armor-reduction mod is -16 in Dire Bear Form (target has aura
    // 9634) else -27; applied as a triggered custom cast of 25503 with that
    // single base point.
    // if spell.script_name == "spell_druid_enrage" {
    //     return self.druid_enrage_on_effect_execute(caster, spell, victim, eff_index);
    /// `DruidEnrageScript::OnEffectExecute` (`spell_druid.cpp:79-90`).
    fn druid_enrage_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index != 1 {
            return true;
        }
        // Reduce base armor by 27% in Bear Form and 16% in Dire Bear Form.
        // Dire Bear check = target has aura 9634, read off its holders.
        let reduction_mod: i32 = if self.unit_has_aura_spell(victim, 9634) {
            -16
        } else {
            -27
        };
        // Fork: `GetUnitTarget()->CastCustomSpell(GetUnitTarget(), 25503,
        // reductionMod, {}, {}, true)` — caster and target are the same unit.
        self.cast_custom_spell(victim, victim, 25503, [Some(reduction_mod), None, None]);
        let _ = caster;
        true
    }

    /// Holder scan backing the Dire Bear Form check above
    /// (fork `Unit::HasAura(9634)`; `unit_base` per `motion.rs:591`).
    fn unit_has_aura_spell(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.unit_base(guid)
            .is_some_and(|u| u.auras.iter().any(|h| h.spell_id == spell_id))
    }

    // ===== spell_druid_swiftmend (spell_druid.cpp:96) spells: 18562
    // Fork (`#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_10_2`): `OnCheckCast`
    // fails with `SPELL_FAILED_TARGET_AURASTATE` unless the unit target carries a
    // `SPELL_AURA_PERIODIC_HEAL` aura of `SPELLFAMILY_DRUID` with family mask
    // `0x50` (Rejuvenation / Regrowth). `OnEffectExecute` eff 0 with a unit
    // target: picks the shortest-duration Rejuvenation-or-Regrowth periodic-heal
    // aura on the target (family `CF_DRUID_REJUVENATION` / `CF_DRUID_REGROWTH`),
    // counts its ticks (Regrowth 6 ticks per "18 sec", Rejuvenation 4 per
    // "12 sec"), consumes the aura, and adds `tickheal * tickcount` to
    // `spell->damage` (rides the `script_damage` channel). Missing aura after a
    // passing check logs in the fork and returns false.
    // if spell.script_name == "spell_druid_swiftmend" {
    //     return self.druid_swiftmend_on_effect_execute(
    //         caster, spell, victim, eff_index, script_damage,
    //     );
    /// `DruidSwiftmendScript::OnCheckCast` (`spell_druid.cpp:98-106`).
    /// `target` is the raw `SpellTargets` unit target (may be absent).
    fn druid_swiftmend_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = caster;
        let CastTarget::Unit(victim) = target else {
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        };
        // Fork: `target->GetAura(SPELL_AURA_PERIODIC_HEAL, SPELLFAMILY_DRUID,
        // uint64(0x50))` — a periodic-heal effect on a druid-family aura whose
        // affect mask covers Rejuvenation (bit 4, 0x10) / Regrowth (bit 6,
        // 0x40); `SpellClassMask.h:172,174`.
        if self.unit_has_family_heal(victim) {
            None
        } else {
            Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE)
        }
    }

    /// Borrow-safe two-phase scan: collect candidate holder spell ids under a
    /// short shared borrow, then resolve family data from the stores.
    fn unit_has_family_heal(&self, victim: ObjectGuid) -> bool {
        let mut periodic_heal_holders: [u32; 48] = [0; 48];
        let mut count: usize = 0;
        if let Some(unit) = self.unit_base(victim) {
            for holder in &unit.auras {
                if holder
                    .effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_HEAL)
                    && count < periodic_heal_holders.len()
                {
                    periodic_heal_holders[count] = holder.spell_id;
                    count += 1;
                }
            }
        }
        periodic_heal_holders[..count].iter().any(|id| {
            self.stores.spells.get(*id).is_some_and(|entry| {
                entry.spell_family_name == 7 // SPELLFAMILY_DRUID
                    && (entry.spell_family_flags & 0x50) != 0
            })
        })
    }

    /// `DruidSwiftmendScript::OnEffectExecute` (`spell_druid.cpp:108-156`).
    #[allow(clippy::items_after_statements, clippy::cast_precision_loss)]
    fn druid_swiftmend_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        let _ = (caster, spell);
        if eff_index != 0 {
            return true;
        }
        // Find the shortest-duration Rejuvenation or Regrowth holder: snapshot
        // (spell id, remaining duration, family flags, periodic-heal tick
        // amount) under a short borrow, then act.
        struct Candidate {
            spell_id: u32,
            duration_ms: i32,
            family_flags: u64,
            tick_heal: i32,
        }
        let mut best: Option<Candidate> = None;
        if let Some(unit) = self.unit_base(victim) {
            for holder in &unit.auras {
                let Some(entry) = self.stores.spells.get(holder.spell_id) else {
                    continue;
                };
                // Regrowth or Rejuvenation
                if entry.spell_family_name != 7 || (entry.spell_family_flags & 0x50) == 0 {
                    continue;
                }
                let mut tick_heal: Option<i32> = None;
                for (idx, slot) in holder.effects.iter().enumerate() {
                    let Some(state) = slot else { continue };
                    if entry.effects[idx].apply_aura_name
                        == mangos_spells::defines::aura::PERIODIC_HEAL
                    {
                        tick_heal = Some(state.amount);
                        break;
                    }
                }
                let Some(tick_heal) = tick_heal else { continue };
                let replace = best
                    .as_ref()
                    .is_none_or(|b: &Candidate| holder.duration_ms < b.duration_ms);
                if replace {
                    best = Some(Candidate {
                        spell_id: holder.spell_id,
                        duration_ms: holder.duration_ms,
                        family_flags: entry.spell_family_flags,
                        tick_heal,
                    });
                }
            }
        }
        let Some(found) = best else {
            // Fork logs "has aurastate AURA_STATE_SWIFTMEND but no matching
            // aura" and returns false.
            return false;
        };
        let mut tickcount = 0;
        // Regrowth: "18 sec of Regrowth" -> 6 ticks (mask bit 6, 0x40).
        if found.family_flags & 0x40 != 0 {
            tickcount = 6;
        }
        // Rejuvenation: "12 sec of Rejuvenation" -> 4 ticks (bit 4, 0x10).
        if found.family_flags & 0x10 != 0 {
            tickcount = 4;
        }
        // Consumes the Regrowth or Rejuvenation aura, then the heal lands with
        // the consumed ticks' worth added to `spell->damage`.
        self.remove_auras_due_to_spell(victim, found.spell_id);
        *script_damage += found.tick_heal as f32 * tickcount as f32;
        true
    }

    // ===== spell_meteor (spell_special.cpp:21) spells: 24340, 26558, 28884, 26789

    // ===== spell_darkmoon_steam_tonk_control_console (spell_special.cpp:41) spells: 24934

    // NEEDS-HOST: OnInit seam: no `SpellScript::OnInit` dispatch exists in the
    // Rust host (fork calls it from `Spell` init, `Spell.cpp`). Wire a
    // `cast_spell` pre-seam that runs once per cast before validation.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn darkmoon_steam_tonk_control_console_on_init(&mut self, caster: ObjectGuid) {
        // Fork: `pPlayer->UnsummonPetTemporaryIfAny()` so a Hunter/Warlock pet
        // does not soft-lock the player in place when the Tonk is summoned.
        // UNMODELED: pet system (`UnsummonPetTemporaryIfAny`) has no Rust runtime.
        let _ = caster;
    }

    // ===== spell_darkmoon_steam_tonk_cannon (spell_special.cpp:56) spells: 24933

    // ===== spell_showlabel_off (spell_special.cpp:71) spells: 456

    // ===== spell_showlabel_on (spell_special.cpp:87) spells: 2765

    // ===== spell_gm_off (spell_special.cpp:103) spells: 1509

    // ===== spell_gm_on (spell_special.cpp:119) spells: 18139

    // ===== spell_invis_off (spell_special.cpp:135) spells: 6147

    // ===== spell_invis_on (spell_special.cpp:151) spells: 2763

    // ===== spell_bm_off (spell_special.cpp:168) spells: 20114, 24675

    // ===== spell_bm_on (spell_special.cpp:185) spells: 20115, 24676

    // ===== spell_clear_all_cooldowns (spell_special.cpp:201) spells: 29313

    // ===== spell_opening_battleground_banner (spell_special.cpp:214) spells: 21651

    // NEEDS-HOST: OnSuccessfulStart seam: no `SpellScript::OnSuccessfulStart`
    // dispatch exists in the Rust host (fork calls it on success during
    // `Spell::Prepare`, `Spell.cpp`). Wire a post-prepare hook in `cast_spell`.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn opening_battleground_banner_on_successful_start(&mut self, caster: ObjectGuid) {
        // Fork: with a valid GO target whose lock has `Index[1] ==
        // LOCKTYPE_SLOW_OPEN`, prepare a triggered visual spell 24390.
        // (Fork comment: `SPELL_EFFECT_OPEN_LOCK` can succeed with lockId 0,
        // so the lock check guards the visual.)
        // UNMODELED: GO targets (`m_targets.getGOTarget`), the lock store
        // (`sLockStore`), and `LOCKTYPE_SLOW_OPEN` have no Rust runtime;
        // the gated cast below is the reachable part.
        // self.cast_spell(caster, 24390, CastTarget::Unit(caster), CastOptions {
        //     triggered: true, triggered_by_aura: false, skip_known: true,
        //     cast_item: None, custom_base_points: [None; 3],
        //     triggered_by_aura_spell: 0, triggered_by_spell: 0,
        // });
        let _ = caster;
    }

    // ===== spell_cannibalize (spell_special.cpp:238) spells: 20577

    // NEEDS-HOST: OnSuccessfulFinish seam: no `SpellScript::OnSuccessfulFinish`
    // dispatch exists in the Rust host (fork calls it inside `Spell::finish`,
    // `Spell.cpp`; for channels only after a full channel). Wire a
    // finish-time hook in the cast completion path.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn cannibalize_on_successful_finish(
        &mut self,
        caster: ObjectGuid,
        has_unit_or_corpse_target: bool,
    ) {
        // SPELL_CANNIBALIZE_AURA = 20578.
        if has_unit_or_corpse_target {
            self.cast_spell(
                caster,
                20578,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: 0,
                },
            );
        }
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

    // NEEDS-HOST: OnPeriodicTrigger seam: no `AuraScript::OnPeriodicTrigger`
    // dispatch exists in the Rust host (fork calls it from
    // `Aura::PeriodicTriggerSpell`, `SpellAuras.cpp`, letting the script
    // replace or null `spellInfo`). Wire a trigger-time hook that can veto
    // the triggered spell. `spell_haunting_spirits` precedent covers the
    // periodic-dummy variant only, not this one.
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

    // NEEDS-HOST: OnPeriodicTrigger seam: same seam as `spell_activate_mg_turret`
    // (shared `DarkmoonFaireManaConsumptionAuraScript` struct); see above.
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

    /// `MeteorScript::OnEffectExecute` (`spell_special.cpp:23-35`): on
    /// effect 0, divide the damage across every target hit by this effect.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn meteor_on_effect_execute(&mut self, script_damage: &mut f32) -> bool {
        // UNMODELED: the divisor (count of `m_UniqueTargetInfo` with
        // `effectMask & (1 << effIdx)`) is not in the dispatcher's scope;
        // the host must pass the per-effect target count and this line
        // becomes `*script_damage /= count as f32`.
        let _ = script_damage;
        true
    }

    /// `DarkmoonSteamTonkCannonScript::OnEffectExecute`
    /// (`spell_special.cpp:58-65`): triggered 27766 on the unit target.
    fn darkmoon_steam_tonk_cannon_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> bool {
        self.cast_spell(
            caster,
            27766,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        true
    }

    /// `ShowlabelOffScript::OnEffectExecute` (`spell_special.cpp:73-82`):
    /// `SetGMChat(false, true)` on the caster player.
    /// UNMODELED: GM-chat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn showlabel_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `ShowlabelOnScript::OnEffectExecute` (`spell_special.cpp:89-98`):
    /// `SetGMChat(true, true)` on the caster player.
    /// UNMODELED: GM-chat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn showlabel_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `GMOffScript::OnEffectExecute` (`spell_special.cpp:105-114`):
    /// `SetGameMaster(false, true)` on the caster player.
    /// UNMODELED: game-master flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn gm_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `GMOnScript::OnEffectExecute` (`spell_special.cpp:121-130`):
    /// `SetGameMaster(true, true)` on the caster player.
    /// UNMODELED: game-master flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn gm_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `InvisOffScript::OnEffectExecute` (`spell_special.cpp:137-146`):
    /// `SetGMVisible(true, true)` on the caster player.
    /// UNMODELED: GM-visibility flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn invis_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `InvisOnScript::OnEffectExecute` (`spell_special.cpp:153-162`):
    /// `SetGMVisible(false, true)` on the caster player.
    /// UNMODELED: GM-visibility flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn invis_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `BMOffScript::OnEffectExecute` (`spell_special.cpp:171-180`):
    /// `SetCheatBeastmaster(false, true)` on the caster player.
    /// UNMODELED: beastmaster-cheat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn bm_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `BMOnScript::OnEffectExecute` (`spell_special.cpp:187-196`):
    /// `SetCheatBeastmaster(true, true)` on the caster player.
    /// UNMODELED: beastmaster-cheat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn bm_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `ClearAllCooldownsScript::OnEffectExecute`
    /// (`spell_special.cpp:203-209`): `RemoveAllCooldowns` on the caster.
    /// UNMODELED: cooldown storage has no Rust runtime yet.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn clear_all_cooldowns_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    /// `CannibalizeAuraScript::OnAfterApply` (`spell_special.cpp:257-269`):
    /// fork gates on `GetEffIndex() == EFFECT_INDEX_0`; on removal clears
    /// the cannibalize emote (`HandleEmoteCommand(EMOTE_STATE_NONE)`).
    /// UNMODELED: emote commands have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn cannibalize_aura_on_after_apply(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        let _ = (victim, spell_id, apply);
    }

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

    /// `ShadowmeldAuraScript::OnBeforeApply` (`spell_special.cpp:437-452`):
    /// fork gates on removal + `GetEffIndex() == EFFECT_INDEX_0` + player
    /// target; removes the Elusiveness visual 21009.
    fn shadowmeld_on_before_apply(&mut self, victim: ObjectGuid) {
        self.remove_auras_due_to_spell(victim, 21009);
    }

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

    /// `SilithystAuraScript::OnAfterApply` (`spell_special.cpp:295-333`):
    /// fork gates on `GetEffIndex() == EFFECT_INDEX_1` (1.12.1+ only, per
    /// the fork's `#if SUPPORTED_CLIENT_BUILD >= CLIENT_BUILD_1_12_1`;
    /// Silithyst `PvP` was added in 1.12) + player target in zone 1377
    /// (Silithus); on apply casts the team buff (29894 Alliance / 29895
    /// Horde), on removal drops it, and on cancel/death/dispel removal
    /// notifies the zone script (`HandleDropFlag`).
    /// UNMODELED: zone ids, player teams, team-buff removal gating and the
    /// zone-script `HandleDropFlag` callback have no Rust runtime.
    /// Reachable part: the buff spell ids 29894 / 29895 and zone 1377.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn silithyst_on_after_apply(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        let _ = (victim, spell_id, apply);
    }

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

    /// `HeavyArmorKitScript::OnCheckCast` (`spell_item.cpp:22-28`): target
    /// item must be at least item level 15.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn heavy_armor_kit_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: the check reads `spell->m_targets.getItemTarget()` and
        // `ItemProto::ItemLevel`; `CastTarget` carries no item target and item
        // protos are not in the lane runtime. Fork returns
        // `SPELL_FAILED_LOWLEVEL` (43) when `ItemLevel < 15`.
        None
    }

    // ===== spell_deviate_fish (spell_item.cpp:34) spells: 8063
    // (insert before the final `else` in `spell_script_on_effect_execute`)

    /// `DeviateFishScript::OnEffectExecute` (`spell_item.cpp:36-55`): effect 0
    /// casts one random buff on the caster, triggered.
    fn deviate_fish_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index != 0 {
            return true;
        }
        // Fork requires a player caster (`ToPlayer`, else `false`); the lane
        // has no player/creature split, so any unit proceeds (see unresolved).
        // 8064 Sleepy, 8065 Invigorate, 8066 Shrink, 8067 Party Time!,
        // 8068 Healthy Spirit, 8070 Rejuvenation.
        let table = [8064, 8065, 8066, 8067, 8068, 8070];
        let pick = table[self.spell_rng().irand(0, 5) as usize];
        self.cast_spell(
            caster,
            pick,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_cooked_deviate_fish (spell_item.cpp:61) spells: 8213

    /// `CookedDeviateFishScript::OnEffectExecute` (`spell_item.cpp:63-90`):
    /// effect 0 casts one random fun effect on the caster, triggered.
    fn cooked_deviate_fish_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index != 0 {
            return true;
        }
        // Fork requires a player caster (`ToPlayer`, else `false`); the lane
        // has no player/creature split, so any unit proceeds.
        // `GENDER_MALE = 0` (`UnitBase::gender`).
        let male = self.unit_base(caster).is_none_or(|u| u.gender() == 0);
        let table = [
            if male { 8219 } else { 8220 }, // Flip Out - ninja
            if male { 8221 } else { 8222 }, // Yaaarrrr - pirate
            8223,                           // Oops - goo
            8215,                           // Rapid Cast
            8224,                           // Cowardice
            8226,                           // Fake Death
        ];
        // Had additional effects before BWL patch: `spells[urand(0, 5)]` at
        // `SUPPORTED_CLIENT_BUILD <= CLIENT_BUILD_1_5_1`; 1.12.1 uses
        // `spells[urand(0, 1)]`.
        let pick = table[self.spell_rng().irand(0, 1) as usize];
        self.cast_spell(
            caster,
            pick,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_noggenfogger_elixir (spell_item.cpp:96) spells: 16589

    /// `NoggenfoggerElixirScript::OnEffectExecute` (`spell_item.cpp:98-128`):
    /// effect 0 casts skeleton (60%), mini (20%) or slow fall (20%).
    fn noggenfogger_elixir_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index != 0 {
            return true;
        }
        // Fork requires a player caster (`ToPlayer`, else `false`); the lane
        // has no player/creature split, so any unit proceeds.
        let mut pick = 16591; // skeleton (60%)
        match self.spell_rng().irand(1, 10) {
            1 | 2 => pick = 16595, // mini (20%)
            3 | 4 => pick = 16593, // slow fall (20%)
            _ => {}
        }
        self.cast_spell(
            caster,
            pick,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    // ===== spell_linkens_boomerang (spell_item.cpp:133) spells: 15712

    /// `LinkensBoomerangScript::OnEffectExecute` (`spell_item.cpp:135-150`):
    /// `false` skips the stun (effect 1) / disarm (effect 2) proc.
    fn linkens_boomerang_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        // 10% chance to proc stun, 3% chance to proc disarm (dubious numbers):
        // literally `urand(0, 30)` / `urand(0, 10)` nonzero skips, i.e. the
        // effect only fires on a 0 roll (1/31 and 1/11).
        if eff_index == 1 {
            if self.spell_rng().irand(0, 30) != 0 {
                return false;
            }
        } else if eff_index == 2 && self.spell_rng().irand(0, 10) != 0 {
            return false;
        }
        true
    }

    // ===== spell_scorpid_surprise (spell_item.cpp:155) spells: 6410

    /// `ScorpidSurpriseScript::OnEffectExecute` (`spell_item.cpp:157-168`):
    /// `false` skips the poison-sac proc (effect 1).
    fn scorpid_surprise_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        if eff_index == 1 {
            // Heals 294 damage over 21 sec, assuming you don't bite down on a
            // poison sac. 10% proc rate (no source!): literally the effect only
            // fires on a 0 roll of `urand(0, 10)` (1/11).
            if self.spell_rng().irand(0, 10) != 0 {
                return false;
            }
        }
        true
    }

    // ===== spell_brittle_armor_dummy (spell_item.cpp:173) spells: 29284

    /// `BrittleArmorDummyScript::OnEffectExecute` (`spell_item.cpp:175-183`,
    /// Zandalarian Hero Badge): effect 0 triggers 24575 on the unit target.
    fn brittle_armor_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index == 0 {
            self.cast_spell(
                caster,
                24575,
                CastTarget::Unit(victim),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

    // ===== spell_mercurial_shield_dummy (spell_item.cpp:188) spells: 29286

    /// `MercurialShieldDummyScript::OnEffectExecute` (`spell_item.cpp:190-199`,
    /// Petrified Scarab): effect 0 triggers 26464 on the unit target.
    fn mercurial_shield_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index == 0 {
            self.cast_spell(
                caster,
                26464,
                CastTarget::Unit(victim),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

    // ===== spell_everlook_transporter (spell_item.cpp:203) spells: 23442
    // (insert before the final `else` in `spell_script_on_effect_execute`)

    /// `EverlookTransporterScript::OnEffectExecute` (`spell_item.cpp:205-220`,
    /// Dimensional Ripper - Everlook): effect 0 rolls malfunction sub-spells.
    fn everlook_transporter_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        // Fork guards on `m_casterUnit`; a missing unit here stands in for that.
        if eff_index == 0 && self.unit_base(caster).is_some() {
            let r = self.spell_rng().irand(0, 119);
            if r >= 70 {
                // 7/12 success (no sub-cast); 4/12 evil twin, 1/12 fire.
                let sub = if r < 100 { 23445 } else { 23449 };
                self.cast_spell(
                    caster,
                    sub,
                    CastTarget::Unit(caster),
                    CastOptions {
                        triggered: true,
                        skip_known: true,
                        ..CastOptions::default()
                    },
                );
            }
        }
        true
    }

    // ===== spell_gdr_channel (spell_item.cpp:231) spells: 13278 (enum: 13278 channel, 13493 periodic, 13279 hit)

    /// `GDRChannelScript::OnEffectExecute` (`spell_item.cpp:233-239`): effect 1
    /// applies the periodic damage aura (13493), triggered.
    fn gdr_channel_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index == 1 {
            // Fork passes a NULL target (`CastSpell(nullptr, ...)`); the spell's
            // implicit targets resolve it, mapped here to the caster (see
            // unresolved).
            self.cast_spell(
                caster,
                13493, // SPELL_GDR_PERIODIC_DAMAGE
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

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

    // ===== spell_target_dummy (spell_item.cpp:274) spells: 4071, 4072, 19805
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (called after summoning a
    // creature; `ScriptMgr.h:165`). No summon hook in the lane yet.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn target_dummy_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, summon, spell);
        // UNMODELED: fork sets the summon's faction to the caster's faction
        // template (`SetFactionTemporary(..., TEMPFACTION_NONE)`) and
        // `UNIT_FIELD_FLAGS = UNIT_FLAG_PLAYER_CONTROLLED`; the lane has no
        // faction-template, temp-faction, or unit-flag write APIs.
    }

    // ===== spell_chained_essence_of_eranikus (spell_item.cpp:286) spells: 12766
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn chained_essence_of_eranikus_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, summon, spell);
        // UNMODELED: fork whispers `PickRandomValue(4438, 4439, 4440, 4441, 4442,
        // 4443, 4444, 4445)` from the summon to `m_casterUnit` (`MonsterWhisper`);
        // creature text/whisper is not in the lane runtime. The roll itself would
        // be `self.spell_rng().irand(0, 7)` over that table. Reachable part: the
        // fork's `m_casterUnit` null guard maps to the unit lookup below.
        let _ = self.unit_base(caster).is_some();
    }

    // ===== spell_release_umis_yeti (spell_item.cpp:301) spells: 17166
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn release_umis_yeti_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, summon, spell);
        // UNMODELED: quest "Are We There, Yeti?" part 3 — fork plays
        // `MonsterTextEmote(6327)` + `MonsterSay(9055)`, then per `GetAreaId`:
        // 541 Un'Goro Crater -> follow NPC_QUIXXIL (10977), say 6314, walk off;
        // 976 Tanaris -> follow NPC_SPRINKLE (7583), emote 6301, walk off;
        // 2255 Winterspring -> follow NPC_LEGACKI (10978), emote 6306, walk off
        // (`MoveFollow(0.6, PI)`, `SetWalk(false)`,
        // `MoveWaypoint(0, 0, 0, 0, 0, false)`, `FindNearestCreature(..., 30.0)`).
        // Motion, area ids, and creature text are not in the lane runtime.
    }

    // ===== spell_vanquished_tentacle_of_cthun (spell_item.cpp:345) spells: 26391
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn vanquished_tentacle_of_cthun_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, summon, spell);
        // UNMODELED: fork pins the summon (`SetIsAtStay(true)`,
        // `SetCommandState(COMMAND_STAY)`, `SetIsCommandFollow(false)`,
        // `SaveStayPosition`); charm-state writes are not in the lane runtime.
    }

    // ===== spell_goblin_jumper_cables (spell_item.cpp:360) spells: 8342

    /// `GoblinJumperCablesScript::OnEffectExecute` (`spell_item.cpp:362-376`):
    /// 67% of the time the defibrillation backfires as 8338 on the caster and
    /// the effect is skipped.
    fn goblin_jumper_cables_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index == 0 {
            // Fork double-checks `m_casterUnit` (redundant with the outer guard).
            if self.roll_chance_i(67) {
                // UNMODELED: fork forwards `spell->m_CastItem` to the 8338 cast;
                // the effect seam carries no cast-item context, mapped to `None`.
                self.cast_spell(
                    caster,
                    8338,
                    CastTarget::Unit(caster),
                    CastOptions {
                        triggered: true,
                        skip_known: true,
                        ..CastOptions::default()
                    },
                );
                return false;
            }
        }
        true
    }

    // ===== spell_goblin_jumper_cables_xl (spell_item.cpp:381) spells: 22999

    /// `GoblinJumperCablesXLScript::OnEffectExecute` (`spell_item.cpp:383-397`):
    /// 50% of the time the defibrillation backfires as 23055 on the caster and
    /// the effect is skipped.
    fn goblin_jumper_cables_xl_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index == 0 && self.roll_chance_i(50) {
            // UNMODELED: fork forwards `spell->m_CastItem` to the 23055 cast;
            // the effect seam carries no cast-item context, mapped to `None`.
            self.cast_spell(
                caster,
                23055,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
            return false;
        }
        true
    }

    // ===== spell_reindeer_transformation (spell_item.cpp:402) spells: 25860

    /// `ReindeerTransformationScript::OnCheckCast` (`spell_item.cpp:404-410`,
    /// Fresh/Preserved Holly): requires the caster to be mounted.
    fn reindeer_transformation_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // Fork fails when `!m_casterUnit || !HasAuraType(SPELL_AURA_MOUNTED)`.
        // `defines::aura::MOUNTED` name to be verified against
        // `mangos-spells/src/defines.rs` at integration time.
        let mounted = self.unit_base(caster).is_some_and(|u| {
            u.auras.iter().any(|h| {
                h.effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == mangos_spells::defines::aura::MOUNTED)
            })
        });
        if !mounted {
            return Some(83); // SPELL_FAILED_ONLY_MOUNTED
        }
        None
    }

    // ===== spell_bag_of_gold (spell_item.cpp:415) spells: 25720 (quest 8606 Decoy!)

    /// `BagOfGoldScript::OnCheckCast` (`spell_item.cpp:417-424`): the (affective)
    /// caster must wear Narain's Turban (aura 25688).
    fn bag_of_gold_on_check_cast(&mut self, caster: ObjectGuid, target: CastTarget) -> Option<u8> {
        let _ = target;
        // Fork checks the affective caster (charm owner) as a player; the lane
        // has no charm-owner or player/creature split, so the caster is checked.
        let has_turban = self
            .unit_base(caster)
            .is_some_and(|u| u.auras.iter().any(|h| h.spell_id == 25688));
        if !has_turban {
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        }
        None
    }

    // ===== spell_instant_cast (spell_item.cpp:430) spells: 4067, 13237
    // NEEDS-HOST: OnSuccessfulStart seam: fork `Spell::Prepare` success path
    // (`ScriptMgr.h:145`; `SetCastTime(0)` + `ReSetTimer`). No prepare seam in
    // the lane — the lane starts casts from `cast_spell` with DBC cast times.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn instant_cast_on_successful_start(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        let _ = (caster, spell);
        // UNMODELED: fork zeroes the remaining cast time and resets the cast
        // timer so the Big Bronze Bomb / Goblin Mortar fires instantly; the lane
        // has no in-flight cast-time override API. If the DBC already lists these
        // as instant, this script is a no-op in the lane.
    }

    // ===== spell_digging_claw (spell_item.cpp:442) spells: 5166 (quest 868 Egg Hunt)

    /// `DiggingClawScript::OnEffectExecute` (`spell_item.cpp:444-458`): effects 1
    /// and 2 roll 20% unless the summoned creature is already nearby.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn digging_claw_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        if eff_index == 1 || eff_index == 2 {
            // UNMODELED: fork reads `EffectMiscValue[effIdx]` as the creature id,
            // returns `false` when `FindNearestCreature(id, 10.0f)` finds it, and
            // otherwise returns `roll_chance_i(20)`; effect misc values and
            // nearest-creature search are not in the lane runtime.
            return true;
        }
        true
    }

    // ===== spell_tanaris_field_sampling (spell_item.cpp:465) spells: 11759, 11760, 11761

    /// `TanarisFieldSamplingScript::OnEffectExecute` (`spell_item.cpp:467-474`,
    /// Untested Basilisk/Hyena/Scorpid Sample): effect 0 is a 50% roll.
    fn tanaris_field_sampling_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell, victim);
        if eff_index == 0 {
            return self.roll_chance_i(50);
        }
        true
    }

    // ===== spell_elunes_candle (spell_item.cpp:479) spells: 26374
    // NEEDS-HOST: OnCast seam: fork `Spell::cast` post-checks path
    // (`ScriptMgr.h:159`). No cast seam in the lane yet.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn elunes_candle_on_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
        spell: &SpellEntry,
    ) {
        // Omen spells for entry 15467.
        const OMEN: [u32; 5] = [26622, 26623, 26624, 26625, 26649];
        let _ = spell;
        let mut sub = 26636;
        // UNMODELED: fork keys off the unit target's entry (`GetEntry() == 15467`
        // Omen -> random OMEN spell; `== 15466` -> 26624); target entry lookup is
        // not in the lane runtime, so the default 26636 always fires here.
        if let CastTarget::Unit(victim) = target {
            let _ = victim;
        }
        if false {
            // Placeholder for the entry-keyed branch once target entries exist:
            // 15467 -> OMEN[urand(0, 4)], 15466 -> 26624.
            sub = OMEN[self.spell_rng().irand(0, 4) as usize];
        }
        self.cast_spell(
            caster,
            sub,
            target,
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_first_aid (spell_item.cpp:510) spells: 746, 1159, 3267, 3268, 7926, 7927, 10838, 10839, 18608, 18610, 20803, 23567, 23568, 23569, 23696, 24412, 24413, 24414, 30020
    // NEEDS-HOST: OnAfterHit seam: fork `DoAllEffectOnTarget` post damage/heal
    // path (`ScriptMgr.h:163`). No hit seam in the lane yet.
    fn first_aid_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = spell;
        // Fork casts 11196 Recently Bandaged on the unit target, triggered with
        // `m_CastItem`; the effect seam carries no cast-item context here.
        // UNMODELED: `m_CastItem` forwarding (mapped to `None`).
        self.cast_spell(
            caster,
            11196, // SPELL_RECENTLY_BANDAGED
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_wolfshead_helm (spell_item.cpp:527) spells: 17770
    // NEEDS-HOST: OnSuccessfulFinish seam: fork `Spell::finish` success path
    // (`ScriptMgr.h:147`). No finish seam in the lane yet.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn wolfshead_helm_on_successful_finish(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        let _ = spell;
        // Fork guards on `m_casterUnit`; cast 29940 on self, triggered.
        if self.unit_base(caster).is_some() {
            self.cast_spell(
                caster,
                29940, // SPELL_WOLFSHEAD_HELM_ENERGY
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
    }

    // ===== spell_kodo_kombobulator (spell_item.cpp:544) spells: 18153

    /// `KodoKombobulatorScript::OnCheckCast` (`spell_item.cpp:546-552`): refuses
    /// while the quest-credit aura (18172) is still present.
    fn kodo_kombobulator_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // Fork only fails when a caster unit exists AND carries the credit aura;
        // a missing unit passes (the `&&` short-circuits), matched by `is_some_and`.
        let has_credit = self.unit_base(caster).is_some_and(|u| {
            u.auras.iter().any(|h| h.spell_id == 18172) // SPELL_KODO_KOMBOBULATOR_QEUST_CREDIT
        });
        if has_credit {
            return Some(40); // SPELL_FAILED_ITEM_NOT_READY
        }
        None
    }

    // ===== spell_melodius_rapture (spell_item.cpp:563) spells: 21050 (Rat Catcher's Flute)

    /// `MelodiusRaptureScript::OnCheckCast` (`spell_item.cpp:565-571`): the unit
    /// target must be a Deeprun Rat (entry 13016).
    #[allow(clippy::unused_self)] // UNMODELED body
    fn melodius_rapture_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = caster;
        // Reachable part: fork fails with `SPELL_FAILED_BAD_TARGETS` when there is
        // no unit target at all.
        let Some(_victim) = Self::script_unit_target(target) else {
            return Some(mangos_spells::defines::cast_result::FAILED_BAD_TARGETS);
        };
        // UNMODELED: fork additionally requires `GetEntry() == 13016`
        // (NPC_DEEPRUN_RAT); target entry lookup is not in the lane runtime, so a
        // wrong-entry unit target currently passes.
        None
    }

    // ===== spell_purify_and_place_food (spell_item.cpp:577) spells: 16072 (Filled Cleansing Bowl)

    /// `PurifyAndPlaceFoodScript::OnCheckCast` (`spell_item.cpp:579-587`): quest
    /// The Blackwood Corrupted — refuse while event 3938 already runs.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn purify_and_place_food_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // `spell->m_spellInfo->Id == 16072` is readable via `spell.id` at the arm
        // site; the map-event gate below is not in the lane runtime.
        // UNMODELED: `GetMap()->GetScriptedMapEvent(3938)` -> `SPELL_FAILED_NOT_READY`.
        None
    }

    // ===== spell_summon_black_qiraji_battle_tank (spell_item.cpp:592) spells: 26656

    /// `SummonBlackQirajiBattleTankScript::OnCheckCast` (`spell_item.cpp:594-621`,
    /// Black Qiraji Resonating Crystal): mount-only rules, dismounting first.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn summon_black_qiraji_battle_tank_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: the whole body keys off state with no lane API —
        // `IsMounted()` (true branch also calls `RemoveSpellsCausingAura(MOUNTED)`
        // then returns `SPELL_FAILED_DONT_REPORT` (23)); `IsInWater()` (+
        // `IsPlayer`/`IsInHighLiquid`) -> `SPELL_FAILED_ONLY_ABOVEWATER` (80);
        // `GetTransport()` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `GetMapId() != MAP_AHN_QIRAJ_TEMPLE && !IsMountAllowed() &&
        // !IsTriggered()` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `GetAreaId() == 35` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `IsInDisallowedMountForm()` -> `SPELL_FAILED_NOT_SHAPESHIFT` (61).
        // Only the mount-presence half is lane-readable (aura-type scan, as in
        // `reindeer_transformation_on_check_cast`), but the dismount side effect
        // has no lane API, so even that is left to the host wiring.
        None
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

    /// `NefarianCorruptedTotemsScript::OnEffectExecute`
    /// (`boss_nefarian.cpp:553-561`): effect 0 only; the fork
    /// null-checks the caster, subsumed here by the seam (caster is
    /// always a unit). Picks one of 23419/23420/23422/23423 and casts
    /// it on the caster, triggered.
    fn nefarian_corrupted_totems_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index != 0 {
            return true;
        }
        // PickRandomValue(23419, 23420, 23422, 23423)
        let table = [23_419_u32, 23_420, 23_422, 23_423];
        let pick = table[self.spell_rng().irand(0, 3) as usize];
        self.cast_spell(
            caster,
            pick,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    /// `NefarianShadowFlamePassiveScript::OnEffectExecute`
    /// (`boss_nefarian.cpp:574-580`): effect 0 with caster and unit
    /// target; the caster casts 22986 on the target, triggered.
    fn nefarian_shadow_flame_passive_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index != 0 {
            return true;
        }
        self.cast_spell(
            caster,
            22_986,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
        true
    }

    /// `NefarianClassCallWarlockScript::OnEffectExecute`
    /// (`boss_nefarian.cpp:591-604`): effect 0 with a unit target;
    /// the target casts 23426 on itself twice (2 Corrupted Infernals),
    /// triggered. The fork's null-target early-out is subsumed by the
    /// seam (victim is always a unit here).
    fn nefarian_class_call_warlock_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = (caster, spell);
        if eff_index != 0 {
            return true;
        }
        for _ in 0..2 {
            self.cast_spell(
                victim,
                23_426,
                CastTarget::Unit(victim),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
        true
    }

    /// `NefarianClassCallRogueScript::OnEffectExecute`
    /// (`boss_nefarian.cpp:613-640`): effect 0 with caster and unit
    /// target; teleports a player target to a random position near the
    /// caster (5.0yd out, facing `angle - PI`, same map).
    fn nefarian_class_call_rogue_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        let _ = spell;
        if eff_index != 0 {
            return true;
        }
        // Fork Player* cast: non-players ignore the teleport.
        if !victim.is_player() {
            return true;
        }
        let Some(cpos) = self.unit_position(caster) else {
            return true;
        };
        // frand(0, M_PI_F * 2)
        let angle = self.spell_rng().frand01() * 2.0 * std::f32::consts::PI;
        // GetFirstCollisionPosition (boss_nefarian.cpp:630) has no Rust
        // seam: straight-line point, caster height, fork facing.
        // UNMODELED: wall/collision clamp of the landing point.
        let dest = Position::new(
            cpos.x + 5.0 * angle.cos(),
            cpos.y + 5.0 * angle.sin(),
            cpos.z,
            angle - std::f32::consts::PI,
        );
        self.teleport_player_in_map(victim, dest);
        true
    }

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

    // NEEDS-HOST: OnPeriodicTickEnd seam (fork
    // `Aura::PeriodicTickEnd` path): call this from the new seam with
    // the aura bearer and holder.
    /// `NefarianClassCallMageAuraScript::OnPeriodicTickEnd`
    /// (`boss_nefarian.cpp:657-698`): every 5s the bearer (a living
    /// player mage without 23603) forces a random eligible raid member
    /// to be polymorphed via a non-triggered 23603 cast.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
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
        // UNMODELED: pMage->InterruptNonMeleeSpells(false): no pub
        // interrupt seam in the Rust runtime.
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

    /// `ThaddiusPositiveChargeScript::OnEffectExecute`
    /// (`boss_thaddius.cpp:1193-1202`): effect 0 with a unit target;
    /// a target already bearing 28059 takes no damage.
    fn thaddius_positive_charge_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        let _ = spell;
        if eff_index == 0 && self.unit_has_aura(victim, 28_059) {
            // Fork sets `spell->damage = 0`; host channel
            // is additive, so subtract the rolled base
            // (re-roll caveat as in the judgement arm).
            *script_damage -= self.effect_value(spell, eff_index, self.unit_level(caster));
        }
        true
    }

    /// `ThaddiusNegativeChargeScript::OnEffectExecute`
    /// (`boss_thaddius.cpp:1211-1220`): effect 0 with a unit target;
    /// a target already bearing 28084 takes no damage.
    fn thaddius_negative_charge_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        let _ = spell;
        if eff_index == 0 && self.unit_has_aura(victim, 28_084) {
            // Fork sets `spell->damage = 0`; host channel
            // is additive, so subtract the rolled base
            // (re-roll caveat as in the judgement arm).
            *script_damage -= self.effect_value(spell, eff_index, self.unit_level(caster));
        }
        true
    }

    /// `ThaddiusMagneticPullScript::OnEffectExecute`
    /// (`boss_thaddius.cpp:1227-1237`): effect 0 with a unit target;
    /// manual knockback, then skip the effect (return false).
    fn thaddius_magnetic_pull_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // speedXY = EffectMiscValue[effIdx] * 0.1; speedZ =
        // dist(victim, caster) / speedXY * 0.5 * 20.0. Fork GetDistance
        // is 3d; f32 division keeps the fork's unguarded shape.
        #[allow(clippy::cast_precision_loss)]
        let speed_xy = spell.effects[eff_index].misc_value as f32 * 0.1;
        let dist = match (self.unit_position(victim), self.unit_position(caster)) {
            (Some(v), Some(c)) => {
                let dx = v.x - c.x;
                let dy = v.y - c.y;
                let dz = v.z - c.z;
                (dx * dx + dy * dy + dz * dz).sqrt()
            }
            _ => 0.0,
        };
        let speed_z = dist / speed_xy * 0.5 * 20.0;
        // UNMODELED: victim->KnockBackFrom(caster, -speedXY, speedZ):
        // knock_back_from is private to the spells module, so this lane
        // cannot apply it; the effect is still skipped per fork.
        let _ = (speed_xy, speed_z);
        false
    }

    /// `ThaddiusPositiveChargeAuraScript::OnAfterApply`
    /// (`boss_thaddius.cpp:1113-1125`): on remove only (fork: effect 0;
    /// the holder-level seam carries no effect index, so this runs once
    /// per removal); drop the 29659 amplify aura.
    fn thaddius_positive_charge_aura_on_after_apply(&mut self, victim: ObjectGuid) {
        if self.unit_has_aura(victim, 29_659) {
            self.remove_auras_due_to_spell(victim, 29_659);
        }
    }

    /// `ThaddiusNegativeChargeAuraScript::OnAfterApply`
    /// (`boss_thaddius.cpp:1173-1185`): on remove only (fork: effect 0,
    /// same seam note as above); drop the 29660 amplify aura.
    fn thaddius_negative_charge_aura_on_after_apply(&mut self, victim: ObjectGuid) {
        if self.unit_has_aura(victim, 29_660) {
            self.remove_auras_due_to_spell(victim, 29_660);
        }
    }

    // NEEDS-HOST: OnPeriodicTrigger seam (fork
    // `Aura::PeriodicTriggerSpell` path): call this from the new seam
    // with the aura bearer and holder.
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

    // NEEDS-HOST: OnPeriodicTrigger seam, same as above.
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
    // NEEDS-HOST: OnSetTargetMap seam (fork Spell targeting radius path,
    // `ScriptMgr.h:153`): call this from the new seam with the caster, the
    // spell entry, the effect index and the inout radius.

    // ===== spell_grobbulus_mutagen_explosion (boss_grobbulus.cpp:279) spells: 28206 (x1.5 expired / /1.5 dispelled)

    /// `GrobbulusMutatingInjectionScript::OnBeforeApply`
    /// (`boss_grobbulus.cpp:230-250`): remove path only. The bearer
    /// casts Poison Cloud 28240 on itself, triggered by the injection
    /// aura; the caster casts Mutagen Explosion 28206 on the bearer
    /// (see NEEDS-HOST below).
    fn grobbulus_mutating_injection_on_before_apply(
        &mut self,
        victim: ObjectGuid,
        holder: &mut AuraHolder,
    ) {
        let spell_id = holder.spell_id;
        let caster = holder.caster;
        let _ = caster;
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
        // NEEDS-HOST: the 28206 branch needs the aura remove mode
        // (boss_grobbulus.cpp:235-244), and AuraHolder carries none:
        // dispel (AURA_REMOVE_BY_DISPEL) -> caster casts 28206 on the
        // bearer with no aura context; any other removal -> same cast
        // with aura context (triggered_by_spell 28169, which the
        // explosion script reads as its x1.5 gate). Neither cast is
        // wired here until the seam carries the remove mode.
    }

    // NEEDS-HOST: OnSetTargetMap seam, see block above.
    /// `GrobbulusCloudPoisonScript::OnSetTargetMap`
    /// (`boss_grobbulus.cpp:258-273`): the DBC radius is 30yd, but the
    /// visual grows no further than ~20yd, so cap it there: the cloud
    /// gets 2yd instantly and grows to 20 over the 28158 holder's life.
    fn grobbulus_cloud_poison_on_set_target_map(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        radius: &mut f32,
    ) {
        let _ = (spell, eff_index);
        if let Some(cloud) = self
            .aura_holders(caster)
            .into_iter()
            .find(|h| h.spell_id == 28_158)
        {
            if cloud.max_duration_ms > 0 {
                let curr = cloud.max_duration_ms - cloud.duration_ms;
                #[allow(clippy::cast_precision_loss)]
                {
                    *radius = 18.0 / cloud.max_duration_ms as f32 * curr as f32 + 2.0;
                }
            }
        }
    }

    /// `GrobbulusMutagenExplosionScript::OnEffectExecute`
    /// (`boss_grobbulus.cpp:281-295`): effect 0 with a unit target
    /// (subsumed by the seam). All sources agree the explosion does
    /// ~4.5k physical on expiry but less on dispel: expired 28169 sets
    /// `m_triggeredBySpellInfo` (damage x1.5), dispelled leaves it null
    /// (damage /1.5).
    /// The dispatcher now threads `triggered_by_spell`; the
    /// 28206 caster-cast side still waits on remove-mode (above).
    fn grobbulus_mutagen_explosion_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
        triggered_by_spell: u32,
    ) -> bool {
        let _ = victim;
        if eff_index != 0 {
            return true;
        }
        // Fork multiplies/divides the rolled damage;
        // host channel is additive (re-roll caveat as
        // in the judgement arm). `triggered_by_spell`
        // != 0 mirrors `m_triggeredBySpellInfo` set
        // (expired 28169); the dispel branch casts
        // without it once remove-mode lands.
        let base = self.effect_value(spell, eff_index, self.unit_level(caster));
        if triggered_by_spell != 0 {
            *script_damage += base * 0.5;
        } else {
            *script_damage -= base / 3.0;
        }
        true
    }

    // ===== spell_maexxna_web_spray (boss_maexxna.cpp:415) spells: 29484

    // NEEDS-HOST: OnCheckTarget seam: the Rust host has no per-unit
    // `Spell::CheckTarget` dispatch (fork calls it from `Spell.cpp`
    // targeting). Wire a target-filter seam in the cast pipeline, then
    // this predicate stands as the arm body.
    fn maexxna_web_spray_on_check_target(&self, target: ObjectGuid) -> bool {
        // Maexxna Web Spray should not hit players under Web Wrap or Petrification.
        // 17624 Petrification, 28622 Web Wrap (triggered).
        !(self.bot_has_aura(target, 17_624) || self.bot_has_aura(target, 28_622))
    }

    // ===== spell_maexxna_spider_web (boss_maexxna.cpp:429) spells: 28434

    /// `MaexxnaSpiderWebScript::OnEffectExecute`
    /// (`boss_maexxna.cpp:431-452`): effect 0 launches the victim
    /// toward the caster (see `boss_maexxnaAI::DoCastWebWrap` for
    /// this rather weird implementation) and skips the effect
    /// (`return false`).
    fn maexxna_spider_web_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> bool {
        let (Some(caster_pos), Some(victim_pos)) =
            (self.unit_position(caster), self.unit_position(victim))
        else {
            return true;
        };
        let dx = victim_pos.x - caster_pos.x;
        let dy = victim_pos.y - caster_pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let y_dist = caster_pos.z - victim_pos.z;
        let horizontal_speed = dist / 1.5;
        let vertical_speed = 12.0 + y_dist * 0.5;
        // Fork `GetAngle(casterX, casterY)` on the victim: direction
        // from the victim toward the caster.
        let angle = (caster_pos.y - victim_pos.y).atan2(caster_pos.x - victim_pos.x);
        // UNMODELED: `plr->SetLaunched(true)` anticheat-immune flag —
        // no Rust runtime for the launched state.
        // NEEDS-HOST: directed knockback along the explicit `angle`
        // above (fork `Unit::KnockBack(angle, hSpeed, vSpeed)` travels
        // TOWARD the caster/wrap point, as in `DoCastWebWrap`). The
        // existing `knock_back_from(victim, source, h, v)` knocks AWAY
        // from the source and recomputes its own angle, so it would
        // launch the victim the wrong way here; it is also private
        // (`spells.rs`). Expose an angle-explicit seam, then call it
        // with (`angle`, `horizontal_speed`, `vertical_speed`).
        let _ = (angle, horizontal_speed, vertical_speed);
        false
    }

    // ===== spell_kelthuzad_void_blast (boss_kelthuzad.cpp:1022) spells: 27812

    /// `KelThuzadVoidBlastScript::OnEffectExecute`
    /// (`boss_kelthuzad.cpp:1024-1036`): effect 0 deals no damage
    /// while the victim holds the Chains of Kel'Thuzad aura.
    fn kelthuzad_void_blast_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // If target has the chains of kel'thuzad aura the spell should not do any damage.
        // This check should not be necessary as you should be friendly to the caster of
        // the spell, but some bug caused players to take damage anyway, and even if that is fixed,
        // this is a safetycheck.
        if self.bot_has_aura(victim, 28_410) {
            // `spell->damage = 0`: cancel the rolled base through the
            // `script_damage` channel (final is
            // `ditheru(value + script_damage)`, `spells.rs:2441`).
            // Caveat shared with the judgement-of-command arm:
            // `effect_value` re-rolls the dice component, so this is
            // exact when the effect has no dice variance and no
            // custom base points.
            *script_damage -= self.effect_value(spell, eff_index, self.unit_level(caster));
        }
        true
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

    // NEEDS-HOST: OnCheckTarget seam: the Rust host has no per-unit
    // `Spell::CheckTarget` dispatch (fork calls it from `Spell.cpp`
    // targeting). Wire a target-filter seam in the cast pipeline, then
    // this predicate stands as the arm body.
    fn unrelenting_rider_shadow_bolt_volley_on_check_target(&self, target: ObjectGuid) -> bool {
        // Shadow Bolt volley which should only target players with the Shadow Mark debuff.
        // 27825 Shadow Mark.
        self.bot_has_aura(target, 27_825)
    }

    // ===== spell_sapphiron_life_drain (boss_sapphiron.cpp:600) spells: 28542

    // NEEDS-HOST: OnSetTargetMap seam: the Rust host has no
    // `SpellScript::OnSetTargetMap` dispatch (fork calls it from
    // targeting, `Spell.cpp`). Wire a target-map-parameter seam that
    // runs before target selection, then this helper supplies
    // `unMaxTargets`.
    fn sapphiron_life_drain_max_targets(&mut self) -> u32 {
        self.spell_rng().irand(7, 10) as u32
    }

    // ===== spell_noth_curse_of_the_plaguebringer (boss_noth.cpp:432) spells: 29213

    // NEEDS-HOST: OnSetTargetMap seam: the Rust host has no
    // `SpellScript::OnSetTargetMap` dispatch (fork calls it from
    // targeting, `Spell.cpp`). Wire a target-map-parameter seam that
    // runs before target selection, then this helper supplies
    // `selectClosestTargets`.
    fn noth_curse_of_the_plaguebringer_select_closest_targets() -> bool {
        true
    }

    // ===== spell_loatheb_corrupted_mind_aoe (boss_loatheb.cpp:496) spells: 29201

    /// `LoathebCorruptedMindAoEScript::OnEffectExecute`
    /// (`boss_loatheb.cpp:498-526`): effect 0 casts the class-
    /// specific Corrupted Mind sub-spell on the victim, triggered.
    /// Non-healer classes get no sub-spell (`return false` skips
    /// the effect).
    fn loatheb_corrupted_mind_aoe_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> bool {
        // Loatheb Corrupted Mind triggered sub spells.
        // Class ids are DBC (`ChrClasses.dbc`): Paladin 2, Priest 5,
        // Shaman 7, Druid 11.
        let sub = match self
            .unit_base(victim)
            .map(mangos_entities::unit::UnitBase::class)
        {
            // Priests should be getting 29185, but it triggers on dmg effects as well, don't know why.
            // Stealing druid version for priests until anyone has a reason priests cant smite.
            Some(5 | 11) => 29_194, // CLASS_PRIEST (druid version stands in; 29185 kept out) + CLASS_DRUID
            Some(2) => 29_196,      // CLASS_PALADIN
            Some(7) => 29_198,      // CLASS_SHAMAN
            _ => return false,
        };
        // `m_caster->CastSpell(target, spellid, true)`: triggered
        // cast, mirroring the holy-shock exemplar options.
        self.cast_spell(
            caster,
            sub,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: 0,
            },
        );
        true
    }

    // ===== spell_heigan_mana_burn (boss_heigan.cpp:475) spells: 29310

    // NEEDS-HOST: OnSetTargetMap seam: the Rust host has no
    // `SpellScript::OnSetTargetMap` dispatch (fork calls it from
    // targeting, `Spell.cpp`). Wire a target-map-parameter seam that
    // runs before target selection, then this helper supplies
    // `radius`.
    fn heigan_mana_burn_radius() -> f32 {
        // Without a bigger radius its possible to tank heigan in one corner of the platform, and have ranged stay in the other corner.
        28.0
    }

    // ===== spell_gluth_decimate (boss_gluth.cpp:371) spells: 28375

    /// `GluthDecimateScript::OnEffectExecute`
    /// (`boss_gluth.cpp:373-381`): effect 0 sets the damage so the
    /// victim is left at 5% of max health, never reduced below it.
    #[allow(clippy::cast_precision_loss)]
    fn gluth_decimate_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        let Some(u) = self.unit_base(victim) else {
            return true;
        };
        // Damage should put target at maximum 5% hp, but not reduce it below that.
        // Fork integer math: `max(0, health - uint32(maxHealth * 0.05f))`.
        #[allow(clippy::cast_possible_truncation)]
        let five_pct = (u.max_health() as f32 * 0.05) as u32;
        let desired = u.health().saturating_sub(five_pct);
        // `spell->damage` absolute set: offset the rolled base through
        // the `script_damage` channel (final is
        // `ditheru(value + script_damage)`, `spells.rs:2441`).
        // Same dice re-roll caveat as the void-blast arm above.
        *script_damage =
            desired as f32 - self.effect_value(spell, eff_index, self.unit_level(caster));
        true
    }

    // ===== spell_faerlina_poison_bolt_volley (boss_faerlina.cpp:212) spells: 28796

    // NEEDS-HOST: OnSetTargetMap seam: the Rust host has no
    // `SpellScript::OnSetTargetMap` dispatch (fork calls it from
    // targeting, `Spell.cpp`). Wire a target-map-parameter seam that
    // runs before target selection, then this helper supplies
    // `unMaxTargets`.
    fn faerlina_poison_bolt_volley_max_targets() -> u32 {
        10
    }

    // ===== spell_emperor_mutate_bug (boss_twinemperors.cpp:1014) spells: 802

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wire a dispatch over the
    // target-mode pass that lets scripts clamp `un_max_targets`.
    fn emperor_mutate_bug_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Mutate Bug hits exactly 1 bug target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_emperor_explode_bug (boss_twinemperors.cpp:1022) spells: 804

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Same seam as
    // `emperor_mutate_bug_on_set_target_map` above.
    fn emperor_explode_bug_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Explode Bug hits exactly 1 bug target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_huhuran_wyvern_sting (boss_huhuran.cpp:166) spells: 26180

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wire a dispatch over the
    // target-mode pass that lets scripts force `selectClosestTargets`.
    fn huhuran_wyvern_sting_on_set_target_map(&mut self, select_closest_targets: &mut bool) {
        // Fork: Wyvern Sting (AQ40, Princess Huhuran) retargets the closest
        // targets first.
        let _ = self;
        *select_closest_targets = true;
    }

    // ===== spell_huhuran_poison_bolt_volley (boss_huhuran.cpp:175) spells: 26052

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Same seam as
    // `huhuran_wyvern_sting_on_set_target_map` above.
    fn huhuran_poison_bolt_volley_on_set_target_map(&mut self, select_closest_targets: &mut bool) {
        // Fork: Poison Bolt Volley (AQ40, Princess Huhuran) retargets the
        // closest targets first.
        let _ = self;
        *select_closest_targets = true;
    }

    // ===== spell_aq20_drain_mana (ruins_of_ahnqiraj.cpp:962) spells: 25676, 25754

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wire a dispatch over the
    // target-mode pass that lets scripts clamp `un_max_targets`.
    fn aq20_drain_mana_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Drain Mana (25676 Obsidian Destroyer, 25754 Moam) hits at most
        // 6 targets.
        let _ = self;
        *un_max_targets = 6;
    }

    // NEEDS-HOST: OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spell::CheckTarget`, `Spells/Spell.cpp:7761`; the GO overload at
    // `Spell.cpp:1204` is untouched by this script). Wire a per-target dispatch
    // returning accept/reject.
    #[allow(clippy::cast_precision_loss)]
    fn aq20_drain_mana_on_check_target(&self, target: ObjectGuid) -> bool {
        // Fork: avoid targeting players with no mana.
        let Some(unit) = self.unit_base(target) else {
            return false;
        };
        if unit.power_type() != mangos_entities::unit::POWER_MANA as u8 {
            return false;
        }
        let max = unit.max_power(mangos_entities::unit::POWER_MANA);
        if max == 0 {
            return false;
        }
        // Fork: `target->GetPowerPercent(POWER_MANA) < 1.0f` rejects.
        (unit.power(mangos_entities::unit::POWER_MANA) as f32) * 100.0 / (max as f32) >= 1.0
    }

    // ===== spell_rajaxx_thundercrash (ruins_of_ahnqiraj.cpp:978) spells: 25599

    #[allow(clippy::cast_precision_loss)]
    fn rajaxx_thundercrash_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        // Fork: percent from health with min.
        let health = self
            .unit_base(victim)
            .map_or(0, mangos_entities::unit::UnitBase::health);
        let mut damage = health / 2;
        if damage < 200 {
            damage = 200;
        }
        // Fork `#if SUPPORTED_CLIENT_BUILD >= CLIENT_BUILD_1_11_2` arm (active
        // on this 1.12.1 fork): `spell->damage = damage` rides the
        // `script_damage` channel into the damage arms. The `#else` arm
        // (pre-1.11.2) instead casts 25598 on the target via
        // `CastCustomSpell(spell->GetUnitTarget(), 25598, damage, {}, {}, true)`.
        // Host channel is additive: offset the
        // rolled base (re-roll caveat as in the
        // judgement arm).
        *script_damage =
            damage as f32 - self.effect_value(spell, eff_index, self.unit_level(caster));
        true
    }

    // ===== spell_aq40_drain_mana (instance_temple_of_ahnqiraj.cpp:914) spells: 26457, 26559

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wire a dispatch over the
    // target-mode pass that lets scripts clamp `un_max_targets`.
    fn aq40_drain_mana_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Drain Mana (26457 Obsidian Eradicator, 26559 Obsidian Nullifier)
        // hits at most 12 targets.
        let _ = self;
        *un_max_targets = 12;
    }

    // NEEDS-HOST: OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spell::CheckTarget`, `Spells/Spell.cpp:7761`; the GO overload at
    // `Spell.cpp:1204` is untouched by this script). Wire a per-target dispatch
    // returning accept/reject.
    #[allow(clippy::cast_precision_loss)]
    fn aq40_drain_mana_on_check_target(&self, target: ObjectGuid) -> bool {
        // Fork: avoid targeting players with no mana.
        let Some(unit) = self.unit_base(target) else {
            return false;
        };
        if unit.power_type() != mangos_entities::unit::POWER_MANA as u8 {
            return false;
        }
        let max = unit.max_power(mangos_entities::unit::POWER_MANA);
        if max == 0 {
            return false;
        }
        // Fork: `target->GetPowerPercent(POWER_MANA) < 1.0f` rejects.
        (unit.power(mangos_entities::unit::POWER_MANA) as f32) * 100.0 / (max as f32) >= 1.0
    }

    // ===== spell_simone_seductress_chain_lightning (ungoro_crater.cpp:884) spells: 23206

    fn simone_seductress_chain_lightning_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        // Fork: reduce damage by 75% if target has Aspect of the Wild (Rank 2).
        // Additive host channel: subtract 75% of the
        // rolled base (re-roll caveat as in the
        // judgement arm).
        if self.bot_has_aura(victim, 20_190) {
            *script_damage -= self.effect_value(spell, eff_index, self.unit_level(caster)) * 0.75;
        }
        true
    }

    // ===== spell_emerald_dragons_dream_fog (boss_dragon_of_nightmare.cpp:232) spells: 24781

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wire a dispatch over the
    // target-mode pass that lets scripts clamp `un_max_targets`.
    fn emerald_dragons_dream_fog_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Dream Fog (Emerald Dragons) hits exactly 1 target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_haunting_phantoms (stratholme.cpp:636) spells: 16336

    // ===== spell_eye_of_naxxramas_summon_rockwing_gargoyles (stratholme.cpp:663) spells: 16381

    // NEEDS-HOST: OnSummon seam: fork calls it from the summon effect handlers
    // (`Spells/SpellEffects.cpp:2362,2840,2864`, also 2697/3575/4900/5120/5365
    // for the other summon kinds). Wire a post-summon dispatch carrying the
    // caster guid and the summoned creature guid.
    #[allow(dead_code, clippy::unused_self)] // NO-DISPATCHER-YET (step-4 remainder)
    fn eye_of_naxxramas_summon_rockwing_gargoyles_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
    ) {
        // Fork: if the caster unit exists, the summon attacks the caster's
        // `GetAttackerForHelper` target.
        // UNMODELED: attacker-for-helper/threat lookup and `AttackStart` have no
        // Rust runtime; no summon plumbing exists to route this through.
        let _ = (caster, summon);
    }

    // ===== spell_uldaman_awaken_vault_warder (uldaman.cpp:226) spells: 10258

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spells/Spell.cpp:2173`). Wire a dispatch over the target-mode pass that
    // lets scripts clamp `un_max_targets`.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn uldaman_awaken_vault_warder_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: cap the warder awakening at 2 targets.
        *un_max_targets = 2;
    }

    // ===== spell_gnomeregan_collecting_fallout (gnomeregan.cpp:877) spells: 12709

    // NEEDS-HOST: OnInit seam: fork calls it from `Spell` init
    // (`Spells/Spell.cpp:309,346`). Wire a once-per-cast dispatch that stores
    // per-cast state: the fork keeps `chosenEffect` as a per-script-instance
    // member picked here, which `OnEffectExecute` below reads.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn gnomeregan_collecting_fallout_on_init(&mut self, chosen_effect: &mut u32) {
        // Fork: `chosenEffect = urand(EFFECT_INDEX_0, EFFECT_INDEX_1)` — exactly
        // one of the two trigger-spell effects fires per cast.
        *chosen_effect = self.spell_rng().irand(0, 1) as u32;
    }

    // ===== spell_shazzrah_gate (boss_shazzrah.cpp:143) spells: 23138

    // NEEDS-HOST: OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spells/Spell.cpp:2173`). Same seam as
    // `uldaman_awaken_vault_warder_on_set_target_map` above.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn shazzrah_gate_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Gate of Shazzrah blinks to exactly 1 target.
        *un_max_targets = 1;
    }

    // ===== spell_golemaggs_trust (boss_golemagg.cpp:352) spells: 20556 (casts 20553)

    // ===== spell_ubrs_freeze_rookery_egg (instance_blackrock_spire.cpp:883) spells: 15748, 16028

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

    // NEEDS-HOST: OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spells/Spell.cpp:1204` and `Spell::CheckTarget`, `Spell.cpp:7761`). Wire a
    // per-target dispatch returning accept/reject.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn death_from_below_cannon_fire_on_check_target(&self, target: ObjectGuid) -> bool {
        // Fork: `return !target->IsCharmerOrOwnerPlayerOrPlayerItself();` — the
        // cannon fire must not hit players / player-owned units.
        // UNMODELED: charmer/owner chain has no Rust runtime (host only splits
        // player-vs-creature via `script_is_friendly`); kept as reject-all-hostile
        // placeholder until owner tracking exists.
        // Player arm modeled; charmed/owned-by-player
        // creatures still pass (owner chain subset).
        !target.is_player()
    }

    /// `SpellScript::OnSummon` dispatch (fork summon
    /// effect handlers, `SpellEffects.cpp:2362,2840,2864`).
    pub(crate) fn spell_script_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        match spell.id {
            // `spell_av_create_shredder`
            // (`battleground_alterac.cpp:4879`).
            21544 | 21565 => self.av_create_shredder_on_summon(caster, summon, spell),
            _ => {}
        }
    }

    /// `AVCreateShredderScript::OnSummon`: the possess
    /// (effect 2) cannot target the shredder at
    /// target-selection time, so the summon is stamped
    /// with `UNIT_CREATED_BY_SPELL =
    /// EffectTriggerSpell[1]` and the caster as
    /// creator.
    fn av_create_shredder_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = self.object_set_field(
            summon,
            u32::from(offsets::UNIT_CREATED_BY_SPELL),
            spell.effects[1].trigger_spell,
        );
        if let Some(u) = self.unit_base_mut(summon) {
            u.values().set_guid(offsets::UNIT_FIELD_CREATEDBY, caster);
        }
    }

    // ---- helpers (impl-Map style, after the arms) ----

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

    /// `GnomereganCollectingFalloutScript::OnEffectExecute`
    /// (`gnomeregan.cpp:883-887`): only the pre-rolled effect fires.
    /// `chosen_effect` is the NEEDS-HOST per-cast state from `OnInit`.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn gnomeregan_collecting_fallout_on_effect_execute(
        &mut self,
        eff_index: usize,
        chosen_effect: u32,
    ) -> bool {
        let _ = self;
        eff_index as u32 == chosen_effect
    }

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

    /// `UBRSFreezeRookeryEggScript::OnEffectExecute`
    /// (`instance_blackrock_spire.cpp:885-895`): effect 0 with a gameobject
    /// target that is `GO_READY` triggers it (`UseDoorOrButton`); any other
    /// effect passes through (`true`).
    #[allow(clippy::unused_self)] // UNMODELED body
    fn ubrs_freeze_rookery_egg_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> bool {
        let _ = (caster, victim);
        // UNMODELED: GO target access, `getLootState() == GO_READY` and
        // `UseDoorOrButton(0, true)` have no Rust runtime (spells 15748 and
        // its prototype 16028 share this script). Effect-0 gate is transcribed;
        // the GO use is not.
        false
    }
}
