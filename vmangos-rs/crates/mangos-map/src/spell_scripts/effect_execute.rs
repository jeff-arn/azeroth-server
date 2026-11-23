//! Spell-script `OnInit` + `OnEffectExecute` arms + dispatch (per-effect execution).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // fork hook context + arm table
    /// `SpellScript::OnInit` dispatch (fork
    /// `Spell` construction, `Spell.cpp:309`):
    /// rolls per-cast script state into
    /// `script_effect_pick`. The Darkmoon tonk
    /// arm stays unwired: it needs the fork's
    /// temporary-pet unsummon/resummon pair
    /// (`m_temporaryUnsummonedPetNumber`), which
    /// the pet runtime does not carry yet —
    /// unsummoning without resummon would strand
    /// hunter pets, so the no-op is safer.
    pub(crate) fn spell_script_on_init(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        script_effect_pick: &mut Option<u32>,
    ) {
        let _ = caster;
        if spell.script_name == "spell_gnomeregan_collecting_fallout" {
            let mut chosen = 0u32;
            self.gnomeregan_collecting_fallout_on_init(&mut chosen);
            *script_effect_pick = Some(chosen);
        }
    }

    /// `SpellScript::OnEffectExecute` dispatch: `false` skips
    /// the effect. `script_damage` is the mutable
    /// `Spell::damage` equivalent for this victim's effect pass.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn spell_script_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
        script_effect_pick: Option<u32>,
        cast_item: Option<CastItemCtx>,
        caster_level: u32,
        bp: [Option<i32>; 3],
    ) -> bool {
        let _ = triggered_by_spell;
        match spell.script_name.as_str() {
            "spell_gnomeregan_collecting_fallout" => self
                .gnomeregan_collecting_fallout_on_effect_execute(
                    eff_index,
                    script_effect_pick.unwrap_or(0),
                ),
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
            "spell_goblin_jumper_cables" => self.goblin_jumper_cables_on_effect_execute(
                caster, spell, victim, eff_index, cast_item,
            ),
            "spell_goblin_jumper_cables_xl" => self.goblin_jumper_cables_xl_on_effect_execute(
                caster, spell, victim, eff_index, cast_item,
            ),
            "spell_shaman_flametongue_proc_dummy" => self
                .shaman_flametongue_proc_dummy_on_effect_execute(
                    caster,
                    spell,
                    victim,
                    eff_index,
                    caster_level,
                    bp,
                    cast_item,
                ),
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
    // OnAfterHit seam: fork `Spells/Spell.cpp:1657` (`DoAllEffectOnTarget` unit case, after damage/heal is dealt and procs happen; `ScriptMgr.h:163`). Wired via `spell_script_on_after_hit`.

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

    // ===== spell_shaman_flametongue_proc_dummy (spell_shaman.cpp:20) spells: 8026, 8028, 8029, 8248, 8253, 10445, 10523, 16343, 16344, 16389 (Flametongue Weapon/Totem Proc)

    /// `ShamanFlametongueProcDummyScript::OnEffectExecute`
    /// (`spell_shaman.cpp:22-41`): weapon-speed-scaled 10444 proc.
    /// Fork quirk comments kept: damage coefficients 0.381% per 0.1
    /// speed / 15.244 per 4.0 speed observed; 3.85 factor used (at most
    /// one point off published values).
    /// `weapon_delay_ms` is threaded as `m_CastItem->GetProto()->Delay`;
    /// `spell_damage` = `SpellBaseDamageBonusDone(GetSpellSchoolMask())`.
    /// Missing cast item logs `Spell::EffectDummy: spell %i requires
    /// cast Item` and returns false — no logger at this seam, the
    /// skip is the observable half.
    #[allow(clippy::too_many_arguments)] // fork hook context + cast-item lane
    fn shaman_flametongue_proc_dummy_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        caster_level: u32,
        bp_override: [Option<i32>; 3],
        cast_item: Option<CastItemCtx>,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        let Some(item) = cast_item else {
            return false;
        };
        // A missing prototype is the same missing-item case.
        // Copied out before the `&mut self` damage seams below.
        let Some(weapon_delay_ms) = self
            .stores
            .item_prototypes
            .get(item.item_entry)
            .map(|proto| proto.delay)
        else {
            return false;
        };
        // Fork input `spell->damage`: the effect's `CalculateDamage`
        // result, computed by the handler after the script gate in
        // this lane, so the arm reads it directly. The DUMMY handler
        // recomputes the same value afterwards (identical inputs;
        // zero RNG when the effect has no die sides, as here) and
        // ignores it for this spell — a recorded deviation from the
        // fork's single computation, confined to this arm.
        let base_damage =
            self.effect_value_bp(spell, eff_index, caster_level, bp_override[eff_index]);
        // The versus-creature-type fold the Rust helper adds rides a
        // zero mask here (the fork adds versus separately in
        // `SpellDamageBonusDone`, not in this arm).
        let school_mask = 1i32 << spell.school; // `GetSpellSchoolMask` seam
        let spell_damage = self.spell_base_damage_bonus_done(caster, school_mask, 0);
        #[allow(clippy::cast_precision_loss)]
        let weapon_speed = weapon_delay_ms as f32 / 1_000.0;
        #[allow(clippy::cast_precision_loss)]
        let total = (base_damage + 3.85 * spell_damage as f32) * 0.01 * weapon_speed;
        let bp = mangos_spells::entry::dither(total, self.spell_rng());
        self.cast_custom_spell_item(caster, victim, 10444, [Some(bp), None, None], Some(item));
        true
    }

    // ===== spell_shaman_mana_tide (spell_shaman.cpp:47) spells: 16191
    // OnPeriodicTrigger seam: periodic-trigger tick before the
    // default `EffectTriggerSpell` cast (ScriptMgr.h:207). Wired via
    // `aura_script_on_periodic_trigger`; `true` suppresses the default
    // cast (`spellInfo = nullptr`).
    //     suppress = self.shaman_mana_tide_on_periodic_trigger(tick.target, trigger_spell_id, modifier_amount, tick.spell_id);

    // ===== spell_rogue_eviscerate (spell_rogue.cpp:20) spells: 2098, 6760, 6761, 6762, 8623, 8624, 11299, 11300, 31016

    /// `RogueEviscerateScript::OnEffectExecute`
    /// (`spell_rogue.cpp:22-40`): AP-scaled bonus per combo point.
    /// Fork gate `#if SUPPORTED_CLIENT_BUILD > 1_11_2` (patch 1.12.0
    /// note: Eviscerate scales with AP; rank 9 manual off Blackhand
    /// Assassins) is ACTIVE for this 1.12.1 target — transcribed
    /// unconditionally.
    fn rogue_eviscerate_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        _victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        if eff_index != 0 {
            return true;
        }
        // `IsFitToFamilyMask<CF_ROGUE_EVISCERATE>`
        // (`SpellClassMask.h:224`: ROGUE family,
        // CM0 0x00020000).
        if spell.spell_family_name != 8 || (spell.spell_family_flags & 0x0002_0000) == 0 {
            return true;
        }
        if !caster.is_player() {
            return true;
        }
        let combo = self.combo_points(caster);
        if combo > 0 {
            let Some(u) = self.unit_base(caster) else {
                return true;
            };
            // `GetTotalAttackPowerValue(BASE_ATTACK)`
            // reachable subset (same as the bloodthirst
            // arm; versus-aura folding unmodeled there).
            let attack_power =
                u.base_attack_power + u.attack_power_mods[0] + u.attack_power_mods[1];
            *script_damage += attack_power * f32::from(combo) * 0.03;
        }
        true
    }

    // ===== spell_rogue_vanish (spell_rogue.cpp:46) spells: 1856, 1857, 27617

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

    // ===== spell_warlock_demonic_sacrifice (spell_warlock.cpp:20) spells: 18788

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

    // ===== spell_warlock_conflagrate (spell_warlock.cpp:56) spells: 17962, 18930, 18931, 18932

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

    // ===== spell_warlock_life_tap (spell_warlock.cpp:105) spells: 1454, 1455, 1456, 11687, 11688, 11689

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
    // (Wired via `spell_script_on_successful_dispel`.)

    /// Improved Healthstone ranks (`spell_warlock.cpp:246-251`).
    const WARLOCK_IMPROVED_HEALTHSTONE_R1: u32 = 18_692;
    const WARLOCK_IMPROVED_HEALTHSTONE_R2: u32 = 18_693;

    // ===== spell_warlock_create_healthstone (spell_warlock.cpp:245) spells: 5699, 6201, 6202, 11729, 11730

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

    // ===== spell_warlock_create_healthstone (spell_warlock.cpp:245) spells: 5699, 6201, 6202, 11729, 11730

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

    // ===== spell_priest_touch_of_weakness (spell_priest.cpp:20) spells: 28598
    // Fork: `OnEffectExecute` eff 0, only when there is a unit target and the cast
    // was triggered by the Touch of Weakness aura (`m_triggeredByAuraSpell`):
    // rank map 2652 -> 2943, 19261 -> 19249, 19262 -> 19251, 19264 -> 19252,
    // 19265 -> 19253, 19266 -> 19254, cast triggered on the unit target.
    // Unknown aura id logs in the fork and returns false.
    // (`triggered_by_aura_spell` = the active cast's `m_triggeredByAuraSpell->Id`,
    // 0 = none; threaded from the cast pipeline through the dispatcher):
    // if spell.script_name == "spell_priest_touch_of_weakness" {
    //     return self.priest_touch_of_weakness_on_effect_execute(
    //         caster, spell, victim, eff_index, triggered_by_aura_spell,
    //     );
    // `m_triggeredByAuraSpell` context: the cast pipeline threads the active
    // cast's `triggered_by_aura_spell` (a `CastOptions` field on the way in)
    // through the effect-execute dispatcher to this arm.
    // Seam: `Spell::EffectDummy` path that fires spell 28598.
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

    // ===== spell_druid_ferocious_bite (spell_druid.cpp:20) spells: 22568, 22827, 22828, 22829, 31018
    // Fork (1.12.1 path, `SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_11_2`): eff 0
    // with a unit target, caster must be a player. Damage rides `spell->damage`:
    // `damage += GetTotalAttackPowerValue(BASE_ATTACK) * combo * 0.03` when combo
    // points are up (1.12 "greater potency with attack power" note), then
    // `damage += GetPower(POWER_ENERGY) * m_spellInfo.DmgMultiplier[effIdx]`;
    // energy is then set to 0. The `#else` (pre-1.7.1) branch casting per-rank
    // damage spells 22851/22853/22861/22862 is NOT transcribed (dead fork for the
    // 1.12.1 client). `spell->damage` rides the `script_damage` channel.
    /// `DruidFerociousBiteScript::OnEffectExecute` (`spell_druid.cpp:22-71`).
    fn druid_ferocious_bite_on_effect_execute(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        eff_index: usize,
        script_damage: &mut f32,
    ) -> bool {
        let _ = victim;
        if eff_index != 0 {
            return true;
        }
        if !caster.is_player() {
            return true;
        }
        let combo = self.combo_points(caster);
        if combo > 0 {
            let Some(u) = self.unit_base(caster) else {
                return true;
            };
            // `GetTotalAttackPowerValue(BASE_ATTACK)`
            // reachable subset (same as the bloodthirst
            // arm).
            let attack_power =
                u.base_attack_power + u.attack_power_mods[0] + u.attack_power_mods[1];
            *script_damage += attack_power * f32::from(combo) * 0.03;
        }
        // `damage += GetPower(POWER_ENERGY) *
        // m_spellInfo.DmgMultiplier[effIdx]`, then the
        // energy drains to 0 (`spell_druid.cpp:40-43`).
        let energy = self.unit_power(caster, mangos_spells::defines::power::ENERGY);
        #[allow(clippy::cast_precision_loss)]
        let energy_f = energy as f32;
        *script_damage += energy_f * spell.effects[eff_index].dmg_multiplier;
        self.unit_set_power(caster, mangos_spells::defines::power::ENERGY, 0);
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

    // ===== spell_druid_swiftmend (spell_druid.cpp:96) spells: 18562

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

    // ===== spell_darkmoon_steam_tonk_control_console (spell_special.cpp:41) spells: 24934

    // OnInit seam (fork calls it from `Spell` init, `Spell.cpp`): the
    // `spell_script_on_init` dispatch exists but does not wire this arm —
    // the pet runtime has no temporary-unsummon pair (see dispatcher doc).
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

    // ===== spell_meteor (spell_special.cpp:21) spells: 24340, 26558, 28884, 26789

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

    // ===== spell_darkmoon_steam_tonk_cannon (spell_special.cpp:56) spells: 24933

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

    // ===== spell_showlabel_off (spell_special.cpp:71) spells: 456

    /// `ShowlabelOffScript::OnEffectExecute` (`spell_special.cpp:73-82`):
    /// `SetGMChat(false, true)` on the caster player.
    /// UNMODELED: GM-chat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn showlabel_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_showlabel_on (spell_special.cpp:87) spells: 2765

    /// `ShowlabelOnScript::OnEffectExecute` (`spell_special.cpp:89-98`):
    /// `SetGMChat(true, true)` on the caster player.
    /// UNMODELED: GM-chat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn showlabel_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_gm_off (spell_special.cpp:103) spells: 1509

    /// `GMOffScript::OnEffectExecute` (`spell_special.cpp:105-114`):
    /// `SetGameMaster(false, true)` on the caster player.
    /// UNMODELED: game-master flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn gm_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_gm_on (spell_special.cpp:119) spells: 18139

    /// `GMOnScript::OnEffectExecute` (`spell_special.cpp:121-130`):
    /// `SetGameMaster(true, true)` on the caster player.
    /// UNMODELED: game-master flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn gm_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_invis_off (spell_special.cpp:135) spells: 6147

    /// `InvisOffScript::OnEffectExecute` (`spell_special.cpp:137-146`):
    /// `SetGMVisible(true, true)` on the caster player.
    /// UNMODELED: GM-visibility flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn invis_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_invis_on (spell_special.cpp:151) spells: 2763

    /// `InvisOnScript::OnEffectExecute` (`spell_special.cpp:153-162`):
    /// `SetGMVisible(false, true)` on the caster player.
    /// UNMODELED: GM-visibility flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn invis_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_bm_off (spell_special.cpp:168) spells: 20114, 24675

    /// `BMOffScript::OnEffectExecute` (`spell_special.cpp:171-180`):
    /// `SetCheatBeastmaster(false, true)` on the caster player.
    /// UNMODELED: beastmaster-cheat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn bm_off_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_bm_on (spell_special.cpp:185) spells: 20115, 24676

    /// `BMOnScript::OnEffectExecute` (`spell_special.cpp:187-196`):
    /// `SetCheatBeastmaster(true, true)` on the caster player.
    /// UNMODELED: beastmaster-cheat flags have no Rust runtime.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn bm_on_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_clear_all_cooldowns (spell_special.cpp:201) spells: 29313

    /// `ClearAllCooldownsScript::OnEffectExecute`
    /// (`spell_special.cpp:203-209`): `RemoveAllCooldowns` on the caster.
    /// UNMODELED: cooldown storage has no Rust runtime yet.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn clear_all_cooldowns_on_effect_execute(&mut self, caster: ObjectGuid) -> bool {
        let _ = caster;
        true
    }

    // ===== spell_deviate_fish (spell_item.cpp:34) spells: 8063

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

    // ===== spell_everlook_transporter (spell_item.cpp:203) spells: 23442

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
        cast_item: Option<CastItemCtx>,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index == 0 {
            // Fork double-checks `m_casterUnit` (redundant with the outer guard).
            if self.roll_chance_i(67) {
                // Fork forwards `spell->m_CastItem` to the 8338 cast.
                self.cast_spell(
                    caster,
                    8338,
                    CastTarget::Unit(caster),
                    CastOptions {
                        triggered: true,
                        skip_known: true,
                        cast_item,
                        ..CastOptions::default()
                    },
                );
                return false;
            }
        }
        true
    }

    // ===== spell_goblin_jumper_cables_xl (spell_item.cpp:381) spells: 22999

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
        cast_item: Option<CastItemCtx>,
    ) -> bool {
        let _ = (spell, victim);
        if eff_index == 0 && self.roll_chance_i(50) {
            // Fork forwards `spell->m_CastItem` to the 23055 cast.
            self.cast_spell(
                caster,
                23055,
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    cast_item,
                    ..CastOptions::default()
                },
            );
            return false;
        }
        true
    }

    // ===== spell_reindeer_transformation (spell_item.cpp:402) spells: 25860

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

    // ===== spell_nefarian_corrupted_totems (boss_nefarian.cpp:551) spells: 23424 -> {23419, 23420, 23422, 23423}

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

    // ===== spell_nefarian_shadow_flame_passive (boss_nefarian.cpp:571) spells: 22992 -> 22986

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

    // ===== spell_nefarian_class_call_warlock (boss_nefarian.cpp:587) spells: 23427 -> 23426 x2

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

    // ===== spell_nefarian_class_call_rogue (boss_nefarian.cpp:611) spells: 23414 (teleport)

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

    // ===== spell_thaddius_positive_charge (boss_thaddius.cpp:1191) spells: 28062 (nil damage if target has 28059)

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

    // ===== spell_thaddius_negative_charge (boss_thaddius.cpp:1209) spells: 28085 (nil damage if target has 28084)

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

    // ===== spell_thaddius_magnetic_pull (boss_thaddius.cpp:1225) spells: 28337 (knockback, skip effect)

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

    // ===== spell_grobbulus_mutagen_explosion (boss_grobbulus.cpp:279) spells: 28206 (x1.5 expired / /1.5 dispelled)

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

    // ===== spell_gnomeregan_collecting_fallout (gnomeregan.cpp:877) spells: 12709

    // OnInit seam: fork calls it from `Spell` init
    // (`Spells/Spell.cpp:309,346`). Wired via `spell_script_on_init` as
    // once-per-cast state: the fork keeps `chosenEffect` as a
    // per-script-instance member picked here, which `OnEffectExecute` reads.
    fn gnomeregan_collecting_fallout_on_init(&mut self, chosen_effect: &mut u32) {
        // Fork: `chosenEffect = urand(EFFECT_INDEX_0, EFFECT_INDEX_1)` — exactly
        // one of the two trigger-spell effects fires per cast.
        *chosen_effect = self.spell_rng().irand(0, 1) as u32;
    }

    // ===== spell_shazzrah_gate (boss_shazzrah.cpp:143) spells: 23138

    // ===== spell_gnomeregan_collecting_fallout (gnomeregan.cpp:877) spells: 12709

    /// `GnomereganCollectingFalloutScript::OnEffectExecute`
    /// (`gnomeregan.cpp:883-887`): only the pre-rolled effect fires.
    /// `chosen_effect` is the NEEDS-HOST per-cast state from `OnInit`.
    #[allow(clippy::unused_self)]
    fn gnomeregan_collecting_fallout_on_effect_execute(
        &mut self,
        eff_index: usize,
        chosen_effect: u32,
    ) -> bool {
        let _ = self;
        eff_index as u32 == chosen_effect
    }

    // ===== spell_ubrs_freeze_rookery_egg (instance_blackrock_spire.cpp:883) spells: 15748, 16028

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
