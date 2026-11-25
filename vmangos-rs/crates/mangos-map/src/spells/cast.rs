//! Spell cast preparation, execution, channels and cast ticking.

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    #[allow(clippy::too_many_lines)] // one prepare() port
    pub fn cast_spell(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        target: CastTarget,
        options: CastOptions,
    ) -> CastOutcome {
        let original_caster = self.next_original_caster.take();
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return CastOutcome::UnknownSpell;
        };
        // `SpellScript::OnInit` (fork `Spell`
        // construction, `Spell.cpp:309`): per-cast
        // script state, rolled before checks.
        let mut script_effect_pick: Option<u32> = None;
        self.spell_script_on_init(caster, &spell, &mut script_effect_pick);
        let fail = |map: &mut Map, reason: u8| {
            if !options.triggered {
                map.send_cast_result(caster, spell_id, reason);
            }
            CastOutcome::Failed(reason)
        };

        let alive = self.unit_base_mut(caster).is_some_and(|u| u.is_alive());
        // Triggered casts skip the dead-caster gate
        // (EventAI DEATH casts like Eject Sneed).
        if !alive && !options.triggered {
            return fail(self, cast_result::FAILED_CASTER_DEAD);
        }
        // Cast prevention by loss-of-control flags
        // (fork Spell.cpp:6826-6838; mechanic-immunity
        // and stun-interrupt-flag nuances unmodeled).
        {
            let flags = self.unit_value_u32(caster, offsets::UNIT_FIELD_FLAGS);
            if flags & UNIT_FLAG_STUNNED != 0 {
                return fail(self, cast_result::FAILED_STUNNED);
            }
            if flags & UNIT_FLAG_CONFUSED != 0 {
                return fail(self, cast_result::FAILED_CONFUSED);
            }
            if flags & UNIT_FLAG_FLEEING != 0 {
                return fail(self, cast_result::FAILED_FLEEING);
            }
            if spell.prevention_type == 1 && flags & UNIT_FLAG_SILENCED != 0 {
                return fail(self, cast_result::FAILED_SILENCED);
            }
            if spell.prevention_type == 2 && flags & UNIT_FLAG_PACIFIED != 0 {
                return fail(self, cast_result::FAILED_PACIFIED);
            }
        }
        if !options.skip_known
            && options.cast_item.is_none()
            && !self.player_knows_spell(caster, spell_id)
        {
            return fail(self, cast_result::FAILED_UNKNOWN);
        }
        // The current-cast slot gate applies only to
        // non-triggered casts (fork prepare:
        // `!m_IsTriggeredSpell && IsNonMeleeSpellCasted`).
        if !options.triggered && self.casts.contains_key(&caster) {
            return fail(self, cast_result::FAILED_SPELL_IN_PROGRESS);
        }
        // Projectile spells (`speed > 0`) cast
        // normally; their per-target effects are
        // delayed at execution by distance/speed
        // (`SPELL_STATE_DELAYED`).
        // Charged spell modifiers dropped for this
        // cast (`m_appliedMods`).
        let mut applied_mods: Vec<(u32, u8)> = Vec::new();
        if !options.triggered {
            let now = self.now_ms;
            let gcd = self
                .unit_base_mut(caster)
                .is_some_and(|u| u.gcd_until_ms > now);
            if gcd || self.has_cooldown(caster, &spell, options.cast_item) {
                return fail(self, cast_result::FAILED_NOT_READY);
            }
            // `GetErrorAtShapeshiftedCast` strict gate
            // (`Spell::CheckCast`, non-triggered only).
            if let Some(reason) = self.shapeshift_cast_error(caster, &spell) {
                return fail(self, reason);
            }
            // Item casts are free
            // (`CalculatePowerCost` returns 0 with a
            // cast item); GCD + cooldown gates above
            // still run for them.
            if options.cast_item.is_none() {
                let cost = spell.power_cost(
                    self.unit_power(caster, spell.power_type),
                    self.unit_max_power(caster, spell.power_type),
                    self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
                    self.unit_base(caster).map_or(0, |u| u.create_mana),
                    self.unit_base(caster).map_or(0, |u| u.create_health),
                );
                // `SPELLMOD_COST` at the power check never
                // drops a charge (the take site does).
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let cost = self
                    .apply_spell_mod(
                        caster,
                        &spell,
                        spellmod::COST,
                        cost as f32,
                        false,
                        Some(&mut applied_mods),
                    )
                    .max(0.0) as u32;
                if self.unit_power(caster, spell.power_type) < cost {
                    return fail(self, cast_result::FAILED_NO_POWER);
                }
            }
        }
        // `CheckItems` reagent/totem arm (fork
        // `Spell::CheckCast` runs it for every
        // non-passive cast, triggered included; the
        // `IgnoreItemRequirements` rule inside decides
        // whether a triggered cast pays its own).
        if let Some(reason) = self.check_reagents(caster, &spell, &options) {
            return fail(self, reason);
        }
        // Finisher combo gate (`Spell::CheckCast`,
        // `Spell.cpp:7108`): a player finisher with an
        // explicitly selected unit target requires the
        // combo target to be that unit. Warriors read
        // `FAILED_BAD_TARGETS` (Overpower marking rides
        // the same path client-side).
        if caster.is_player() && spell.needs_combo_points() {
            let explicit = matches!(
                spell.effects[0].implicit_target_a,
                tgt::UNIT_ENEMY
                    | tgt::UNIT_FRIEND
                    | tgt::UNIT
                    | tgt::UNIT_PARTY
                    | tgt::UNIT_FRIEND_CHAIN_HEAL
                    | tgt::LOCATION_CASTER_TARGET_POSITION
                    | tgt::UNIT_RAID
                    | tgt::UNIT_RAID_AND_CLASS
            );
            if explicit {
                let unit_target = match target {
                    CastTarget::Unit(g) => Some(g),
                    CastTarget::SelfCast | CastTarget::Dest(_) | CastTarget::Corpse(_) => None,
                };
                if unit_target != Some(self.combo_target(caster)) {
                    let class =
                        (self.unit_value_u32(caster, offsets::UNIT_FIELD_BYTES_0) >> 8) & 0xFF;
                    return fail(
                        self,
                        if class == 1 {
                            cast_result::FAILED_BAD_TARGETS
                        } else {
                            cast_result::FAILED_NO_COMBO_POINTS
                        },
                    );
                }
            }
        }
        // TargetAuraState gate (`Spell::CheckCast`,
        // `Spell.cpp:5868-5875`): HEALTHLESS_20 requires
        // the unit target strictly below 20% health (the
        // fork maintains the bit in `SetHealth`; computed
        // live here — no aura-state field is maintained).
        if spell.target_aura_state == aura_state::HEALTHLESS_20_PERCENT {
            match target {
                CastTarget::Unit(victim) => {
                    let hp = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
                    let max = self.unit_value_u32(victim, offsets::UNIT_FIELD_MAXHEALTH);
                    #[allow(clippy::cast_precision_loss)]
                    let below = max > 0 && (hp as f32) < (max as f32) * 0.20;
                    if !below {
                        return fail(self, cast_result::FAILED_BAD_TARGETS);
                    }
                }
                CastTarget::SelfCast | CastTarget::Dest(_) | CastTarget::Corpse(_) => {
                    return fail(self, cast_result::FAILED_BAD_IMPLICIT_TARGETS);
                }
            }
        }
        // Behind-target gate (`Spell::CheckCast`,
        // `Spell.cpp:5772-5777`): a from-behind-only
        // spell fails `SPELL_FAILED_NOT_BEHIND` unless
        // the caster is behind the unit target
        // (`Unit::IsBehindTarget` strict: a creature
        // whose current victim is the caster counts as
        // facing it unless incapacitated or casting).
        if spell.is_from_behind_only() {
            if let CastTarget::Unit(victim) = target {
                let flags = self.unit_value_u32(victim, offsets::UNIT_FIELD_FLAGS);
                let incapacitated = flags
                    & (UNIT_FLAG_STUNNED
                        | UNIT_FLAG_CONFUSED
                        | UNIT_FLAG_FLEEING
                        | UNIT_FLAG_POSSESSED)
                    != 0
                    || self.unit_base(victim).is_some_and(|u| {
                        u.has_unit_state(
                            mangos_entities::unit::UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL,
                        )
                    });
                let creature_facing_caster = self.creatures.contains_key(&victim)
                    && self.unit_base(victim).and_then(|u| u.melee_victim) == Some(caster)
                    && !self.casts.contains_key(&victim)
                    && !incapacitated;
                let behind = !creature_facing_caster
                    && !self.has_in_arc(victim, caster, std::f32::consts::PI);
                if !behind {
                    return fail(self, cast_result::FAILED_NOT_BEHIND);
                }
            }
        }
        // Target validation (subset of CheckCast target
        // rules driven by effect 0's implicit target).
        if let CastOutcome::Failed(reason) =
            self.check_target(caster, &spell, target, &mut applied_mods, options.triggered)
        {
            return fail(self, reason);
        }
        // `SpellScript::OnCheckCast` (end of `CheckCast`).
        if let Some(reason) = self.spell_script_on_check_cast(caster, &spell, target) {
            return fail(self, reason);
        }
        // `BattleGround::CheckSpellCast` override
        // (AV shredder charms 21544/21565;
        // `SPELL_FAILED_SPELL_UNAVAILABLE` = 0x2f).
        if self.bg.as_ref().is_some_and(|b| b.bg_type_id == 1)
            && !self.av_check_spell_cast(caster, spell.id)
        {
            return fail(self, 0x2F);
        }

        // Triggered casts keep their DBC cast time
        // (fork `CalculateCastTime` has no triggered
        // zeroing; the spell simply shows no cast bar
        // and ticks outside the current-cast slot).
        let mut cast_time = {
            let raw = spell.cast_time_ms(&self.stores.spells.tables);
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            {
                self.apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::CASTING_TIME,
                    raw as f32,
                    true,
                    Some(&mut applied_mods),
                )
                .max(0.0) as u32
            }
        };
        // `SpellScript::OnSuccessfulStart` (fork
        // `Spell::Prepare`, `Spell.cpp:3585`).
        self.spell_script_on_successful_start(caster, &spell, &mut cast_time);
        let counter = self.unit_base_mut(caster).map_or(0, |u| {
            let c = u.cast_counter;
            u.cast_counter = u.cast_counter.wrapping_add(1);
            c
        });
        let position = self.unit_position(caster).unwrap_or_default();
        let from_proc = self.in_proc_cast;
        let (proc_attacker_flags, proc_victim_flags, can_trigger_procs) =
            crate::procs::spell_proc_context(&spell, options.triggered, from_proc);
        let cast = ActiveCast {
            spell_id,
            cast_counter: counter,
            target,
            remaining_ms: i32::try_from(cast_time).unwrap_or(i32::MAX),
            total_ms: cast_time,
            triggered: options.triggered,
            triggered_by_aura: options.triggered_by_aura,
            delay_count: 0,
            channeled: spell.is_channeled(),
            channeling: false,
            channel_cost_ms: 1000,
            applied_mods,
            proc_attacker_flags,
            proc_victim_flags,
            can_trigger_procs,
            from_proc,
            cast_item: options.cast_item,
            custom_base_points: options.custom_base_points,
            triggered_by_aura_spell: options.triggered_by_aura_spell,
            triggered_by_spell: options.triggered_by_spell,
            script_effect_pick,
            position,
            original_caster,
        };
        if cast_time > 0 && !options.triggered {
            self.send_spell_start(caster, &cast, &spell);
        }
        // Slot placement (fork prepare): a triggered
        // non-channeled cast never occupies the
        // current-cast slot — timer 0 executes in
        // this call point, timer > 0 ticks on the
        // pending list. Channeled casts take the
        // slot (`SetCurrentCastedSpell`).
        if options.triggered && !cast.channeled {
            if cast_time == 0 {
                self.execute_cast(caster, cast);
            } else {
                self.pending_triggered.push((caster, cast));
            }
            return CastOutcome::Started;
        }
        self.casts.insert(caster, cast);
        if cast_time == 0 {
            self.update_cast(caster, 0);
        }
        CastOutcome::Started
    }

    /// `Unit::CastSpell` with an explicit original
    /// caster (fork `Spell::m_originalCasterGUID`),
    /// used by the spirit-healer gossip arm.
    pub fn cast_spell_with_original_caster(
        &mut self,
        caster: ObjectGuid,
        original_caster: ObjectGuid,
        spell_id: u32,
        target: CastTarget,
        options: CastOptions,
    ) -> CastOutcome {
        let previous = self.next_original_caster.replace(original_caster);
        let outcome = self.cast_spell(caster, spell_id, target, options);
        if self.next_original_caster == Some(original_caster) {
            self.next_original_caster = previous;
        }
        outcome
    }

    /// `SPELL_EFFECT_SKINNING` branch (fork
    /// `Spell.cpp:6069-6094`): runs before every other
    /// target check — corpses fail the alive gate below
    /// by definition. The spell range still applies
    /// (fork checks range at cast; `SendLoot` skips it
    /// for skinning, `Player.cpp:7884`). Returns `None`
    /// for non-skinning spells.
    fn check_skinning_target(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
        applied_mods: &mut Vec<(u32, u8)>,
        triggered: bool,
    ) -> Option<CastOutcome> {
        if !spell.effects.iter().any(|e| e.effect == effect::SKINNING) {
            return None;
        }
        let CastTarget::Unit(victim) = target else {
            return Some(CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS));
        };
        let Some(vpos) = self.unit_position(victim) else {
            return Some(CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS));
        };
        let cpos = self.unit_position(caster).unwrap_or_default();
        let range = self.apply_spell_mod(
            caster,
            spell,
            spellmod::RANGE,
            spell.range_max(&self.stores.spells.tables),
            false,
            Some(applied_mods),
        ) + self.bounding_radius_of(caster)
            + self.bounding_radius_of(victim);
        let dist = ((cpos.x - vpos.x).powi(2) + (cpos.y - vpos.y).powi(2)).sqrt();
        if dist > range {
            return Some(CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE));
        }
        if let Some(reason) = self.skinning_cast_error(caster, victim, triggered) {
            return Some(CastOutcome::Failed(reason));
        }
        Some(CastOutcome::Started)
    }

    /// `SPELL_EFFECT_PICKPOCKET` branch (fork
    /// `Spell.cpp:6188-6198`): the victim-side pockets
    /// check. The alive/range/hostile gates stay in the
    /// generic unit arm below; this returns `None` for
    /// non-pickpocket spells and for victims that pass.
    /// Explicit non-goal: no `TARGET_IN_COMBAT` gate
    /// (compiled out at `SUPPORTED=5875`, 1.12.1 —
    /// usable in combat while stealthed).
    fn check_pickpocket_target(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<CastOutcome> {
        if !spell.effects.iter().any(|e| e.effect == effect::PICKPOCKET) {
            return None;
        }
        let CastTarget::Unit(victim) = target else {
            return Some(CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS));
        };
        if let Some(reason) = self.pickpocket_cast_error(caster, victim) {
            return Some(CastOutcome::Failed(reason));
        }
        None
    }

    /// `SPELL_EFFECT_SKIN_PLAYER_CORPSE` branch (fork
    /// `Spell.cpp:3305` + `:4870`): the effect carries
    /// its own corpse/dead-player resolution (fork
    /// `SpellEffects.cpp:5735-5760`) and has no
    /// range/LOS corpse gate — every other check
    /// (`CheckTarget` alive gates, `CheckRange`) would
    /// reject the corpse guid or the dead player, so the
    /// branch accepts any unit target here, exactly as
    /// the fork's pass-through group does.
    fn check_insignia_target(spell: &SpellEntry, target: CastTarget) -> Option<CastOutcome> {
        if !spell
            .effects
            .iter()
            .any(|e| e.effect == effect::SKIN_PLAYER_CORPSE)
        {
            return None;
        }
        let CastTarget::Unit(_) = target else {
            return Some(CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS));
        };
        Some(CastOutcome::Started)
    }

    /// Spell-loot victim gates (fork `Spell.cpp:6069-6094`
    /// skinning + `:6188-6198` pickpocket + `:3305`
    /// insignia): run before every other target check —
    /// corpses fail the alive gate below by definition,
    /// and the pickpocket pockets check keys on the
    /// victim only. Returns `None` for other spells and
    /// passing victims.
    fn check_spell_loot_target(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
        applied_mods: &mut Vec<(u32, u8)>,
        triggered: bool,
    ) -> Option<CastOutcome> {
        if let Some(outcome) = Self::check_insignia_target(spell, target) {
            return Some(outcome);
        }
        if let Some(outcome) = self.check_pickpocket_target(caster, spell, target) {
            return Some(outcome);
        }
        self.check_skinning_target(caster, spell, target, applied_mods, triggered)
    }

    #[allow(clippy::too_many_lines)] // one CheckCast-target port
    fn check_target(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
        applied_mods: &mut Vec<(u32, u8)>,
        triggered: bool,
    ) -> CastOutcome {
        if let Some(outcome) =
            self.check_spell_loot_target(caster, spell, target, applied_mods, triggered)
        {
            return outcome;
        }
        // Resurrect corpse arm (`Spell::CheckCast`
        // resurrect case, `Spell.cpp:5907-5920`): a
        // corpse target must resolve to a corpse on
        // this map (range to the corpse like the fork
        // CheckRange; LOS is unwired in this module).
        // Unit/self shapes keep the generic arms below
        // (the effect itself no-ops on living targets).
        if spell
            .effects
            .iter()
            .any(|e| matches!(e.effect, effect::RESURRECT | effect::RESURRECT_NEW))
            && let CastTarget::Corpse(corpse_guid) = target
        {
            let Some(corpse) = self.corpses.get(&corpse_guid) else {
                return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
            };
            let corpse_pos = corpse.world.position();
            let range = self.apply_spell_mod(
                caster,
                spell,
                spellmod::RANGE,
                spell.range_max(&self.stores.spells.tables),
                false,
                Some(applied_mods),
            ) + self.bounding_radius_of(caster);
            let Some(cpos) = self.unit_position(caster) else {
                return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
            };
            let dist = ((cpos.x - corpse_pos.x).powi(2) + (cpos.y - corpse_pos.y).powi(2)).sqrt();
            if range > 0.0 && dist > range {
                return CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE);
            }
            return CastOutcome::Started;
        }
        let mode = spell
            .effects
            .iter()
            .find(|e| e.effect != effect::NONE)
            .map_or(tgt::UNIT_CASTER, |e| e.implicit_target_a);
        // Target mode 0 (NONE in DBC) behaves as
        // caster-targeted for check purposes.
        match mode {
            0 | tgt::UNIT_CASTER
            | tgt::UNIT_CASTER_MASTER
            | tgt::UNIT_CASTER_PET
            | tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::UNIT_PARTY
            | tgt::UNIT_FRIEND_AND_PARTY
            // Destination-at-caster spells (summons):
            // the destination is the caster position in
            // the subset, so no unit target is required.
            // Caster-centered AoE enumerations likewise
            // start (victim expansion is a recorded
            // Slice 5 gap; effects land on the caster
            // target slot). `UNIT_CASTER_MASTER`
            // (Sacrifice) resolves to the owner or
            // nothing at fill time
            // (`Spell.cpp:2856`) with no check-time
            // failure; the caster's pet likewise
            // starts (fork skips pet-target-existence
            // checks, `Spell.cpp:7211`) while
            // execution resolves the pet server-side.
            // Player master/pet casts collect on the
            // caster/pet respectively.
            | tgt::LOCATION_CASTER_DEST
            | tgt::LOCATION_UNIT_MINION_POSITION
            | tgt::LOCATION_CASTER_SRC
            // Directional caster locations (Portal's
            // TRANS_DOOR at LOCATION_CASTER_FRONT):
            // the destination is computed server-side
            // from caster position + orientation, so
            // no explicit target is required and the
            // fork check passes.
            | tgt::LOCATION_CASTER_FRONT
            | tgt::LOCATION_CASTER_BACK
            | tgt::LOCATION_CASTER_LEFT
            | tgt::LOCATION_CASTER_RIGHT
            | tgt::LOCATION_CASTER_FRONT_LEFT
            | tgt::LOCATION_CASTER_FRONT_RIGHT
            | tgt::LOCATION_CASTER_BACK_LEFT
            | tgt::LOCATION_CASTER_BACK_RIGHT
            | tgt::LOCATION_CASTER_FRONT_LEAP
            | tgt::ENUM_UNITS_ENEMY_IN_CONE => CastOutcome::Started,
            // Ground enumerations: destination comes
            // from the client (or the caster for
            // scripted casts); range gates to the
            // destination like the fork's CheckRange.
            // The fishing spot (39) rides along: the
            // client sends the bobber destination and
            // the water gate runs in the effect itself
            // (fork `SpellEffects.cpp:5538-5678`), so a
            // non-dest target starts unchecked here and
            // the radius branch places it there.
            tgt::ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION
            | tgt::LOCATION_CASTER_FISHING_SPOT => {
                if let CastTarget::Dest(pos) = target {
                    let range = self.apply_spell_mod(
                        caster,
                        spell,
                        spellmod::RANGE,
                        spell.range_max(&self.stores.spells.tables),
                        false,
                        Some(applied_mods),
                    ) + self.bounding_radius_of(caster);
                    let Some(cpos) = self.unit_position(caster) else {
                        return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                    };
                    let dist = ((cpos.x - pos.x).powi(2) + (cpos.y - pos.y).powi(2)).sqrt();
                    if dist > range {
                        return CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE);
                    }
                }
                CastOutcome::Started
            }
            tgt::UNIT_ENEMY | tgt::UNIT_FRIEND | tgt::UNIT | tgt::UNIT_FRIEND_CHAIN_HEAL | tgt::UNIT_RAID | tgt::UNIT_RAID_AND_CLASS => {
                let CastTarget::Unit(victim) = target else {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_IMPLICIT_TARGETS);
                };
                let Some(vpos) = self.unit_position(victim) else {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                };
                let alive = self
                    .creatures
                    .get(&victim)
                    .is_some_and(|c| c.unit.is_alive())
                    || self
                        .players
                        .get(&victim)
                        .is_some_and(|p| p.player.unit.is_alive());
                if !alive {
                    return CastOutcome::Failed(cast_result::FAILED_TARGETS_DEAD);
                }
                // Creature-type gate (`CheckCast`
                // :5710): explicit non-caster targets
                // only; players get the dedicated
                // reason.
                if victim != caster && !self.check_target_creature_type(spell, victim) {
                    return CastOutcome::Failed(if victim.is_player() {
                        cast_result::FAILED_TARGET_IS_PLAYER
                    } else {
                        cast_result::FAILED_BAD_TARGETS
                    });
                }
                // Positive-spell immunity gate
                // (`Spell::CheckTarget`).
                #[allow(clippy::cast_possible_wrap)]
                if victim != caster
                    && spell.is_positive_spell()
                    && (self.is_immune_to_spell(victim, spell, false)
                        || self.is_immune_to_damage(
                            victim,
                            1i32 << spell.school,
                            Some(spell),
                        ))
                {
                    return CastOutcome::Failed(cast_result::FAILED_TARGET_AURASTATE);
                }
                if mode == tgt::UNIT_ENEMY && !Self::is_hostile_pair(caster, victim) {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                // Chain heal is an explicit friendly unit target
                // (fork `SpellEntry.h` IsExplicitPositiveTarget):
                // the single-target component is the unit target.
                if (mode == tgt::UNIT_FRIEND || mode == tgt::UNIT_FRIEND_CHAIN_HEAL || mode == tgt::UNIT_RAID || mode == tgt::UNIT_RAID_AND_CLASS)
                    && Self::is_hostile_pair(caster, victim)
                {
                    return CastOutcome::Failed(cast_result::FAILED_BAD_TARGETS);
                }
                let cpos = self.unit_position(caster).unwrap_or_default();
                // `SPELLMOD_RANGE` (`CheckRange`; range
                // checks never drop charges).
                let range = self.apply_spell_mod(
                    caster,
                    spell,
                    spellmod::RANGE,
                    spell.range_max(&self.stores.spells.tables),
                    false,
                    Some(applied_mods),
                ) + self.bounding_radius_of(caster)
                    + self.bounding_radius_of(victim);
                let dist = ((cpos.x - vpos.x).powi(2) + (cpos.y - vpos.y).powi(2)).sqrt();
                if dist > range {
                    return CastOutcome::Failed(cast_result::FAILED_OUT_OF_RANGE);
                }
                CastOutcome::Started
            }
            _ => CastOutcome::Failed(cast_result::FAILED_UNKNOWN),
        }
    }

    fn targets_write(target: CastTarget, caster: ObjectGuid, payload: &mut PacketWriter) {
        // `SpellCastTargets::write` subset: u16 mask +
        // packed unit guid.
        match target {
            CastTarget::Unit(guid) => {
                payload.write_u16_le(0x0002); // TARGET_FLAG_UNIT
                payload.write_bytes(&Self::packed_guid_bytes(guid));
            }
            CastTarget::SelfCast => {
                payload.write_u16_le(0x0002);
                payload.write_bytes(&Self::packed_guid_bytes(caster));
            }
            CastTarget::Dest(pos) => {
                payload.write_u16_le(0x0040); // TARGET_FLAG_DEST_LOCATION
                payload.write_f32_le(pos.x);
                payload.write_f32_le(pos.y);
                payload.write_f32_le(pos.z);
            }
            CastTarget::Corpse(guid) => {
                payload.write_u16_le(0x8000); // TARGET_FLAG_CORPSE_ALLY
                payload.write_bytes(&Self::packed_guid_bytes(guid));
            }
        }
    }

    fn send_spell_start(&mut self, caster: ObjectGuid, cast: &ActiveCast, spell: &SpellEntry) {
        let target_guid = match cast.target {
            CastTarget::Unit(g) | CastTarget::Corpse(g) => g,
            CastTarget::SelfCast | CastTarget::Dest(_) => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(Self::cast_first_guid(
            caster, cast,
        )));
        payload.write_bytes(&Self::packed_guid_bytes(target_guid));
        payload.write_u8(cast.cast_counter);
        payload.write_u32_le(spell.id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(cast.total_ms);
        Self::targets_write(cast.target, caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_START, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// `InterruptSpell` + cancel-cast: emits the failure
    /// packets for timed casts.
    pub fn cancel_cast(&mut self, caster: ObjectGuid, interrupted: bool) {
        // Channel interrupt closes the bar via
        // `MSG_CHANNEL_UPDATE(0)` in `end_channel`
        // (no SPELL_FAILURE family).
        if self.casts.get(&caster).is_some_and(|c| c.channeling) {
            self.end_channel(caster, true);
            return;
        }
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        if cast.total_ms == 0 {
            return;
        }
        if interrupted {
            self.send_cast_result(caster, cast.spell_id, cast_result::FAILED_INTERRUPTED);
        }
        // SMSG_SPELL_FAILURE (caster session) +
        // SMSG_SPELL_FAILED_OTHER (set): raw GUID + id
        // (+ reason on FAILURE).
        let mut payload = PacketWriter::new();
        payload.write_u64_le(caster.raw());
        payload.write_u32_le(cast.spell_id);
        if interrupted {
            let mut with_reason = payload.clone();
            with_reason.write_u8(cast_result::FAILED_INTERRUPTED);
            let packet = WorldPacket::with_body(op::SMSG_SPELL_FAILURE, with_reason.into_vec());
            self.send_to_player(caster, packet);
        }
        let packet = WorldPacket::with_body(op::SMSG_SPELL_FAILED_OTHER, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// `SpellCaster::InterruptNonMeleeSpells`
    /// (`SpellCaster.cpp:1783-1796`): interrupts the
    /// single-slot cast when present, filtered by
    /// `spell_id` (0 = any). Divergences from the fork,
    /// all recorded: `with_delayed` is a no-op (there is
    /// no delayed spell state in this subset — the
    /// `delay_spell_at_damage` comment shows the fork
    /// delayed arm is unreachable, and `DelayedSpellHit`
    /// is a separate hit list, matching the fork where
    /// delayed hits are also not slot-interrupted);
    /// there is no autorepeat or melee slot (hunter
    /// auto-shot lives elsewhere); channeled casts close
    /// via `end_channel` with no `SMSG_SPELL_FAILED_OTHER`
    /// (consistent with the existing channel convention);
    /// no GCD is reset (the GCD is added at execution,
    /// so a preparing cast has none to clear — the same
    /// outcome as the fork's `ResetGCD`).
    /// `WorldSession::HandleCancelChanneling`
    /// (`SpellHandler.cpp:491`): interrupt the current
    /// channeled spell (spell id in the packet is
    /// unused); triggered channels are immune. The
    /// remote-control mover arm has no Rust slot.
    pub fn interrupt_channeling(&mut self, caster: ObjectGuid) {
        let live = self
            .casts
            .get(&caster)
            .is_some_and(|cast| cast.channeling && !cast.triggered);
        if live {
            self.cancel_cast(caster, true);
        }
    }

    pub fn interrupt_non_melee_spells(
        &mut self,
        caster: ObjectGuid,
        with_delayed: bool,
        spell_id: u32,
    ) {
        let _ = with_delayed;
        let hit = self
            .casts
            .get(&caster)
            .is_some_and(|cast| spell_id == 0 || cast.spell_id == spell_id);
        if !hit {
            return;
        }
        self.cancel_cast(caster, true);
    }

    /// `Unit::InterruptSpellsCastedOnMe`
    /// (`Unit.cpp:10238-10268`): interrupts every
    /// non-melee cast whose unit target is `target`.
    /// Fork-flag mapping: the delayed-event half of
    /// `killDelayed` is a no-op (no delayed spell state
    /// in this subset); the friendliness skip
    /// (`interruptPositiveSpells=false`) rides the
    /// `is_hostile_pair` subset; the stalked exemption
    /// (`onlyIfNotStalked`) is unmodeled — no stalked
    /// tracking exists. Range is the fork's 100m
    /// object-range check.
    pub(crate) fn interrupt_spells_casted_on_me(&mut self, target: ObjectGuid, kill_delayed: bool) {
        let _ = kill_delayed;
        let Some(center) = self.unit_position(target) else {
            return;
        };
        let casters: Vec<ObjectGuid> = self
            .casts
            .iter()
            .filter_map(|(caster, cast)| {
                if !matches!(cast.target, CastTarget::Unit(victim) if victim == target) {
                    return None;
                }
                if !Self::is_hostile_pair(*caster, target) {
                    return None;
                }
                let in_range = self.unit_position(*caster).is_some_and(|pos| {
                    let dx = pos.x - center.x;
                    let dy = pos.y - center.y;
                    let dz = pos.z - center.z;
                    dx * dx + dy * dy + dz * dz <= 100.0 * 100.0
                });
                in_range.then_some(*caster)
            })
            .collect();
        for caster in casters {
            self.interrupt_non_melee_spells(caster, true, 0);
        }
    }

    /// The shared pushback resist total
    /// (`GetTotalAuraModifier(SPELL_AURA_RESIST_PUSHBACK)`;
    /// the fork adds it to the spellmod-adjusted 100
    /// then subtracts 100, and spell mods are identity
    /// in this subset).
    fn pushback_resist_chance(&self, victim: ObjectGuid) -> i32 {
        self.aura_holders(victim)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| e.aura_type == aura_type::RESIST_PUSHBACK)
            .map(|e| e.amount)
            .sum()
    }

    /// The fork pushback resist roll shared by
    /// `Delayed`/`DelayedChannel`: 100 through
    /// `SPELLMOD_NOT_LOSE_CASTING_TIME`, plus the
    /// `RESIST_PUSHBACK` aura total, minus 100,
    /// vs `irand(0, 99)`.
    fn pushback_resisted(&mut self, victim: ObjectGuid, spell: &SpellEntry) -> bool {
        let mut applied = self
            .casts
            .get_mut(&victim)
            .map_or_else(Vec::new, |c| std::mem::take(&mut c.applied_mods));
        #[allow(clippy::cast_possible_truncation)]
        let modded = self.apply_spell_mod(
            victim,
            spell,
            spellmod::NOT_LOSE_CASTING_TIME,
            100.0,
            true,
            Some(&mut applied),
        ) as i32;
        if let Some(c) = self.casts.get_mut(&victim) {
            c.applied_mods = applied;
        }
        let resist_chance = modded + self.pushback_resist_chance(victim) - 100;
        resist_chance > self.spell_rng().irand(0, 99)
    }

    /// Damage-taken cast interference (the
    /// `DealDamage` current-spell loop, `Unit.cpp`
    /// ~896): channeled casts take the
    /// `DelayedChannel`/channel-cancel arm; preparing
    /// player casts take `Spell::Delayed`
    /// (`Spell.cpp:7686`) — `DAMAGE_CANCELS` interrupts,
    /// otherwise pushback gated by `DAMAGE_PUSHBACK`
    /// and the resist roll. The fork delayed-state arm
    /// is unreachable (no projectile delay), and
    /// creature casts are never pushed.
    pub(super) fn delay_spell_at_damage(&mut self, attacker: ObjectGuid, victim: ObjectGuid) {
        let Some((spell_id, channeling)) =
            self.casts.get(&victim).map(|c| (c.spell_id, c.channeling))
        else {
            return;
        };
        if channeling {
            let Some(spell) = self.stores.spells.get(spell_id) else {
                return;
            };
            let cflags = spell.channel_interrupt_flags;
            if cflags & aura_interrupt_flag::DAMAGE_CHANNEL_DURATION != 0 {
                // Self-damage never shortens a channel,
                // and `DelayedChannel` is players-only.
                if attacker != victim && self.players.contains_key(&victim) {
                    self.delay_channel(victim);
                }
            } else if cflags & aura_interrupt_flag::DAMAGE_CANCELS != 0 {
                self.end_channel(victim, true);
            }
            return;
        }
        if !self.players.contains_key(&victim) {
            return;
        }
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        let flags = spell.interrupt_flags;
        if flags & spell_interrupt_flag::DAMAGE_CANCELS != 0 {
            self.cancel_cast(victim, true);
            return;
        }
        if flags & spell_interrupt_flag::DAMAGE_PUSHBACK == 0 {
            return;
        }
        if self.pushback_resisted(victim, &spell) {
            return;
        }
        let Some(cast) = self.casts.get_mut(&victim) else {
            return;
        };
        let mut delay_ms = next_delay_at_damage_ms(&mut cast.delay_count);
        #[allow(clippy::cast_possible_wrap)]
        let total_ms = cast.total_ms as i32;
        if cast.remaining_ms + delay_ms > total_ms {
            delay_ms = total_ms - cast.remaining_ms;
            cast.remaining_ms = total_ms;
        } else {
            cast.remaining_ms += delay_ms;
        }
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        #[allow(clippy::cast_sign_loss)]
        payload.write_u32_le(delay_ms as u32);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_DELAYED, payload.into_vec());
        self.send_to_player(victim, packet);
    }

    /// `Spell::DelayedChannel` (`Spell.cpp:7725`):
    /// shorten the channel and its holders by the
    /// scheduled delay, then refresh the client bar
    /// (or interrupt when nothing remains).
    fn delay_channel(&mut self, victim: ObjectGuid) {
        let Some(spell_id) = self.casts.get(&victim).map(|c| c.spell_id) else {
            return;
        };
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        if self.pushback_resisted(victim, &spell) {
            return;
        }
        let Some((spell_id, target, remaining_ms, delay_ms)) =
            self.casts.get_mut(&victim).map(|cast| {
                let mut delay_ms = next_delay_at_damage_ms(&mut cast.delay_count);
                if cast.remaining_ms < delay_ms {
                    delay_ms = cast.remaining_ms;
                    cast.remaining_ms = 0;
                } else {
                    cast.remaining_ms -= delay_ms;
                }
                (cast.spell_id, cast.target, cast.remaining_ms, delay_ms)
            })
        else {
            return;
        };
        if let CastTarget::Unit(unit) = target
            && unit != victim
        {
            self.delay_aura_holders(unit, spell_id, victim, delay_ms);
        }
        self.delay_aura_holders(victim, spell_id, victim, delay_ms);
        if remaining_ms <= 0 {
            self.end_channel(victim, true);
        } else {
            let mut payload = PacketWriter::new();
            #[allow(clippy::cast_sign_loss)]
            payload.write_u32_le(remaining_ms as u32);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_UPDATE, payload.into_vec());
            self.send_to_player(victim, packet);
        }
    }

    /// `Unit::DelaySpellAuraHolder` +
    /// `RefreshAuraPeriodicTimers`/`UpdatePeriodicTimer`:
    /// shorten matching holders and realign their
    /// periodic timers to the new duration so the
    /// remaining tick count still divides evenly.
    fn delay_aura_holders(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        caster: ObjectGuid,
        delay_ms: i32,
    ) {
        let mut duration_packets: Vec<(u8, i32)> = Vec::new();
        if let Some(unit) = self.unit_base_mut(victim) {
            for holder in &mut unit.auras {
                if holder.spell_id != spell_id || holder.caster != caster || holder.duration_ms <= 0
                {
                    continue;
                }
                holder.duration_ms = if holder.duration_ms < delay_ms {
                    0
                } else {
                    holder.duration_ms - delay_ms
                };
                for eff in holder.effects.iter_mut().flatten() {
                    if !eff.is_periodic() || eff.period_ms == 0 {
                        continue;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    let period = eff.period_ms as i32;
                    let mut timer = if holder.duration_ms > period {
                        holder.duration_ms % period
                    } else {
                        holder.duration_ms
                    };
                    if timer == 0 {
                        timer = period;
                    }
                    eff.periodic_timer = timer;
                }
                duration_packets.push((holder.slot, holder.duration_ms));
            }
        }
        for (slot, duration) in duration_packets {
            self.send_aura_duration(victim, slot, duration);
        }
    }

    /// `Spell::update` + `Spell::cast` subset.
    #[allow(clippy::too_many_lines)] // one cast() port
    pub(crate) fn update_cast(&mut self, caster: ObjectGuid, diff_ms: u32) {
        if self.casts.get(&caster).is_some_and(|c| c.channeling) {
            self.update_channel(caster, diff_ms);
            return;
        }
        let done = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                cast.remaining_ms <= 0
            }
            None => return,
        };
        if !done {
            return;
        }
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        self.execute_cast(caster, cast);
    }

    /// Fork per-target projectile flight time
    /// (`floor(dist / speed * 1000)`, center distance
    /// clamped to at least 5; self hits never delay).
    fn projectile_delay_ms(&self, caster: ObjectGuid, victim: ObjectGuid, speed: f32) -> i32 {
        if speed <= 0.0 || victim == caster {
            return 0;
        }
        let (Some(from), Some(to)) = (self.unit_position(caster), self.unit_position(victim))
        else {
            return 0;
        };
        let dist = ((from.x - to.x).powi(2) + (from.y - to.y).powi(2) + (from.z - to.z).powi(2))
            .sqrt()
            .max(5.0);
        #[allow(clippy::cast_possible_truncation)]
        let ms = (dist / speed * 1000.0).floor() as i32;
        ms
    }

    /// Land one projectile-delayed hit
    /// (`handle_delayed` -> `DoAllEffectOnTarget`),
    /// then the cast's `finish()` tail on its last
    /// scheduled hit.
    fn apply_delayed_hit(&mut self, mut hit: DelayedSpellHit) {
        let Some(spell) = self.stores.spells.get(hit.spell_id).cloned() else {
            return;
        };
        // The fork keeps the spell while its caster
        // is in the world (a dead caster's projectile
        // still lands); a caster removed from the map
        // deletes the spell, and a removed victim
        // drops just this hit.
        if self.unit_base(hit.caster).is_none() && !self.gameobjects.contains_key(&hit.caster) {
            return;
        }
        if self.unit_base(hit.victim).is_some() {
            // Delayed hits never carry the aborting
            // arm (channeled casts land immediately),
            // so the flag is dropped here.
            let _ = self.apply_effects(
                hit.caster,
                hit.original_caster.unwrap_or(hit.caster),
                hit.caster_level,
                &spell,
                hit.victim,
                match hit.target {
                    CastTarget::Dest(pos) => Some(pos),
                    _ => None,
                },
                Self::script_unit_target(hit.target),
                Some(&mut hit.applied_mods),
                Some(&hit.ctx),
                hit.custom_base_points,
                hit.cast_item,
                hit.cast_counter,
                hit.triggered_by_aura_spell,
                hit.triggered_by_spell,
                hit.script_effect_pick,
            );
        }
        if hit.finish {
            self.clear_combo_on_spell_finish(hit.caster, &spell, hit.missed_non_caster);
            self.spell_script_on_successful_finish(hit.caster, &spell, hit.target);
            self.remove_exhausted_spell_mods(hit.caster, &hit.applied_mods);
        }
    }

    /// The `cast()` execution body for a cast whose
    /// timer has elapsed (also run inline for
    /// slot-less triggered casts, fork prepare:
    /// triggered timer 0 executes in the call point).
    #[allow(clippy::too_many_lines)] // one cast() port (split from update_cast)
    /// `IgnoreItemRequirements` (`Spell.cpp:7141`):
    /// a triggered cast ignores item requirements
    /// unless its master spell exists and carries no
    /// reagent in slot 0 — then the triggered cast
    /// pays its own. The traded-item-target arm has
    /// no row flag here.
    fn ignore_item_requirements(&self, triggered: bool, triggered_by_spell: u32) -> bool {
        if !triggered {
            return false;
        }
        !(triggered_by_spell != 0
            && self
                .stores
                .spells
                .get(triggered_by_spell)
                .is_some_and(|master| master.reagents[0] == 0))
    }

    /// The reagent + totem-item arm of `CheckItems`
    /// (`Spell.cpp:7315`): player casters only,
    /// passive spells skip (fork `CheckCast` gate).
    /// Missing reagent -> `ITEM_NOT_READY`; missing
    /// totem item -> `ITEM_GONE`. The cast-item
    /// charge nuance is unmodeled (`CastItemCtx`
    /// carries no charges), so a cast item counts
    /// toward its own reagent like any stack.
    fn check_reagents(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        options: &CastOptions,
    ) -> Option<u8> {
        const ATTR_PASSIVE: u32 = 0x0000_0040;
        if spell.attributes & ATTR_PASSIVE != 0 {
            return None;
        }
        let player = self.player(caster)?;
        if self.ignore_item_requirements(options.triggered, options.triggered_by_spell) {
            return None;
        }
        for (i, reagent) in spell.reagents.iter().enumerate() {
            if *reagent <= 0 {
                continue;
            }
            if player.quest_item_count(*reagent as u32) < spell.reagent_counts[i] {
                return Some(cast_result::FAILED_ITEM_NOT_READY);
            }
        }
        for totem in spell.totems {
            if totem != 0 && player.quest_item_count(totem) == 0 {
                return Some(cast_result::FAILED_ITEM_GONE);
            }
        }
        None
    }

    /// `CheckTargetCreatureType` (`Spell.cpp:7646`):
    /// aura 8179 exempts the target; Curse of Doom
    /// (603) refuses player-owned targets outright
    /// and widens to all types; Dismiss Pet (2641)
    /// and Taming Lesson (23356) clear the mask.
    /// A zero target mask (creature type NONE)
    /// passes any spell mask.
    pub(crate) fn check_target_creature_type(
        &self,
        spell: &SpellEntry,
        target: ObjectGuid,
    ) -> bool {
        if self.aura_holders(target).iter().any(|h| h.spell_id == 8179) {
            return true;
        }
        let mut mask = spell.target_creature_type;
        if spell.id == 603 {
            // `GetCharmerOrOwnerPlayerOrPlayerItself`
            // via the SUMMONEDBY/CHARMEDBY fields.
            let player_owned = target.is_player()
                || self.unit_base(target).is_some_and(|unit| {
                    let values = unit.world.object.values();
                    let owner = values.get_guid(offsets::UNIT_FIELD_SUMMONEDBY);
                    let charmer = values.get_guid(offsets::UNIT_FIELD_CHARMEDBY);
                    owner.is_player() || charmer.is_player()
                });
            if player_owned {
                return false;
            }
            mask = 0x7FF;
        }
        if spell.id == 2641 || spell.id == 23356 {
            mask = 0;
        }
        if mask == 0 {
            return true;
        }
        let target_mask = self.creature_type_mask(target);
        target_mask == 0 || (i32::from_ne_bytes(mask.to_ne_bytes()) & target_mask) != 0
    }

    /// `TakeReagents` (`Spell.cpp:5257`): destroy the
    /// reagent counts from the player caster's
    /// inventory; the actor persists the row changes
    /// from the emitted event.
    fn take_reagents(&mut self, caster: ObjectGuid, spell: &SpellEntry, cast: &ActiveCast) {
        if !caster.is_player()
            || self.ignore_item_requirements(cast.triggered, cast.triggered_by_spell)
        {
            return;
        }
        let mut changes: Vec<(u32, u32)> = Vec::new();
        for (i, reagent) in spell.reagents.iter().enumerate() {
            if *reagent <= 0 || spell.reagent_counts[i] == 0 {
                continue;
            }
            if let Some(player) = self.player_mut(caster) {
                changes.extend(player.destroy_item_count(*reagent as u32, spell.reagent_counts[i]));
            }
        }
        if !changes.is_empty() {
            self.events.push(MapEvent::ItemCountsChanged {
                player: caster,
                changes,
            });
        }
    }

    #[allow(clippy::too_many_lines)] // one cast() port
    pub(crate) fn execute_cast(&mut self, caster: ObjectGuid, mut cast: ActiveCast) {
        let Some(spell) = self.stores.spells.get(cast.spell_id).cloned() else {
            return;
        };
        let alive = self.unit_base_mut(caster).is_some_and(|u| u.is_alive());
        // Triggered casts complete even from a dead
        // caster (death-event spells).
        if !alive && !cast.triggered {
            return;
        }

        // TakePower + GCD + cooldowns at cast(),
        // fork split: `TakePower` early-returns only
        // for cast-item and aura-triggered casts (a
        // plain triggered cast still pays, unchecked);
        // GCD is added only for non-triggered casts
        // (prepare); `SendSpellCooldown` adds the
        // spell/category cooldown for every cast.
        if cast.cast_item.is_none()
            && !cast.triggered_by_aura
            && (caster.is_player() || caster.is_creature_or_pet())
        {
            let power_cost = spell.power_cost(
                self.unit_power(caster, spell.power_type),
                self.unit_max_power(caster, spell.power_type),
                self.unit_value_u32(caster, offsets::UNIT_FIELD_HEALTH),
                self.unit_base(caster).map_or(0, |u| u.create_mana),
                self.unit_base(caster).map_or(0, |u| u.create_health),
            );
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let power_cost = self
                .apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::COST,
                    power_cost as f32,
                    true,
                    Some(&mut cast.applied_mods),
                )
                .max(0.0) as u32;
            let current = self.unit_power(caster, spell.power_type);
            self.unit_set_power(caster, spell.power_type, current.saturating_sub(power_cost));
            // `Spell::TakePower` five-second timer
            // (`Spell.cpp:5252-5255`): a positive mana
            // spend without
            // `SPELL_ATTR_EX2_DONT_BLOCK_MANA_REGEN`
            // (0x02000000) arms `SetLastManaUse`.
            if spell.power_type as usize == mangos_entities::unit::POWER_MANA
                && power_cost > 0
                && spell.attributes_ex2 & 0x0200_0000 == 0
                && let Some(u) = self.unit_base_mut(caster)
            {
                u.set_last_mana_use(spell.id);
            }
        }
        if !cast.triggered && spell.start_recovery_time > 0 {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let gcd = self
                .apply_spell_mod(
                    caster,
                    &spell,
                    spellmod::GLOBAL_COOLDOWN,
                    spell.start_recovery_time as f32,
                    true,
                    Some(&mut cast.applied_mods),
                )
                .max(0.0) as u32;
            let until = self.now_ms + gcd;
            if let Some(u) = self.unit_base_mut(caster) {
                u.gcd_until_ms = until;
            }
        }
        if let Some(item) = cast.cast_item {
            self.add_cooldowns_item(caster, &spell, item, &mut cast.applied_mods);
        } else {
            self.add_cooldowns(caster, &spell, &mut cast.applied_mods, None);
        }
        // `TakeReagents` (fork `Spell::cast`, before
        // `HandleEffects` so a crafted item can land
        // in the freed slot).
        self.take_reagents(caster, &spell, &cast);
        // `SpellScript::OnCast` (fork `Spell::cast`
        // after TakePower/TakeReagents,
        // `Spell.cpp:3827`).
        self.spell_script_on_cast(caster, &spell, cast.target);

        let victims = self.collect_targets(caster, &spell, cast.target);
        // `EffectPersistentAA` runs once per cast at
        // the destination (fork executes it from the
        // location target list, not per unit victim).
        if let Some(eff_index) = spell
            .effects
            .iter()
            .position(|e| e.effect == effect::PERSISTENT_AREA_AURA)
        {
            let dest = self.cast_dest(caster, &spell, cast.target);
            self.spawn_dynobject(
                caster,
                &spell,
                eff_index,
                dest,
                Some(&mut cast.applied_mods),
            );
        }
        let caster_level = self.unit_level(caster);
        let mut hits: Vec<ObjectGuid> = Vec::new();
        let mut misses: Vec<(ObjectGuid, u8)> = Vec::new();
        let mut hit_victims: Vec<ObjectGuid> = Vec::new();
        for victim in &victims {
            // Spell-level immunity (`Spell::TakeTarget`
            // missCondition): damage-immune (hostile
            // only) or spell-immune -> SPELL_MISS_IMMUNE.
            #[allow(clippy::cast_possible_wrap)]
            let school_mask = 1i32 << spell.school;
            if *victim != caster
                && ((!spell_is_positive_for_target(&spell)
                    && self.is_immune_to_damage(*victim, school_mask, Some(&spell)))
                    || self.is_immune_to_spell(*victim, &spell, false))
            {
                misses.push((*victim, 7)); // SPELL_MISS_IMMUNE
                continue;
            }
            if spell_meleedamage(spell.dmg_class) {
                // DoMeleeAttackIfReady-style hit roll.
                let hit_chance = self.melee_spell_hit_chance(caster, *victim, &spell);
                let roll = self.spell_rng().frand01() * 100.0;
                if roll < hit_chance {
                    hits.push(*victim);
                    hit_victims.push(*victim);
                } else {
                    misses.push((*victim, 1)); // SPELL_MISS_MISS
                }
            } else if spell_is_positive_for_target(&spell) {
                hits.push(*victim);
                hit_victims.push(*victim);
            } else {
                let hit_chance = self.magic_hit_chance(caster, *victim, &spell);
                let roll = self.spell_rng().frand01() * 100.0;
                if roll < hit_chance {
                    hits.push(*victim);
                    hit_victims.push(*victim);
                } else {
                    misses.push((*victim, 2)); // SPELL_MISS_RESIST
                }
            }
        }

        self.send_spell_go(caster, &cast, &spell, &hits, &misses);
        // `SpellScript::OnHit` miss path (fork
        // `Spell.cpp:1584`).
        if cast.can_trigger_procs {
            for (victim, _) in &misses {
                self.spell_script_on_hit(caster, &spell, *victim, false);
            }
        }
        // `Spell::DoSpellHit` miss arm for pickpocket
        // (fork `Spell.cpp:1393-1417`): a failed
        // pickpocket breaks stealth and pulls aggro; a
        // hit keeps stealth. Gated on the effect, not
        // the DBC attribute row (the slice keys on
        // effect 71 + unit target, DBC-agnostic).
        if spell.effects.iter().any(|e| e.effect == effect::PICKPOCKET) && !misses.is_empty() {
            self.remove_spells_causing_aura(caster, aura_type::MOD_STEALTH);
            for (victim, _) in &misses {
                if victim.is_creature() && self.creatures.contains_key(victim) {
                    self.enter_combat(*victim, caster);
                }
            }
        }
        // `TakeCastItem` runs in `cast()` right after
        // `SendSpellGo` (the item must outlive the GO
        // packet's cast-item GUID); triggered casts
        // skip it outside trade-item targets, which
        // are unmodeled.
        if let Some(item) = cast.cast_item
            && !cast.triggered
            && caster.is_player()
        {
            self.take_cast_item(caster, item);
        }
        // Cast-end proc raise (fork raises it in
        // `cast()` before effect application).
        if cast.can_trigger_procs {
            let main_target = victims.first().copied();
            let ex = if main_target.is_none_or(|t| hit_victims.contains(&t)) {
                proc_ex::CAST_END | proc_ex::NORMAL_HIT
            } else {
                proc_ex::CAST_END | proc_ex::MISS
            };
            let args = ProcArgs {
                target: main_target,
                attacker_flags: cast.proc_attacker_flags,
                victim_flags: proc_flag::NONE,
                proc_ex: ex,
                amount: 1,
                original_amount: 1,
                att_type: if spell.dmg_class == dmg_class::RANGED {
                    proc_att_type::RANGED
                } else {
                    proc_att_type::BASE
                },
                proc_spell_id: Some(spell.id),
                triggered_by_aura: cast.from_proc,
                proc_time_ms: self.now_ms,
                applied_mods: &cast.applied_mods,
            };
            self.proc_damage_and_spell(caster, args);
        }
        let ctx = ProcRaiseCtx {
            attacker_flags: cast.proc_attacker_flags,
            victim_flags: cast.proc_victim_flags,
            can_trigger: cast.can_trigger_procs,
            triggered_by_aura: cast.from_proc,
            att_type: if spell.dmg_class == dmg_class::RANGED {
                proc_att_type::RANGED
            } else {
                proc_att_type::BASE
            },
            applied_mods: cast.applied_mods.clone(),
        };
        // Projectile delay (`SPELL_STATE_DELAYED`):
        // hits with a flight time are scheduled and
        // land in `update_casts`; the rest apply now.
        // Channeled casts keep the immediate path (no
        // channeled spell in the modeled subset has a
        // projectile speed).
        // `needDrop` input for the finisher tail
        // (`Spell.cpp:4467-4478`): any non-caster miss
        // keeps the combo points.
        let missed_non_caster = misses.iter().any(|(missed, _)| *missed != caster);
        let mut delayed: Vec<DelayedSpellHit> = Vec::new();
        for victim in hit_victims {
            let delay = if cast.channeled {
                0
            } else {
                self.projectile_delay_ms(caster, victim, spell.speed)
            };
            if delay > 0 {
                delayed.push(DelayedSpellHit {
                    caster,
                    spell_id: spell.id,
                    caster_level,
                    victim,
                    target: cast.target,
                    applied_mods: cast.applied_mods.clone(),
                    ctx: ctx.clone(),
                    custom_base_points: cast.custom_base_points,
                    cast_item: cast.cast_item,
                    cast_counter: cast.cast_counter,
                    triggered_by_aura_spell: cast.triggered_by_aura_spell,
                    triggered_by_spell: cast.triggered_by_spell,
                    script_effect_pick: cast.script_effect_pick,
                    original_caster: cast.original_caster,
                    remaining_ms: delay,
                    finish: false,
                    missed_non_caster,
                });
                continue;
            }
            if !self.apply_effects(
                caster,
                cast.original_caster.unwrap_or(caster),
                caster_level,
                &spell,
                victim,
                match cast.target {
                    CastTarget::Dest(pos) => Some(pos),
                    _ => None,
                },
                Self::script_unit_target(cast.target),
                Some(&mut cast.applied_mods),
                Some(&ctx),
                cast.custom_base_points,
                cast.cast_item,
                cast.cast_counter,
                cast.triggered_by_aura_spell,
                cast.triggered_by_spell,
                cast.script_effect_pick,
            ) {
                // Fork `finish()` inside an effect (the
                // fishing water gate): the arm already
                // sent the fail packet; the cast ends
                // here with no channel and no
                // successful-finish tail.
                return;
            }
        }
        // Start channeling (the `handle_immediate`
        // tail). Effects are already applied in this
        // subset, so `MSG_CHANNEL_START` follows their
        // packets where the fork sends it first.
        if cast.channeled && !cast.channeling {
            self.start_channel(caster, cast, &spell, &hits);
        } else if let Some(last) = delayed.last_mut() {
            // `finish()` runs when the last delayed
            // target lands (fork `handle_delayed`).
            last.finish = true;
            self.pending_delayed.extend(delayed);
        } else {
            // `SpellScript::OnSuccessfulFinish`
            // (fork `Spell::finish`, `Spell.cpp:4519`)
            // with the finishing-move combo tail.
            self.clear_combo_on_spell_finish(caster, &spell, missed_non_caster);
            self.spell_script_on_successful_finish(caster, &spell, cast.target);
            self.remove_exhausted_spell_mods(caster, &cast.applied_mods);
        }
    }

    /// `SendChannelStart` + channel state entry
    /// (`Spell.cpp:4002-4012, 5138`): fields, direct
    /// start packet to a player caster, cast retained
    /// with the channel duration as its timer.
    fn start_channel(
        &mut self,
        caster: ObjectGuid,
        mut cast: ActiveCast,
        spell: &SpellEntry,
        hits: &[ObjectGuid],
    ) {
        let duration = self.modded_duration_ms(caster, spell, Some(&mut cast.applied_mods));
        if duration <= 0 || !self.unit_is_alive(caster) {
            return;
        }
        let channel_target = hits.iter().copied().find(|g| *g != caster);
        cast.channeling = true;
        cast.remaining_ms = duration;
        #[allow(clippy::cast_sign_loss)]
        {
            cast.total_ms = duration as u32;
        }
        cast.channel_cost_ms = 1000;
        if let Some(u) = self.unit_base_mut(caster) {
            let values = u.values();
            // Fork `SendChannelStart` (`Spell.cpp:5123-5127`)
            // only touches the channel object for a
            // non-caster first hit; a null target leaves
            // the field alone, so an effect-set object
            // (the fishing bobber, set after this point
            // in the fork and before it here) survives.
            if let Some(target) = channel_target {
                values.set_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT, target.raw());
            }
            values.set_u32(offsets::UNIT_CHANNEL_SPELL, spell.id);
        }
        if self.players.contains_key(&caster) {
            let mut payload = PacketWriter::new();
            payload.write_u32_le(spell.id);
            #[allow(clippy::cast_sign_loss)]
            payload.write_u32_le(duration as u32);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_START, payload.into_vec());
            self.send_to_player(caster, packet);
        }
        let applied = cast.applied_mods.clone();
        self.casts.insert(caster, cast);
        // Fork sweeps exhausted mods right after
        // `SendChannelStart`.
        self.remove_exhausted_spell_mods(caster, &applied);
    }

    /// Channeled tick (`Spell::update` CASTING arm +
    /// the holder `UpdateHolder` cost/range arms,
    /// folded into the cast because holder ticking
    /// itself stays in `update_auras` here).
    fn update_channel(&mut self, caster: ObjectGuid, diff_ms: u32) {
        let Some((spell_id, target)) = self.casts.get(&caster).map(|c| (c.spell_id, c.target))
        else {
            return;
        };
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            self.end_channel(caster, true);
            return;
        };
        // Target validity + range (single-target
        // subset of `HasValidUnitPresentInTargetList`
        // and the holder range removal).
        if let CastTarget::Unit(victim) = target
            && victim != caster
        {
            if !self.unit_is_alive(victim) || self.unit_position(victim).is_none() {
                self.end_channel(caster, true);
                return;
            }
            let mut max_range = spell.range_max(&self.stores.spells.tables);
            if Self::is_hostile_pair(caster, victim) {
                max_range *= 1.33;
            } else {
                max_range += 1.25;
            }
            let cpos = self.unit_position(caster).unwrap_or_default();
            let vpos = self.unit_position(victim).unwrap_or_default();
            let dist = ((cpos.x - vpos.x).powi(2) + (cpos.y - vpos.y).powi(2)).sqrt();
            if dist > max_range {
                self.end_channel(caster, true);
                return;
            }
        }
        // Per-second drain (holder `m_timeCla`).
        let drain_due = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.channel_cost_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                if cast.channel_cost_ms <= 0 {
                    cast.channel_cost_ms = 1000;
                    true
                } else {
                    false
                }
            }
            None => return,
        };
        if drain_due {
            #[allow(clippy::cast_possible_wrap)]
            let amount = spell.mana_per_second as i32
                + spell.mana_per_second_per_level as i32 * self.unit_level(caster) as i32;
            if amount > 0 {
                #[allow(clippy::cast_sign_loss)]
                let amount = amount as u32;
                let current = self.unit_power(caster, spell.power_type);
                // Health-cost channels never pay the
                // last point (fork funnel guard).
                let can_pay = if spell.power_type == power::HEALTH {
                    current > amount
                } else {
                    current >= amount
                };
                if can_pay {
                    self.unit_set_power(caster, spell.power_type, current - amount);
                } else {
                    self.end_channel(caster, true);
                    if self.players.contains_key(&caster) {
                        self.send_cast_result(caster, spell_id, cast_result::FAILED_FIZZLE);
                    }
                    return;
                }
            }
        }
        let done = match self.casts.get_mut(&caster) {
            Some(cast) => {
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                cast.remaining_ms <= 0
            }
            None => return,
        };
        if done {
            self.end_channel(caster, false);
        }
    }

    /// Remove one caster's holders of a spell from a
    /// unit (`RemoveAurasByCasterSpell`).
    pub(crate) fn remove_auras_by_caster_spell(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        caster: ObjectGuid,
    ) {
        loop {
            let index = self.unit_base_mut(victim).and_then(|u| {
                u.auras
                    .iter()
                    .rposition(|h| h.spell_id == spell_id && h.caster == caster)
            });
            let Some(index) = index else { return };
            self.remove_aura_at(victim, index);
        }
    }

    /// Channel end (`SendChannelUpdate(0)` + field
    /// reset). Interrupted channels strip their
    /// holders now; a natural finish leaves them for
    /// `update_auras` later this tick so the final
    /// periodic tick still lands (the fork updates
    /// holders inside the spell update first). The
    /// fork's 1 s delayed field reset on natural
    /// finish is an immediate clear here.
    fn end_channel(&mut self, caster: ObjectGuid, interrupted: bool) {
        let Some(cast) = self.casts.remove(&caster) else {
            return;
        };
        if interrupted {
            if let CastTarget::Unit(victim) = cast.target
                && victim != caster
            {
                self.remove_auras_by_caster_spell(victim, cast.spell_id, caster);
            }
            self.remove_auras_by_caster_spell(caster, cast.spell_id, caster);
        }
        if let Some(u) = self.unit_base_mut(caster) {
            let values = u.values();
            values.set_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT, 0);
            values.set_u32(offsets::UNIT_CHANNEL_SPELL, 0);
        }
        // RemoveDynObject(spellId) at channel end
        // (fork cancel + finish paths).
        self.remove_dynobjects_by_spell(caster, cast.spell_id);
        if self.players.contains_key(&caster) {
            let mut payload = PacketWriter::new();
            payload.write_u32_le(0);
            let packet = WorldPacket::with_body(op::MSG_CHANNEL_UPDATE, payload.into_vec());
            self.send_to_player(caster, packet);
        }
    }

    /// `Player::FinishSpell(CURRENT_CHANNELED_SPELL)`
    /// (fork `GameObject.cpp` bobber arms): natural
    /// channel end for a non-spell caller. No-op when
    /// the caster channels nothing (the fork's missing
    /// current-spell guard).
    pub(crate) fn finish_channeled_spell(&mut self, caster: ObjectGuid) {
        self.end_channel(caster, false);
    }

    /// Resolved cast destination (`m_targets` dest):
    /// client destination, unit-target position, or
    /// the caster position (the fork's mask-0 fill).
    pub(super) fn cast_dest(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Position {
        if let CastTarget::Dest(pos) = target {
            return pos;
        }
        // `TARGET_LOCATION_DATABASE` fill for
        // non-teleport effects (`Spell.cpp` target-map
        // build): the store row supplies the dest when
        // it is on this map.
        if spell.effects.iter().any(|e| {
            e.implicit_target_a == tgt::LOCATION_DATABASE && e.effect != effect::TELEPORT_UNITS
        }) && let Some(loc) = self.stores.spell_target_positions.get(spell.id)
            && u32::from(loc.map_id) == self.map_id
        {
            return loc.position();
        }
        match target {
            CastTarget::Dest(pos) => pos,
            CastTarget::Unit(g) => self
                .unit_position(g)
                .or_else(|| self.world_object_position(caster))
                .unwrap_or_default(),
            CastTarget::Corpse(g) => self
                .corpses
                .get(&g)
                .map(|corpse| corpse.world.position())
                .or_else(|| self.world_object_position(caster))
                .unwrap_or_default(),
            CastTarget::SelfCast => self.world_object_position(caster).unwrap_or_default(),
        }
    }

    /// A bare `SendSpellGo` for effect customs that
    /// re-emit the parent cast's GO (Shrink Ray):
    /// caster as first guid (cast-item first-guid
    /// unmodeled here), one hit, unit target.
    pub(crate) fn send_spell_go_simple(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        target: ObjectGuid,
        cast_counter: u8,
    ) {
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(target));
        payload.write_u8(cast_counter);
        payload.write_u32_le(spell_id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(self.now_ms);
        payload.write_u8(1);
        payload.write_u64_le(target.raw());
        payload.write_u8(0);
        Self::targets_write(CastTarget::Unit(target), caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_GO, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    fn send_spell_go(
        &mut self,
        caster: ObjectGuid,
        cast: &ActiveCast,
        spell: &SpellEntry,
        hits: &[ObjectGuid],
        misses: &[(ObjectGuid, u8)],
    ) {
        let target_guid = match cast.target {
            CastTarget::Unit(g) | CastTarget::Corpse(g) => g,
            CastTarget::SelfCast | CastTarget::Dest(_) => caster,
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(Self::cast_first_guid(
            caster, cast,
        )));
        payload.write_bytes(&Self::packed_guid_bytes(target_guid));
        payload.write_u8(cast.cast_counter);
        payload.write_u32_le(spell.id);
        payload.write_u32_le(CAST_FLAG_NONE);
        payload.write_u32_le(self.now_ms);
        payload.write_u8(hits.len() as u8);
        for h in hits {
            payload.write_u64_le(h.raw());
        }
        payload.write_u8(misses.len() as u8);
        for (m, cond) in misses {
            payload.write_u64_le(m.raw());
            payload.write_u8(*cond);
        }
        Self::targets_write(cast.target, caster, &mut payload);
        let packet = WorldPacket::with_body(op::SMSG_SPELL_GO, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }

    /// Advance all in-progress casts (`Spell::update`).
    pub(crate) fn update_casts(&mut self, diff_ms: u32) {
        let casters: Vec<ObjectGuid> = self.casts.keys().copied().collect();
        for caster in casters {
            self.update_cast(caster, diff_ms);
        }
        // Slot-less triggered casts tick on their
        // own timers (fork spell events); they are
        // not pushed back or interrupted by movement.
        if !self.pending_triggered.is_empty() {
            let pending = std::mem::take(&mut self.pending_triggered);
            let mut keep = Vec::new();
            let mut due = Vec::new();
            for (caster, mut cast) in pending {
                // GO casters (Tier 4 B0-5) tick here
                // too; the fork runs their spells from
                // `GameObject::Update`.
                if self.unit_base(caster).is_none() && !self.gameobjects.contains_key(&caster) {
                    continue;
                }
                cast.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                if cast.remaining_ms <= 0 {
                    due.push((caster, cast));
                } else {
                    keep.push((caster, cast));
                }
            }
            self.pending_triggered = keep;
            for (caster, cast) in due {
                self.execute_cast(caster, cast);
            }
        }
        // Projectile-delayed hits (`handle_delayed`):
        // each lands on its own flight timer.
        if !self.pending_delayed.is_empty() {
            let pending = std::mem::take(&mut self.pending_delayed);
            let mut keep = Vec::new();
            let mut due = Vec::new();
            for mut hit in pending {
                hit.remaining_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                if hit.remaining_ms <= 0 {
                    due.push(hit);
                } else {
                    keep.push(hit);
                }
            }
            self.pending_delayed = keep;
            for hit in due {
                self.apply_delayed_hit(hit);
            }
        }
    }
}
