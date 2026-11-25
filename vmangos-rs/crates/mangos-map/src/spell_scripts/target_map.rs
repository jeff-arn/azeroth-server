//! Spell-script target-lane arms + dispatch (`OnSetTargetMap`, `OnCheckTarget`).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
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

    // ===== spell_warrior_intimidating_shout (spell_warrior.cpp:20) spells: 5246

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

    // ===== spell_grobbulus_cloud_poison (boss_grobbulus.cpp:256) spells: 28241 (radius from 28158 holder)

    // OnSetTargetMap seam, see block above (wired via
    // `spell_script_on_set_target_map`).
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

    // ===== spell_maexxna_web_spray (boss_maexxna.cpp:415) spells: 29484

    // OnCheckTarget seam (fork calls it from `Spell.cpp` targeting).
    // Wired via `spell_script_on_check_target`; this predicate stands
    // as the arm body.
    fn maexxna_web_spray_on_check_target(&self, target: ObjectGuid) -> bool {
        // Maexxna Web Spray should not hit players under Web Wrap or Petrification.
        // 17624 Petrification, 28622 Web Wrap (triggered).
        !(self.bot_has_aura(target, 17_624) || self.bot_has_aura(target, 28_622))
    }

    // ===== spell_maexxna_spider_web (boss_maexxna.cpp:429) spells: 28434

    // ===== spell_unrelenting_rider_shadow_bolt_volley (instance_naxxramas.cpp:2029) spells: 27831

    // OnCheckTarget seam (fork calls it from `Spell.cpp` targeting).
    // Wired via `spell_script_on_check_target`; this predicate stands
    // as the arm body.
    fn unrelenting_rider_shadow_bolt_volley_on_check_target(&self, target: ObjectGuid) -> bool {
        // Shadow Bolt volley which should only target players with the Shadow Mark debuff.
        // 27825 Shadow Mark.
        self.bot_has_aura(target, 27_825)
    }

    // ===== spell_sapphiron_life_drain (boss_sapphiron.cpp:600) spells: 28542

    // ===== spell_sapphiron_life_drain (boss_sapphiron.cpp:600) spells: 28542

    // OnSetTargetMap seam (fork calls it from targeting, `Spell.cpp`).
    // Wired via `spell_script_on_set_target_map`; this helper supplies
    // `unMaxTargets`.
    fn sapphiron_life_drain_max_targets(&mut self) -> u32 {
        self.spell_rng().irand(7, 10) as u32
    }

    // ===== spell_noth_curse_of_the_plaguebringer (boss_noth.cpp:432) spells: 29213

    // ===== spell_noth_curse_of_the_plaguebringer (boss_noth.cpp:432) spells: 29213

    // OnSetTargetMap seam (fork calls it from targeting, `Spell.cpp`).
    // Wired via `spell_script_on_set_target_map`; this helper supplies
    // `selectClosestTargets`.
    fn noth_curse_of_the_plaguebringer_select_closest_targets() -> bool {
        true
    }

    // ===== spell_loatheb_corrupted_mind_aoe (boss_loatheb.cpp:496) spells: 29201

    // ===== spell_heigan_mana_burn (boss_heigan.cpp:475) spells: 29310

    // OnSetTargetMap seam (fork calls it from targeting, `Spell.cpp`).
    // Wired via `spell_script_on_set_target_map`; this helper supplies
    // `radius`.
    fn heigan_mana_burn_radius() -> f32 {
        // Without a bigger radius its possible to tank heigan in one corner of the platform, and have ranged stay in the other corner.
        28.0
    }

    // ===== spell_gluth_decimate (boss_gluth.cpp:371) spells: 28375

    // ===== spell_faerlina_poison_bolt_volley (boss_faerlina.cpp:212) spells: 28796

    // OnSetTargetMap seam (fork calls it from targeting, `Spell.cpp`).
    // Wired via `spell_script_on_set_target_map`; this helper supplies
    // `unMaxTargets`.
    fn faerlina_poison_bolt_volley_max_targets() -> u32 {
        10
    }

    // ===== spell_emperor_mutate_bug (boss_twinemperors.cpp:1014) spells: 802

    // ===== spell_emperor_mutate_bug (boss_twinemperors.cpp:1014) spells: 802

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wired via
    // `spell_script_on_set_target_map`, which lets scripts clamp
    // `un_max_targets`.
    fn emperor_mutate_bug_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Mutate Bug hits exactly 1 bug target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_emperor_explode_bug (boss_twinemperors.cpp:1022) spells: 804

    // ===== spell_emperor_explode_bug (boss_twinemperors.cpp:1022) spells: 804

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Same seam as
    // `emperor_mutate_bug_on_set_target_map` above (wired via
    // `spell_script_on_set_target_map`).
    fn emperor_explode_bug_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Explode Bug hits exactly 1 bug target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_huhuran_wyvern_sting (boss_huhuran.cpp:166) spells: 26180

    // ===== spell_huhuran_wyvern_sting (boss_huhuran.cpp:166) spells: 26180

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wired via
    // `spell_script_on_set_target_map`, which lets scripts force
    // `selectClosestTargets`.
    fn huhuran_wyvern_sting_on_set_target_map(&mut self, select_closest_targets: &mut bool) {
        // Fork: Wyvern Sting (AQ40, Princess Huhuran) retargets the closest
        // targets first.
        let _ = self;
        *select_closest_targets = true;
    }

    // ===== spell_huhuran_poison_bolt_volley (boss_huhuran.cpp:175) spells: 26052

    // ===== spell_huhuran_poison_bolt_volley (boss_huhuran.cpp:175) spells: 26052

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Same seam as
    // `huhuran_wyvern_sting_on_set_target_map` above (wired via
    // `spell_script_on_set_target_map`).
    fn huhuran_poison_bolt_volley_on_set_target_map(&mut self, select_closest_targets: &mut bool) {
        // Fork: Poison Bolt Volley (AQ40, Princess Huhuran) retargets the
        // closest targets first.
        let _ = self;
        *select_closest_targets = true;
    }

    // ===== spell_aq20_drain_mana (ruins_of_ahnqiraj.cpp:962) spells: 25676, 25754

    // ===== spell_aq20_drain_mana (ruins_of_ahnqiraj.cpp:962) spells: 25676, 25754

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wired via
    // `spell_script_on_set_target_map`, which lets scripts clamp
    // `un_max_targets`.
    fn aq20_drain_mana_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Drain Mana (25676 Obsidian Destroyer, 25754 Moam) hits at most
        // 6 targets.
        let _ = self;
        *un_max_targets = 6;
    }

    // ===== spell_aq20_drain_mana (ruins_of_ahnqiraj.cpp:962) spells: 25676, 25754

    // OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spell::CheckTarget`, `Spells/Spell.cpp:7761`; the GO overload at
    // `Spell.cpp:1204` is untouched by this script). Wired via
    // `spell_script_on_check_target` (per-target accept/reject).
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

    // ===== spell_aq40_drain_mana (instance_temple_of_ahnqiraj.cpp:914) spells: 26457, 26559

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wired via
    // `spell_script_on_set_target_map`, which lets scripts clamp
    // `un_max_targets`.
    fn aq40_drain_mana_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Drain Mana (26457 Obsidian Eradicator, 26559 Obsidian Nullifier)
        // hits at most 12 targets.
        let _ = self;
        *un_max_targets = 12;
    }

    // ===== spell_aq40_drain_mana (instance_temple_of_ahnqiraj.cpp:914) spells: 26457, 26559

    // OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spell::CheckTarget`, `Spells/Spell.cpp:7761`; the GO overload at
    // `Spell.cpp:1204` is untouched by this script). Wired via
    // `spell_script_on_check_target` (per-target accept/reject).
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

    // ===== spell_emerald_dragons_dream_fog (boss_dragon_of_nightmare.cpp:232) spells: 24781

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spell::SetTargetMap`, `Spells/Spell.cpp:2173`). Wired via
    // `spell_script_on_set_target_map`, which lets scripts clamp
    // `un_max_targets`.
    fn emerald_dragons_dream_fog_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Dream Fog (Emerald Dragons) hits exactly 1 target.
        let _ = self;
        *un_max_targets = 1;
    }

    // ===== spell_haunting_phantoms (stratholme.cpp:636) spells: 16336

    // ===== spell_eye_of_naxxramas_summon_rockwing_gargoyles (stratholme.cpp:663) spells: 16381

    // ===== spell_uldaman_awaken_vault_warder (uldaman.cpp:226) spells: 10258

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spells/Spell.cpp:2173`). Wired via `spell_script_on_set_target_map`,
    // which lets scripts clamp `un_max_targets`.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn uldaman_awaken_vault_warder_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: cap the warder awakening at 2 targets.
        *un_max_targets = 2;
    }

    // ===== spell_gnomeregan_collecting_fallout (gnomeregan.cpp:877) spells: 12709

    // ===== spell_shazzrah_gate (boss_shazzrah.cpp:143) spells: 23138

    // OnSetTargetMap seam: fork calls it from target-map setup
    // (`Spells/Spell.cpp:2173`). Same seam as
    // `uldaman_awaken_vault_warder_on_set_target_map` above (wired via
    // `spell_script_on_set_target_map`).
    #[allow(clippy::unused_self)] // UNMODELED body
    fn shazzrah_gate_on_set_target_map(&mut self, un_max_targets: &mut u32) {
        // Fork: Gate of Shazzrah blinks to exactly 1 target.
        *un_max_targets = 1;
    }

    // ===== spell_golemaggs_trust (boss_golemagg.cpp:352) spells: 20556 (casts 20553)

    // ===== spell_ubrs_freeze_rookery_egg (instance_blackrock_spire.cpp:883) spells: 15748, 16028

    // ===== spell_five_fat_finger_exploding_heart_technique (blackrock_depths.cpp:2275) spells: 27673 (triggers 27676)

    // ===== spell_death_from_below_cannon_fire (arathi_highlands.cpp:438) spells: <id not stated in fork comments>

    // OnCheckTarget (Unit) seam: fork calls it from target validation
    // (`Spells/Spell.cpp:1204` and `Spell::CheckTarget`, `Spell.cpp:7761`).
    // Wired via `spell_script_on_check_target` (per-target accept/reject).
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
}
