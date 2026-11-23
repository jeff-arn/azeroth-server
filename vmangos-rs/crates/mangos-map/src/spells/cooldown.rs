//! Unit/spell helpers, cooldowns, spell mods and bot spell helpers.

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    pub(crate) fn spell_rng(&mut self) -> &mut dyn mangos_spells::SpellRng {
        &mut self.rng
    }

    /// `SpellCaster::GetLevel` dispatch: units read
    /// `UNIT_FIELD_LEVEL`; gameobjects read the fork
    /// `GameObject::GetLevel` (`GameObject.cpp:2408`:
    /// chest/trap info level, else `GAMEOBJECT_LEVEL`,
    /// else `PLAYER_MAX_LEVEL`). Victim-side callers
    /// only ever pass unit guids.
    pub(crate) fn unit_level(&self, guid: ObjectGuid) -> u32 {
        if guid.is_game_object() {
            return self.gameobjects.get(&guid).map_or(0, |go| {
                let info_level = match go.info.go_type {
                    3 => go.info.data_u32(9), // chest.level
                    6 => go.info.data_u32(1), // trap.level
                    _ => 0,
                };
                let level = if info_level != 0 {
                    info_level
                } else {
                    go.world.object.values().get_u32(offsets::GAMEOBJECT_LEVEL)
                };
                if level > 0 { level } else { 60 }
            });
        }
        self.unit_value_u32(guid, offsets::UNIT_FIELD_LEVEL)
    }

    pub(crate) fn unit_value_u32(&self, guid: ObjectGuid, index: u16) -> u32 {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.world.object.values().get_u32(index);
        }
        self.creatures
            .get(&guid)
            .map_or(0, |c| c.unit.world.object.values().get_u32(index))
    }

    fn unit_power_index_of(power_type: u32) -> Option<u16> {
        Some(match power_type {
            power::MANA => offsets::UNIT_FIELD_POWER1,
            power::RAGE => offsets::UNIT_FIELD_POWER2,
            power::FOCUS => offsets::UNIT_FIELD_POWER3,
            power::ENERGY => offsets::UNIT_FIELD_POWER4,
            _ => return None,
        })
    }

    fn unit_max_power_index_of(power_type: u32) -> Option<u16> {
        Some(match power_type {
            power::MANA => offsets::UNIT_FIELD_MAXPOWER1,
            power::RAGE => offsets::UNIT_FIELD_MAXPOWER2,
            power::FOCUS => offsets::UNIT_FIELD_MAXPOWER3,
            power::ENERGY => offsets::UNIT_FIELD_MAXPOWER4,
            _ => return None,
        })
    }

    pub(crate) fn unit_power(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == power::HEALTH {
            return self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH);
        }
        Self::unit_power_index_of(power_type).map_or(0, |i| self.unit_value_u32(guid, i))
    }

    pub(super) fn unit_max_power(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == power::HEALTH {
            return self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
        }
        Self::unit_max_power_index_of(power_type).map_or(0, |i| self.unit_value_u32(guid, i))
    }

    pub(crate) fn unit_set_power(&mut self, guid: ObjectGuid, power_type: u32, value: u32) {
        let index = if power_type == power::HEALTH {
            offsets::UNIT_FIELD_HEALTH
        } else if let Some(i) = Self::unit_power_index_of(power_type) {
            i
        } else {
            return;
        };
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values().set_u32(index, value);
        }
    }

    pub(crate) fn is_hostile_pair(a: ObjectGuid, b: ObjectGuid) -> bool {
        // No faction/reaction system yet: player<->creature
        // is hostile, same-kind is not.
        a.is_player() != b.is_player()
    }

    pub(crate) fn send_to_player(&mut self, guid: ObjectGuid, packet: WorldPacket) {
        if guid.is_player() {
            self.events.push(MapEvent::Packet { to: guid, packet });
        }
    }

    /// `SendCastResult(Player*)` (failures only here).
    pub(crate) fn send_cast_result(&mut self, caster: ObjectGuid, spell_id: u32, result: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(spell_id);
        payload.write_u8(result);
        // No extra-reason reasons are produced by the
        // subset (equipment/reagent/etc.).
        let packet = WorldPacket::with_body(op::SMSG_CAST_RESULT, payload.into_vec());
        self.send_to_player(caster, packet);
    }

    /// `_ItemSpell` row for one spell on an item
    /// prototype (`pickCooldowns` lookup).
    fn item_spell_row(
        &self,
        item_entry: u32,
        spell_id: u32,
    ) -> Option<mangos_world_data::item_prototype::ItemSpell> {
        self.stores
            .item_prototypes
            .get(item_entry)?
            .spells
            .iter()
            .find(|row| row.spell_id == spell_id)
            .copied()
    }

    /// `Player::AddCooldown` duration pick: an item
    /// spell row overrides the category (when
    /// nonzero), the spell cooldown (when >= 0),
    /// and the category cooldown (when >= 0).
    /// Returns (recovery, category,
    /// `category_recovery`, `item_id`).
    fn cooldown_durations(
        &self,
        spell: &SpellEntry,
        item_entry: Option<u32>,
    ) -> (u32, u32, u32, u32) {
        if let Some(entry) = item_entry
            && let Some(row) = self.item_spell_row(entry, spell.id)
        {
            #[allow(clippy::cast_sign_loss)]
            return (
                if row.cooldown >= 0 {
                    row.cooldown as u32
                } else {
                    spell.recovery_time
                },
                if row.category != 0 {
                    row.category
                } else {
                    spell.category
                },
                if row.category_cooldown >= 0 {
                    row.category_cooldown as u32
                } else {
                    spell.category_recovery_time
                },
                entry,
            );
        }
        (
            spell.recovery_time,
            spell.category,
            spell.category_recovery_time,
            0,
        )
    }

    /// `SPELLMOD_COOLDOWN` as a duration.
    fn modded_cooldown(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        raw: u32,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> u32 {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        {
            self.apply_spell_mod(caster, spell, spellmod::COOLDOWN, raw as f32, true, applied)
                .max(0.0) as u32
        }
    }

    pub(crate) fn has_cooldown(
        &self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        cast_item: Option<CastItemCtx>,
    ) -> bool {
        // `IsSpellCDExpired`/`IsCatCDExpired`: a
        // permanent (cooldown-on-event) hold never
        // expires until converted.
        let now = self.now_ms;
        let active = |e: &CooldownEntry| e.permanent || e.expiry_ms > now;
        let spell_cd = self.cooldowns.get(&(caster, spell.id)).is_some_and(active);
        let category = self
            .cooldown_durations(spell, cast_item.map(|item| item.item_entry))
            .1;
        let cat_cd = category != 0
            && self
                .cooldowns
                .get(&(caster, category | 0x8000_0000))
                .is_some_and(active);
        spell_cd || cat_cd
    }

    pub(crate) fn add_cooldowns(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied_mods: &mut Vec<(u32, u8)>,
        forced: Option<u32>,
    ) {
        // `SendSpellCooldown` never records passives
        // (the proc forced add calls `AddCooldown`
        // directly and stays ungated).
        const ATTR_PASSIVE: u32 = 0x40;
        if forced.is_none() && spell.attributes & ATTR_PASSIVE != 0 {
            return;
        }
        // Already-existing guard (`Player::AddCooldown`):
        // an unexpired non-permanent entry (spell or
        // its recorded category) is left untouched —
        // a triggered recast executes but does not
        // refresh its own running cooldown.
        let now = self.now_ms;
        if let Some(entry) = self.cooldowns.get(&(caster, spell.id)).copied()
            && !entry.permanent
        {
            let category_active = entry.category != 0
                && self
                    .cooldowns
                    .get(&(caster, entry.category | 0x8000_0000))
                    .is_some_and(|c| c.permanent || c.expiry_ms > now);
            if entry.expiry_ms > now || category_active {
                return;
            }
        }
        // `SPELL_ATTR_COOLDOWN_ON_EVENT` spells enter
        // a permanent hold at cast (`Spell.cpp:4173`
        // passes `permanent`); the timed cooldown only
        // starts when the aura fades (conversion in
        // `remove_aura_at`). Fork still records the
        // would-be expiry. Forced adds pass
        // `permanent=false` explicitly.
        let permanent = forced.is_none() && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0;
        let base_recovery = forced.unwrap_or(spell.recovery_time);
        if base_recovery > 0 {
            let recovery = self.modded_cooldown(caster, spell, base_recovery, Some(applied_mods));
            if recovery > 0 {
                self.cooldowns.insert(
                    (caster, spell.id),
                    CooldownEntry {
                        expiry_ms: self.now_ms + recovery,
                        item_id: 0,
                        permanent,
                        category: spell.category,
                        spell_id: spell.id,
                    },
                );
            }
        }
        if spell.category != 0 && spell.category_recovery_time > 0 {
            let recovery = self.modded_cooldown(
                caster,
                spell,
                spell.category_recovery_time,
                Some(applied_mods),
            );
            if recovery > 0 {
                self.cooldowns.insert(
                    (caster, spell.category | 0x8000_0000),
                    CooldownEntry {
                        expiry_ms: self.now_ms + recovery,
                        item_id: 0,
                        permanent,
                        category: 0,
                        spell_id: spell.id,
                    },
                );
            }
        }
    }

    /// `Player::AddCooldown` with a cast item
    /// (`Player.cpp:21871`): item duration overrides
    /// from `cooldown_durations`, `SPELLMOD_COOLDOWN`
    /// on the spell recovery when nonzero else the
    /// category recovery, and the spell-key entry
    /// records the item id + effective category.
    pub(crate) fn add_cooldowns_item(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        item: CastItemCtx,
        applied_mods: &mut Vec<(u32, u8)>,
    ) {
        // Same `SendSpellCooldown` passive gate and
        // already-existing guard as the plain path
        // (one fork `Player::AddCooldown`).
        const ATTR_PASSIVE: u32 = 0x40;
        if spell.attributes & ATTR_PASSIVE != 0 {
            return;
        }
        let now = self.now_ms;
        if let Some(entry) = self.cooldowns.get(&(caster, spell.id)).copied()
            && !entry.permanent
        {
            let category_active = entry.category != 0
                && self
                    .cooldowns
                    .get(&(caster, entry.category | 0x8000_0000))
                    .is_some_and(|c| c.permanent || c.expiry_ms > now);
            if entry.expiry_ms > now || category_active {
                return;
            }
        }
        let (rec, category, cat_rec, item_id) =
            self.cooldown_durations(spell, Some(item.item_entry));
        let (rec, cat_rec) = if rec > 0 {
            (
                self.modded_cooldown(caster, spell, rec, Some(applied_mods)),
                cat_rec,
            )
        } else if category != 0 && cat_rec > 0 {
            (
                rec,
                self.modded_cooldown(caster, spell, cat_rec, Some(applied_mods)),
            )
        } else {
            (rec, cat_rec)
        };
        let permanent = spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0;
        if rec > 0 {
            self.cooldowns.insert(
                (caster, spell.id),
                CooldownEntry {
                    expiry_ms: self.now_ms + rec,
                    item_id,
                    permanent,
                    category,
                    spell_id: spell.id,
                },
            );
        }
        if category != 0 && cat_rec > 0 {
            self.cooldowns.insert(
                (caster, category | 0x8000_0000),
                CooldownEntry {
                    expiry_ms: self.now_ms + cat_rec,
                    item_id: 0,
                    permanent,
                    category: 0,
                    spell_id: spell.id,
                },
            );
        }
    }

    /// Convert a permanent cooldown-on-event hold
    /// into the timed cooldown (`AddCooldown` at
    /// aura fade, `SpellAuras.cpp` holder cleanup):
    /// fresh recovery from now, spellmod applied
    /// with the fork null spell pointer (value
    /// applies, no charge drop). No-op without a
    /// permanent spell-key hold.
    pub(super) fn start_event_cooldown(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        let Some(held) = self
            .cooldowns
            .get(&(caster, spell.id))
            .copied()
            .filter(|e| e.permanent)
        else {
            return;
        };
        self.cooldowns.remove(&(caster, spell.id));
        // The fork re-picks item cooldown durations
        // from the held entry's item when the hold
        // came from an item cast (`wasPermanent` arm
        // of `Player::AddCooldown`).
        let (rec, category, cat_rec, item_id) =
            self.cooldown_durations(spell, (held.item_id != 0).then_some(held.item_id));
        let held_category = if held.category != 0 {
            held.category
        } else {
            category
        };
        if held_category != 0 {
            self.cooldowns
                .remove(&(caster, held_category | 0x8000_0000));
        }
        let recovery = self.modded_cooldown(caster, spell, rec, None);
        if recovery > 0 {
            self.cooldowns.insert(
                (caster, spell.id),
                CooldownEntry {
                    expiry_ms: self.now_ms + recovery,
                    item_id,
                    permanent: false,
                    category,
                    spell_id: spell.id,
                },
            );
        }
        if category != 0 && cat_rec > 0 {
            let cat_recovery = self.modded_cooldown(caster, spell, cat_rec, None);
            if cat_recovery > 0 {
                self.cooldowns.insert(
                    (caster, category | 0x8000_0000),
                    CooldownEntry {
                        spell_id: spell.id,
                        ..CooldownEntry::timed(self.now_ms + cat_recovery)
                    },
                );
            }
        }
    }

    /// Seed loaded cooldown rows at login
    /// (`Player::_LoadSpellCooldowns` enforcement
    /// half: the client packet is built session-side;
    /// without this the server would not block).
    pub fn seed_cooldowns(&mut self, caster: ObjectGuid, rows: &[LoadedCooldown]) {
        // Login reload is authoritative: the fork's
        // cooldown map lives on the Player and dies
        // at logout, so any entries a previous
        // session left in this map are dropped
        // before seeding from the DB rows.
        self.cooldowns.retain(|(guid, _), _| *guid != caster);
        for row in rows {
            if row.spell_remaining_ms > 0 {
                self.cooldowns.insert(
                    (caster, row.spell),
                    CooldownEntry {
                        expiry_ms: self.now_ms + row.spell_remaining_ms,
                        item_id: row.item_id,
                        permanent: false,
                        category: row.category,
                        spell_id: row.spell,
                    },
                );
            }
            if row.category != 0 && row.category_remaining_ms > 0 {
                self.cooldowns.insert(
                    (caster, row.category | 0x8000_0000),
                    CooldownEntry {
                        spell_id: row.spell,
                        ..CooldownEntry::timed(self.now_ms + row.category_remaining_ms)
                    },
                );
            }
        }
    }

    /// `Player::ApplySpellMod` (`Player.cpp:22149`,
    /// players only as `HandleAddModifier` gates):
    /// collect the caster's
    /// `ADD_FLAT_MODIFIER`/`ADD_PCT_MODIFIER`
    /// effects for `op`, filter by family name +
    /// affect mask (`IsAffectedOnSpell`), compose in
    /// fork order, and drop charges once per cast
    /// (`applied` dedupe = fork `m_appliedMods`;
    /// `None` mirrors a null spell pointer: applies
    /// but never drops and ignores the exhausted
    /// gate). Modifier state lives on the holder
    /// effects, so removal is inherently reversible.
    #[allow(
        clippy::float_cmp, // fork compares basevalue to T(0) exactly
        clippy::cast_precision_loss
    )]
    /// `CalculateSpellEffectValue` + creature-level
    /// scaling tail (class-1 CLS melee-damage ratio).
    pub(crate) fn effect_value(
        &mut self,
        spell: &SpellEntry,
        eff_index: usize,
        caster_level: u32,
    ) -> f32 {
        self.effect_value_bp(spell, eff_index, caster_level, None)
    }

    /// [`Self::effect_value`] with a cast-custom
    /// base-points override (`m_currentBasePoints`).
    pub(crate) fn effect_value_bp(
        &mut self,
        spell: &SpellEntry,
        eff_index: usize,
        caster_level: u32,
        bp: Option<i32>,
    ) -> f32 {
        let value =
            spell.calculate_effect_value_bp(eff_index, caster_level, 0, self.spell_rng(), bp);
        let cls = &self.stores.cls;
        spell.creature_level_scaled(eff_index, value, caster_level, &|level| {
            cls.get(1, level).melee_damage
        })
    }

    pub(crate) fn apply_spell_mod(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        op: u32,
        base: f32,
        drop_charge: bool,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> f32 {
        // SPELL_ATTR_EX3_IGNORE_CASTER_MODIFIERS.
        if !self.players.contains_key(&caster) || spell.attributes_ex3 & 0x2000_0000 != 0 {
            return base;
        }
        // (holder index, source spell, effect index,
        // is_pct, value, charges)
        let mut hits: Vec<(usize, u32, u8, bool, i32, i32)> = Vec::new();
        for (holder_idx, holder) in self.aura_holders(caster).iter().enumerate() {
            let Some(source) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if source.spell_family_name != spell.spell_family_name {
                continue;
            }
            for (eff_idx, eff) in holder.effects.iter().enumerate() {
                let Some(state) = eff else { continue };
                let is_pct = match state.aura_type {
                    aura_type::ADD_FLAT_MODIFIER => false,
                    aura_type::ADD_PCT_MODIFIER => true,
                    _ => continue,
                };
                if u32::try_from(state.misc_value) != Ok(op) {
                    continue;
                }
                if spell.spell_family_flags & source.effects[eff_idx].item_type == 0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                hits.push((
                    holder_idx,
                    holder.spell_id,
                    eff_idx as u8,
                    is_pct,
                    state.amount,
                    state.mod_charges,
                ));
            }
        }
        if hits.is_empty() {
            return base;
        }
        let mut applied = applied;
        let mut total_flat = 0.0f32;
        let mut total_pct = 0.0f32;
        let mut drops: Vec<(usize, u8)> = Vec::new();
        for (holder_idx, source_id, eff_idx, is_pct, value, charges) in hits {
            let key = (source_id, eff_idx);
            // Exhausted mods only still affect the cast
            // that consumed them.
            if charges == -1
                && let Some(list) = &applied
                && !list.contains(&key)
            {
                continue;
            }
            if is_pct {
                if base == 0.0 {
                    continue;
                }
                // Instant-cast mods never apply to
                // 10 s+ casts.
                if op == spellmod::CASTING_TIME && base >= 10_000.0 && value <= -100 {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                {
                    total_pct += value as f32;
                }
            } else {
                #[allow(clippy::cast_precision_loss)]
                {
                    total_flat += value as f32;
                }
            }
            if drop_charge
                && charges > 0
                && let Some(list) = &mut applied
                && !list.contains(&key)
            {
                list.push(key);
                drops.push((holder_idx, eff_idx));
            }
            // A -100% casting-time mod zeroes the cast
            // outright (fork Barkskin/Swiftness fix).
            if op == spellmod::CASTING_TIME && is_pct && value == -100 {
                total_pct = -100.0;
                total_flat = 0.0;
                break;
            }
        }
        if !drops.is_empty()
            && let Some(unit) = self.unit_base_mut(caster)
        {
            for (holder_idx, eff_idx) in drops {
                if let Some(state) = unit
                    .auras
                    .get_mut(holder_idx)
                    .and_then(|h| h.effects[usize::from(eff_idx)].as_mut())
                    && state.mod_charges > 0
                {
                    state.mod_charges -= 1;
                    if state.mod_charges == 0 {
                        state.mod_charges = -1;
                    }
                }
            }
        }
        spell_mod_apply(base, total_flat, total_pct)
    }

    /// `Player::RemoveSpellMods` subset: holders whose
    /// modifier this cast exhausted are removed.
    pub(crate) fn remove_exhausted_spell_mods(
        &mut self,
        caster: ObjectGuid,
        applied: &[(u32, u8)],
    ) {
        let mut sources: Vec<u32> = Vec::new();
        for (source, _) in applied {
            if !sources.contains(source) {
                sources.push(*source);
            }
        }
        for source in sources {
            let exhausted = self
                .aura_holders(caster)
                .iter()
                .filter(|h| h.spell_id == source)
                .any(|h| h.effects.iter().flatten().any(|e| e.mod_charges == -1));
            if exhausted {
                self.cancel_aura(caster, source);
            }
        }
    }

    /// Spell duration with `SPELLMOD_DURATION`
    /// applied (positive durations only; a modded
    /// result <= 0 suppresses the aura where the fork
    /// would expire it on the first update).
    pub(super) fn modded_duration_ms(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) -> i32 {
        let raw = spell.calculate_duration_ms(&self.stores.spells.tables);
        if raw <= 0 {
            return raw;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        {
            self.apply_spell_mod(caster, spell, spellmod::DURATION, raw as f32, true, applied)
                as i32
        }
    }

    /// Known-spell check for players
    /// (`Player::HasActiveSpell` subset over the
    /// deferred `character_spell` rows; creatures always
    /// pass).
    /// Bot query: `HasAura`.
    #[must_use]
    pub fn bot_has_aura(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.aura_holders(guid)
            .iter()
            .any(|h| h.spell_id == spell_id)
    }

    /// Bot query: `HasAuraType(PERIODIC_HEAL)`.
    #[must_use]
    pub fn bot_has_periodic_heal(&self, guid: ObjectGuid) -> bool {
        self.aura_holders(guid).iter().any(|h| {
            h.effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_HEAL)
        })
    }

    /// Bot query: loss-of-control state
    /// (`UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL`
    /// approximated by holder aura types:
    /// `MOD_CONFUSE` 5, `MOD_FEAR` 7, `MOD_STUN`
    /// 12, `MOD_ROOT` 26).
    #[must_use]
    pub fn bot_is_ccd(&self, guid: ObjectGuid) -> bool {
        self.aura_holders(guid).iter().any(|h| {
            h.effects
                .iter()
                .flatten()
                .any(|e| matches!(e.aura_type, 5 | 7 | 12 | 26))
        })
    }

    /// Bot query: `HasAuraType`.
    #[must_use]
    pub fn bot_has_aura_type(&self, guid: ObjectGuid, aura_type: u32) -> bool {
        self.aura_holders(guid)
            .iter()
            .any(|h| h.effects.iter().flatten().any(|e| e.aura_type == aura_type))
    }

    /// Bot verb: `RemoveSpellsCausingAura(MOD_SHAPESHIFT)`.
    pub fn bot_remove_shapeshift(&mut self, guid: ObjectGuid) {
        let spells: Vec<u32> = self
            .aura_holders(guid)
            .iter()
            .filter(|h| h.effects.iter().flatten().any(|e| e.aura_type == 36))
            .map(|h| h.spell_id)
            .collect();
        for spell_id in spells {
            self.cancel_aura(guid, spell_id);
        }
    }

    /// Bot verb: `SetFacingToObject`.
    pub fn bot_face(&mut self, guid: ObjectGuid, target: ObjectGuid) {
        let (Some(from), Some(to)) = (self.unit_position(guid), self.unit_position(target)) else {
            return;
        };
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.world.position.o = (to.y - from.y).atan2(to.x - from.x);
        }
    }

    /// Bot verb: `AttackStop` (clears melee victim).
    pub fn bot_attack_stop(&mut self, guid: ObjectGuid) {
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.melee_victim = None;
        }
    }

    /// Bot query: melee reach (plain 5y gate; the
    /// fork subtracts combat reaches via
    /// `GetCombatDistance`).
    #[must_use]
    pub fn bot_can_reach_melee(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool {
        let (Some(a), Some(b)) = (self.unit_position(attacker), self.unit_position(target)) else {
            return false;
        };
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt() <= 5.0
    }

    /// In-progress cast spell id for a caster
    /// (`GetCurrentSpell` generic+channeled reduced
    /// to the single active-cast store), for the
    /// bot situation snapshot.
    #[must_use]
    pub fn bot_casting_spell(&self, caster: ObjectGuid) -> Option<u32> {
        self.casts.get(&caster).map(|cast| cast.spell_id)
    }

    /// Active spellbook ids for a player (bot
    /// spell classification input).
    #[must_use]
    pub fn bot_spell_ids(&self, guid: ObjectGuid) -> Vec<u32> {
        self.players.get(&guid).map_or_else(Vec::new, |entry| {
            entry
                .player
                .deferred
                .spells
                .iter()
                .filter(|s| s.active != 0)
                .map(|s| s.spell)
                .collect()
        })
    }

    /// Known-spell check for Host API consumers
    /// (`Player::HasSpell` parity helper).
    #[must_use]
    pub fn bot_knows_spell(&self, caster: ObjectGuid, spell_id: u32) -> bool {
        self.player_knows_spell(caster, spell_id)
    }

    pub(crate) fn player_knows_spell(&self, caster: ObjectGuid, spell_id: u32) -> bool {
        if !caster.is_player() {
            return true;
        }
        self.players.get(&caster).is_some_and(|entry| {
            // `HasSpell`: persisted rows plus the
            // race/class defaults `LearnDefaultSpells`
            // adds as dependent spells at create and at
            // every load (`Player.cpp:521`, `:14872`).
            // Dependent spells are never written to
            // `character_spell` (fork `_SaveSpells`
            // skips them), so they cannot live in
            // `deferred.spells`; a character created
            // through the client knows only defaults
            // until its first trained spell.
            entry
                .player
                .deferred
                .spells
                .iter()
                .any(|s| s.spell == spell_id && s.active != 0)
                || self
                    .stores
                    .player_infos
                    .default_spells(
                        u32::from(entry.player.unit.race()),
                        u32::from(entry.player.unit.class()),
                    )
                    .contains(&spell_id)
        })
    }

    /// `Unit::CastSpell` + `Spell::prepare` subset.
    /// Starts the cast (instant spells complete inside
    /// this call, like C++ update(0) chaining).
    /// `SpellEntry::GetErrorAtShapeshiftedCast`
    /// (`SpellEntry.cpp:1012-1054`) for the caster's current
    /// `UNIT_FIELD_BYTES_1` byte 2 form. `None` is
    /// `SPELL_CAST_OK`.
    pub(super) fn shapeshift_cast_error(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
    ) -> Option<u8> {
        // Talent learn spells ignore stance requirements
        // (client-side stance display only).
        if spell
            .effects
            .iter()
            .any(|effect| effect.effect == effect::LEARN_SPELL)
            && self.stores.spells.talent_spell_cost(spell.id) > 0
        {
            return None;
        }
        let form = self
            .unit_base_mut(caster)
            .map_or(0, |u| u.values().get_byte(offsets::UNIT_FIELD_BYTES_1, 2));
        let stance_mask = if form == 0 {
            0
        } else {
            1u32 << (u32::from(form) - 1)
        };
        if stance_mask & spell.stances_not != 0 {
            return Some(cast_result::FAILED_NOT_SHAPESHIFT);
        }
        if stance_mask & spell.stances != 0 {
            return None;
        }
        let act_as_shifted = if form > 0 {
            // Fork: an unknown form logs and returns OK. This
            // also leaves test worlds without the DBC table on
            // their previous behavior for non-explicit forms;
            // production loads every form row.
            let flags1 = self
                .stores
                .shapeshift_forms
                .get(&u32::from(form))
                .copied()?;
            flags1 & SHAPESHIFT_FLAG_STANCE == 0
        } else {
            false
        };
        if act_as_shifted {
            if spell.attributes & ATTR_NOT_SHAPESHIFT != 0 {
                return Some(cast_result::FAILED_NOT_SHAPESHIFT);
            }
            if spell.stances != 0 {
                return Some(cast_result::FAILED_ONLY_SHAPESHIFT);
            }
        } else if spell.attributes_ex2 & ATTR_EX2_ALLOW_WHILE_NOT_SHAPESHIFTED == 0
            && spell.stances != 0
        {
            return Some(cast_result::FAILED_ONLY_SHAPESHIFT);
        }
        None
    }
}
