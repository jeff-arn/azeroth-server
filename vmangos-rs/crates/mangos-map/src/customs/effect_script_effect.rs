//! `Spell::EffectScriptEffect` customs (`SpellEffects.cpp:3578-4467`).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `Spell::EffectScriptEffect`.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn effect_script_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        damage: f32,
        has_cast_item: bool,
    ) {
        // Family arms (fork switches on family
        // first; generic ids never overlap).
        match spell.spell_family_name {
            // SPELLFAMILY_PALADIN
            // (SpellEffects.cpp:4378-4437).
            10 => {
                self.script_paladin(caster, spell, victim, damage);
                return;
            }
            // SPELLFAMILY_DRUID shapeshift form
            // effect (SpellEffects.cpp:4338-4377).
            7 if spell.id == 9033 => {
                self.script_shapeshift_form(victim);
                return;
            }
            _ => {}
        }
        match spell.id {
            // Rockbiter Weapon Proc threat
            // (SHAMAN arm, SpellEffects.cpp:4441-4456).
            20865 | 20866 | 20867 | 20868 | 20870 | 20871 => {
                self.threat_from_weapon_proc(caster, victim, damage);
            }
            // Bending Shinbone
            // (SpellEffects.cpp:3586-3594). Item
            // *target* gate approximated by the
            // cast-item flag (see 17271).
            8856 => {
                if !has_cast_item && !caster.is_player() {
                    return;
                }
                let spell_id = if self.urand(1, 5) == 1 { 8854 } else { 8855 };
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Jubling Cooldown
            // (SpellEffects.cpp:3638-3646).
            23853 => {
                if let Some(entry) = self.stores.spells.get(23851).cloned() {
                    let ctx = CastItemCtx {
                        item_guid: 0,
                        item_entry: 19462,
                    };
                    let mut mods = Vec::new();
                    self.add_cooldowns_item(victim, &entry, ctx, &mut mods);
                }
            }
            // Uther's / Grom's Tribute
            // (SpellEffects.cpp:3647-3698).
            24194 | 24195 => {
                if !caster.is_player() {
                    return;
                }
                let race = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::race);
                let spell_id = match (spell.id, race) {
                    (24194, 1) => 24105,
                    (24194, 3) => 24107,
                    (24194, 4) => 24108,
                    (24194, 7) => 24106,
                    (24195, 2) => 24104,
                    (24195, 5) => 24103,
                    (24195, 6) => 24102,
                    (24195, 8) => 24101,
                    _ => 0,
                };
                if spell_id != 0 {
                    self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
                }
            }
            // Stack burns
            // (SpellEffects.cpp:3722-3727, 3952-3954).
            24590 => {
                self.remove_aura_holder_from_stack(victim, 24575);
            }
            24660 => {
                self.remove_aura_holder_from_stack(victim, 24659);
            }
            26465 => {
                self.remove_aura_holder_from_stack(victim, 26464);
            }
            // Trick (SpellEffects.cpp:3739-3761).
            24714 => {
                if !caster.is_player() {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spells: [u32; 8] = [
                    if male { 24708 } else { 24709 },
                    if male { 24711 } else { 24710 },
                    if male { 24712 } else { 24713 },
                    if male { 24735 } else { 24736 },
                    24723,
                    24732,
                    24740,
                    24753,
                ];
                let spell_id = self.pick_spell(&spells);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Hallowed wand costumes
            // (SpellEffects.cpp:3762-3860).
            24717 | 24718 | 24719 | 24737 => {
                if !victim.is_player()
                    || self
                        .unit_base(victim)
                        .is_some_and(mangos_entities::unit::UnitBase::is_in_combat)
                {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spell_id = match (spell.id, male) {
                    (24717, true) => 24708,
                    (24717, false) => 24709,
                    (24718, true) => 24711,
                    (24718, false) => 24710,
                    (24719, true) => 24712,
                    (24719, false) => 24713,
                    (24737, true) => 24735,
                    _ => 24736,
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Random Costume
            // (SpellEffects.cpp:3801-3839).
            24720 => {
                if !victim.is_player()
                    || self
                        .unit_base(victim)
                        .is_some_and(mangos_entities::unit::UnitBase::is_in_combat)
                {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spell_id = match self.urand(0, 6) {
                    0 => {
                        if male {
                            24708
                        } else {
                            24709
                        }
                    }
                    1 => {
                        if male {
                            24711
                        } else {
                            24710
                        }
                    }
                    2 => {
                        if male {
                            24712
                        } else {
                            24713
                        }
                    }
                    3 => 24723,
                    4 => 24732,
                    5 => {
                        if male {
                            24735
                        } else {
                            24736
                        }
                    }
                    _ => 24740,
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Cannon Fire / Magic Wings
            // (SpellEffects.cpp:3840-3868).
            24731 => {
                if caster.is_player() {
                    self.cast_custom_spell(victim, caster, 24742, [None; 3]);
                }
            }
            24742 => {
                self.remove_auras_due_to_spell(victim, 24754);
            }
            // Trick or Treat
            // (SpellEffects.cpp:3869-3880).
            24751 => {
                if !victim.is_player() {
                    return;
                }
                self.cast_custom_spell(victim, victim, 24755, [None; 3]);
                let spell_id = if self.roll_chance_i(50) { 24714 } else { 24715 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Mistletoe (SpellEffects.cpp:3913-3936).
            26004 => {
                self.cast_custom_spell(victim, victim, 26005, [None; 3]);
            }
            26218 => {
                if !victim.is_player() {
                    return;
                }
                let spell_id = if self.urand(0, 1) == 1 { 26206 } else { 26207 };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Rotate Trigger
            // (SpellEffects.cpp:3921-3928).
            26137 => {
                let spell_id = if self.urand(0, 1) == 1 { 26009 } else { 26136 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Winter Wondervolt TRAP
            // (SpellEffects.cpp:3937-3951).
            26275 => {
                if !self.within_dist(caster, victim, 1.0) {
                    return;
                }
                for old in [26272u32, 26157, 26273, 26274] {
                    if self.unit_has_aura(victim, old) {
                        self.remove_auras_due_to_spell(victim, old);
                    }
                }
                let spell_id = self.pick_spell(&[26272, 26157, 26273, 26274]);
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Summon Black Qiraji Battle Tank
            // (SpellEffects.cpp:3955-3974). No
            // Map.dbc mount flag is loaded, so the
            // outdoor spell is used (recorded).
            26656 => {
                self.remove_spells_causing_aura(victim, aura_type::MOUNTED);
                self.cast_custom_spell(victim, victim, 26655, [None; 3]);
            }
            // Bag of Candies
            // (SpellEffects.cpp:4260-4268).
            26678 => {
                let spell_id =
                    self.pick_spell(&[26668, 26670, 26671, 26672, 26673, 26674, 26675, 26676]);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Knock Away (SpellEffects.cpp:3595-3603).
            10101 => {
                self.modify_threat_percent_of(caster, victim, -100);
            }
            // Piccolo of the Flaming Fire
            // (SpellEffects.cpp:3604-3612).
            17512 => {
                if victim.is_player() {
                    self.handle_emote_command(victim, 10);
                }
            }
            // Shadow Flame (SpellEffects.cpp:3613-3637).
            22539 | 22972 | 22975 | 22976 | 22977 | 22978 | 22979 | 22980 | 22981 | 22982
            | 22983 | 22984 | 22985 => {
                if self.unit_is_alive(victim) && !self.unit_has_aura(victim, 22683) {
                    self.cast_custom_spell(caster, victim, 22682, [None; 3]);
                }
            }
            // Poisonous Blood
            // (SpellEffects.cpp:3699-3703).
            24320 => {
                self.cast_custom_spell(victim, victim, 24321, [None; 3]);
            }
            // Blood Siphon (SpellEffects.cpp:3704-3721).
            24324 => {
                if !self.unit_is_alive(victim) {
                    return;
                }
                let spell_id = if self.unit_has_aura(victim, 24321) {
                    24323
                } else {
                    24322
                };
                self.cast_custom_spell(victim, caster, spell_id, [None; 3]);
            }
            // Drain Mana (SpellEffects.cpp:3894-3912).
            25671 | 25755 => {
                self.cast_custom_spell(victim, caster, 26639, [None; 3]);
            }
            25676 | 26559 => {
                self.cast_custom_spell(caster, victim, 25671, [None; 3]);
            }
            25754 | 26457 => {
                self.cast_custom_spell(caster, victim, 25755, [None; 3]);
            }
            // Valentine, guards
            // (SpellEffects.cpp:3975-4028).
            26923 => {
                if !victim.is_player() || self.unit_has_aura(victim, 26898) {
                    return;
                }
                let tokens = self
                    .players
                    .get(&victim)
                    .map_or(0, |e| e.player.quest_item_count(21815));
                if tokens == 0 {
                    return;
                }
                let faction = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::faction_template);
                let pair = match faction {
                    11 => (27242, 27510),
                    85 => (27247, 27507),
                    57 => (27244, 27506),
                    68 | 71 => (27246, 27515),
                    79 => (27245, 27504),
                    105 => (27248, 27513),
                    _ => return,
                };
                self.valentine(victim, pair);
            }
            // Valentine, citizens
            // (SpellEffects.cpp:4029-4090).
            26663 | 27541 | 27547 | 27548 | 27549 | 27550 => {
                if !victim.is_player() {
                    return;
                }
                let faction = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::faction_template);
                let pair = match faction {
                    12 | 123 => (27525, 27509),
                    29 | 83 | 85 | 126 | 876 | 1215 => (27523, 27505),
                    55 | 57 | 875 => (27520, 27503),
                    68 => (27529, 27512),
                    80 => (27519, 26901),
                    104 | 105 | 412 => (27524, 27511),
                    _ => return,
                };
                self.valentine(victim, pair);
            }
            // Amorous timers
            // (SpellEffects.cpp:4091-4102).
            27654 | 26870 => {
                if victim.is_player() {
                    return;
                }
                if let Some(u) = self.unit_base_mut(victim) {
                    let mut flags = u.values().get_u32(offsets::UNIT_NPC_FLAGS);
                    flags |= 0x0000_0001;
                    if flags & 0x0000_0080 != 0 {
                        flags |= 0x0000_0004;
                    }
                    u.values().set_u32(offsets::UNIT_NPC_FLAGS, flags);
                }
                let spell_id = if spell.id == 26870 { 26869 } else { 27741 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Valentine End Check
            // (SpellEffects.cpp:4269-4280). Game
            // events are unmodeled; event 8 reads
            // inactive, as in the fork off-season.
            27657 => {
                for id in [26870u32, 27742, 26869, 27654, 27741] {
                    self.remove_auras_due_to_spell(caster, id);
                }
            }
            // Teleport to Razelikh, group
            // (SpellEffects.cpp:4281-4296).
            27686 => {
                let Some(group) = self.players.get(&victim).and_then(|e| e.player.group_id) else {
                    return;
                };
                let members: Vec<ObjectGuid> = self
                    .players
                    .iter()
                    .filter(|(_, e)| e.player.group_id == Some(group))
                    .map(|(g, _)| *g)
                    .collect();
                for member in members {
                    if self.within_dist(caster, member, 20.0) {
                        self.cast_custom_spell(member, member, 13142, [None; 3]);
                    }
                }
            }
            // Summon Bone Minions / Mages
            // (SpellEffects.cpp:4103-4124).
            27687 => {
                for id in [27690u32, 27691, 27692, 27693] {
                    self.cast_custom_spell(victim, victim, id, [None; 3]);
                }
            }
            27695 => {
                for id in [27696u32, 27697, 27698, 27699] {
                    self.cast_custom_spell(victim, victim, id, [None; 3]);
                }
            }
            // Breath of Sargeras
            // (SpellEffects.cpp:4220-4225).
            28352 => {
                self.cast_custom_spell(caster, victim, 28342, [None; 3]);
            }
            // Chains of Kel'Thuzad
            // (SpellEffects.cpp:4141-4181).
            28408 => {
                let mut viable: Vec<ObjectGuid> = self
                    .threat_targets(caster)
                    .into_iter()
                    .filter(|g| g.is_player() && self.unit_is_alive(*g))
                    .collect();
                #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
                let mut num_targets = viable.len().min(5) as i32 - 1;
                if let Some(maintank) = self.current_victim(caster) {
                    viable.retain(|g| *g != maintank);
                    num_targets -= 1;
                    self.cast_custom_spell(maintank, maintank, 28409, [None; 3]);
                    self.cast_custom_spell(caster, maintank, 28410, [None; 3]);
                }
                while num_targets > 0 && !viable.is_empty() {
                    #[allow(clippy::cast_possible_truncation)]
                    let idx = self.urand(0, viable.len() as u32 - 1) as usize;
                    let target = viable.remove(idx);
                    self.cast_custom_spell(target, target, 28409, [None; 3]);
                    self.cast_custom_spell(caster, target, 28410, [None; 3]);
                    num_targets -= 1;
                }
            }
            // Summon Blizzard
            // (SpellEffects.cpp:4182-4189).
            28560 => {
                self.cast_custom_spell(victim, victim, 28561, [None; 3]);
            }
            // Widow's Embrace
            // (SpellEffects.cpp:4297-4302).
            28732 => {
                if self.unit_is_alive(caster) {
                    self.kill_unit(caster, None);
                }
            }
            // Teleports, player self-cast
            // (SpellEffects.cpp:4199-4210).
            29182 => {
                if victim.is_player() {
                    self.cast_custom_spell(victim, victim, 29181, [None; 3]);
                }
            }
            29188 => {
                if victim.is_player() {
                    self.cast_custom_spell(victim, victim, 29190, [None; 3]);
                }
            }
            // Ribbon Pole Channel Trigger
            // (SpellEffects.cpp:4303-4308).
            29710 => {
                if victim.is_player() {
                    let spell_id = self.pick_spell(&[29705, 29726, 29727]);
                    self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
                }
            }
            // Despawn Ice Block
            // (SpellEffects.cpp:4211-4219).
            30132 => {
                if victim.is_player() {
                    self.remove_auras_due_to_spell(victim, 31800);
                    self.remove_auras_due_to_spell(victim, 28522);
                }
            }
            // Improved Sprint
            // (SpellEffects.cpp:4190-4198).
            30918 => {
                self.remove_auras_at_mechanic_immunity(victim, ROOT_AND_SNARE_MASK, 30918, true);
            }
            // EPL tower capture
            // (SpellEffects.cpp:4325-4330).
            31929 if victim.is_player() => {
                self.cast_custom_spell(victim, victim, 32061, [None; 3]);
            }
            _ => {}
        }
    }

    /// Shared Valentine body (guards/citizens):
    /// 1-in-6 heartbroken, else adored + pledge.
    fn valentine(&mut self, victim: ObjectGuid, pair: (u32, u32)) {
        let (friendship, adoration) = pair;
        let broken = self.urand(0, 5) == 0;
        let pledge = if !broken && !self.unit_has_aura(victim, 26680) {
            adoration
        } else {
            friendship
        };
        self.cast_custom_spell(
            victim,
            victim,
            if broken { 26898 } else { 26680 },
            [None; 3],
        );
        if broken {
            return;
        }
        self.cast_custom_spell(victim, victim, pledge, [None; 3]);
        self.cast_custom_spell(victim, victim, 26879, [None; 3]);
    }

    /// Rockbiter / Seal of Fury threat term.
    fn threat_from_weapon_proc(&mut self, caster: ObjectGuid, victim: ObjectGuid, damage: f32) {
        if !victim.is_creature() {
            return;
        }
        if self.threat_on(victim, caster) != 0.0 {
            let attack_time =
                f32::from_bits(self.unit_value_u32(caster, offsets::UNIT_FIELD_BASEATTACKTIME));
            self.add_threat(victim, caster, damage * attack_time / 1000.0);
        }
    }

    /// Paladin family arms
    /// (SpellEffects.cpp:4378-4437).
    fn script_paladin(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        damage: f32,
    ) {
        // Flash of Light carrier chain.
        if spell.spell_icon_id == 242 {
            if !self.unit_is_alive(victim) {
                return;
            }
            let mut heal = mangos_spells::dither(damage, self.spell_rng());
            if self.unit_has_aura(caster, 28853) {
                heal += 53;
            }
            if self.unit_has_aura(caster, 28851) {
                heal += 83;
            }
            #[allow(clippy::cast_possible_wrap)]
            let bp1 = spell.id as i32;
            self.cast_custom_spell(caster, victim, 19993, [Some(heal), Some(bp1), None]);
            return;
        }
        // Judgement: consume the seal, cast its
        // judgement spell (CF_PALADIN_JUDGEMENT,
        // CM0 bit 23).
        if spell.spell_family_flags & 0x0080_0000 != 0 {
            if !self.unit_is_alive(victim) {
                return;
            }
            let seal = self.aura_holders(caster).iter().find_map(|h| {
                let dummy_at_2 = h.effects[2]
                    .as_ref()
                    .is_some_and(|e| e.aura_type == aura_type::DUMMY);
                if !dummy_at_2 {
                    return None;
                }
                let proto = self.stores.spells.get(h.spell_id)?;
                let is_seal = proto.spell_family_name == 10
                    && proto.spell_family_flags & (0x200 | 0x0200_0000 | 0x0800_0000) != 0;
                let judgement = proto.simple_value(2);
                (is_seal && judgement > 1).then_some((h.spell_id, judgement))
            });
            if let Some((seal_id, judgement)) = seal {
                self.remove_auras_due_to_spell(caster, seal_id);
                #[allow(clippy::cast_sign_loss)]
                self.cast_custom_spell(caster, victim, judgement as u32, [None; 3]);
            }
            return;
        }
        // Seal of Fury proc.
        if spell.spell_family_flags == 4096 {
            self.threat_from_weapon_proc(caster, victim, damage);
        }
    }

    /// Druid shapeshift form effect 9033
    /// (SpellEffects.cpp:4340-4375): strip roots
    /// and removable snares.
    fn script_shapeshift_form(&mut self, target: ObjectGuid) {
        let mut remove: Vec<u32> = Vec::new();
        for h in self.aura_holders(target) {
            let Some(proto) = self.stores.spells.get(h.spell_id) else {
                continue;
            };
            let mask = Self::mechanic_mask(proto);
            if mask == 0 {
                continue;
            }
            let has_root = h
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_ROOT);
            if has_root {
                remove.push(h.spell_id);
                continue;
            }
            let has_slow = h
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_DECREASE_SPEED);
            if !has_slow {
                continue;
            }
            if mask & NOT_REMOVED_BY_SHAPESHIFT != 0 {
                continue;
            }
            // Daze-by-icon exception.
            if proto.spell_icon_id == 15 && proto.dispel == 0 && mask & (1 << 10) == 0 {
                continue;
            }
            remove.push(h.spell_id);
        }
        for id in remove {
            self.remove_auras_due_to_spell(target, id);
        }
    }
}
