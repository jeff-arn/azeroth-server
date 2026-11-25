//! Spell-script `OnSummon` arms + dispatch (summon-effect path).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    // ===== spell_warlock_inferno (spell_warlock.cpp:223) spells: 1122, 24670

    // NEEDS-HOST: OnSummon(Creature) seam: fork summon-effect path hands the
    // freshly summoned creature (`spell->m_caster` in scope).
    /// `WarlockInfernoScript::OnSummon` (`spell_warlock.cpp:225-235`): binds the
    /// Infernal with a free Enslave plus its root and burn-in visuals.
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

    // ===== spell_target_dummy (spell_item.cpp:274) spells: 4071, 4072, 19805
    // OnSummon seam: fork summon-effect path (called after summoning a
    // creature; `ScriptMgr.h:165`). Wired via `spell_script_on_summon`.
    fn target_dummy_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = spell;
        // `SetFactionTemporary(caster faction,
        // TEMPFACTION_NONE)`: no restore condition,
        // so a direct faction write is fork-equal
        // for a temp summon.
        let faction = self
            .unit_base(caster)
            .map_or(0, mangos_entities::unit::UnitBase::faction_template);
        if let Some(u) = self.unit_base_mut(summon) {
            u.set_faction_template(faction);
            // `UNIT_FLAG_PLAYER_CONTROLLED` (0x8),
            // assigned (not OR-ed) as in the fork.
            u.values().set_u32(offsets::UNIT_FIELD_FLAGS, 0x8);
        }
    }

    // ===== spell_chained_essence_of_eranikus (spell_item.cpp:286) spells: 12766
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    fn chained_essence_of_eranikus_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = spell;
        // `MonsterWhisper(PickRandomValue(4438..4445),
        // m_casterUnit)` under the fork's caster-unit
        // null guard.
        if self.unit_base(caster).is_none() {
            return;
        }
        let text_id = 4438 + self.spell_rng().irand(0, 7);
        self.do_script_text(text_id, summon, Some(caster), -1);
    }

    // ===== spell_release_umis_yeti (spell_item.cpp:301) spells: 17166
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    fn release_umis_yeti_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, spell);
        // Quest "Are We There, Yeti?" part 3.
        self.do_script_text(6327, summon, None, -1);
        self.do_script_text(9055, summon, None, -1);
        let Some(pos) = self.unit_position(summon) else {
            return;
        };
        let (_, area_id) = self.zone_and_area_id(pos);
        let (entry, text_id) = match area_id {
            541 => (10977, 6314),  // Un'Goro: Quixxil
            976 => (7583, 6301),   // Tanaris: Sprinkle
            2255 => (10978, 6306), // Winterspring: Legacki
            _ => return,
        };
        let Some(npc) = self.nearest_creature_with_entry(summon, entry, 30.0) else {
            return;
        };
        self.creature_move_follow(summon, npc, 0.6, std::f32::consts::PI);
        self.do_script_text(text_id, npc, None, -1);
        self.set_walk(npc, false);
        // `MoveWaypoint(0,...)`: resume the
        // default (waypoint) generator.
        self.motion_initialize(npc);
    }

    // ===== spell_vanquished_tentacle_of_cthun (spell_item.cpp:345) spells: 26391
    // NEEDS-HOST: OnSummon seam: fork summon-effect path (`ScriptMgr.h:165`).
    fn vanquished_tentacle_of_cthun_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = (caster, spell);
        // Fork pins the guardian at stay
        // (`CharmInfo` stay state + saved stay
        // position). Pet-registered summons take
        // COMMAND_STAY; plain temp summons go
        // idle at the summon point (their evade
        // home — the fork's saved stay position).
        if let Some(st) = self.pets.get_mut(&summon) {
            st.command_state = crate::pets::COMMAND_STAY;
            st.commanded_victim = None;
        } else {
            self.creature_move_idle(summon);
        }
    }

    // ===== spell_goblin_jumper_cables (spell_item.cpp:360) spells: 8342

    // ===== spell_eye_of_naxxramas_summon_rockwing_gargoyles (stratholme.cpp:663) spells: 16381

    // OnSummon seam: fork calls it from the summon effect handlers
    // (`Spells/SpellEffects.cpp:2362,2840,2864`, also 2697/3575/4900/5120/5365
    // for the other summon kinds). Wired via `spell_script_on_summon`
    // (post-summon dispatch carrying caster and summoned creature guids).
    fn eye_of_naxxramas_summon_rockwing_gargoyles_on_summon(
        &mut self,
        caster: ObjectGuid,
        summon: ObjectGuid,
    ) {
        // `GetAttackerForHelper`: the caster's
        // current victim, else its first attacker.
        if self.unit_base(caster).is_none() {
            return;
        }
        let target = self
            .current_victim(caster)
            .or_else(|| self.bot_attackers_of(caster).into_iter().next());
        if let Some(target) = target {
            self.attack_start(summon, target);
        }
    }

    // ===== spell_uldaman_awaken_vault_warder (uldaman.cpp:226) spells: 10258

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
            // `spell_warlock_inferno`
            // (`spell_warlock.cpp:223`).
            1122 | 24670 => self.warlock_inferno_on_summon(caster, summon),
            // `spell_target_dummy` (`spell_item.cpp:274`).
            4071 | 4072 | 19805 => self.target_dummy_on_summon(caster, summon, spell),
            // `spell_chained_essence_of_eranikus`
            // (`spell_item.cpp:286`).
            12766 => self.chained_essence_of_eranikus_on_summon(caster, summon, spell),
            // `spell_release_umis_yeti`
            // (`spell_item.cpp:301`).
            17166 => self.release_umis_yeti_on_summon(caster, summon, spell),
            // `spell_vanquished_tentacle_of_cthun`
            // (`spell_item.cpp:345`).
            26391 => self.vanquished_tentacle_of_cthun_on_summon(caster, summon, spell),
            // `spell_eye_of_naxxramas_summon_rockwing_
            // gargoyles` (`stratholme.cpp:663`).
            16381 => self.eye_of_naxxramas_summon_rockwing_gargoyles_on_summon(caster, summon),
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
}
