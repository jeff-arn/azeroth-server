//! Totem summon/lifecycle (`Totem.cpp`, `TotemAI.cpp` subset).

#[allow(clippy::wildcard_imports)]
use super::*;

/// `UNIT_FLAG_PLAYER_CONTROLLED` (`UnitDefines.h:544`).
const UNIT_FLAG_PLAYER_CONTROLLED: u32 = 0x0000_0008;

impl Map {
    /// `Spell::EffectSummonTotem` (`SpellEffects.cpp:4817`):
    /// unsummon the slot's old totem, create the new one at
    /// the slot angle 2 yd out, stamp owner/level/faction/
    /// created-by fields, set health from the effect value
    /// when nonzero, register the slot, and (passive
    /// totems) cast the totem spell on itself. Spawn/despawn
    /// anims and creature-owner AI hooks are unmodeled.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::too_many_lines
    )] // one EffectSummonTotem port
    pub(crate) fn summon_totem_effect(
        &mut self,
        caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        i: usize,
        bp: Option<i32>,
    ) {
        let eff = spell.effects[i];
        let slot = match eff.effect {
            effect::SUMMON_TOTEM_SLOT1 => Some(0),
            effect::SUMMON_TOTEM_SLOT2 => Some(1),
            effect::SUMMON_TOTEM_SLOT3 => Some(2),
            effect::SUMMON_TOTEM_SLOT4 => Some(3),
            _ => None,
        };
        if eff.misc_value <= 0 {
            return;
        }
        let entry = eff.misc_value as u32;
        let Some(info) = self.stores.creature_templates.get(entry).cloned() else {
            return;
        };
        if let Some(slot) = slot {
            let old = self
                .unit_base(caster)
                .map_or(ObjectGuid::EMPTY, |u| u.totem_slots[slot]);
            if !old.is_empty() {
                self.unsummon_totem(old);
            }
        }
        let Some(cpos) = self.unit_position(caster) else {
            return;
        };
        let angle = slot.map_or(0.0, |s| {
            std::f32::consts::PI / 4.0 - (s as f32) * 2.0 * std::f32::consts::PI / 4.0
        });
        let heading = cpos.o + angle;
        let mut pos = cpos;
        pos.x += 2.0 * heading.cos();
        pos.y += 2.0 * heading.sin();
        pos.z = self.ground_z(pos.x, pos.y, pos.z);
        pos.o = cpos.o;
        // Totems manage their own lifecycle in
        // `update_totems`; the temp-summon row the generic
        // summon creates would double-expire them.
        let Some(totem) = self.summon_creature(entry, pos, caster, 0, 0) else {
            return;
        };
        self.summons.remove(&totem);
        let duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        let totem_spell_id = info.spells.first().copied().unwrap_or(0);
        let active = totem_spell_id != 0
            && self
                .stores
                .spells
                .get(totem_spell_id)
                .is_some_and(|s| s.cast_time_ms(&self.stores.spells.tables) > 0);
        let owner_faction = self
            .unit_base(caster)
            .map_or(0, mangos_entities::unit::UnitBase::faction_template);
        let owner_level = self.unit_level(caster);
        let value = self.effect_value_bp(spell, i, caster_level, bp);
        if let Some(unit) = self.unit_base_mut(totem) {
            unit.set_faction_template(owner_faction);
            let v = unit.world.object.values_mut();
            v.set_guid(offsets::UNIT_FIELD_SUMMONEDBY, caster);
            v.set_guid(offsets::UNIT_FIELD_CREATEDBY, caster);
            v.set_u32(offsets::UNIT_CREATED_BY_SPELL, spell.id);
            v.set_u32(offsets::UNIT_FIELD_LEVEL, owner_level);
            if caster.is_player() {
                let flags = v.get_u32(offsets::UNIT_FIELD_FLAGS);
                v.set_u32(
                    offsets::UNIT_FIELD_FLAGS,
                    flags | UNIT_FLAG_PLAYER_CONTROLLED,
                );
            }
            if value > 0.0 {
                unit.set_max_health(value as u32);
                unit.set_health(value as u32);
            }
        }
        if let Some(slot) = slot
            && let Some(unit) = self.unit_base_mut(caster)
        {
            unit.totem_slots[slot] = totem;
        }
        self.totems.insert(
            totem,
            crate::map::TotemState {
                owner: caster,
                slot,
                totem_spell_id,
                active,
                duration_ms: if duration > 0 {
                    Some(duration as u32)
                } else {
                    None
                },
                victim: ObjectGuid::EMPTY,
            },
        );
        self.spell_script_on_summon(caster, totem, spell);
        // Passive totems cast their spell on themselves at
        // summon (`Totem::Summon`); area auras then pulse
        // from the totem through the usual machinery.
        if totem_spell_id != 0 && !active {
            self.cast_spell(
                totem,
                totem_spell_id,
                CastTarget::SelfCast,
                CastOptions {
                    triggered: true,
                    triggered_by_aura: false,
                    skip_known: true,
                    cast_item: None,
                    custom_base_points: [None; 3],
                    triggered_by_aura_spell: 0,
                    triggered_by_spell: spell.id,
                },
            );
        }
    }

    /// `Totem::UnSummon` subset: drop the totem spell's
    /// auras from the totem, its owner, and same-group
    /// players on the map; clear the owner slot; despawn
    /// without respawn. Despawn anim and creature-owner
    /// AI hooks are unmodeled.
    pub(crate) fn unsummon_totem(&mut self, totem: ObjectGuid) {
        let Some(state) = self.totems.remove(&totem) else {
            return;
        };
        if state.totem_spell_id != 0 {
            self.remove_auras_due_to_spell(totem, state.totem_spell_id);
            self.remove_auras_due_to_spell(state.owner, state.totem_spell_id);
            let owner_group = self.player(state.owner).and_then(|p| p.group_id);
            if let Some(group_id) = owner_group {
                let members: Vec<ObjectGuid> = self
                    .players
                    .keys()
                    .copied()
                    .filter(|g| {
                        *g != state.owner
                            && self.player(*g).and_then(|p| p.group_id) == Some(group_id)
                    })
                    .collect();
                for member in members {
                    self.remove_auras_due_to_spell(member, state.totem_spell_id);
                }
            }
        }
        if let Some(slot) = state.slot
            && let Some(unit) = self.unit_base_mut(state.owner)
            && unit.totem_slots[slot] == totem
        {
            unit.totem_slots[slot] = ObjectGuid::EMPTY;
        }
        if self.creatures.contains_key(&totem) {
            self.despawn_creature_now(totem, u32::MAX);
        }
    }

    /// `Totem::Update`: unsummon when the owner is gone
    /// (player owners only — creature-owned totems
    /// persist past owner death), the totem is dead,
    /// or the duration runs out. The final creature
    /// update before expiry happens through the normal
    /// AI tick order.
    pub(crate) fn update_totems(&mut self, diff_ms: u32) {
        if self.totems.is_empty() {
            return;
        }
        let mut expired: Vec<ObjectGuid> = Vec::new();
        let guids: Vec<ObjectGuid> = self.totems.keys().copied().collect();
        for guid in guids {
            let Some(state) = self.totems.get(&guid) else {
                continue;
            };
            let owner = state.owner;
            let owner_gone = if owner.is_player() {
                !self.players.contains_key(&owner) || !self.unit_is_alive(owner)
            } else {
                !self.creatures.contains_key(&owner)
            };
            if owner_gone || !self.unit_is_alive(guid) {
                expired.push(guid);
                continue;
            }
            if let Some(state) = self.totems.get_mut(&guid)
                && let Some(remaining) = &mut state.duration_ms
            {
                if *remaining <= diff_ms {
                    expired.push(guid);
                    continue;
                }
                *remaining -= diff_ms;
            }
        }
        for guid in expired {
            self.unsummon_totem(guid);
        }
    }

    /// `TotemAI::UpdateAI` active arm: keep the current
    /// victim while it stays valid (alive, attackable,
    /// in spell range), else take the owner's helper
    /// attacker, else the nearest attackable unit in
    /// range; cast the totem spell at it. Visibility
    /// nuance unmodeled.
    pub(crate) fn totem_ai_update(&mut self, guid: ObjectGuid) {
        let Some(state) = self.totems.get(&guid) else {
            return;
        };
        if !state.active || state.totem_spell_id == 0 || self.casts.contains_key(&guid) {
            return;
        }
        let (owner, spell_id, mut victim) = (state.owner, state.totem_spell_id, state.victim);
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        let max_range = spell.range_max(&self.stores.spells.tables);
        // Hostility is evaluated as the owner: the fork
        // totem carries the owner's faction, and the
        // subset hostility model keys on player-vs-creature.
        let valid = |map: &Self, v: ObjectGuid| {
            !v.is_empty()
                && map.unit_is_alive(v)
                && Self::is_hostile_pair(owner, v)
                && map
                    .unit_position(v)
                    .zip(map.unit_position(guid))
                    .is_some_and(|(a, b)| {
                        (a.x - b.x).powi(2) + (a.y - b.y).powi(2) <= max_range * max_range
                    })
        };
        if !valid(self, victim) {
            // Owner's attackers first (`GetAttackerForHelper`
            // approximation: a live unit meleeing the owner).
            victim = self
                .creatures
                .keys()
                .copied()
                .chain(self.players.keys().copied())
                .find(|g| {
                    valid(self, *g)
                        && self
                            .unit_base(*g)
                            .is_some_and(|u| u.melee_victim == Some(owner))
                })
                .unwrap_or(ObjectGuid::EMPTY);
        }
        if !valid(self, victim) {
            let center = self.world_object_position(guid).unwrap_or_default();
            victim = self
                .units_around(owner, center, max_range, true)
                .into_iter()
                .find(|g| valid(self, *g))
                .unwrap_or(ObjectGuid::EMPTY);
        }
        if let Some(state) = self.totems.get_mut(&guid) {
            state.victim = victim;
        }
        if !victim.is_empty() {
            self.cast_spell(
                guid,
                spell_id,
                CastTarget::Unit(victim),
                CastOptions {
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
    }
}
