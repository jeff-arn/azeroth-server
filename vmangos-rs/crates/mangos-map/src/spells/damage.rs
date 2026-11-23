//! Damage/heal bonuses, dealing damage, kill/corpse and hit math.

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `Unit::GetCreatureTypeMask`: players read as
    /// humanoid (fork race-table default; shapeshift
    /// creature types unmodeled).
    pub(crate) fn creature_type_mask(&self, guid: ObjectGuid) -> i32 {
        let creature_type = if self.players.contains_key(&guid) {
            7
        } else {
            self.creatures
                .get(&guid)
                .map_or(0, |c| c.info.creature_type)
        };
        if creature_type == 0 {
            0
        } else {
            #[allow(clippy::cast_possible_wrap)]
            {
                1i32 << (creature_type as i32 - 1)
            }
        }
    }

    /// `Aura::IsAffectedOnSpell` non-spellmod arm.
    fn override_affected(&self, holder: &AuraHolder, eff_idx: usize, spell: &SpellEntry) -> bool {
        let Some(source) = self.stores.spells.get(holder.spell_id) else {
            return false;
        };
        source.spell_family_name == spell.spell_family_name
            && spell.spell_family_flags & source.effects[eff_idx].item_type != 0
    }

    /// `SpellCaster::SpellBonusWithCoeffs`
    /// (`SpellCaster.cpp:1465`): template coefficient
    /// wins over the default formula and skips the
    /// level penalty; custom coefficients next; the
    /// `SPELL_BONUS_DAMAGE` spellmod scales the
    /// coefficient through ×100. `mod_subject` is the
    /// fork `ToUnit()` (caster for Done, victim for
    /// Taken — fork quirk preserved).
    #[allow(clippy::too_many_arguments)]
    fn spell_bonus_with_coeffs(
        &mut self,
        mod_subject: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        total: f32,
        benefit: f32,
        dot: bool,
        done_part: bool,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        if benefit == 0.0 {
            return total;
        }
        let tables = &self.stores.spells.tables;
        let template_coeff = spell.effects[eff_index].bonus_coefficient;
        let (coeff, lvl_penalty) = if template_coeff >= 0.0 {
            (template_coeff, 1.0)
        } else {
            (
                spell.default_coefficient(tables, dot),
                spell.level_penalty(),
            )
        };
        let mut coeff = spell.custom_coefficient(coeff, done_part, 1, None);
        coeff *= 100.0;
        coeff = self.apply_spell_mod(
            mod_subject,
            spell,
            spellmod::SPELL_BONUS_DAMAGE,
            coeff,
            false,
            applied_mods,
        );
        coeff /= 100.0;
        total + benefit * coeff * lvl_penalty
    }

    /// `SpellBaseDamageBonusDone` +
    /// `MOD_FLAT_SPELL_DAMAGE_VERSUS` (wand equipped-item
    /// gates unmodeled — no equipped fields on the entry
    /// subset; pet `GetBonusDamage` unmodeled).
    pub(crate) fn spell_base_damage_bonus_done(
        &self,
        caster: ObjectGuid,
        school_mask: i32,
        creature_type_mask: i32,
    ) -> i32 {
        let mut benefit =
            self.total_aura_modifier(caster, aura_type::MOD_DAMAGE_DONE, Some(school_mask));
        if self.players.contains_key(&caster) {
            for e in self
                .aura_holders(caster)
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| {
                    e.aura_type == aura_type::MOD_SPELL_DAMAGE_OF_STAT_PERCENT
                        && e.misc_value & school_mask != 0
                })
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                {
                    benefit += (self.unit_stat(caster, 4) * e.amount as f32 / 100.0) as i32;
                }
            }
        }
        benefit
            + self.total_aura_modifier(
                caster,
                aura_type::MOD_FLAT_SPELL_DAMAGE_VERSUS,
                Some(creature_type_mask),
            )
    }

    /// `SpellDamageBonusDone` (`SpellCaster.cpp:1286`)
    /// magic arm. Totem owner delegation and the
    /// paladin-seal percent arm are recorded
    /// exclusions (no totem casters / equipped fields).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_damage_bonus_done(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        damage: f32,
        dot: bool,
        stack: u32,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const CUSTOM_FIXED_DAMAGE: u32 = 0x10;
        if spell.custom & CUSTOM_FIXED_DAMAGE != 0 || spell.attributes_ex3 & 0x2000_0000 != 0 {
            return damage;
        }
        // Ignite already includes modifiers (mage family
        // flag bit 27, CM0 0x08000000).
        if spell.spell_family_name == 3 && spell.spell_family_flags & 0x0800_0000 != 0 {
            return damage;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let creature_type_mask = self.creature_type_mask(victim);
        let mut done_total_mod = 1.0f32;
        let mut done_total = 0.0f32;
        let pet_owner = self.pet_owner(caster);
        if let Some(creature) = self.creatures.get(&caster)
            && !pet_owner.is_some_and(|o| self.players.contains_key(&o))
        {
            done_total_mod *= self
                .config
                .creature_rates
                .spell_damage_by_rank(creature.info.rank);
        }
        done_total_mod *= self.total_aura_multiplier(
            caster,
            aura_type::MOD_DAMAGE_PERCENT_DONE,
            Some(school_mask),
        );
        done_total_mod *= self.total_aura_multiplier(
            caster,
            aura_type::MOD_DAMAGE_DONE_VERSUS,
            Some(creature_type_mask),
        );
        done_total += self.total_aura_modifier(
            caster,
            aura_type::MOD_DAMAGE_DONE_CREATURE,
            Some(creature_type_mask),
        ) as f32;
        // Override class scripts (owner's auras).
        let script_owner = pet_owner.unwrap_or(caster);
        for holder in self.aura_holders(script_owner) {
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(e) = eff else { continue };
                if e.aura_type != aura_type::OVERRIDE_CLASS_SCRIPTS
                    || !self.override_affected(&holder, eff_idx, spell)
                {
                    continue;
                }
                match e.misc_value {
                    4418 | 4554 => done_total += e.amount as f32,
                    4555 => {
                        let divisor = if dot {
                            100.0 * f32::from(spell.aura_max_ticks(&self.stores.spells.tables))
                        } else {
                            800.0
                        };
                        if divisor != 0.0 {
                            done_total += e.amount as f32 * damage / divisor;
                        }
                    }
                    _ => {}
                }
            }
        }
        // Hunter pet happiness.
        if self
            .pets
            .get(&caster)
            .is_some_and(crate::pets::PetState::is_hunter)
        {
            let happiness = self.unit_power(caster, 4);
            if happiness >= 666_000 {
                done_total_mod *= 1.25;
            } else if happiness < 333_000 {
                done_total_mod *= 0.75;
            }
        }
        let benefit = self.spell_base_damage_bonus_done(caster, school_mask, creature_type_mask);
        done_total = self.spell_bonus_with_coeffs(
            caster,
            spell,
            eff_index,
            done_total,
            benefit as f32,
            dot,
            true,
            applied_mods.as_deref_mut(),
        );
        #[allow(clippy::cast_possible_wrap)]
        let mut out = (damage + done_total * stack as i32 as f32) * done_total_mod;
        out = self.apply_spell_mod(
            caster,
            spell,
            if dot { spellmod::DOT } else { spellmod::DAMAGE },
            out,
            true,
            applied_mods,
        );
        if out > 0.0 { out } else { 0.0 }
    }

    /// `SpellDamageBonusTaken` (`Unit.cpp:5344`).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_damage_bonus_taken(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        damage: f32,
        dot: bool,
        stack: u32,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const EX4_IGNORE_TAKEN: u32 = 0x0000_0100;
        if spell.attributes_ex4 & EX4_IGNORE_TAKEN != 0 {
            return damage;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let taken_total_mod = self.total_aura_multiplier(
            victim,
            aura_type::MOD_DAMAGE_PERCENT_TAKEN,
            Some(school_mask),
        );
        let taken_flat =
            self.total_aura_modifier(victim, aura_type::MOD_DAMAGE_TAKEN, Some(school_mask));
        #[allow(clippy::cast_possible_wrap)]
        let mut taken_flat = self.spell_bonus_with_coeffs(
            victim,
            spell,
            eff_index,
            0.0,
            taken_flat as f32,
            dot,
            false,
            applied_mods,
        ) * stack as i32 as f32;
        if taken_flat < 0.0 && -taken_flat > damage / 2.0 {
            taken_flat = -(damage / 2.0);
        }
        let out = (damage + taken_flat) * taken_total_mod;
        if out > 0.0 { out } else { 0.0 }
    }

    /// `GetMaxPositive/NegativeAuraModifier` over holders.
    pub(crate) fn max_aura_modifier(&self, guid: ObjectGuid, kind: u32, positive: bool) -> i32 {
        let holders = self.aura_holders(guid);
        let amounts = holders
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| e.aura_type == kind && (e.amount > 0) == positive && e.amount != 0)
            .map(|e| e.amount);
        if positive {
            amounts.max().unwrap_or(0)
        } else {
            amounts.min().unwrap_or(0)
        }
    }

    /// `SpellBaseHealingBonusDone` (`SpellCaster.cpp:1255`).
    fn spell_base_healing_bonus_done(&self, caster: ObjectGuid, school_mask: i32) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let mut benefit =
            self.total_aura_modifier(caster, aura_type::MOD_HEALING_DONE, Some(school_mask)) as f32;
        if self.players.contains_key(&caster) {
            for e in self
                .aura_holders(caster)
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == aura_type::MOD_SPELL_HEALING_OF_STAT_PERCENT)
            {
                #[allow(clippy::cast_precision_loss)]
                {
                    benefit += self.unit_stat(caster, 4) * e.amount as f32 / 100.0;
                }
            }
        }
        benefit
    }

    /// `SpellHealingBonusDone` (`SpellCaster.cpp:1184`).
    /// Totem owner delegation recorded (no totem casters).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_healing_bonus_done(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        heal: f32,
        dot: bool,
        stack: u32,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        const CUSTOM_FIXED_DAMAGE: u32 = 0x10;
        const ATTR_PASSIVE: u32 = 0x40;
        let gated = (spell.dmg_class == dmg_class::NONE && spell.attributes & ATTR_PASSIVE != 0)
            || spell.custom & CUSTOM_FIXED_DAMAGE != 0
            || spell.attributes_ex3 & 0x2000_0000 != 0;
        if gated {
            return if heal < 0.0 { 0.0 } else { heal };
        }
        let done_total_mod =
            self.total_aura_multiplier(caster, aura_type::MOD_HEALING_DONE_PERCENT, None);
        let mut done_total = 0.0f32;
        let script_owner = self.pet_owner(caster).unwrap_or(caster);
        for holder in self.aura_holders(script_owner) {
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(e) = eff else { continue };
                if e.aura_type != aura_type::OVERRIDE_CLASS_SCRIPTS
                    || !self.override_affected(&holder, eff_idx, spell)
                {
                    continue;
                }
                match e.misc_value {
                    4415 => done_total += e.amount as f32 / 4.0,
                    3736 => done_total += e.amount as f32,
                    _ => {}
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let benefit = self.spell_base_healing_bonus_done(caster, school_mask);
        done_total = self.spell_bonus_with_coeffs(
            caster,
            spell,
            eff_index,
            done_total,
            benefit,
            dot,
            true,
            applied_mods.as_deref_mut(),
        );
        #[allow(clippy::cast_possible_wrap)]
        let mut out = (heal + done_total * stack as i32 as f32) * done_total_mod;
        out = self.apply_spell_mod(
            caster,
            spell,
            if dot { spellmod::DOT } else { spellmod::DAMAGE },
            out,
            true,
            applied_mods,
        );
        if out < 0.0 { 0.0 } else { out }
    }

    /// `SpellHealingBonusTaken` (`Unit.cpp:5491`).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    pub(crate) fn spell_healing_bonus_taken(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        heal: f32,
        dot: bool,
        stack: u32,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        let mut taken_total_mod = 1.0f32;
        let minval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, false);
        if minval != 0 {
            taken_total_mod *= (100.0 + minval as f32) / 100.0;
        }
        let maxval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, true);
        if maxval != 0 {
            taken_total_mod *= (100.0 + maxval as f32) / 100.0;
        }
        if spell.dmg_class == dmg_class::NONE {
            let out = heal * taken_total_mod;
            return if out < 0.0 { 0.0 } else { out };
        }
        // Healing Way (29203) dummy for Healing Wave
        // (shaman family 11, CM0 bit 6).
        if spell.spell_family_name == 11 && spell.spell_family_flags & 0x40 != 0 {
            for e in self
                .aura_holders(victim)
                .iter()
                .filter(|h| h.spell_id == 29203)
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == aura_type::DUMMY)
            {
                taken_total_mod *= (e.amount as f32 + 100.0) / 100.0;
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let mut taken_flat =
            self.total_aura_modifier(victim, aura_type::MOD_HEALING, Some(school_mask)) as f32;
        // Blessing of Light dummies (paladin family 10;
        // flash bit 13, holy light bit 31, blessings
        // bit 28, visual 300).
        if spell.spell_family_name == 10 && spell.spell_family_flags & (0x2000 | 0x8000_0000) != 0 {
            for holder in self.aura_holders(victim) {
                let Some(source) = self.stores.spells.get(holder.spell_id) else {
                    continue;
                };
                if source.spell_family_flags & 0x1000_0000 == 0 || source.spell_visual != 300 {
                    continue;
                }
                for (eff_idx, eff) in holder.effects.iter().enumerate() {
                    let Some(e) = eff else { continue };
                    if e.aura_type != aura_type::DUMMY {
                        continue;
                    }
                    let holy_light = spell.spell_family_flags & 0x8000_0000 != 0 && eff_idx == 0;
                    let flash = spell.spell_family_flags & 0x2000 != 0 && eff_idx == 1;
                    if holy_light || flash {
                        taken_flat += e.amount as f32;
                    }
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let mut taken_flat = self.spell_bonus_with_coeffs(
            victim,
            spell,
            eff_index,
            0.0,
            taken_flat,
            dot,
            false,
            applied_mods,
        ) * stack as i32 as f32;
        if taken_flat < 0.0 && -taken_flat > heal / 2.0 {
            taken_flat = -(heal / 2.0);
        }
        let out = (heal + taken_flat) * taken_total_mod;
        if out < 0.0 { 0.0 } else { out }
    }

    /// `SpellCaster::CalculateDamage` tail +
    /// `SpellNonMeleeDamageLog` + health application.
    /// `aura_holder` is Some for periodic ticks
    /// (PERIODICAURALOG instead of NONMELEE log).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::too_many_arguments,
        clippy::needless_option_as_deref // reborrows of the applied-mods list
    )]
    #[allow(clippy::too_many_lines)] // one CalculateDamage port
    pub(crate) fn school_damage(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        base_damage: i32,
        eff_index: u32,
        aura_holder: Option<u32>,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        proc_ctx: Option<&ProcRaiseCtx>,
    ) {
        if base_damage <= 0 {
            return;
        }
        // Fork `CalculateDamage` order: bonus Done →
        // Taken → crit → armor → ditheru. Periodic
        // amounts carry Done from the apply-time
        // snapshot, so ticks take Taken(DOT) only.
        // Melee/ranged-class spells keep the pre-slice
        // math (`MeleeDamageBonus` audit is separate).
        let dot = aura_holder.is_some();
        #[allow(clippy::cast_precision_loss)]
        let mut damage_f = base_damage as f32;
        if spell_meleedamage(spell.dmg_class) {
            damage_f = self.apply_spell_mod(
                caster,
                spell,
                if dot { spellmod::DOT } else { spellmod::DAMAGE },
                damage_f,
                true,
                applied_mods.as_deref_mut(),
            );
            if spell.attributes_ex3 & 0x2000_0000 == 0 {
                #[allow(clippy::cast_possible_wrap)]
                let school_mask = 1i32 << spell.school;
                let pct = self.total_aura_multiplier(
                    caster,
                    aura_type::MOD_DAMAGE_PERCENT_DONE,
                    Some(school_mask),
                ) * self.total_aura_multiplier(
                    victim,
                    aura_type::MOD_DAMAGE_PERCENT_TAKEN,
                    Some(school_mask),
                );
                damage_f *= pct;
            }
        } else if dot {
            let stack = aura_holder
                .and_then(|spell_id| {
                    self.aura_holders(victim)
                        .into_iter()
                        .find(|h| h.spell_id == spell_id && h.caster == caster)
                        .map(|h| h.stack)
                })
                .unwrap_or(1);
            damage_f = self.spell_damage_bonus_taken(
                victim,
                spell,
                eff_index as usize,
                damage_f,
                true,
                stack,
                applied_mods.as_deref_mut(),
            );
        } else {
            damage_f = self.spell_damage_bonus_done(
                caster,
                victim,
                spell,
                eff_index as usize,
                damage_f,
                false,
                1,
                applied_mods.as_deref_mut(),
            );
            damage_f = self.spell_damage_bonus_taken(
                victim,
                spell,
                eff_index as usize,
                damage_f,
                false,
                1,
                applied_mods.as_deref_mut(),
            );
        }

        // Crit: direct magic damage only (periodic ticks
        // never crit in 1.12.1); skipped entirely under
        // EX3 ignore-caster-modifiers (fork gate). Bonus
        // is +50% for magic (`SpellCriticalDamageBonus`
        // default arm, spellmod on the bonus).
        let mut crit = false;
        if !dot
            && spell.dmg_class == dmg_class::MAGIC
            && spell.attributes_ex3 & 0x2000_0000 == 0
            && damage_f > 0.0
        {
            let crit_chance = self.spell_crit_percent(caster, spell, applied_mods.as_deref_mut());
            let roll = self.spell_rng().frand01() * 100.0;
            if roll < crit_chance {
                crit = true;
                let bonus = self.apply_spell_mod(
                    caster,
                    spell,
                    spellmod::CRIT_DAMAGE_BONUS,
                    damage_f / 2.0,
                    true,
                    applied_mods.as_deref_mut(),
                );
                damage_f += bonus;
            }
        }

        // Armor after bonus+crit (fork mitigation
        // stage), NORMAL school only, then ditheru.
        if damage_f > 0.0 && spell.school == school::NORMAL {
            let victim_armor = self.unit_resistance(victim, school::NORMAL);
            damage_f = mangos_spells::armor_reduced_damage(
                damage_f,
                victim_armor,
                self.unit_level(caster),
            );
        }
        #[allow(clippy::cast_possible_truncation)]
        let mut damage = if damage_f > 0.0 {
            mangos_spells::ditheru(damage_f, self.spell_rng()) as i32
        } else {
            0
        };

        // Partial resist (`CalculateDamageAbsorbAndResist`):
        // skipped for the NORMAL school, binary spells and
        // IGNORE_RESISTANCES; negative resistance chance
        // (vulnerability) still amplifies.
        let mut resisted: i32 = 0;
        if damage > 0 && spell.school != school::NORMAL {
            let can_resist =
                !spell_is_binary(spell) && spell.attributes_ex4 & ATTR_EX4_IGNORE_RESISTANCES == 0;
            let resist_chance = self.spell_resist_chance(caster, victim, spell.school, true);
            if can_resist || resist_chance < 0.0 {
                let multiplier = resist_multiplier_outcome(
                    resist_chance,
                    aura_holder.is_some(),
                    spell.id,
                    self.spell_rng(),
                );
                let raw = damage as f32 * multiplier;
                #[allow(clippy::cast_possible_truncation)]
                {
                    resisted = if raw >= 0.0 {
                        mangos_spells::ditheru(raw, self.spell_rng()) as i32
                    } else {
                        -(mangos_spells::ditheru(-raw, self.spell_rng()) as i32)
                    };
                }
                damage -= resisted;
            }
        }

        if damage <= 0 && resisted <= 0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation)]
        let final_damage = damage.max(0) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let final_resisted = resisted.max(0) as u32;
        self.send_damage_log(
            caster,
            victim,
            spell,
            final_damage,
            final_resisted,
            crit,
            aura_holder,
        );
        if final_damage > 0 {
            self.deal_damage_school(
                caster,
                victim,
                final_damage,
                aura_holder.is_some(),
                spell.school,
            );
        }
        // Proc raise for the landed hit (fork raises in
        // `DealDamage` with the spell's proc info).
        if let Some(ctx) = proc_ctx {
            let ex = if final_damage == 0 {
                proc_ex::MISS
            } else if crit {
                proc_ex::CRITICAL_HIT
            } else {
                proc_ex::NORMAL_HIT
            };
            self.raise_damage_procs(
                caster,
                victim,
                Some(spell.id),
                ctx,
                ex,
                final_damage,
                final_damage + final_resisted,
            );
        }
    }

    /// Fork `Spell::EffectHealMaxHealth` base (`SpellEffects.cpp:3414`):
    /// the caster's max health (the victim's when the caster unit is
    /// unavailable), through the done/taken percent modifiers only —
    /// no spellpower benefit, unlike `spell_healing_bonus_done`.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn heal_max_health_value(&self, caster: ObjectGuid, victim: ObjectGuid) -> f32 {
        let base = if self.unit_is_alive(caster) {
            self.unit_value_u32(caster, offsets::UNIT_FIELD_MAXHEALTH)
        } else {
            self.unit_value_u32(victim, offsets::UNIT_FIELD_MAXHEALTH)
        } as f32;
        let mut value =
            base * self.total_aura_multiplier(caster, aura_type::MOD_HEALING_DONE_PERCENT, None);
        let minval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, false);
        if minval != 0 {
            value *= (100.0 + minval as f32) / 100.0;
        }
        let maxval = self.max_aura_modifier(victim, aura_type::MOD_HEALING_PCT, true);
        if maxval != 0 {
            value *= (100.0 + maxval as f32) / 100.0;
        }
        value
    }

    /// Health increase clamped to max (heal log packet
    /// is a recorded slice deviation).
    pub(crate) fn heal_unit(&mut self, target: ObjectGuid, amount: u32) {
        if amount == 0 {
            return;
        }
        let current = self.unit_value_u32(target, offsets::UNIT_FIELD_HEALTH);
        let max = self.unit_value_u32(target, offsets::UNIT_FIELD_MAXHEALTH);
        if let Some(u) = self.unit_base_mut(target) {
            u.set_health((current + amount).min(max));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_damage_log(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
        damage: u32,
        resisted: u32,
        crit: bool,
        aura_holder: Option<u32>,
    ) {
        let mut payload = PacketWriter::new();
        if let Some(_holder) = aura_holder {
            // SMSG_PERIODICAURALOG:
            // packed(target) packed(caster) spellid
            // count=1 auraType amount school absorb resisted
            payload.write_bytes(&Self::packed_guid_bytes(victim));
            payload.write_bytes(&Self::packed_guid_bytes(caster));
            payload.write_u32_le(spell.id);
            payload.write_u32_le(1);
            payload.write_u32_le(aura_type::PERIODIC_DAMAGE);
            payload.write_u32_le(damage);
            payload.write_u32_le(school::mask(spell.school));
            payload.write_u32_le(0); // absorb
            payload.write_u32_le(resisted);
            let packet = WorldPacket::with_body(op::SMSG_PERIODICAURALOG, payload.into_vec());
            self.broadcast_movement(caster, &packet);
        } else {
            payload.write_bytes(&Self::packed_guid_bytes(victim));
            payload.write_bytes(&Self::packed_guid_bytes(caster));
            payload.write_u32_le(spell.id);
            payload.write_u32_le(damage);
            payload.write_u32_le(0); // overkill
            #[allow(clippy::cast_possible_truncation)]
            payload.write_u8(spell.school as u8);
            payload.write_u32_le(0); // absorb
            payload.write_u32_le(resisted);
            payload.write_u8(0); // physical log?
            payload.write_u32_le(0); // blocked
            payload.write_u16_le(if crit { SPELL_HIT_TYPE_CRIT } else { 0 });
            payload.write_u8(0); // debug output
            let packet =
                WorldPacket::with_body(op::SMSG_SPELLNONMELEEDAMAGELOG, payload.into_vec());
            self.broadcast_movement(caster, &packet);
        }
    }

    /// `Player::RewardRage` (`Player.cpp:2249-2272`):
    /// rage from direct damage dealt, or from damage
    /// taken while surviving it. Power is stored in
    /// tenths, hence the final `* 10` conversion.
    pub(crate) fn reward_rage(&mut self, player_guid: ObjectGuid, damage: u32, attacker: bool) {
        if damage == 0 || !player_guid.is_player() {
            return;
        }
        let Some(player) = self.players.get(&player_guid).map(|entry| &entry.player) else {
            return;
        };
        if u32::from(player.unit.power_type()) != power::RAGE {
            return;
        }
        let berserker_rage = !attacker
            && player.unit.auras.iter().any(|holder| {
                holder.spell_id == SPELL_BERSERKER_RAGE && holder.effects[0].is_some()
            });
        let amount = rage_reward_amount(
            player.unit.level(),
            damage,
            attacker,
            berserker_rage,
            self.config.regen_rates.rage_income,
        );
        if amount == 0 {
            return;
        }
        if let Some(unit) = self.unit_base_mut(player_guid) {
            let current = unit.power(mangos_entities::unit::POWER_RAGE);
            let max = unit.max_power(mangos_entities::unit::POWER_RAGE);
            unit.set_power(
                mangos_entities::unit::POWER_RAGE,
                current.saturating_add(amount).min(max),
            );
        }
    }

    /// Health application + combat flag + death
    /// (`Unit::DealDamage` subset). `is_dot` is the
    /// fork `DamageEffectType::DOT` arm (periodic ticks
    /// never push back or cancel casts).
    pub(crate) fn deal_damage(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        damage: u32,
        is_dot: bool,
    ) {
        self.deal_damage_school(attacker, victim, damage, is_dot, 0);
    }

    /// `deal_damage` with the damage school (absorb
    /// mask matching); melee/script paths use NORMAL.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn deal_damage_school(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        mut damage: u32,
        is_dot: bool,
        school_id: u32,
    ) {
        if damage == 0 {
            return;
        }
        // Damage-immunity backstop (melee/script
        // funnels; spell paths resolve immunity before
        // their damage logs).
        #[allow(clippy::cast_possible_wrap)]
        if self.is_immune_to_damage(victim, 1i32 << school_id, None) {
            return;
        }
        if victim.is_creature() {
            if attacker != victim {
                self.creature_script_attacked_by(victim, attacker);
            }
            // `CreatureAI::DamageTaken` may rewrite the amount.
            let mut scripted = damage;
            self.creature_script_damage_taken(victim, attacker, &mut scripted);
            damage = scripted;
            if damage == 0 {
                return;
            }
            // Loot tap (`DealDamage` `SetLootRecipient`
            // block, before absorb/kill resolution).
            self.tap_creature(victim, attacker);
        }
        // `SetInvincibilityHpThreshold` floor (Slice 6b).
        let mut damage = damage;
        if victim.is_creature()
            && let Some(threshold) = self
                .ai
                .get(&victim)
                .map(|ai| ai.invincibility_hp)
                .filter(|t| *t > 0)
        {
            let current = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
            if current <= threshold {
                return;
            }
            damage = damage.min(current - threshold);
            if damage == 0 {
                return;
            }
        }
        // `SPELL_AURA_SCHOOL_ABSORB` pools (fork
        // absorbs inside `DealDamage` before health;
        // a holder drops when its pool is spent).
        let absorbed_full;
        {
            #[allow(clippy::cast_possible_wrap)]
            let mask = 1i32 << school_id;
            let mut spent_holders: Vec<u32> = Vec::new();
            if let Some(u) = self.unit_base_mut(victim) {
                for holder in &mut u.auras {
                    if damage == 0 {
                        break;
                    }
                    for eff in holder.effects.iter_mut().flatten() {
                        if eff.aura_type == aura_type::SCHOOL_ABSORB
                            && eff.misc_value & mask != 0
                            && eff.amount > 0
                        {
                            #[allow(clippy::cast_possible_truncation)]
                            let take = damage.min(eff.amount as u32);
                            #[allow(clippy::cast_possible_wrap)]
                            {
                                eff.amount -= take as i32;
                            }
                            damage -= take;
                            if eff.amount == 0 {
                                spent_holders.push(holder.spell_id);
                            }
                            if damage == 0 {
                                break;
                            }
                        }
                    }
                }
            }
            absorbed_full = damage == 0;
            for spell_id in spent_holders {
                self.remove_auras_due_to_spell(victim, spell_id);
            }
        }
        let now = self.now_ms;
        for guid in [attacker, victim] {
            if let Some(u) = self.unit_base_mut(guid) {
                let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
                u.values()
                    .set_u32(offsets::UNIT_FIELD_FLAGS, flags | UNIT_FLAG_IN_COMBAT);
                u.last_combat_ms = now;
            }
        }
        let current = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
        let new_health = current.saturating_sub(damage);
        if let Some(u) = self.unit_base_mut(victim) {
            u.set_health(new_health);
        }
        // `UnitDamaged` (player victims): honor
        // damage history for `RewardHonorOnDeath`;
        // attacker resolves to the affecting player
        // (owner for pets/summons) or EMPTY.
        if victim.is_player() && victim != attacker {
            let affecting = if attacker.is_player() {
                attacker
            } else if let Some(owner) = self.pet_owner(attacker).filter(|g| g.is_player()) {
                owner
            } else if let Some(summoner) = self
                .summons
                .get(&attacker)
                .map(|st| st.summoner)
                .filter(|g| g.is_player())
            {
                summoner
            } else {
                ObjectGuid::EMPTY
            };
            if let Some(u) = self.unit_base_mut(victim) {
                u.record_damage_taken(affecting, damage, now);
            }
        }
        // Rage from direct damage dealt (`DealDamage`
        // attacker arm). The amount is the post-absorb
        // damage, matching the value reaching health.
        if !is_dot && attacker != victim {
            self.reward_rage(attacker, damage, true);
        }
        if absorbed_full {
            // Fully absorbed: combat + pushback above
            // still apply in the fork; no kill/threat.
            if !is_dot {
                self.delay_spell_at_damage(attacker, victim);
            }
            return;
        }
        // Damage taken while casting (`DealDamage`
        // player arm): DAMAGE_CANCELS interrupts, else
        // `Spell::Delayed` pushback.
        if !is_dot {
            self.delay_spell_at_damage(attacker, victim);
        }
        if new_health == 0 {
            self.kill_unit(victim, Some(attacker));
            return;
        }
        // Rage from damage received, only in the fork's
        // non-lethal branch.
        if victim.is_player() && victim != attacker {
            self.reward_rage(victim, damage, false);
        }
        // Slice 6: `AttackedBy` — creature victims take
        // threat and engage (damage threat is 1:1 in the
        // no-modifier subset).
        if victim.is_creature() && attacker != victim && self.unit_is_alive(attacker) {
            let kind_basic = self
                .creatures
                .get(&victim)
                .is_some_and(Self::ai_kind_is_basic);
            if kind_basic {
                if !self.ai_in_combat(victim) {
                    self.enter_combat(victim, attacker);
                }
                #[allow(clippy::cast_precision_loss)]
                self.add_threat(victim, attacker, damage as f32);
            }
        }
    }

    /// Death: state, cleanup, respawn scheduling
    /// (`Kill`/`setDeathState(JUST_DIED)` subset).
    /// Public: GM-kill and live death-flow probes
    /// enter here, like the fork's `Unit::Kill`.
    #[allow(clippy::too_many_lines)]
    pub fn kill_unit(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        let is_creature = victim.is_creature();
        if let Some(killer) = killer
            && self.unit_is_alive(victim)
            && killer != victim
        {
            self.raise_kill_proc(killer, victim);
        }
        if let Some(u) = self.unit_base_mut(victim) {
            if !u.is_alive() {
                return;
            }
            u.death_state = DeathState::JustDied;
            u.set_health(0);
        }
        if is_creature {
            // `Creature::SetDeathState(JUST_DIED)`
            // arms the corpse decay timer and promotes
            // to `CORPSE` in the same call (flying
            // `CORPSE_FALLING` unmodeled); without
            // this the corpse fails the alive-viewer
            // grid rule on the next visibility scan
            // and despawns instantly, unlootable.
            if let Some(c) = self.creatures.get_mut(&victim) {
                c.corpse_decay_timer = if c.info.despawn_instantly() {
                    1
                } else {
                    i64::from(c.corpse_delay) * 1000
                };
                c.unit.death_state = DeathState::Corpse;
            }
        }
        if victim.is_player() {
            // `RewardHonorOnDeath` (`Unit::Kill`
            // player-victim arm).
            self.reward_honor_on_death(victim);
            if self.bg.is_none() {
                // `Player::KillPlayer` (fork converts
                // `JUST_DIED` → `CORPSE` one update
                // later): repop timer, durability,
                // reclaim-delay ladder.
                self.kill_player(victim, killer);
            } else if let Some(u) = self.unit_base_mut(victim) {
                // Battlegrounds keep their spirit-guide
                // ghost wave, which consumes `DEAD`.
                u.death_state = DeathState::Dead;
            }
        }
        // Kill XP (`Unit::Kill` rewards precede the
        // AI death hooks in the fork as well).
        if is_creature {
            self.reward_kill_xp(victim, killer);
            // Kill reputation (`RewardReputation` runs
            // between honor and XP in
            // `RewardSinglePlayerAtKill`; grouped with XP
            // here, after it).
            self.reward_kill_reputation(victim, killer);
            self.quest_kill_credit(victim, killer);
            // Corpse loot generation (fork `Kill`
            // generates before `JustDied` hooks).
            self.generate_creature_loot(victim, killer);
            // `JUST_DIED` skinnable flag (fork
            // `Creature.cpp:2143-2146`).
            self.set_skinnable_on_kill(victim);
        }
        // EventAI death/kill hooks run while the AI
        // state still exists (`JustDied`/`KilledUnit`).
        self.eventai_on_death(victim, killer);
        if victim.is_player() {
            self.instance_on_player_death(victim);
        }
        // BG kill scoring (`BattleGround::HandleKillPlayer` /
        // `HandleKillUnit`, Tier 3 Slice 3; no-op off BG maps).
        if self.bg.is_some() {
            if victim.is_player() {
                self.bg_handle_kill_player(victim, killer);
            } else if victim.is_creature() {
                self.bg_handle_kill_unit(victim, killer);
            }
        }
        // `CreatureAI::KilledUnit` fires for any victim type
        // (fork `Unit::Kill`); bodies gate on player victims
        // themselves.
        if let Some(k) = killer
            && k != victim
            && k.is_creature()
        {
            self.creature_script_killed_unit(k, victim);
        }
        if is_creature {
            self.instance_on_creature_death(victim);
            self.go_ai_summoned_creature_just_died(victim);
            // Tier 4 B0-6: escort/follower `JustDied`
            // quest-fail arm.
            self.escort_on_just_died(victim);
            self.creature_script_just_died(victim, killer);
            self.creature_script_summoned_creature_just_died(victim);
            if let Some(summoner) = self.summons.get(&victim).map(|st| st.summoner)
                && summoner.is_creature()
            {
                self.eventai_process_type(
                    summoner,
                    mangos_scripts_engine::event_type::SUMMONED_JUST_DIED,
                    Some(victim),
                );
            }
        }
        // Death ends a channel: strip the target-side
        // holders and clear the channel fields (no
        // channel packets to the dead; own holders go
        // with `remove_all_auras` below).
        if let Some(cast) = self.casts.remove(&victim)
            && cast.channeling
        {
            if let CastTarget::Unit(target) = cast.target
                && target != victim
            {
                self.remove_auras_by_caster_spell(target, cast.spell_id, victim);
            }
            if let Some(u) = self.unit_base_mut(victim) {
                let values = u.values();
                values.set_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT, 0);
                values.set_u32(offsets::UNIT_CHANNEL_SPELL, 0);
            }
        }
        self.attack_stop(victim);
        self.ai.remove(&victim);
        self.remove_all_auras(victim);
        self.stop_moving(victim);
        if let Some(mut master) = self.motion.remove(&victim) {
            master.clear(self, victim);
            self.motion.entry(victim).or_insert(master);
        }
        if is_creature {
            if self.is_pet(victim) {
                // Pet corpses persist for revive until
                // unsummoned (no DB respawn, no decay).
            } else if self.summons.contains_key(&victim) {
                // Temp summons never hit the DB respawn
                // path: the corpse is removed by the
                // summon lifecycle (Slice 7).
                let state = self.summons.remove(&victim);
                let corpse_delay = match state.as_ref().map(|st| st.kind) {
                    Some(5) => 0,
                    Some(_) => state.map_or(60_000, |st| u64::from(st.despawn_delay_ms).max(1)),
                    None => 60_000,
                };
                self.despawns_due
                    .insert(victim, (u64::from(self.now_ms) + corpse_delay, u32::MAX));
            } else {
                let delay = self.creatures.get(&victim).map_or(0, |c| c.respawn_delay);
                let when = self.now_unix + u64::from(delay);
                self.respawns_due.insert(victim, when);
                self.events.push(MapEvent::StoreCreatureRespawn {
                    guid_low: victim.counter(),
                    respawn_time: when,
                    map_id: self.map_id,
                    instance_id: self.creature_respawn_key(victim.counter()),
                });
            }
        }
        self.events.push(MapEvent::UnitDied {
            guid: victim,
            killer,
        });
    }

    /// Corpse decay tick (`Creature::Update`
    /// `CORPSE` arm subset): count the decay timer
    /// down; on expiry `RemoveCorpse` — state
    /// `DEAD` (invisible to the alive-viewer grid
    /// rule) and any unlooted loot is gone. The
    /// fork's respawn-time removal arm is covered
    /// by `update_respawns` re-materializing the
    /// spawn. Dead-by-default corpses never decay.
    pub(crate) fn update_corpse_decay(&mut self, diff_ms: u32) {
        let diff = i64::from(diff_ms);
        let mut expired = Vec::new();
        for (guid, creature) in &mut self.creatures {
            if creature.unit.death_state != DeathState::Corpse || creature.is_dead_by_default {
                continue;
            }
            if creature.corpse_decay_timer <= diff {
                expired.push(*guid);
            } else {
                creature.corpse_decay_timer -= diff;
            }
        }
        for guid in expired {
            if let Some(creature) = self.creatures.get_mut(&guid) {
                creature.corpse_decay_timer = 0;
                creature.unit.death_state = DeathState::Dead;
            }
            // Same drops as the respawn path (loot,
            // taps, skinning timer/flag, open windows).
            self.clear_loot_for_creature(guid);
        }
    }

    /// Respawn due creatures (`Creature::Respawn`
    /// re-materialization subset).
    pub(crate) fn update_respawns(&mut self) {
        let now = self.now_unix;
        let due: Vec<ObjectGuid> = self
            .respawns_due
            .iter()
            .filter(|&(_, &when)| when <= now)
            .map(|(&g, _)| g)
            .collect();
        for guid in due {
            self.respawns_due.remove(&guid);
            // `spawn_creature` re-materializes from the DB
            // spawn row, replaces the corpse in the
            // creature map and re-initializes motion.
            self.spawn_creature(guid.counter(), &RespawnTimes::default());
            self.creature_script_just_respawned(guid);
            self.events.push(MapEvent::ClearCreatureRespawn {
                guid_low: guid.counter(),
                instance_id: self.creature_respawn_key(guid.counter()),
            });
        }
    }

    // ------------------------------------------------ hit/resist/crit math

    /// `SpellCaster::GetSpellResistChance` (fraction;
    /// no target-resistance penetration auras in the
    /// slice). Build 5875 takes the >1.8.4 clamp.
    #[allow(clippy::cast_precision_loss)]
    fn spell_resist_chance(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        school_id: u32,
        innate_resists: bool,
    ) -> f32 {
        let base_resistance = self.unit_resistance(victim, school_id) as f32;
        let mut chance = base_resistance;
        if chance < 0.0 && base_resistance >= 0.0 {
            chance = 0.0;
        }
        if chance < 0.0 {
            // Magic vulnerability: skill = max(level*5, 100).
            let skill = (self.unit_level(victim) * 5).max(100) as f32;
            chance /= skill;
            return chance.max(-0.75);
        }
        let caster_level = self.unit_level(caster).max(1);
        if innate_resists && !victim.is_player() {
            #[allow(clippy::cast_possible_wrap)]
            let level_diff = self.unit_level(victim) as i32 - self.unit_level(caster) as i32;
            #[allow(clippy::cast_possible_truncation)]
            {
                chance += ((8.0 * level_diff as f32) * caster_level as f32 / 63.0) as i32 as f32;
            }
        }
        chance *= 0.15 / caster_level as f32;
        chance.clamp(0.0, 0.75)
    }

    pub(crate) fn unit_resistance(&self, guid: ObjectGuid, school_id: u32) -> i32 {
        let index = offsets::UNIT_FIELD_RESISTANCES + school_id as u16;
        // `MOD_RESISTANCE` auras fold in at read (the
        // client resistance field is not refreshed
        // with them in this subset; recorded).
        i32::from_ne_bytes(self.unit_value_u32(guid, index).to_ne_bytes())
            + self.total_aura_modifier(guid, aura_type::MOD_RESISTANCE, Some(1 << school_id))
    }

    /// `GetTotalAuraModifier(ByMiscMask)`: sum of
    /// holder effect amounts for one aura type,
    /// optionally filtered by misc-value bitmask
    /// (school masks for the damage family).
    pub(crate) fn total_aura_modifier(
        &self,
        guid: ObjectGuid,
        aura_type_id: u32,
        misc_mask: Option<i32>,
    ) -> i32 {
        self.aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| {
                e.aura_type == aura_type_id && misc_mask.is_none_or(|mask| e.misc_value & mask != 0)
            })
            .map(|e| e.amount)
            .sum()
    }

    /// `GetTotalAuraMultiplier(ByMiscMask)`: product
    /// of `(100 + amount) / 100`.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn total_aura_multiplier(
        &self,
        guid: ObjectGuid,
        aura_type_id: u32,
        misc_mask: Option<i32>,
    ) -> f32 {
        self.aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .filter(|e| {
                e.aura_type == aura_type_id && misc_mask.is_none_or(|mask| e.misc_value & mask != 0)
            })
            .map(|e| (100.0 + e.amount as f32) / 100.0)
            .product()
    }

    /// `MagicSpellHitChance` returned as a percent
    /// (no spell mods / hit-chance auras in the slice).
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn magic_hit_chance(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
    ) -> f32 {
        if spell.attributes_ex3 & ATTR_EX3_ALWAYS_HIT != 0 {
            return 100.0;
        }
        #[allow(clippy::cast_possible_wrap)]
        let level_dif = self.unit_level(victim) as i32 - self.unit_level(caster) as i32;
        let l_chance: i32 = if victim.is_player() { 7 } else { 11 };
        let mut hit = if level_dif < 3 {
            (96 - level_dif) as f32
        } else {
            (94 - (level_dif - 2) * l_chance) as f32
        };
        if hit < 22.0 {
            hit = 22.0;
        }
        if spell_is_binary(spell) {
            hit *= 1.0 - self.spell_resist_chance(caster, victim, spell.school, false);
        }
        // C++ returns int hundredths clamped [100, 9900].
        #[allow(clippy::cast_possible_truncation)]
        let hundredths = (hit * 100.0) as i32;
        hundredths.clamp(100, 9900) as f32 / 100.0
    }

    /// `MeleeSpellMissChance` complement (weapon/defense
    /// skills at their level maxima — no skill system in
    /// the slice; dodge/parry/block not rolled, recorded).
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn melee_spell_hit_chance(
        &self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
    ) -> f32 {
        if spell.attributes_ex3 & ATTR_EX3_ALWAYS_HIT != 0 {
            return 100.0;
        }
        #[allow(clippy::cast_possible_wrap)]
        let skill_diff =
            (self.unit_level(caster) * 5) as i32 - (self.unit_level(victim) * 5) as i32;
        let mut miss = if victim.is_player() {
            5.0 - skill_diff as f32 * 0.04
        } else if skill_diff < -10 {
            5.0 - skill_diff as f32 * 0.2
        } else {
            5.0 - skill_diff as f32 * 0.1
        };
        let victim_level = self.unit_level(victim);
        if !victim.is_player() && victim_level < 10 {
            miss *= victim_level as f32 / 10.0;
        }
        100.0 - miss.clamp(0.0, 60.0)
    }

    /// `GetSpellCritFromIntellect` (creatures flat
    /// 5%) with `SPELLMOD_CRITICAL_CHANCE` applied.
    #[allow(clippy::cast_precision_loss)]
    fn spell_crit_percent(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied_mods: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        // crit_data {base, rate0, rate1} per class
        // (`Unit.cpp:2833`).
        const CRIT_DATA: [[f32; 3]; 12] = [
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 10.0],
            [3.70, 14.77, 0.65],
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 10.0],
            [2.97, 10.03, 0.82],
            [0.0, 0.0, 10.0],
            [3.54, 11.51, 0.80],
            [3.70, 14.77, 0.65],
            [3.18, 11.30, 0.82],
            [0.0, 0.0, 10.0],
            [3.33, 12.41, 0.79],
        ];
        let base = if caster.is_player() {
            let class = self.unit_value_u32(caster, offsets::UNIT_FIELD_BYTES_0) >> 8 & 0xFF;
            let idx = (class as usize).min(11);
            let intellect = self.unit_stat(caster, 3);
            let level = self.unit_level(caster) as f32;
            let ratio = CRIT_DATA[idx][1] + CRIT_DATA[idx][2] * level;
            (CRIT_DATA[idx][0] + intellect / ratio).max(0.0)
        } else {
            5.0
        };
        let modded = self.apply_spell_mod(
            caster,
            spell,
            spellmod::CRITICAL_CHANCE,
            base,
            true,
            applied_mods,
        );
        // `MOD_CRIT_PERCENT` aura term.
        modded + self.total_aura_modifier(caster, aura_type::MOD_CRIT_PERCENT, None) as f32
    }

    fn unit_stat(&self, guid: ObjectGuid, stat: u16) -> f32 {
        let index = offsets::UNIT_FIELD_STAT0 + stat;
        f32::from_bits(self.unit_value_u32(guid, index))
    }
}
