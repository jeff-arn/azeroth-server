//! Aura holders, modifiers, apply/remove and periodic ticks.

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    // ------------------------------------------------ auras

    /// Read-only holder snapshot (tests/inspection).
    #[must_use]
    pub fn aura_holders(&self, guid: ObjectGuid) -> Vec<AuraHolder> {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.auras.clone();
        }
        self.creatures
            .get(&guid)
            .map_or_else(Vec::new, |c| c.unit.auras.clone())
    }

    /// Savable aura rows for one player (`SaveAura`
    /// filter, `Player.cpp:16503`): passive and
    /// channeled holders and not-own single-target
    /// holders are skipped, as are the sight/charm
    /// aura types. Deviations: the
    /// leave/enter-world interrupt-flag gate needs
    /// `AuraInterruptFlags`, which the spell stores do
    /// not parse yet; cast-item guid and proc charges
    /// are unmodeled and written 0; area-aura holders
    /// are unmodeled (no `IsAreaAura`).
    #[must_use]
    pub fn player_savable_auras(&self, guid: ObjectGuid) -> Vec<SavableAura> {
        self.savable_auras_from(guid, self.aura_holders(guid))
    }

    /// Savable auras for a player currently outside a
    /// map (forced logout save during far teleport).
    #[must_use]
    pub fn savable_auras_for_player(
        &self,
        player: &mangos_entities::player::Player,
    ) -> Vec<SavableAura> {
        self.savable_auras_from(player.unit.guid(), player.unit.auras.clone())
    }

    fn savable_auras_from(&self, guid: ObjectGuid, holders: Vec<AuraHolder>) -> Vec<SavableAura> {
        // `SPELL_ATTR_PASSIVE`, channeled spells.
        const ATTR_PASSIVE: u32 = 0x0000_0040;
        let mut rows = Vec::new();
        for holder in holders {
            let Some(spell) = self.stores.spells.get(holder.spell_id) else {
                continue;
            };
            if spell.attributes & ATTR_PASSIVE != 0 || spell.is_channeled() {
                continue;
            }
            let single_target = spell
                .effects
                .iter()
                .any(|e| e.effect != 0 && e.implicit_target_a == 1);
            if single_target && holder.caster != guid {
                continue;
            }
            let mut base_points = [0.0f32; 3];
            let mut periodic_time = [0i32; 3];
            let mut mask = 0u8;
            let mut skipped = false;
            for (i, eff) in spell.effects.iter().enumerate() {
                if matches!(eff.apply_aura_name, 1 | 2 | 6 | 76 | 177) {
                    skipped = true;
                    break;
                }
                if let Some(state) = &holder.effects[i] {
                    #[allow(clippy::cast_precision_loss)]
                    {
                        base_points[i] = state.amount as f32;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    {
                        periodic_time[i] = state.period_ms as i32;
                    }
                    mask |= 1u8 << i;
                }
            }
            if skipped || mask == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_wrap)]
            rows.push(SavableAura {
                caster_guid: holder.caster.raw(),
                item_guid: 0,
                spell: holder.spell_id,
                stacks: holder.stack as i32,
                charges: 0,
                base_points,
                periodic_time,
                max_duration: holder.max_duration_ms,
                duration: holder.duration_ms,
                effect_index_mask: mask,
            });
        }
        rows
    }

    /// Live cooldown rows for one player
    /// (`_SaveSpellCooldowns`): split spell/category
    /// keys merge into the fork's per-spell
    /// `CooldownData` shape; expiries convert from map
    /// time to epoch seconds. Item-created rows keep
    /// their `item_id` and effective category.
    #[must_use]
    pub fn player_cooldown_rows(&self, guid: ObjectGuid, now_unix: i64) -> Vec<CooldownSnapshot> {
        let to_epoch = |expiry_ms: u32| -> i64 {
            now_unix + i64::from(expiry_ms.saturating_sub(self.now_ms)) / 1000
        };
        let mut rows = Vec::new();
        for ((caster, key), entry) in &self.cooldowns {
            // Permanent holds are never saved
            // (`_SaveSpellCooldowns` skips
            // `IsPermanent()`).
            if *caster != guid
                || entry.permanent
                || entry.expiry_ms <= self.now_ms
                || *key & 0x8000_0000 != 0
            {
                continue;
            }
            let category = if entry.category != 0 {
                entry.category
            } else {
                self.stores.spells.get(*key).map_or(0, |s| s.category)
            };
            let category_expire_time = if category != 0 {
                self.cooldowns
                    .get(&(guid, category | 0x8000_0000))
                    .filter(|e| !e.permanent && e.expiry_ms > self.now_ms)
                    .map_or(0, |e| to_epoch(e.expiry_ms))
            } else {
                0
            };
            rows.push(CooldownSnapshot {
                spell: *key,
                spell_expire_time: to_epoch(entry.expiry_ms),
                category,
                category_expire_time,
                item_id: entry.item_id,
            });
        }
        rows.sort_by_key(|r| r.spell);
        rows
    }

    pub(crate) fn send_aura_duration(&mut self, target: ObjectGuid, slot: u8, duration_ms: i32) {
        if slot >= MAX_AURAS || duration_ms < 0 {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u8(slot);
        #[allow(clippy::cast_possible_truncation)]
        payload.write_u32_le(duration_ms as u32);
        let packet = WorldPacket::with_body(op::SMSG_UPDATE_AURA_DURATION, payload.into_vec());
        self.send_to_player(target, packet);
    }

    /// Slot update-field writes (`SetAura`/`SetAuraFlag`/
    /// `SetAuraLevel`/`UpdateAuraApplication`).
    fn write_aura_slot(unit: &mut UnitBase, holder: &AuraHolder, remove: bool) {
        let slot = holder.slot;
        if slot == NULL_AURA_SLOT || slot >= MAX_AURAS {
            return;
        }
        let values = unit.values();
        values.set_u32(
            offsets::UNIT_FIELD_AURA + u16::from(slot),
            if remove { 0 } else { holder.spell_id },
        );
        let flags_index = offsets::UNIT_FIELD_AURAFLAGS + u16::from(slot >> 3);
        let shift = u32::from((slot & 7) << 2);
        let current = values.get_u32(flags_index);
        let cleared = current & !(0xFu32 << shift);
        let next = if remove {
            cleared
        } else {
            cleared | (holder.effect_flags() << shift)
        };
        values.set_u32(flags_index, next);
        #[allow(clippy::cast_possible_truncation)]
        let level_byte = holder.caster_level as u8;
        values.set_byte(
            offsets::UNIT_FIELD_AURALEVELS + u16::from(slot / 4),
            slot % 4,
            if remove { 0 } else { level_byte },
        );
        #[allow(clippy::cast_possible_truncation)]
        let stack_byte = (holder.stack.saturating_sub(1)).min(254) as u8;
        values.set_byte(
            offsets::UNIT_FIELD_AURAAPPLICATIONS + u16::from(slot / 4),
            slot % 4,
            if remove { 0 } else { stack_byte },
        );
    }

    /// `HandleAttackPowerModifier(MELEE, flat)` field
    /// write subset (no pct mods in the slice).
    #[allow(clippy::cast_precision_loss)]
    fn apply_attack_power_mod(unit: &mut UnitBase, positive: bool, amount: i32, apply: bool) {
        let bucket = usize::from(!positive);
        if apply {
            unit.attack_power_mods[bucket] += amount as f32;
        } else {
            unit.attack_power_mods[bucket] -= amount as f32;
        }
        for i in 0..2 {
            #[allow(clippy::cast_possible_truncation)]
            let value = unit.attack_power_mods[i] as i16;
            #[allow(clippy::cast_possible_truncation)]
            unit.values()
                .set_i16(offsets::UNIT_FIELD_ATTACK_POWER_MODS, i as u8, value);
        }
    }

    fn aura_effect_apply(unit: &mut UnitBase, holder: &AuraHolder, apply: bool) {
        for eff in holder.effects.iter().flatten() {
            match eff.aura_type {
                aura_type::MOD_ATTACK_POWER => {
                    Self::apply_attack_power_mod(unit, holder.positive, eff.amount, apply);
                }
                aura_type::MOD_STAT
                | aura_type::MOD_PERCENT_STAT
                | aura_type::MOD_TOTAL_STAT_PERCENTAGE => {
                    Self::apply_stat_aura_mod(unit, eff, apply);
                }
                // `HandleModScale`: multiplicative
                // object-scale change, inverted on
                // removal (fork apply/unapply pair).
                aura_type::MOD_SCALE => {
                    #[allow(clippy::cast_precision_loss)]
                    let factor = (100.0 + eff.amount as f32) / 100.0;
                    if factor > 0.0 {
                        let cur = unit.world.object.object_scale();
                        let next = if apply { cur * factor } else { cur / factor };
                        unit.world.object.set_object_scale(next);
                    }
                }
                // Other aura types are state-only in the
                // slice (periodic handled by the tick loop;
                // speed/root/stun recorded as pending).
                _ => {}
            }
        }
    }

    /// `ApplyModSignedFloatValue` on one pos/neg display
    /// field.
    fn apply_stat_buff_mod(unit: &mut UnitBase, stat: usize, amount: f32, apply: bool) {
        #[allow(clippy::cast_possible_truncation)]
        let idx = stat as u16;
        let field = if amount > 0.0 {
            offsets::PLAYER_FIELD_POSSTAT0 + idx
        } else {
            offsets::PLAYER_FIELD_NEGSTAT0 + idx
        };
        let values = unit.values();
        let cur = values.get_f32(field);
        values.set_f32(field, if apply { cur + amount } else { cur - amount });
    }

    /// `ApplyStatPercentBuffMod`: percent-scale both
    /// display halves.
    fn apply_stat_percent_buff_mod(unit: &mut UnitBase, stat: usize, amount: f32, apply: bool) {
        #[allow(clippy::cast_possible_truncation)]
        let idx = stat as u16;
        let values = unit.values();
        for field in [
            offsets::PLAYER_FIELD_POSSTAT0 + idx,
            offsets::PLAYER_FIELD_NEGSTAT0 + idx,
        ] {
            let mut cur = values.get_f32(field);
            apply_percent_mod_float_var(&mut cur, amount, apply);
            values.set_f32(field, cur);
        }
    }

    /// Fork miscvalue stat selection (negative = all).
    fn stat_matches(misc_value: i32, stat: usize) -> bool {
        #[allow(clippy::cast_possible_wrap)]
        {
            misc_value < 0 || misc_value == stat as i32
        }
    }

    /// `HandleAuraModStat` / `HandleModPercentStat` /
    /// `HandleModTotalPercentStat` register writes (the
    /// `Update*` dispatch is `refresh_derived_stats` in
    /// the add/remove wrappers). The Scorpid Sting
    /// override-class-script branch of `HandleAuraModStat`
    /// is deferred with spell mods.
    fn apply_stat_aura_mod(
        unit: &mut UnitBase,
        eff: &mangos_entities::aura::AuraEffectState,
        apply: bool,
    ) {
        let is_player = unit.guid().is_player();
        #[allow(clippy::cast_precision_loss)]
        let amount = eff.amount as f32;
        match eff.aura_type {
            aura_type::MOD_STAT => {
                if !(-2..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::TotalValue, amount, apply);
                        if is_player {
                            Self::apply_stat_buff_mod(unit, stat, amount, apply);
                        }
                    }
                }
            }
            aura_type::MOD_PERCENT_STAT => {
                // Only players have base stats.
                if !is_player || !(-1..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::BasePct, amount, apply);
                    }
                }
            }
            aura_type::MOD_TOTAL_STAT_PERCENTAGE => {
                if !(-1..=4).contains(&eff.misc_value) {
                    return;
                }
                for stat in 0..MAX_STATS {
                    if Self::stat_matches(eff.misc_value, stat) {
                        unit.handle_stat_modifier(stat, StatModKind::TotalPct, amount, apply);
                        if is_player {
                            Self::apply_stat_percent_buff_mod(unit, stat, amount, apply);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn holder_has_stat_mod(holder: &AuraHolder) -> bool {
        holder.effects.iter().flatten().any(|e| {
            matches!(
                e.aura_type,
                aura_type::MOD_STAT
                    | aura_type::MOD_PERCENT_STAT
                    | aura_type::MOD_TOTAL_STAT_PERCENTAGE
            )
        })
    }

    /// `UNIT_MOD_HEALTH` / `UNIT_MOD_POWER_*`
    /// TOTAL-group presence (`MOD_INCREASE_HEALTH`/
    /// `..._PERCENT`, `MOD_INCREASE_ENERGY`/
    /// `..._PERCENT`): drives the derived-stat
    /// refresh and the fork handlers' current-value
    /// adjustments.
    fn holder_has_health_energy_mod(holder: &AuraHolder) -> bool {
        holder.effects.iter().flatten().any(|e| {
            matches!(
                e.aura_type,
                aura_type::MOD_INCREASE_HEALTH
                    | aura_type::MOD_INCREASE_HEALTH_PERCENT
                    | aura_type::MOD_INCREASE_ENERGY
                    | aura_type::MOD_INCREASE_ENERGY_PERCENT
            )
        })
    }

    /// Post-refresh current health/power adjustments
    /// of `HandleAuraModIncreaseHealth[Percent]` and
    /// `HandleAuraModIncreaseEnergy[Percent]` for a
    /// just-applied holder (`SpellAuras.cpp:4723+`).
    /// `pre` is (health, max health, powers, max
    /// powers) captured before the holder landed.
    fn apply_health_energy_adjustments(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        pre: Option<(u32, u32, [u32; 5], [u32; 5])>,
    ) {
        let Some((old_hp, old_max, old_powers, old_max_powers)) = pre else {
            return;
        };
        let flat_health: i32 = holder
            .effects
            .iter()
            .flatten()
            .filter(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH)
            .map(|e| e.amount)
            .sum();
        let has_health_pct = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH_PERCENT);
        if let Some(unit) = self.unit_base_mut(victim).filter(|u| u.is_alive()) {
            if holder.spell_id == 12976 {
                // Last Stand: current health follows
                // the flat amount.
                let cur = unit.health();
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                let next = (i64::from(cur) + i64::from(flat_health)).clamp(0, i64::from(u32::MAX));
                unit.set_health(next as u32);
            } else if matches!(holder.spell_id, 1178 | 9635) {
                // Bear form: ceil percent preservation.
                if old_max > 0 {
                    #[allow(
                        clippy::cast_precision_loss,
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss
                    )]
                    let next =
                        (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)).ceil() as u32;
                    unit.set_health(next);
                }
            } else if has_health_pct && old_max > 0 {
                // SetHealthPercent (truncating).
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)) as u32;
                unit.set_health(next);
            }
            for e in holder.effects.iter().flatten() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let power = e.misc_value as usize;
                if power >= 5 {
                    continue;
                }
                match e.aura_type {
                    aura_type::MOD_INCREASE_ENERGY => {
                        let cur = i64::from(unit.power(power)) + i64::from(e.amount);
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let next = cur.clamp(0, i64::from(unit.max_power(power))) as u32;
                        unit.set_power(power, next);
                    }
                    aura_type::MOD_INCREASE_ENERGY_PERCENT => {
                        // SetPower by preserved percent.
                        let old_max = old_max_powers[power];
                        if old_max > 0 {
                            #[allow(
                                clippy::cast_precision_loss,
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss
                            )]
                            let next = (unit.max_power(power) as f32
                                * (old_powers[power] as f32 / old_max as f32))
                                as u32;
                            unit.set_power(power, next);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Post-refresh current health/power adjustments
    /// for a just-removed health/energy holder (the
    /// remove arms of the same fork handlers; the
    /// Last Stand pre-adjustment already ran in
    /// `remove_aura_at`).
    fn remove_health_energy_adjustments(
        &mut self,
        victim: ObjectGuid,
        holder: &AuraHolder,
        pre: (u32, u32, [u32; 5], [u32; 5]),
    ) {
        let (old_hp, old_max, old_powers, old_max_powers) = pre;
        let Some(unit) = self.unit_base_mut(victim).filter(|u| u.is_alive()) else {
            return;
        };
        let has_health_pct = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH_PERCENT);
        if old_max > 0 && matches!(holder.spell_id, 1178 | 9635) {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)).ceil() as u32;
            unit.set_health(next);
        } else if old_max > 0 && has_health_pct {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let next = (unit.max_health() as f32 * (old_hp as f32 / old_max as f32)) as u32;
            unit.set_health(next);
        }
        for e in holder.effects.iter().flatten() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let power = e.misc_value as usize;
            if power >= 5 {
                continue;
            }
            match e.aura_type {
                aura_type::MOD_INCREASE_ENERGY => {
                    let cur = i64::from(unit.power(power)) - i64::from(e.amount);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let next = cur.clamp(0, i64::from(unit.max_power(power))) as u32;
                    unit.set_power(power, next);
                }
                aura_type::MOD_INCREASE_ENERGY_PERCENT => {
                    let old_max_power = old_max_powers[power];
                    if old_max_power > 0 {
                        #[allow(
                            clippy::cast_precision_loss,
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss
                        )]
                        let next = (unit.max_power(power) as f32
                            * (old_powers[power] as f32 / old_max_power as f32))
                            as u32;
                        unit.set_power(power, next);
                    }
                }
                _ => {}
            }
        }
    }

    /// The HP-ratio preservation of
    /// `HandleModTotalPercentStat`: stamina-misc percent
    /// mods on `Attributes & 0x10` spells.
    fn hp_ratio_preserve(&self, holder: &AuraHolder) -> bool {
        let has = holder.effects.iter().flatten().any(|e| {
            e.aura_type == aura_type::MOD_TOTAL_STAT_PERCENTAGE && e.misc_value == STAT_STAMINA_I32
        });
        has && self
            .stores
            .spells
            .get(holder.spell_id)
            .is_some_and(|s| s.attributes & 0x10 != 0)
    }

    /// `HandleStatModifier`'s derived-stat dispatch after
    /// a stat-aura register change.
    fn refresh_derived_stats(&mut self, guid: ObjectGuid) {
        if let Some(entry) = self.players.get_mut(&guid) {
            entry.player.refresh_derived_stats();
            return;
        }
        if let Some(creature) = self.creatures.get_mut(&guid) {
            creature.refresh_derived_stats();
        }
        self.refresh_weapon_damage_fields(guid);
    }

    /// Add a holder: slot allocation (positive slots
    /// 0..32, negative 32..48 — `GetAuraSlot` ranges),
    /// field writes, modifier application.
    fn add_aura_holder(&mut self, victim: ObjectGuid, mut holder: AuraHolder) {
        holder.apply_time_ms = self.now_ms;
        let slot = {
            let range = if holder.positive {
                0..MAX_POSITIVE_AURAS
            } else {
                MAX_POSITIVE_AURAS..MAX_AURAS
            };
            let mut found = NULL_AURA_SLOT;
            if let Some(unit) = self.unit_base_mut(victim) {
                let values = unit.values();
                for s in range {
                    if values.get_u32(offsets::UNIT_FIELD_AURA + u16::from(s)) == 0 {
                        found = s;
                        break;
                    }
                }
            }
            found
        };
        holder.slot = slot;
        let holder_spell_id = holder.spell_id;
        let holder_caster_guid = holder.caster;
        let duration_packet = (slot, holder.duration_ms);
        let has_ap = holder
            .effects
            .iter()
            .flatten()
            .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
        let has_stat = Self::holder_has_stat_mod(&holder);
        let has_health_energy = Self::holder_has_health_energy_mod(&holder);
        let hp_before = if has_stat && self.hp_ratio_preserve(&holder) {
            self.unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| (u.health(), u.max_health()))
        } else {
            None
        };
        let vitals_before = if has_health_energy {
            self.unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| {
                    (
                        u.health(),
                        u.max_health(),
                        core::array::from_fn(|p| u.power(p)),
                        core::array::from_fn(|p| u.max_power(p)),
                    )
                })
        } else {
            None
        };
        // AuraScript::OnBeforeApply (holder level,
        // per present effect) runs before the apply
        // handlers, fork ApplyModifier order.
        self.aura_script_on_before_apply(victim, &mut holder, true, AuraRemoveMode::Default);
        let landed = if let Some(unit) = self.unit_base_mut(victim) {
            Self::aura_effect_apply(unit, &holder, true);
            Self::write_aura_slot(unit, &holder, false);
            unit.auras.push(holder);
            true
        } else {
            false
        };
        if landed {
            self.aura_script_on_after_apply(victim, holder_spell_id, holder_caster_guid, true);
            self.aura_dummy_for_spell(victim, holder_spell_id, true);
        }
        if has_stat || has_health_energy {
            self.refresh_derived_stats(victim);
            if let Some((cur, max)) = hp_before
                && max > 0
                && let Some(unit) = self.unit_base_mut(victim)
            {
                #[allow(clippy::cast_possible_truncation)]
                let scaled =
                    (u64::from(unit.max_health()) * u64::from(cur) / u64::from(max)) as u32;
                unit.set_health(scaled);
            }
            if landed && has_health_energy {
                // Re-borrow the landed holder (last
                // pushed) for its effect amounts.
                let landed_holder = self.unit_base(victim).and_then(|u| u.auras.last().cloned());
                if let Some(h) = landed_holder {
                    self.apply_health_energy_adjustments(victim, &h, vitals_before);
                }
            }
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
        self.send_aura_duration(victim, duration_packet.0, duration_packet.1);
        self.refresh_display(victim);
        self.refresh_immune_state(victim);
        self.refresh_attack_speed(victim);
        self.refresh_skill_bonuses(victim);
        self.refresh_speed(victim);
        self.refresh_cc_flags(victim);
        self.refresh_ghost_flag(victim);
    }

    /// Holder-derived ghost bits
    /// (`Aura::HandleAuraGhost`): the
    /// `UNIT_VIS_FLAGS_GHOST` vis byte on any unit,
    /// plus `PLAYER_FLAGS_GHOST` on players, while
    /// any `SPELL_AURA_GHOST` effect is present.
    /// Recompute-style like `refresh_cc_flags`.
    pub(crate) fn refresh_ghost_flag(&mut self, guid: ObjectGuid) {
        // `UNIT_BYTES_1_OFFSET_VIS_FLAG` /
        // `UNIT_VIS_FLAGS_GHOST` (`UnitDefines.h:88,147`).
        const VIS_FLAG_OFFSET: u8 = 3;
        const VIS_FLAG_GHOST: u8 = 0x01;
        let ghost = self
            .aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .any(|e| e.aura_type == aura_type::GHOST);
        if let Some(unit) = self.unit_base_mut(guid) {
            let values = unit.world.object.values_mut();
            if ghost {
                values.set_byte_flag(offsets::UNIT_FIELD_BYTES_1, VIS_FLAG_OFFSET, VIS_FLAG_GHOST);
            } else {
                values.remove_byte_flag(
                    offsets::UNIT_FIELD_BYTES_1,
                    VIS_FLAG_OFFSET,
                    VIS_FLAG_GHOST,
                );
            }
            if guid.is_player() {
                if ghost {
                    values.set_flag(offsets::PLAYER_FLAGS, 0x0000_0010);
                } else {
                    values.remove_flag(offsets::PLAYER_FLAGS, 0x0000_0010);
                }
            }
        }
    }

    /// Holder-derived `UNIT_FIELD_FLAGS` CC bits
    /// (fork aura handlers `HandleModStun` etc. set
    /// these directly; recomputation is equivalent
    /// and removal-safe). Movement generators for
    /// fear/confuse are not modeled; the flags feed
    /// the cast-prevention gate and client display.
    pub(crate) fn refresh_cc_flags(&mut self, guid: ObjectGuid) {
        let holders = self.aura_holders(guid);
        let mut bits = 0u32;
        for e in holders.iter().flat_map(|h| h.effects.iter().flatten()) {
            bits |= match e.aura_type {
                aura_type::MOD_STUN => UNIT_FLAG_STUNNED,
                aura_type::MOD_CONFUSE => UNIT_FLAG_CONFUSED,
                aura_type::MOD_FEAR => UNIT_FLAG_FLEEING,
                aura_type::MOD_SILENCE => UNIT_FLAG_SILENCED,
                aura_type::MOD_PACIFY => UNIT_FLAG_PACIFIED,
                aura_type::MOD_PACIFY_SILENCE => UNIT_FLAG_PACIFIED | UNIT_FLAG_SILENCED,
                _ => 0,
            };
        }
        if let Some(unit) = self.unit_base_mut(guid) {
            let values = unit.world.object.values_mut();
            let cur = values.get_u32(offsets::UNIT_FIELD_FLAGS);
            let next = (cur & !UNIT_FLAG_CC_MASK) | bits;
            if next != cur {
                values.set_u32(offsets::UNIT_FIELD_FLAGS, next);
            }
        }
    }

    /// `RemoveAurasWithInterruptFlags`.
    /// `RemoveAurasWithInterruptFlags(flags,
    /// exceptSpellId)`.
    pub(crate) fn remove_auras_with_interrupt_flags(
        &mut self,
        victim: ObjectGuid,
        flags: u32,
        except_spell: u32,
    ) {
        let indices: Vec<usize> = self
            .aura_holders(victim)
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                h.spell_id != except_spell
                    && self
                        .stores
                        .spells
                        .get(h.spell_id)
                        .is_some_and(|s| s.aura_interrupt_flags & flags != 0)
            })
            .map(|(i, _)| i)
            .collect();
        for index in indices.into_iter().rev() {
            self.remove_aura_at(victim, index);
        }
    }

    /// `RemoveSpellsCausingAura`.
    pub(crate) fn remove_spells_causing_aura(&mut self, victim: ObjectGuid, kind: u32) {
        let indices: Vec<usize> = self
            .aura_holders(victim)
            .iter()
            .enumerate()
            .filter(|(_, h)| h.effects.iter().flatten().any(|e| e.aura_type == kind))
            .map(|(i, _)| i)
            .collect();
        for index in indices.into_iter().rev() {
            self.remove_aura_at(victim, index);
        }
    }

    fn send_mount_result(&mut self, guid: ObjectGuid, result: u32) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u32_le(result);
        self.send_to_player(
            guid,
            WorldPacket::with_body(op::SMSG_MOUNTRESULT, payload.into_vec()),
        );
    }

    fn send_dismount_result(&mut self, guid: ObjectGuid, result: u32) {
        if !self.players.contains_key(&guid) {
            return;
        }
        let mut payload = PacketWriter::new();
        payload.write_u32_le(result);
        self.send_to_player(
            guid,
            WorldPacket::with_body(op::SMSG_DISMOUNTRESULT, payload.into_vec()),
        );
    }

    /// Holder-derived mount/transform display state
    /// (fork `HandleAuraMounted`/`HandleAuraTransform`
    /// Real paths, `SpellAuras.cpp:2151/2533`, as a
    /// recompute like `refresh_cc_flags`: display ids
    /// are chosen only on state transitions, matching
    /// the fork's apply-time rolls; the transform
    /// winner simulates the fork precedence over
    /// holders in apply order, which equals its
    /// removal-time last-negative-else-last re-apply).
    /// Pet disable/unsummon-at-mount, extra-attack
    /// reset, shapeshift-display fallback, and the
    /// channel mount-interrupt are unmodeled (no
    /// backing state).
    pub(crate) fn refresh_display(&mut self, guid: ObjectGuid) {
        use mangos_entities::creature::{
            DisplayResolver, choose_display_id_for_template, random_gender_display,
        };
        let holders = self.aura_holders(guid);
        let find_effect = |kind: u32| {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten().map(move |e| (h, e)))
                .find(|(_, e)| e.aura_type == kind)
        };
        // ---- MOUNTED (HandleAuraMounted) ----
        let mounted_now = self.unit_base_mut(guid).map_or(0, |u| {
            u.values().get_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID)
        });
        #[allow(clippy::cast_sign_loss)]
        let mount_entry = find_effect(aura_type::MOUNTED).map(|(_, e)| e.misc_value as u32);
        match (mounted_now != 0, mount_entry) {
            (false, Some(entry)) => {
                let stores = &self.stores;
                let resolver = DisplayResolver {
                    scale_for_display: &|id| stores.scale_for_display(id),
                    model_for_display: &|id| stores.model_for_display(id),
                };
                let display = stores.creature_templates.get(entry).map_or(0, |info| {
                    let (id, _) = choose_display_id_for_template(info, &resolver, &mut self.rng);
                    random_gender_display(&stores.display_addons, id, &mut self.rng)
                });
                if display == 0 {
                    // Player::Mount(0): strip + NOTMOUNTABLE.
                    self.remove_spells_causing_aura(guid, aura_type::MOUNTED);
                    self.send_mount_result(guid, 3);
                } else {
                    if let Some(unit) = self.unit_base_mut(guid) {
                        unit.values()
                            .set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, display);
                    }
                    self.remove_auras_with_interrupt_flags(
                        guid,
                        aura_interrupt_flag::MOUNT_CANCELS,
                        0,
                    );
                    self.send_mount_result(guid, 10);
                    self.pet_mount_transition(guid, true);
                }
            }
            (true, None) => {
                if let Some(unit) = self.unit_base_mut(guid) {
                    unit.values().set_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID, 0);
                }
                self.remove_auras_with_interrupt_flags(
                    guid,
                    aura_interrupt_flag::DISMOUNT_CANCELS,
                    0,
                );
                self.send_dismount_result(guid, 3);
                self.pet_mount_transition(guid, false);
            }
            _ => {}
        }
        // ---- TRANSFORM (HandleAuraTransform) ----
        let mut winner: Option<(u32, i32, bool)> = None;
        for h in &holders {
            for e in h.effects.iter().flatten() {
                if e.aura_type != aura_type::TRANSFORM {
                    continue;
                }
                let wins = match winner {
                    None => true,
                    Some((_, _, cur_positive)) => !h.positive || cur_positive,
                };
                if wins {
                    winner = Some((h.spell_id, e.misc_value, h.positive));
                }
            }
        }
        let current_spell = self.unit_base_mut(guid).map_or(0, |u| u.transform_spell);
        match winner {
            None if current_spell != 0 => {
                let native = self.unit_base_mut(guid).map_or(0, |u| {
                    u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID)
                });
                let model = self.stores.model_for_display(native);
                if let Some(unit) = self.unit_base_mut(guid) {
                    unit.transform_spell = 0;
                    // The fork restores native here even
                    // while shapeshifted; the form
                    // display is gone until re-shift.
                    unit.shapeshift_display_id = 0;
                    if native != 0 {
                        unit.set_display_id(native, model);
                    }
                    if let Some(base) = unit.transform_base_scale.take() {
                        unit.world.object.set_object_scale(base);
                    }
                }
                if let Some(equip) = self.creatures.get(&guid).map(|c| c.info.equipment_id) {
                    self.creature_load_equipment(guid, equip);
                }
            }
            Some((spell_id, misc, _)) if spell_id != current_spell => {
                self.apply_transform_display(guid, spell_id, misc);
            }
            _ => {}
        }
        // ---- SHAPESHIFT (HandleAuraModShapeshift,
        // display/form subset) ----
        self.refresh_shapeshift(guid, &holders);
    }

    /// `Player::Mount`/`Unmount` pet halves
    /// (`Player.cpp:18023`): aura mount disables
    /// the pet, unmount re-enables it (default
    /// `PetUnsummonAtMount=false` config). The
    /// config-true temporary-unsummon/resummon
    /// arms have no temporary-pet state yet —
    /// recorded in TIER2-PROGRESS Slice 6.2.
    pub(crate) fn pet_mount_transition(&mut self, guid: ObjectGuid, mounting: bool) {
        if guid.is_player() {
            if let Some(pet) = self.pet_of(guid) {
                self.set_pet_enabled(pet, !mounting);
            }
        }
    }

    /// `Aura::HandleAuraModShapeshift`
    /// (`SpellAuras.cpp:2321`) holder-derived subset:
    /// `SetShapeshiftForm` byte, form exclusivity,
    /// display/scale via `GetShapeshiftDisplayInfo`,
    /// the 9033 shift casts, the
    /// `SHAPESHIFTING_CANCELS` strip, and the 23397
    /// (Nefarian warrior call) → 2458 removal cast.
    /// Power-type switching, Furor, tactical-mastery
    /// rage caps, `HandleShapeshiftBoosts`, and
    /// `InitDataForForm` are recorded in
    /// TIER2-PROGRESS Slice 6.3 (form-driven stat
    /// recompute does not exist yet).
    fn refresh_shapeshift(
        &mut self,
        guid: ObjectGuid,
        holders: &[mangos_entities::aura::AuraHolder],
    ) {
        // Winner: last holder carrying MOD_SHAPESHIFT
        // (application order; the fork strips the
        // others inside the new form's apply).
        let mut winner: Option<(u32, u8)> = None;
        let mut other_forms: Vec<u32> = Vec::new();
        for h in holders {
            for e in h.effects.iter().flatten() {
                if e.aura_type == aura_type::MOD_SHAPESHIFT {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let form = e.misc_value as u8;
                    if let Some((prev, _)) = winner {
                        if prev != h.spell_id {
                            other_forms.push(prev);
                        }
                    }
                    winner = Some((h.spell_id, form));
                }
            }
        }
        let (old_form, old_spell, transformed) =
            self.unit_base_mut(guid).map_or((0, 0, false), |u| {
                (
                    u.values().get_byte(offsets::UNIT_FIELD_BYTES_1, 2),
                    u.shapeshift_spell,
                    u.transform_spell != 0,
                )
            });
        let (new_spell, new_form) = winner.unwrap_or((0, 0));
        // Fork: a form missing from
        // SpellShapeshiftForm.dbc makes the handler
        // log and return without touching the form
        // byte. An unloaded table (test worlds)
        // treats every form as known.
        if new_form != 0
            && !self.stores.shapeshift_forms.is_empty()
            && !self
                .stores
                .shapeshift_forms
                .contains_key(&u32::from(new_form))
        {
            return;
        }
        if new_form == old_form && new_spell == old_spell {
            return;
        }
        // State first: nested refreshes from the
        // removals/casts below must see the
        // transition as done.
        if let Some(unit) = self.unit_base_mut(guid) {
            unit.values()
                .set_byte(offsets::UNIT_FIELD_BYTES_1, 2, new_form);
            unit.shapeshift_spell = new_spell;
        }
        self.shapeshift_display_transition(guid, old_form, new_form, transformed);
        // Exclusivity (fork: inside apply).
        for spell_id in other_forms {
            self.remove_auras_due_to_spell(guid, spell_id);
        }
        // SHAPESHIFTING_CANCELS strip on apply for
        // non-stance forms.
        if new_form != 0 {
            let flags1 = self
                .stores
                .shapeshift_forms
                .get(&u32::from(new_form))
                .copied()
                .unwrap_or(0);
            if flags1 & 1 == 0 {
                self.remove_auras_with_interrupt_flags(
                    guid,
                    aura_interrupt_flag::SHAPESHIFTING_CANCELS,
                    new_spell,
                );
            }
        }
        // 9033 (Shapeshift Form Effect) fires on
        // both apply and remove of the druid forms
        // in the fork; one cast per transition is
        // observably identical (idempotent strip).
        if DRUID_9033_FORMS.contains(&old_form) || DRUID_9033_FORMS.contains(&new_form) {
            self.cast_spell(
                guid,
                9033,
                CastTarget::Unit(guid),
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
        // Removing the Nefarian warrior call (23397)
        // re-casts regular Berserker Stance.
        if old_spell == 23397 && new_spell != 23397 {
            self.cast_spell(
                guid,
                2458,
                CastTarget::Unit(guid),
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

    /// Display half of a shapeshift transition
    /// (fork gates both sides on `!GetTransForm()`).
    fn shapeshift_display_transition(
        &mut self,
        guid: ObjectGuid,
        old_form: u8,
        new_form: u8,
        transformed: bool,
    ) {
        if transformed {
            // Form changed under a transform: fork
            // display handlers all skip; nothing of
            // the form is on screen.
            if let Some(unit) = self.unit_base_mut(guid) {
                unit.shapeshift_display_id = 0;
            }
            return;
        }
        let had_display = self
            .unit_base_mut(guid)
            .is_some_and(|u| u.shapeshift_display_id != 0);
        if old_form != 0 && had_display {
            let native = self.unit_base_mut(guid).map_or(0, |u| {
                u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID)
            });
            let model = self.stores.model_for_display(native);
            if let Some(unit) = self.unit_base_mut(guid) {
                unit.shapeshift_display_id = 0;
                if native != 0 {
                    unit.set_display_id(native, model);
                }
                if let Some(base) = unit.transform_base_scale.take() {
                    unit.world.object.set_object_scale(base);
                }
            }
        }
        if new_form != 0 {
            let (display_id, scale_mod) = self.shapeshift_display_info(guid, new_form);
            if display_id > 0 {
                let model = self.stores.model_for_display(display_id);
                let native_model_scale = self
                    .unit_base_mut(guid)
                    .map(|u| u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID))
                    .and_then(|n| self.stores.model_for_display(n))
                    .map_or(1.0, |m| m.native_scale);
                if let Some(unit) = self.unit_base_mut(guid) {
                    if unit.transform_base_scale.is_none() {
                        unit.transform_base_scale = Some(unit.world.object.object_scale());
                    }
                    unit.set_display_id(display_id, model);
                    unit.shapeshift_display_id = display_id;
                    if scale_mod != 0.0 && native_model_scale != 0.0 {
                        let base = unit.transform_base_scale.unwrap_or(1.0);
                        unit.world
                            .object
                            .set_object_scale(base * scale_mod / native_model_scale);
                    }
                }
            }
        }
    }

    /// `GetShapeshiftDisplayInfo`
    /// (`SpellAuras.cpp:2217`): (display id,
    /// transform-scale mod). Team via
    /// `Player::TeamForRace` on the bytes-0 race.
    fn shapeshift_display_info(&mut self, guid: ObjectGuid, form: u8) -> (u32, f32) {
        let (race, native) = self.unit_base_mut(guid).map_or((0, 0), |u| {
            (
                u.values().get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
            )
        });
        let horde = matches!(race, 2 | 5 | 6 | 8);
        let is_player = guid.is_player();
        match form {
            // FORM_CAT
            1 => (if is_player && horde { 8571 } else { 892 }, 0.80),
            // FORM_TRAVEL
            3 => (632, 0.80),
            // FORM_AQUA
            4 => (2428, 0.80),
            // FORM_BEAR / FORM_DIREBEAR
            5 | 8 => (if is_player && horde { 2289 } else { 2281 }, 1.0),
            // FORM_GHOUL (no IsPlayer check in fork)
            7 => (if horde { 0 } else { 10045 }, 1.0),
            // FORM_CREATUREBEAR: native model switch
            14 => {
                let model_id = self
                    .stores
                    .model_for_display(native)
                    .map_or(0, |m| m.model_id);
                let display = match model_id {
                    // MODEL_NELF_FEMALE/MALE
                    55 | 56 => 2281,
                    // MODEL_TAUREN_FEMALE/MALE
                    59 | 60 => 2289,
                    _ => 902,
                };
                (display, 1.0)
            }
            // FORM_GHOSTWOLF
            16 => (4613, 0.80),
            // FORM_MOONKIN
            31 => (if is_player && horde { 15375 } else { 15374 }, 1.0),
            // FORM_TREE
            2 => (864, 1.0),
            // FORM_SPIRITOFREDEMPTION
            32 => (16031, 1.0),
            _ => (0, 1.0),
        }
    }

    /// `HandleAuraTransform` apply half for a winning
    /// switch (display choice + `SetTransformScale` +
    /// creature equipment swap).
    fn apply_transform_display(&mut self, guid: ObjectGuid, spell_id: u32, misc: i32) {
        use mangos_entities::creature::{
            DisplayResolver, UNIT_DISPLAY_ID_BOX, choose_display_id_for_template,
        };
        let mut display_id = 0u32;
        let mut mod_x = 1.0f32;
        let mut equip: Option<u32> = None;
        if misc == 0 {
            // Orb of Deception (16739) race/gender table;
            // other misc-0 spells log-only in the fork.
            if spell_id == 16739 {
                let (race, cur_display, native) = self.unit_base_mut(guid).map_or((0, 0, 0), |u| {
                    (
                        u.values().get_byte(offsets::UNIT_FIELD_BYTES_0, 0),
                        u.values().get_u32(offsets::UNIT_FIELD_DISPLAYID),
                        u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID),
                    )
                });
                let male = self
                    .stores
                    .display_addons
                    .creature(cur_display)
                    .is_none_or(|a| a.gender == 0);
                let native_scale = self.stores.scale_for_display(native);
                // Fork race ids: human 1, orc 2, dwarf 3,
                // nightelf 4, undead 5, tauren 6, gnome 7,
                // troll 8.
                let pick = |m: u32, f: u32| if male { m } else { f };
                match race {
                    8 => display_id = pick(10135, 10134),
                    6 => {
                        display_id = pick(10136, 10147);
                        if native_scale != 0.0 {
                            mod_x = 1.15 / native_scale;
                        }
                    }
                    1 => display_id = pick(10137, 10138),
                    2 => display_id = pick(10139, 10140),
                    3 => display_id = pick(10141, 10142),
                    4 => display_id = pick(10143, 10144),
                    5 => display_id = pick(10146, 10145),
                    7 => {
                        display_id = pick(10148, 10149);
                        if native_scale != 0.0 {
                            mod_x = if male { 1.35 } else { 1.25 } / native_scale;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            #[allow(clippy::cast_sign_loss)]
            let entry = misc as u32;
            let stores = &self.stores;
            let resolver = DisplayResolver {
                scale_for_display: &|id| stores.scale_for_display(id),
                model_for_display: &|id| stores.model_for_display(id),
            };
            match stores.creature_templates.get(entry) {
                None => display_id = UNIT_DISPLAY_ID_BOX,
                Some(info) => {
                    let (id, scale) =
                        choose_display_id_for_template(info, &resolver, &mut self.rng);
                    display_id = id;
                    if self.creatures.contains_key(&guid) {
                        equip = Some(info.equipment_id);
                        mod_x = scale;
                    }
                }
            }
        }
        let model = self.stores.model_for_display(display_id);
        let native_model_scale = self
            .unit_base_mut(guid)
            .map(|u| u.values().get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID))
            .and_then(|n| self.stores.model_for_display(n))
            .map_or(1.0, |m| m.native_scale);
        if let Some(unit) = self.unit_base_mut(guid) {
            if display_id != 0 {
                if unit.transform_base_scale.is_none() {
                    unit.transform_base_scale = Some(unit.world.object.object_scale());
                }
                unit.set_display_id(display_id, model);
                // The transform owns the display now;
                // any shapeshift display is displaced.
                unit.shapeshift_display_id = 0;
                // SetTransformScale(mod_x): object scale
                // *= mod_x / native model scale.
                if mod_x != 0.0 && native_model_scale != 0.0 {
                    let base = unit.transform_base_scale.unwrap_or(1.0);
                    unit.world
                        .object
                        .set_object_scale(base * mod_x / native_model_scale);
                }
            }
            unit.transform_spell = spell_id;
        }
        if let Some(equip_id) = equip {
            self.creature_load_equipment(guid, equip_id);
        }
    }

    /// `Unit::UpdateSpeed` aura factor (fork
    /// `Unit.cpp:7195`), run + swim subset:
    /// main mod = max-positive increase; bonus =
    /// max(not-stack, always-multiplier); strongest
    /// slow multiplies last. Normalization/ghost
    /// arms unmodeled.
    pub(crate) fn refresh_speed(&mut self, guid: ObjectGuid) {
        let (base_run, base_swim) = {
            let Some(unit) = self.unit_base_mut(guid) else {
                return;
            };
            (
                unit.base_speed_rate[mangos_entities::unit::MOVE_RUN],
                unit.base_speed_rate[3],
            )
        };
        let holders = self.aura_holders(guid);
        let max_pos = |kind: u32| -> i32 {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == kind && e.amount > 0)
                .map(|e| e.amount)
                .max()
                .unwrap_or(0)
        };
        let max_neg = |kind: u32| -> i32 {
            holders
                .iter()
                .flat_map(|h| h.effects.iter().flatten())
                .filter(|e| e.aura_type == kind && e.amount < 0)
                .map(|e| e.amount)
                .min()
                .unwrap_or(0)
        };
        // UpdateSpeed mounted arm: on-mount aura kinds.
        let mounted = self
            .unit_base_mut(guid)
            .is_some_and(|u| u.values().get_u32(offsets::UNIT_FIELD_MOUNTDISPLAYID) != 0);
        #[allow(clippy::cast_precision_loss)]
        let factor = |increase_kind: u32, stack_kinds: Option<(u32, u32)>| -> f32 {
            let main_mod = max_pos(increase_kind);
            let (stack, non_stack) = match stack_kinds {
                Some((always, not_stack)) => (
                    self.total_aura_multiplier(guid, always, None),
                    (100.0 + max_pos(not_stack) as f32) / 100.0,
                ),
                None => (1.0, 1.0),
            };
            let bonus = stack.max(non_stack);
            let mut speed = if main_mod != 0 {
                bonus * (100.0 + main_mod as f32) / 100.0
            } else {
                bonus
            };
            let slow = max_neg(aura_type::MOD_DECREASE_SPEED);
            if slow != 0 {
                speed *= (100.0 + slow as f32) / 100.0;
            }
            speed
        };
        let (run_kind, run_stack) = if mounted {
            (
                aura_type::MOD_INCREASE_MOUNTED_SPEED,
                (
                    aura_type::MOD_MOUNTED_SPEED_ALWAYS,
                    aura_type::MOD_MOUNTED_SPEED_NOT_STACK,
                ),
            )
        } else {
            (
                aura_type::MOD_INCREASE_SPEED,
                (aura_type::MOD_SPEED_ALWAYS, aura_type::MOD_SPEED_NOT_STACK),
            )
        };
        let run = base_run * factor(run_kind, Some(run_stack));
        let swim = base_swim * factor(aura_type::MOD_INCREASE_SWIM_SPEED, None);
        self.set_speed_rate(guid, mangos_entities::unit::MOVE_RUN, run);
        self.set_speed_rate(guid, 3, swim);
    }

    pub(crate) fn remove_aura_at(&mut self, victim: ObjectGuid, index: usize) {
        self.remove_aura_at_mode(victim, index, AuraRemoveMode::Default);
    }

    #[allow(clippy::too_many_lines)] // one RemoveAura port
    pub(crate) fn remove_aura_at_mode(
        &mut self,
        victim: ObjectGuid,
        index: usize,
        mode: AuraRemoveMode,
    ) {
        let mut has_ap = false;
        let mut has_stat = false;
        let mut hp_before = None;
        let peek = self.unit_base_mut(victim).and_then(|u| {
            u.auras.get(index).map(|h| {
                (
                    h.spell_id,
                    h.effects.iter().flatten().any(|e| {
                        e.aura_type == aura_type::MOD_TOTAL_STAT_PERCENTAGE
                            && e.misc_value == STAT_STAMINA_I32
                    }),
                )
            })
        });
        let preserve = peek.is_some_and(|(spell_id, stamina)| {
            stamina
                && self
                    .stores
                    .spells
                    .get(spell_id)
                    .is_some_and(|s| s.attributes & 0x10 != 0)
        });
        if preserve {
            hp_before = self
                .unit_base_mut(victim)
                .filter(|u| u.is_alive())
                .map(|u| (u.health(), u.max_health()));
        }
        // Health/energy TOTAL-group removal state
        // (`HandleAuraModIncreaseHealth[Percent]` /
        // energy remove arms): Last Stand adjusts
        // current health *before* the stat removal;
        // percent forms preserve the ratio across it.
        let he_peek = self.unit_base(victim).and_then(|u| {
            u.auras.get(index).and_then(|h| {
                Self::holder_has_health_energy_mod(h).then(|| {
                    (
                        h.clone(),
                        u.is_alive(),
                        u.health(),
                        u.max_health(),
                        core::array::from_fn(|p| u.power(p)),
                        core::array::from_fn(|p| u.max_power(p)),
                    )
                })
            })
        });
        if let Some((h, alive, hp, _, _, _)) = &he_peek
            && h.spell_id == 12976
            && *alive
        {
            let flat: i32 = h
                .effects
                .iter()
                .flatten()
                .filter(|e| e.aura_type == aura_type::MOD_INCREASE_HEALTH)
                .map(|e| e.amount)
                .sum();
            if let Some(unit) = self.unit_base_mut(victim) {
                let next = if i64::from(*hp) > i64::from(flat) {
                    i64::from(*hp) - i64::from(flat)
                } else {
                    1
                };
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                unit.set_health(next.clamp(0, i64::from(u32::MAX)) as u32);
            }
        }
        let mut removed_holder: Option<(ObjectGuid, u32)> = None;
        let mut has_health_energy = false;
        let extracted = if let Some(unit) = self.unit_base_mut(victim) {
            if index >= unit.auras.len() {
                return;
            }
            Some(unit.auras.remove(index))
        } else {
            None
        };
        if let Some(mut holder) = extracted {
            removed_holder = Some((holder.caster, holder.spell_id));
            has_ap = holder
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_ATTACK_POWER);
            has_stat = Self::holder_has_stat_mod(&holder);
            has_health_energy = Self::holder_has_health_energy_mod(&holder);
            // AuraScript::OnBeforeApply(apply=false)
            // before the unapply handlers (fork
            // ApplyModifier order).
            self.aura_script_on_before_apply(victim, &mut holder, false, mode);
            if let Some(unit) = self.unit_base_mut(victim) {
                Self::aura_effect_apply(unit, &holder, false);
                Self::write_aura_slot(unit, &holder, true);
            }
        }
        if let Some((removed_caster, removed_spell_id)) = removed_holder {
            self.aura_script_on_after_apply(victim, removed_spell_id, removed_caster, false);
            self.aura_dummy_for_spell(victim, removed_spell_id, false);
        }
        if has_stat || has_health_energy {
            self.refresh_derived_stats(victim);
            if let Some((cur, max)) = hp_before
                && max > 0
                && let Some(unit) = self.unit_base_mut(victim)
            {
                #[allow(clippy::cast_possible_truncation)]
                let scaled =
                    (u64::from(unit.max_health()) * u64::from(cur) / u64::from(max)) as u32;
                unit.set_health(scaled);
            }
            if let Some((h, alive, old_hp, old_max, old_powers, old_max_powers)) = he_peek
                && alive
            {
                self.remove_health_energy_adjustments(
                    victim,
                    &h,
                    (old_hp, old_max, old_powers, old_max_powers),
                );
            }
        }
        if has_ap {
            self.refresh_weapon_damage_fields(victim);
        }
        // Cooldown-on-event conversion: the timed
        // cooldown starts when the aura fades
        // (`SpellAuras.cpp` holder cleanup).
        if let Some((caster, spell_id)) = removed_holder
            && let Some(spell) = self.stores.spells.get(spell_id).cloned()
            && spell.attributes & ATTR_COOLDOWN_ON_EVENT != 0
        {
            self.start_event_cooldown(caster, &spell);
        }
        self.refresh_display(victim);
        self.refresh_immune_state(victim);
        self.refresh_attack_speed(victim);
        self.refresh_skill_bonuses(victim);
        self.refresh_speed(victim);
        self.refresh_cc_flags(victim);
        self.refresh_ghost_flag(victim);
    }

    pub(crate) fn remove_all_auras(&mut self, victim: ObjectGuid) {
        loop {
            let len = self.unit_base_mut(victim).map_or(0, |u| u.auras.len());
            if len == 0 {
                return;
            }
            self.remove_aura_at(victim, len - 1);
        }
    }

    /// `SpellAuraHolder::SetStackAmount` reduced: set the
    /// holder's stack count and rewrite its aura slot
    /// (stack byte). Modifier magnitude does not scale
    /// by stack in the Rust aura system yet (global
    /// stacking gap; the fork rescales amounts in the
    /// aura handlers) — recorded in TIER3-PROGRESS.
    pub(crate) fn set_aura_stack_amount(&mut self, victim: ObjectGuid, spell_id: u32, stack: u32) {
        let landed = self.unit_base_mut(victim).and_then(|unit| {
            let holder = unit.auras.iter_mut().find(|h| h.spell_id == spell_id)?;
            holder.stack = stack.max(1);
            let snapshot = holder.clone();
            Self::write_aura_slot(unit, &snapshot, false);
            Some(())
        });
        if landed.is_some() {
            self.refresh_derived_stats(victim);
        }
    }

    /// `RemoveAurasDueToSpell` (no cancelable gate;
    /// proc charge exhaustion path).
    pub fn remove_auras_due_to_spell(&mut self, victim: ObjectGuid, spell_id: u32) {
        loop {
            let found = self
                .unit_base_mut(victim)
                .and_then(|u| u.auras.iter().position(|h| h.spell_id == spell_id));
            match found {
                Some(i) => self.remove_aura_at(victim, i),
                None => return,
            }
        }
    }

    /// `CMSG_CANCEL_AURA` (`RemoveAurasDueToSpell` with
    /// the cancelable gate).
    pub fn cancel_aura(&mut self, victim: ObjectGuid, spell_id: u32) {
        loop {
            let found = self.unit_base_mut(victim).and_then(|u| {
                u.auras
                    .iter()
                    .position(|h| h.spell_id == spell_id && h.cancelable)
            });
            match found {
                Some(i) => self.remove_aura_at_mode(victim, i, AuraRemoveMode::Cancel),
                None => return,
            }
        }
    }

    /// `Spell::EffectApplyAura` subset: build the holder
    /// from every `APPLY_AURA` effect (amounts snapshot at
    /// apply, 1.12.1 rule) or refresh an existing
    /// same-caster application.
    /// Direct aura application by spell id (spawn auras,
    /// `AddAura` at materialization).
    pub(crate) fn apply_aura_by_id(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        victim: ObjectGuid,
    ) {
        if let Some(spell) = self.stores.spells.get(spell_id).cloned() {
            self.apply_spell_aura(caster, &spell, victim, None, [None; 3]);
        }
    }

    #[allow(clippy::too_many_lines, clippy::needless_range_loop)] // one EffectApplyAura port
    pub(crate) fn apply_spell_aura(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        bp: [Option<i32>; 3],
    ) {
        let raw_duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        // 0 and -1 are the permanent shapes; only
        // positive durations take the spellmod.
        let duration = match raw_duration.cmp(&0) {
            std::cmp::Ordering::Equal => -1,
            std::cmp::Ordering::Less => raw_duration,
            std::cmp::Ordering::Greater => {
                let modded = self.modded_duration_ms(caster, spell, applied_mods.as_deref_mut());
                if modded <= 0 {
                    return;
                }
                modded
            }
        };

        let refreshed = self.unit_base_mut(victim).and_then(|unit| {
            unit.auras
                .iter_mut()
                .find(|h| h.spell_id == spell.id && h.caster == caster)
                .map(|h| {
                    h.duration_ms = h.max_duration_ms;
                    (h.slot, h.duration_ms)
                })
        });
        if let Some((slot, dur)) = refreshed {
            self.send_aura_duration(victim, slot, dur);
            return;
        }

        let positive = spell.is_positive_spell();
        let cancelable = positive && spell.attributes & ATTR_NO_AURA_CANCEL == 0;
        let caster_level = self.unit_level(caster);
        let mut holder = AuraHolder::new(
            spell.id,
            caster,
            caster_level,
            duration,
            positive,
            cancelable,
        );
        #[allow(clippy::cast_possible_wrap)]
        {
            holder.proc_charges = spell.proc_charges as i32;
        }
        for i in 0..3 {
            let eff = spell.effects[i];
            // `CreateAura` builds an aura for
            // APPLY_AURA-family and
            // PERSISTENT_AREA_AURA effects alike
            // (Flamestrike's periodic damage rides on
            // the persistent effect itself).
            if eff.effect != effect::APPLY_AURA
                && eff.effect != effect::APPLY_AREA_AURA_PARTY
                && eff.effect != effect::PERSISTENT_AREA_AURA
            {
                continue;
            }
            if eff.apply_aura_name == aura_type::NONE {
                continue;
            }
            let mut value = self.effect_value_bp(spell, i, caster_level, bp[i]);
            // Apply-time Done(DOT) snapshot
            // (`CalculateDotDamage` / periodic-heal
            // 1.11 snapshot rule); melee-class periodic
            // damage routes through MeleeDamageBonus
            // in the fork — recorded exclusion.
            if !spell_meleedamage(spell.dmg_class) {
                match eff.apply_aura_name {
                    aura_type::PERIODIC_DAMAGE | aura_type::PERIODIC_LEECH => {
                        value = self.spell_damage_bonus_done(
                            caster,
                            victim,
                            spell,
                            i,
                            value,
                            true,
                            1,
                            applied_mods.as_deref_mut(),
                        );
                    }
                    aura_type::PERIODIC_HEAL => {
                        value = self.spell_healing_bonus_done(
                            caster,
                            spell,
                            i,
                            value,
                            true,
                            1,
                            applied_mods.as_deref_mut(),
                        );
                    }
                    _ => {}
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            let amount = value as i32;
            // spell_haunting_spirits AuraScript
            // OnBeforeApply: SetPeriodicTimer(5s)
            // (sets isPeriodic + period + timer).
            let haunt = spell.id == 7057 && i == 0;
            let periodic = haunt
                || matches!(
                    eff.apply_aura_name,
                    aura_type::PERIODIC_DAMAGE
                        | aura_type::PERIODIC_HEAL
                        | aura_type::PERIODIC_ENERGIZE
                        | aura_type::PERIODIC_TRIGGER_SPELL
                        | aura_type::PERIODIC_DAMAGE_PERCENT
                        | aura_type::PERIODIC_LEECH
                );
            let period_ms = if haunt {
                5000
            } else if periodic {
                eff.amplitude
            } else {
                0
            };
            #[allow(clippy::cast_possible_wrap)]
            let timer = period_ms as i32;
            // `HandleAddModifier` charge seeding:
            // proc charges, infinite when the source
            // stacks, custom 1 for Shadow Trance /
            // Netherwind Focus.
            let mod_charges = if matches!(
                eff.apply_aura_name,
                aura_type::ADD_FLAT_MODIFIER | aura_type::ADD_PCT_MODIFIER
            ) {
                if matches!(spell.id, 17941 | 22008) {
                    1
                } else if spell.stack_amount > 1 {
                    0
                } else {
                    #[allow(clippy::cast_possible_wrap)]
                    {
                        spell.proc_charges as i32
                    }
                }
            } else {
                0
            };
            holder.effects[i] = Some(AuraEffectState {
                aura_type: eff.apply_aura_name,
                amount,
                base_points: amount,
                period_ms,
                periodic_timer: timer,
                periodic_ticks: 0,
                misc_value: eff.misc_value,
                mod_charges,
            });
        }
        if holder.effects.iter().all(Option::is_none) {
            return;
        }
        let applied_types: Vec<u32> = holder
            .effects
            .iter()
            .flatten()
            .map(|e| e.aura_type)
            .collect();
        self.add_aura_holder(victim, holder);
        self.apply_immunity_purges(victim, spell);
        if victim.is_creature() {
            for aura_type_id in applied_types {
                self.eventai_on_hit_by_aura(victim, aura_type_id, caster);
                if !self.creatures.contains_key(&victim) {
                    break;
                }
            }
        }
    }

    /// Login: apply the deferred `character_aura` rows
    /// (`Player::_LoadAuras` subset — amounts/durations
    /// from the saved row, effects restricted to the
    /// slice's aura types by construction of the tick
    /// loop).
    pub fn apply_saved_auras(&mut self, player_guid: ObjectGuid) {
        let rows = self
            .players
            .get(&player_guid)
            .map_or_else(Vec::new, |e| e.player.deferred.auras.clone());
        for row in rows {
            let Some(spell) = self.stores.spells.get(row.spell).cloned() else {
                continue;
            };
            let caster = ObjectGuid::from_raw(row.caster_guid);
            let positive = spell.is_positive_spell();
            let cancelable = positive && spell.attributes & ATTR_NO_AURA_CANCEL == 0;
            let caster_level = if caster == player_guid {
                self.unit_level(player_guid)
            } else {
                0
            };
            let mut holder = AuraHolder::new(
                row.spell,
                caster,
                caster_level,
                row.duration,
                positive,
                cancelable,
            );
            holder.max_duration_ms = row.max_duration;
            #[allow(clippy::cast_possible_truncation)]
            {
                holder.stack = row.stacks.max(1) as u32;
            }
            // Saved charges are written 0 today; reseed
            // like a fresh application (see mod charges).
            #[allow(clippy::cast_possible_wrap)]
            {
                holder.proc_charges = spell.proc_charges as i32;
            }
            for i in 0..3 {
                if row.effect_index_mask & (1u8 << i) == 0 {
                    continue;
                }
                let eff = spell.effects[i];
                #[allow(clippy::cast_possible_truncation)]
                let amount = row.base_points[i] as i32;
                let period_ms = if row.periodic_time[i] > 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        row.periodic_time[i] as u32
                    }
                } else {
                    0
                };
                // Saved rows carry no live mod charges
                // (save writes 0); a surviving modifier
                // aura therefore reseeds like a fresh
                // application.
                let mod_charges = if matches!(
                    eff.apply_aura_name,
                    aura_type::ADD_FLAT_MODIFIER | aura_type::ADD_PCT_MODIFIER
                ) {
                    if matches!(spell.id, 17941 | 22008) {
                        1
                    } else if spell.stack_amount > 1 {
                        0
                    } else {
                        #[allow(clippy::cast_possible_wrap)]
                        {
                            spell.proc_charges as i32
                        }
                    }
                } else {
                    0
                };
                holder.effects[i] = Some(AuraEffectState {
                    aura_type: eff.apply_aura_name,
                    amount,
                    base_points: amount,
                    period_ms,
                    #[allow(clippy::cast_possible_wrap)]
                    periodic_timer: period_ms as i32,
                    periodic_ticks: 0,
                    misc_value: eff.misc_value,
                    mod_charges,
                });
            }
            if holder.effects.iter().all(Option::is_none) {
                continue;
            }
            self.add_aura_holder(player_guid, holder);
        }
    }

    /// Holder durations + periodic ticks
    /// (`SpellAuraHolder::Update` / `Aura::Update`).
    pub(crate) fn update_auras(&mut self, diff_ms: u32) {
        let targets: Vec<ObjectGuid> = self
            .players
            .keys()
            .chain(self.creatures.keys())
            .copied()
            .collect();
        // `PersistentAreaAura::Update` pre-pass: a
        // holder sourced from a dynamic object is
        // removed when the object is gone or the
        // target left its radius; a persistent holder
        // without a dynobject (reloaded from DB)
        // removes itself immediately. Whole-holder
        // removal where the fork removes per-effect is
        // a recorded divergence.
        let mut orphaned: Vec<(ObjectGuid, usize)> = Vec::new();
        for target in &targets {
            let holders = self.aura_holders(*target);
            for (index, holder) in holders.iter().enumerate() {
                let is_persistent = holder.dynobj != ObjectGuid::EMPTY
                    || self.stores.spells.get(holder.spell_id).is_some_and(|s| {
                        s.effects
                            .iter()
                            .any(|e| e.effect == effect::PERSISTENT_AREA_AURA)
                    });
                if !is_persistent {
                    continue;
                }
                let remove = if holder.dynobj == ObjectGuid::EMPTY {
                    true
                } else if let Some(dynobj) = self.dynobjects.get(&holder.dynobj) {
                    self.unit_position(*target).is_none_or(|pos| {
                        crate::visibility::dist2d(pos, dynobj.world.position()) > dynobj.radius
                    })
                } else {
                    true
                };
                if remove {
                    orphaned.push((*target, index));
                }
            }
        }
        orphaned.sort_by_key(|&(_, index)| std::cmp::Reverse(index));
        for (target, index) in orphaned {
            self.remove_aura_at(target, index);
        }
        let mut ticks: Vec<PeriodicTick> = Vec::new();
        let mut expired: Vec<(ObjectGuid, usize)> = Vec::new();
        for target in &targets {
            let Some(unit) = self.unit_base_mut(*target) else {
                continue;
            };
            if !unit.is_alive() {
                continue;
            }
            for (index, holder) in unit.auras.iter_mut().enumerate() {
                for (eff_index, eff) in holder.effects.iter_mut().enumerate() {
                    let Some(state) = eff else { continue };
                    if !state.is_periodic() {
                        continue;
                    }
                    state.periodic_timer -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                    if state.periodic_timer <= 0 {
                        #[allow(clippy::cast_possible_wrap)]
                        let period = state.period_ms as i32;
                        if state.periodic_timer < -period {
                            state.periodic_timer = 0;
                        } else {
                            state.periodic_timer += period;
                        }
                        state.periodic_ticks += 1;
                        #[allow(clippy::cast_possible_truncation)]
                        let effect_idx = eff_index as u32;
                        ticks.push(PeriodicTick {
                            target: *target,
                            caster: holder.caster,
                            spell_id: holder.spell_id,
                            aura_type: state.aura_type,
                            amount: state.amount,
                            effect_index: effect_idx,
                            misc_value: state.misc_value,
                        });
                    }
                }
                if holder.duration_ms > 0 {
                    holder.duration_ms -= i32::try_from(diff_ms).unwrap_or(i32::MAX);
                    if holder.duration_ms <= 0 {
                        expired.push((*target, index));
                    }
                }
            }
        }
        self.process_periodic_ticks(ticks);
        // Remove expired holders back-to-front per unit.
        expired.sort_by_key(|&(_, index)| std::cmp::Reverse(index));
        for (target, index) in expired {
            self.remove_aura_at_mode(target, index, AuraRemoveMode::Expire);
        }
    }

    #[allow(clippy::too_many_lines)] // one periodic-tick drain port
    fn process_periodic_ticks(&mut self, ticks: Vec<PeriodicTick>) {
        for tick in ticks {
            match tick.aura_type {
                // `Aura::TriggerSpell` generic arm: cast
                // the effect's trigger spell at the aura
                // target (wearer casts with
                // `SPELL_ATTR_WEARER_CASTS_PROC_TRIGGER`).
                // The fork's per-spell trigger customs
                // are not ported.
                aura_type::PERIODIC_TRIGGER_SPELL => {
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    #[allow(clippy::cast_possible_truncation)]
                    let raw_trigger = spell.effects[tick.effect_index as usize].trigger_spell;
                    if !self.unit_is_alive(tick.target) {
                        continue;
                    }
                    let caster = if spell.attributes & 0x0000_0800 != 0 {
                        tick.target
                    } else {
                        tick.caster
                    };
                    if !self.unit_is_alive(caster) {
                        continue;
                    }
                    // AuraScript::OnPeriodicTrigger may
                    // cast a custom spell and suppress
                    // (None) or replace the default
                    // trigger cast (fork TriggerSpell).
                    let mut trigger = (raw_trigger != 0).then_some(raw_trigger);
                    self.aura_script_on_periodic_trigger(&tick, &spell, caster, &mut trigger);
                    let Some(trigger_id) = trigger else {
                        // `Aura::TriggerSpell` tail
                        // (`SpellAuras.cpp:1518-1524`): no
                        // triggered spell and no AuraScript
                        // (empty `script_name` proxy) ->
                        // creature dummy script on the
                        // wearer. The fork logs an error
                        // when unhandled; silent here.
                        if spell.script_name.is_empty() && tick.target.is_creature() {
                            self.effect_dummy_creature(
                                caster,
                                tick.spell_id,
                                tick.effect_index as usize,
                                tick.target,
                            );
                        }
                        continue;
                    };
                    self.cast_spell(
                        caster,
                        trigger_id,
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
                aura_type::PERIODIC_DAMAGE | aura_type::PERIODIC_LEECH => {
                    let is_leech = tick.aura_type == aura_type::PERIODIC_LEECH;
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    // Caster must still exist on the map
                    // for the damage attribution.
                    if self.unit_position(tick.caster).is_none() {
                        continue;
                    }
                    #[allow(clippy::cast_possible_wrap)]
                    if self.is_immune_to_damage(tick.target, 1i32 << spell.school, Some(&spell)) {
                        self.send_spell_or_damage_immune(tick.caster, tick.target, tick.spell_id);
                        continue;
                    }
                    let health_before = if is_leech {
                        self.unit_value_u32(tick.target, offsets::UNIT_FIELD_HEALTH)
                    } else {
                        0
                    };
                    self.school_damage(
                        tick.caster,
                        &spell,
                        tick.target,
                        tick.amount,
                        tick.effect_index,
                        Some(tick.spell_id),
                        None,
                        Some(&ProcRaiseCtx {
                            attacker_flags: proc_flag::DEAL_HARMFUL_PERIODIC,
                            victim_flags: proc_flag::TAKE_HARMFUL_PERIODIC,
                            can_trigger: true,
                            triggered_by_aura: false,
                            att_type: 0,
                            applied_mods: Vec::new(),
                        }),
                    );
                    if is_leech {
                        // Heal the caster for damage
                        // actually dealt (health delta =
                        // fork DealDamage result in this
                        // sequential tick) × multiple value.
                        let dealt = health_before.saturating_sub(
                            self.unit_value_u32(tick.target, offsets::UNIT_FIELD_HEALTH),
                        );
                        if dealt > 0 {
                            let mut multiplier =
                                spell.effects[tick.effect_index as usize].multiple_value;
                            if multiplier <= 0.0 {
                                multiplier = 1.0;
                            }
                            multiplier = self.apply_spell_mod(
                                tick.caster,
                                &spell,
                                spellmod::MULTIPLE_VALUE,
                                multiplier,
                                false,
                                None,
                            );
                            #[allow(
                                clippy::cast_precision_loss,
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss
                            )]
                            let heal = (dealt as f32 * multiplier) as u32;
                            if heal > 0 {
                                self.heal_unit(tick.caster, heal);
                            }
                        }
                    }
                }
                aura_type::PERIODIC_HEAL => {
                    if tick.amount > 0 {
                        // Done snapshotted at apply;
                        // `SpellHealingBonusTaken`(DOT)
                        // per tick, then ditheru.
                        #[allow(clippy::cast_precision_loss)]
                        let amount = match self.stores.spells.get(tick.spell_id).cloned() {
                            Some(spell)
                                if self.is_immune_to_school(
                                    tick.target,
                                    &spell,
                                    tick.effect_index as usize,
                                ) =>
                            {
                                self.send_spell_or_damage_immune(
                                    tick.caster,
                                    tick.target,
                                    tick.spell_id,
                                );
                                0.0
                            }
                            Some(spell) => {
                                let stack = self
                                    .aura_holders(tick.target)
                                    .into_iter()
                                    .find(|h| {
                                        h.spell_id == tick.spell_id && h.caster == tick.caster
                                    })
                                    .map_or(1, |h| h.stack);
                                self.spell_healing_bonus_taken(
                                    tick.target,
                                    &spell,
                                    tick.effect_index as usize,
                                    tick.amount as f32,
                                    true,
                                    stack,
                                    None,
                                )
                            }
                            None => tick.amount as f32,
                        };
                        if amount > 0.0 {
                            let amount = mangos_spells::ditheru(amount, self.spell_rng());
                            self.heal_unit(tick.target, amount);
                        }
                    }
                }
                // `Aura::PeriodicTick` PERIODIC_ENERGIZE
                // (`SpellAuras.cpp:6158`): gain capped at
                // max power. The periodic aura log packet
                // and 0.5x threat assist ride the same
                // deferral as periodic heal's here.
                aura_type::PERIODIC_ENERGIZE => {
                    if tick.amount <= 0 {
                        continue;
                    }
                    let Ok(power_type) = u32::try_from(tick.misc_value) else {
                        continue;
                    };
                    if power_type >= 5 {
                        continue;
                    }
                    let max_power = self.unit_max_power(tick.target, power_type);
                    if max_power == 0 {
                        continue;
                    }
                    let current = self.unit_power(tick.target, power_type);
                    #[allow(clippy::cast_sign_loss)]
                    let gain = (tick.amount as u32).min(max_power - current);
                    if gain > 0 {
                        self.unit_set_power(tick.target, power_type, current + gain);
                    }
                }
                aura_type::PERIODIC_DAMAGE_PERCENT => {
                    let max_health =
                        self.unit_value_u32(tick.target, offsets::UNIT_FIELD_MAXHEALTH);
                    #[allow(clippy::cast_possible_truncation)]
                    let amount =
                        (u64::from(max_health) * tick.amount.unsigned_abs() as u64 / 100) as i32;
                    let Some(spell) = self.stores.spells.get(tick.spell_id).cloned() else {
                        continue;
                    };
                    if self.unit_position(tick.caster).is_none() {
                        continue;
                    }
                    self.school_damage(
                        tick.caster,
                        &spell,
                        tick.target,
                        amount,
                        tick.effect_index,
                        Some(tick.spell_id),
                        None,
                        Some(&ProcRaiseCtx {
                            attacker_flags: proc_flag::DEAL_HARMFUL_PERIODIC,
                            victim_flags: proc_flag::TAKE_HARMFUL_PERIODIC,
                            can_trigger: true,
                            triggered_by_aura: false,
                            att_type: 0,
                            applied_mods: Vec::new(),
                        }),
                    );
                }
                aura_type::DUMMY => {
                    self.periodic_dummy_tick(&tick);
                }
                _ => {}
            }
            // `AuraScript::OnPeriodicTickEnd`
            // (fork `Aura::PeriodicTickEnd`).
            self.aura_script_on_periodic_tick_end(&tick);
        }
    }

    /// `Aura::PeriodicDummyTick` (`SpellAuras.cpp:
    /// 6260`): `AuraScript` `OnPeriodicDummy`
    /// dispatch first, then the fork's hardcoded
    /// family switch. Only ported arms run;
    /// the rest of the switch is recorded in
    /// TIER3-PROGRESS Slice 2 step 4.
    fn periodic_dummy_tick(&mut self, tick: &PeriodicTick) {
        self.aura_script_on_periodic_dummy(tick);
    }
}
