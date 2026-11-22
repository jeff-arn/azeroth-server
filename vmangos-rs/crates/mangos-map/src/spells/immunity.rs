//! Spell/damage immunity entries, purges and immune-state refresh.

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// Immunity entries `(spell_id, type)` for one
    /// fork `m_spellImmune[op]` list, derived from
    /// holders plus permanent creature entries
    /// (spell 0) — the `ApplySpellImmune` list as a
    /// recompute. Ops: 0 damage, 1 dispel, 2 effect,
    /// 3 mechanic, 4 school, 5 state.
    fn immune_entries(&self, victim: ObjectGuid, op: u32) -> Vec<(u32, i32)> {
        let aura_kind = match op {
            0 => aura_type::DAMAGE_IMMUNITY,
            1 => aura_type::DISPEL_IMMUNITY,
            2 => aura_type::EFFECT_IMMUNITY,
            3 => aura_type::MECHANIC_IMMUNITY,
            4 => aura_type::SCHOOL_IMMUNITY,
            _ => aura_type::STATE_IMMUNITY,
        };
        let mut out: Vec<(u32, i32)> = self
            .aura_holders(victim)
            .iter()
            .flat_map(|h| {
                h.effects
                    .iter()
                    .flatten()
                    .filter(move |e| e.aura_type == aura_kind)
                    .map(move |e| (h.spell_id, e.misc_value))
            })
            .collect();
        // Permanent creature entries (`Creature::UpdateEntry`
        // tail): type-based, immunity flags, sessile.
        if let Some(c) = self.creatures.get(&victim) {
            let info = &c.info;
            let mut push = |entry_op: u32, ty: i32| {
                if entry_op == op {
                    out.push((0, ty));
                }
            };
            if info.creature_type == 9 {
                push(2, 10);
                push(2, 67);
                push(5, 8);
                push(5, 53);
            }
            if matches!(info.creature_type, 9 | 4) {
                push(1, 3);
                push(1, 4);
            }
            if info.immunity_flags & 0x02 != 0 {
                push(2, 114);
                push(5, 11);
            }
            if info.immunity_flags & 0x04 != 0 {
                push(5, 29);
                push(5, 137);
            }
            if info.immunity_flags & 0x08 != 0 {
                push(5, 65);
            }
            if info.immunity_flags & 0x10 != 0 {
                push(1, 3);
            }
            if info.immunity_flags & 0x20 != 0 {
                push(1, 4);
            }
            if info.immunity_flags & 0x40 != 0 {
                push(1, 2);
            }
            if info.static_flags1 & 0x100 != 0 {
                push(2, 69);
                push(2, 70);
                push(2, 98);
                push(5, 5);
                push(5, 7);
                push(5, 26);
            }
        }
        out
    }

    /// The shared list-entry positivity rule: permanent
    /// (spell 0) or missing immunity spells always
    /// block; otherwise block on positivity mismatch
    /// or EX immunity-to-hostile-and-friendly.
    fn immune_entry_blocks(&self, immunity_spell_id: u32, incoming_positive: bool) -> bool {
        let Some(imm) = self.stores.spells.get(immunity_spell_id) else {
            return true;
        };
        if immunity_spell_id == 0 {
            return true;
        }
        imm.is_positive_spell() != incoming_positive || imm.attributes_ex & 0x0001_0000 != 0
    }

    /// `Unit::IsImmuneToDamage` + creature school-mask
    /// override (`Unit.cpp:5566`, `Creature.cpp:2323`).
    pub(crate) fn is_immune_to_damage(
        &self,
        victim: ObjectGuid,
        school_mask: i32,
        spell: Option<&SpellEntry>,
    ) -> bool {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        if let Some(spell) = spell
            && spell.attributes & NO_IMMUNITIES != 0
        {
            return false;
        }
        if let Some(c) = self.creatures.get(&victim)
            && c.info.school_immune_mask & school_mask as u32 != 0
        {
            return true;
        }
        for (_id, ty) in self.immune_entries(victim, 0) {
            if ty & school_mask != 0 {
                return true;
            }
        }
        let skip_school = spell.is_some_and(|s| s.attributes_ex2 & 0x0400_0000 != 0);
        if !skip_school {
            let incoming_positive = spell.is_some_and(mangos_spells::SpellEntry::is_positive_spell);
            for (id, ty) in self.immune_entries(victim, 4) {
                if ty & school_mask != 0 && self.immune_entry_blocks(id, incoming_positive) {
                    return true;
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSpell` + creature mask override
    /// (`Unit.cpp:5602`, `Creature.cpp:2306`). The WSG
    /// flag-carrier arm is Tier 3 (battlegrounds).
    pub(crate) fn is_immune_to_spell(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        cast_on_self: bool,
    ) -> bool {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        const PASSIVE: u32 = 0x40;
        if spell.attributes & NO_IMMUNITIES != 0 {
            return false;
        }
        if !cast_on_self && let Some(c) = self.creatures.get(&victim) {
            if spell.mechanic != 0
                && c.info.mechanic_immune_mask & (1u32 << (spell.mechanic - 1)) != 0
            {
                return true;
            }
            if c.info.school_immune_mask & (1u32 << spell.school) != 0 && !spell.is_positive_spell()
            {
                return true;
            }
        }
        let positive = spell.is_positive_spell();
        if spell.attributes & PASSIVE == 0 {
            for (id, ty) in self.immune_entries(victim, 1) {
                #[allow(clippy::cast_possible_wrap)]
                if ty == spell.dispel as i32 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        if spell.attributes_ex & 0x8000 == 0 && spell.attributes_ex2 & 0x0400_0000 == 0 {
            for (id, ty) in self.immune_entries(victim, 4) {
                if ty & school_mask != 0 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        if spell.mechanic != 0 {
            #[allow(clippy::cast_possible_wrap)]
            let mechanic = spell.mechanic as i32;
            for (id, ty) in self.immune_entries(victim, 3) {
                if ty == mechanic && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
            let mask = 1i32 << (mechanic - 1);
            for holder in self.aura_holders(victim) {
                for e in holder.effects.iter().flatten() {
                    if e.aura_type == aura_type::MECHANIC_IMMUNITY_MASK
                        && e.misc_value & mask != 0
                        && self.immune_entry_blocks(holder.spell_id, positive)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSpellEffect` + creature
    /// mechanic-mask override (`Unit.cpp:5706`,
    /// `Creature.cpp:2334`).
    pub(crate) fn is_immune_to_spell_effect(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        cast_on_self: bool,
    ) -> bool {
        let eff = &spell.effects[eff_index];
        if !cast_on_self
            && eff.mechanic != 0
            && let Some(c) = self.creatures.get(&victim)
            && c.info.mechanic_immune_mask & (1u32 << (eff.mechanic - 1)) != 0
        {
            return true;
        }
        let positive = spell.is_positive_effect(eff_index);
        #[allow(clippy::cast_possible_wrap)]
        for (id, ty) in self.immune_entries(victim, 2) {
            if ty == eff.effect as i32 && self.immune_entry_blocks(id, positive) {
                return true;
            }
        }
        if eff.mechanic != 0 {
            #[allow(clippy::cast_possible_wrap)]
            let mechanic = eff.mechanic as i32;
            for (id, ty) in self.immune_entries(victim, 3) {
                if ty == mechanic && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
            let mask = 1i32 << (mechanic - 1);
            for holder in self.aura_holders(victim) {
                for e in holder.effects.iter().flatten() {
                    if e.aura_type == aura_type::MECHANIC_IMMUNITY_MASK
                        && e.misc_value & mask != 0
                        && self.immune_entry_blocks(holder.spell_id, positive)
                    {
                        return true;
                    }
                }
            }
        }
        if eff.apply_aura_name != 0 {
            #[allow(clippy::cast_possible_wrap)]
            for (id, ty) in self.immune_entries(victim, 5) {
                if ty == eff.apply_aura_name as i32 && self.immune_entry_blocks(id, positive) {
                    return true;
                }
            }
        }
        false
    }

    /// `Unit::IsImmuneToSchool` (`Unit.cpp:5791`),
    /// per-effect positivity, self-immunity skipped.
    pub(crate) fn is_immune_to_school(
        &self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) -> bool {
        if spell.attributes_ex & 0x8000 != 0 || spell.attributes_ex2 & 0x0400_0000 != 0 {
            return false;
        }
        #[allow(clippy::cast_possible_wrap)]
        let school_mask = 1i32 << spell.school;
        let positive = spell.is_positive_effect(eff_index);
        for (id, ty) in self.immune_entries(victim, 4) {
            if id == spell.id {
                continue;
            }
            if ty & school_mask != 0 && self.immune_entry_blocks(id, positive) {
                return true;
            }
        }
        false
    }

    /// Mechanic mask of a spell (`HasMechanicMask`):
    /// spell mechanic bit + effect mechanic bits.
    pub(crate) fn mechanic_mask(spell: &SpellEntry) -> i32 {
        let mut mask = 0i32;
        if spell.mechanic != 0 {
            mask |= 1i32 << (spell.mechanic - 1);
        }
        for e in &spell.effects {
            if e.mechanic != 0 {
                mask |= 1i32 << (e.mechanic - 1);
            }
        }
        mask
    }

    /// Immunity-aura apply side effects (fork handlers
    /// at `SpellAuras.cpp:3933-4095`): purges gated by
    /// `EX_IMMUNITY_PURGES_EFFECT`, plus the school
    /// immunity flag-aura strip. WSG flag-carrier arm
    /// of effect immunity is Tier 3.
    pub(crate) fn apply_immunity_purges(&mut self, victim: ObjectGuid, spell: &SpellEntry) {
        const PURGES: u32 = 0x8000;
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        let purges = spell.attributes_ex & PURGES != 0;
        for e in &spell.effects {
            match e.apply_aura_name {
                aura_type::MECHANIC_IMMUNITY if purges && e.misc_value > 0 => {
                    self.remove_auras_at_mechanic_immunity(
                        victim,
                        1i32 << (e.misc_value - 1),
                        spell.id,
                        false,
                    );
                }
                aura_type::MECHANIC_IMMUNITY_MASK if purges => {
                    self.remove_auras_at_mechanic_immunity(victim, e.misc_value, spell.id, false);
                }
                aura_type::STATE_IMMUNITY if purges => {
                    let ids: Vec<u32> = self
                        .aura_holders(victim)
                        .iter()
                        .filter(|h| h.spell_id != spell.id)
                        .filter(|h| {
                            h.effects
                                .iter()
                                .flatten()
                                .any(|x| x.aura_type == e.misc_value as u32)
                        })
                        .map(|h| h.spell_id)
                        .collect();
                    for id in ids {
                        self.remove_auras_due_to_spell(victim, id);
                    }
                }
                aura_type::SCHOOL_IMMUNITY => {
                    if spell.is_positive_spell()
                        && self.players.contains_key(&victim)
                        && self.pet_owner(victim).is_none()
                    {
                        self.remove_auras_with_interrupt_flags(
                            victim,
                            aura_interrupt_flag::INVULNERABILITY_BUFF_CANCELS,
                            0,
                        );
                    }
                    if purges && spell.is_positive_spell() {
                        let ids: Vec<u32> = self
                            .aura_holders(victim)
                            .iter()
                            .filter(|h| !h.positive && h.spell_id != spell.id)
                            .filter(|h| {
                                self.stores.spells.get(h.spell_id).is_some_and(|s| {
                                    (1i32 << s.school) & e.misc_value != 0
                                        && s.attributes & NO_IMMUNITIES == 0
                                })
                            })
                            .map(|h| h.spell_id)
                            .collect();
                        for id in ids {
                            self.remove_auras_due_to_spell(victim, id);
                        }
                    }
                }
                aura_type::DISPEL_IMMUNITY if purges => {
                    let ids: Vec<u32> = self
                        .aura_holders(victim)
                        .iter()
                        .filter(|h| {
                            self.stores.spells.get(h.spell_id).is_some_and(|s| {
                                #[allow(clippy::cast_possible_wrap)]
                                {
                                    s.dispel as i32 == e.misc_value
                                }
                            })
                        })
                        .map(|h| h.spell_id)
                        .collect();
                    for id in ids {
                        self.remove_auras_due_to_spell(victim, id);
                    }
                }
                _ => {}
            }
        }
    }

    /// `Unit::RemoveAurasAtMechanicImmunity`.
    pub(crate) fn remove_auras_at_mechanic_immunity(
        &mut self,
        victim: ObjectGuid,
        mech_mask: i32,
        except_spell_id: u32,
        non_positive: bool,
    ) {
        const NO_IMMUNITIES: u32 = 0x2000_0000;
        let ids: Vec<u32> = self
            .aura_holders(victim)
            .iter()
            .filter(|h| h.spell_id != except_spell_id)
            .filter(|h| !(non_positive && h.positive))
            .filter(|h| {
                self.stores.spells.get(h.spell_id).is_some_and(|s| {
                    s.attributes & NO_IMMUNITIES == 0 && Self::mechanic_mask(s) & mech_mask != 0
                })
            })
            .map(|h| h.spell_id)
            .collect();
        for id in ids {
            self.remove_auras_due_to_spell(victim, id);
        }
    }

    /// Holder-derived `UNIT_FLAG_IMMUNE` +
    /// `UNIT_STATE_ISOLATED` (school-immunity handler
    /// flag arms as a recompute).
    pub(crate) fn refresh_immune_state(&mut self, guid: ObjectGuid) {
        const PURGES: u32 = 0x8000;
        const UNIT_FLAG_IMMUNE: u32 = 0x8000_0000;
        let mut immune_flag = false;
        let mut isolated = false;
        for holder in self.aura_holders(guid) {
            let Some(spell) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if spell.attributes_ex & PURGES != 0
                && spell.is_positive_spell()
                && holder
                    .effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == aura_type::SCHOOL_IMMUNITY)
            {
                immune_flag = true;
            }
            if spell.mechanic == 5 {
                isolated = true;
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let flags = unit.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            let new_flags = if immune_flag {
                flags | UNIT_FLAG_IMMUNE
            } else {
                flags & !UNIT_FLAG_IMMUNE
            };
            if new_flags != flags {
                unit.values().set_u32(offsets::UNIT_FIELD_FLAGS, new_flags);
            }
            if isolated {
                unit.add_unit_state(mangos_entities::unit::UNIT_STATE_ISOLATED);
            } else {
                unit.clear_unit_state(mangos_entities::unit::UNIT_STATE_ISOLATED);
            }
        }
    }

    /// Holder-derived attack-speed percent
    /// (`ApplyAttackTimePercentMod` as a recompute):
    /// rewrites the three attack-time fields by
    /// old→new pct ratio and stores the new pcts.
    /// Haste factor per amount: positive divides
    /// (100/(100+v)), negative multiplies
    /// ((100-v)/100) at build 5875. `SPELLMOD_HASTE`
    /// applies per holder amount (fork applies it
    /// once at aura apply).
    #[allow(clippy::float_cmp)] // exact pct change gate, as in the fork field rewrite
    pub(crate) fn refresh_attack_speed(&mut self, guid: ObjectGuid) {
        #[allow(clippy::cast_precision_loss)]
        let factor = |amount: i32| -> f32 {
            let v = amount as f32;
            if v > 0.0 {
                100.0 / (100.0 + v)
            } else if v < 0.0 {
                (100.0 - v) / 100.0
            } else {
                1.0
            }
        };
        let mut pct = [1.0f32; 3];
        for holder in self.aura_holders(guid) {
            let Some(spell) = self.stores.spells.get(holder.spell_id).cloned() else {
                continue;
            };
            for e in holder.effects.iter().flatten() {
                let slots: &[usize] = match e.aura_type {
                    aura_type::MOD_ATTACKSPEED => &[0, 1, 2],
                    aura_type::MOD_MELEE_HASTE => &[0, 1],
                    aura_type::MOD_RANGED_HASTE | aura_type::MOD_RANGED_AMMO_HASTE => &[2],
                    _ => continue,
                };
                #[allow(clippy::cast_precision_loss)]
                let amount = self.apply_spell_mod(
                    holder.caster,
                    &spell,
                    spellmod::HASTE,
                    e.amount as f32,
                    false,
                    None,
                );
                #[allow(clippy::cast_possible_truncation)]
                let f = factor(amount as i32);
                for &slot in slots {
                    pct[slot] *= f;
                }
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let old = unit.mod_attack_speed_pct;
            if old == pct {
                return;
            }
            for slot in 0..3 {
                if old[slot] == 0.0 || old[slot] == pct[slot] {
                    continue;
                }
                let field = match slot {
                    0 => offsets::UNIT_FIELD_BASEATTACKTIME,
                    1 => offsets::UNIT_FIELD_BASEATTACKTIME + 1,
                    _ => offsets::UNIT_FIELD_RANGEDATTACKTIME,
                };
                let cur = unit.values().get_f32(field);
                if cur != 0.0 {
                    unit.values().set_f32(field, cur / old[slot] * pct[slot]);
                }
            }
            unit.mod_attack_speed_pct = pct;
        }
    }

    /// Holder-derived `PLAYER_SKILL_BONUS` packed
    /// fields (`ModifySkillBonus` as a recompute):
    /// temporary `MOD_SKILL` low half, permanent
    /// `MOD_SKILL_TALENT` high half, keyed by the
    /// skill id in each slot.
    pub(crate) fn refresh_skill_bonuses(&mut self, guid: ObjectGuid) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut temp: std::collections::HashMap<u32, i32> = std::collections::HashMap::new();
        let mut perm: std::collections::HashMap<u32, i32> = std::collections::HashMap::new();
        for holder in self.aura_holders(guid) {
            for e in holder.effects.iter().flatten() {
                let map = match e.aura_type {
                    aura_type::MOD_SKILL => &mut temp,
                    aura_type::MOD_SKILL_TALENT => &mut perm,
                    _ => continue,
                };
                #[allow(clippy::cast_sign_loss)]
                {
                    *map.entry(e.misc_value as u32).or_insert(0) += e.amount;
                }
            }
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            for slot in 0..127u16 {
                let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
                let skill = unit.values().get_u32(base);
                if skill == 0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let packed = |v: i32| (v as i16) as u16 as u32;
                let bonus = packed(temp.get(&skill).copied().unwrap_or(0))
                    | (packed(perm.get(&skill).copied().unwrap_or(0)) << 16);
                if unit.values().get_u32(base + 2) != bonus {
                    unit.values().set_u32(base + 2, bonus);
                }
            }
        }
    }

    /// `SendSpellOrDamageImmune` (`SpellCaster.cpp:673`).
    pub(super) fn send_spell_or_damage_immune(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell_id: u32,
    ) {
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(caster));
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(spell_id);
        payload.write_u8(0);
        let packet = WorldPacket::with_body(op::SMSG_SPELLORDAMAGE_IMMUNE, payload.into_vec());
        self.broadcast_movement(caster, &packet);
    }
}
