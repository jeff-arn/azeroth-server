//! Spell-script `OnCheckCast` arms + dispatch (cast-validation tail).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `SpellScript::OnCheckCast` dispatch: `Some(reason)` fails
    /// the cast with that `SpellCastResult` byte.
    pub(crate) fn spell_script_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        match spell.script_name.as_str() {
            "spell_paladin_holy_shock" => self.holy_shock_on_check_cast(caster, target),
            "spell_warlock_conflagrate" => {
                self.warlock_conflagrate_on_check_cast(caster, spell, target)
            }
            "spell_warlock_life_tap" => self.warlock_life_tap_on_check_cast(caster, spell, target),
            "spell_warlock_create_healthstone" => {
                self.warlock_create_healthstone_on_check_cast(caster, spell, target)
            }
            "spell_warlock_ritual_of_summoning" => {
                self.warlock_ritual_of_summoning_on_check_cast(caster, spell, target)
            }
            "spell_hunter_mongoose_bite" => self.hunter_mongoose_bite_on_check_cast(caster, target),
            "spell_hunter_counterattack" => self.hunter_counterattack_on_check_cast(caster, target),
            "spell_druid_swiftmend" => self.druid_swiftmend_on_check_cast(caster, target),
            "spell_heavy_armor_kit" => self.heavy_armor_kit_on_check_cast(caster, target),
            "spell_reindeer_transformation" => {
                self.reindeer_transformation_on_check_cast(caster, target)
            }
            "spell_bag_of_gold" => self.bag_of_gold_on_check_cast(caster, target),
            "spell_kodo_kombobulator" => self.kodo_kombobulator_on_check_cast(caster, target),
            "spell_melodius_rapture" => self.melodius_rapture_on_check_cast(caster, target),
            "spell_purify_and_place_food" => {
                self.purify_and_place_food_on_check_cast(caster, target)
            }
            "spell_summon_black_qiraji_battle_tank" => {
                self.summon_black_qiraji_battle_tank_on_check_cast(caster, target)
            }
            _ => None,
        }
    }

    /// `PaladinHolyShockScript::OnCheckCast`
    /// (`spell_paladin.cpp:79-85`): hostile target must be in
    /// front of the caster.
    fn holy_shock_on_check_cast(&mut self, caster: ObjectGuid, target: CastTarget) -> Option<u8> {
        let unit_target = Self::script_unit_target(target)?;
        if !Self::script_is_friendly(caster, unit_target)
            && !self.is_facing_target(caster, unit_target)
        {
            return Some(mangos_spells::defines::cast_result::FAILED_UNIT_NOT_INFRONT);
        }
        None
    }

    // ===== spell_warlock_conflagrate (spell_warlock.cpp:56) spells: 17962, 18930, 18931, 18932

    /// `WarlockConflagrateScript::OnCheckCast` (`spell_warlock.cpp:58-80`):
    /// requires the caster's own Immolate on the target.
    fn warlock_conflagrate_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = spell;
        let Some(unit_target) = Self::script_unit_target(target) else {
            return Some(mangos_spells::defines::cast_result::FAILED_BAD_IMPLICIT_TARGETS);
        };
        // for caster applied auras only
        if self
            .warlock_conflagrate_immolate_id(caster, unit_target)
            .is_none()
        {
            // Host cast-result constant (5875 table).
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        }
        None
    }

    // ===== spell_warlock_life_tap (spell_warlock.cpp:105) spells: 1454, 1455, 1456, 11687, 11688, 11689

    /// `WarlockLifeTapScript::OnCheckCast` (`spell_warlock.cpp:107-122`):
    /// fizzles when the health cost would kill the caster.
    fn warlock_life_tap_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // UNMODELED: `ApplySpellMod(SPELLMOD_COST)` (no spell-mod owner access in
        // lane scope) and `SpellDamageBonusDone/Taken` (identity in this slice,
        // `mangos-spells/src/entry.rs`), so cost == base points.
        #[allow(clippy::cast_precision_loss)]
        let cost = spell.effects[0].base_points as f32;
        let health = self
            .unit_base(caster)
            .map(mangos_entities::unit::UnitBase::health)?;
        // use ceil as dithering might round up later.
        if i64::from(health) <= cost.ceil() as i64 {
            // Host cast-result constant (5875 table).
            return Some(mangos_spells::defines::cast_result::FAILED_FIZZLE);
        }
        None
    }

    // ===== spell_warlock_create_healthstone (spell_warlock.cpp:245) spells: 5699, 6201, 6202, 11729, 11730

    /// `WarlockCreateHealthstoneScript::OnCheckCast` (`spell_warlock.cpp:306-325`).
    /// NOTE fork quirk (`spell_warlock.cpp:309-310`): `if (!pCaster) SPELL_CAST_OK;`
    /// is a discarded expression, not a return — a null caster falls through.
    #[allow(clippy::unused_self)] // UNMODELED gates
    fn warlock_create_healthstone_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, spell, target);
        // UNMODELED: player-only gate (`ToPlayer`), `CanStoreNewItem` bag-space
        // check, `SendEquipError` + `SPELL_FAILED_DONT_REPORT` (fork byte 27, no
        // host constant). Rank lookup itself is modeled in the helper above.
        None
    }

    // ===== spell_warlock_ritual_of_summoning (spell_warlock.cpp:348) spells: 698

    /// `WarlockRitualOfSummoningScript::OnCheckCast` (`spell_warlock.cpp:349-376`).
    #[allow(clippy::unused_self)] // UNMODELED gates
    fn warlock_ritual_of_summoning_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, spell, target);
        // UNMODELED (no player/raid/map seams in lane scope): caster must be a
        // player (`ToPlayer`, else fork byte 10 BAD_TARGETS); must have a
        // selection (`GetSelectionGuid`, else 10); selection must be a different
        // player in the same raid (`IsInSameRaidWith`, else 10); target in combat
        // (fork byte 122, no host constant); dungeon maps require the same map
        // (fork byte 141, no host constant); battleground caster fails (fork byte
        // 61 NOT_HERE, no host constant).
        None
    }

    // ===== spell_warlock_curse_of_idiocy (spell_warlock.cpp:382) spells: 1010
    // (Wired via `aura_script_on_periodic_trigger`.)

    // ===== spell_hunter_mongoose_bite (spell_hunter.cpp:96) spells: 1495, 14269, 14270, 14271
    // Fork: `OnCheckCast`: fails with `SPELL_FAILED_BAD_TARGETS` unless the unit
    // target is the caster's current `REACTIVE_DEFENSE` target (dodge proc).
    // (in `spell_script_on_check_cast` match on `spell.script_name.as_str()`;
    // `caster`, `target` in scope):
    /// `HunterMongooseBiteScript::OnCheckCast` (`spell_hunter.cpp:99-104`).
    #[allow(clippy::unused_self)] // UNMODELED reactive-target gate
    fn hunter_mongoose_bite_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: `GetReactiveTarget(REACTIVE_DEFENSE)` — no reactive
        // target store in the lane yet, so the fork's BAD_TARGETS gate cannot
        // be evaluated; returning None (allow) until it exists.
        None
    }

    // ===== spell_hunter_counterattack (spell_hunter.cpp:109) spells: 19306, 20909, 20910
    // Fork: `OnCheckCast`: fails with `SPELL_FAILED_BAD_TARGETS` unless the unit
    // target is the caster's current `REACTIVE_HUNTER_PARRY` target (parry proc).
    /// `HunterCounterAttackScript::OnCheckCast` (`spell_hunter.cpp:112-116`).
    #[allow(clippy::unused_self)] // UNMODELED reactive-target gate
    fn hunter_counterattack_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: `GetReactiveTarget(REACTIVE_HUNTER_PARRY)` — no reactive
        // target store in the lane yet; returning None (allow) until it exists.
        None
    }

    // ===== spell_druid_swiftmend (spell_druid.cpp:96) spells: 18562
    // Fork (`#if SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_10_2`): `OnCheckCast`
    // fails with `SPELL_FAILED_TARGET_AURASTATE` unless the unit target carries a
    // `SPELL_AURA_PERIODIC_HEAL` aura of `SPELLFAMILY_DRUID` with family mask
    // `0x50` (Rejuvenation / Regrowth). `OnEffectExecute` eff 0 with a unit
    // target: picks the shortest-duration Rejuvenation-or-Regrowth periodic-heal
    // aura on the target (family `CF_DRUID_REJUVENATION` / `CF_DRUID_REGROWTH`),
    // counts its ticks (Regrowth 6 ticks per "18 sec", Rejuvenation 4 per
    // "12 sec"), consumes the aura, and adds `tickheal * tickcount` to
    // `spell->damage` (rides the `script_damage` channel). Missing aura after a
    // passing check logs in the fork and returns false.
    // if spell.script_name == "spell_druid_swiftmend" {
    //     return self.druid_swiftmend_on_effect_execute(
    //         caster, spell, victim, eff_index, script_damage,
    //     );
    /// `DruidSwiftmendScript::OnCheckCast` (`spell_druid.cpp:98-106`).
    /// `target` is the raw `SpellTargets` unit target (may be absent).
    fn druid_swiftmend_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = caster;
        let CastTarget::Unit(victim) = target else {
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        };
        // Fork: `target->GetAura(SPELL_AURA_PERIODIC_HEAL, SPELLFAMILY_DRUID,
        // uint64(0x50))` — a periodic-heal effect on a druid-family aura whose
        // affect mask covers Rejuvenation (bit 4, 0x10) / Regrowth (bit 6,
        // 0x40); `SpellClassMask.h:172,174`.
        if self.unit_has_family_heal(victim) {
            None
        } else {
            Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE)
        }
    }

    /// Borrow-safe two-phase scan: collect candidate holder spell ids under a
    /// short shared borrow, then resolve family data from the stores.
    fn unit_has_family_heal(&self, victim: ObjectGuid) -> bool {
        let mut periodic_heal_holders: [u32; 48] = [0; 48];
        let mut count: usize = 0;
        if let Some(unit) = self.unit_base(victim) {
            for holder in &unit.auras {
                if holder
                    .effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == mangos_spells::defines::aura::PERIODIC_HEAL)
                    && count < periodic_heal_holders.len()
                {
                    periodic_heal_holders[count] = holder.spell_id;
                    count += 1;
                }
            }
        }
        periodic_heal_holders[..count].iter().any(|id| {
            self.stores.spells.get(*id).is_some_and(|entry| {
                entry.spell_family_name == 7 // SPELLFAMILY_DRUID
                    && (entry.spell_family_flags & 0x50) != 0
            })
        })
    }

    // ===== spell_heavy_armor_kit (spell_item.cpp:20) spells: 2833

    /// `HeavyArmorKitScript::OnCheckCast` (`spell_item.cpp:22-28`): target
    /// item must be at least item level 15.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn heavy_armor_kit_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: the check reads `spell->m_targets.getItemTarget()` and
        // `ItemProto::ItemLevel`; `CastTarget` carries no item target and item
        // protos are not in the lane runtime. Fork returns
        // `SPELL_FAILED_LOWLEVEL` (43) when `ItemLevel < 15`.
        None
    }

    // ===== spell_deviate_fish (spell_item.cpp:34) spells: 8063
    // (insert before the final `else` in `spell_script_on_effect_execute`)

    // ===== spell_reindeer_transformation (spell_item.cpp:402) spells: 25860

    /// `ReindeerTransformationScript::OnCheckCast` (`spell_item.cpp:404-410`,
    /// Fresh/Preserved Holly): requires the caster to be mounted.
    fn reindeer_transformation_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // Fork fails when `!m_casterUnit || !HasAuraType(SPELL_AURA_MOUNTED)`.
        // `defines::aura::MOUNTED` name to be verified against
        // `mangos-spells/src/defines.rs` at integration time.
        let mounted = self.unit_base(caster).is_some_and(|u| {
            u.auras.iter().any(|h| {
                h.effects
                    .iter()
                    .flatten()
                    .any(|e| e.aura_type == mangos_spells::defines::aura::MOUNTED)
            })
        });
        if !mounted {
            return Some(83); // SPELL_FAILED_ONLY_MOUNTED
        }
        None
    }

    // ===== spell_bag_of_gold (spell_item.cpp:415) spells: 25720 (quest 8606 Decoy!)

    // ===== spell_bag_of_gold (spell_item.cpp:415) spells: 25720 (quest 8606 Decoy!)

    /// `BagOfGoldScript::OnCheckCast` (`spell_item.cpp:417-424`): the (affective)
    /// caster must wear Narain's Turban (aura 25688).
    fn bag_of_gold_on_check_cast(&mut self, caster: ObjectGuid, target: CastTarget) -> Option<u8> {
        let _ = target;
        // Fork checks the affective caster (charm owner) as a player; the lane
        // has no charm-owner or player/creature split, so the caster is checked.
        let has_turban = self
            .unit_base(caster)
            .is_some_and(|u| u.auras.iter().any(|h| h.spell_id == 25688));
        if !has_turban {
            return Some(mangos_spells::defines::cast_result::FAILED_TARGET_AURASTATE);
        }
        None
    }

    // ===== spell_kodo_kombobulator (spell_item.cpp:544) spells: 18153

    /// `KodoKombobulatorScript::OnCheckCast` (`spell_item.cpp:546-552`): refuses
    /// while the quest-credit aura (18172) is still present.
    fn kodo_kombobulator_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = target;
        // Fork only fails when a caster unit exists AND carries the credit aura;
        // a missing unit passes (the `&&` short-circuits), matched by `is_some_and`.
        let has_credit = self.unit_base(caster).is_some_and(|u| {
            u.auras.iter().any(|h| h.spell_id == 18172) // SPELL_KODO_KOMBOBULATOR_QEUST_CREDIT
        });
        if has_credit {
            return Some(40); // SPELL_FAILED_ITEM_NOT_READY
        }
        None
    }

    // ===== spell_melodius_rapture (spell_item.cpp:563) spells: 21050 (Rat Catcher's Flute)

    // ===== spell_melodius_rapture (spell_item.cpp:563) spells: 21050 (Rat Catcher's Flute)

    /// `MelodiusRaptureScript::OnCheckCast` (`spell_item.cpp:565-571`): the unit
    /// target must be a Deeprun Rat (entry 13016).
    #[allow(clippy::unused_self)] // UNMODELED body
    fn melodius_rapture_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = caster;
        // Reachable part: fork fails with `SPELL_FAILED_BAD_TARGETS` when there is
        // no unit target at all.
        let Some(_victim) = Self::script_unit_target(target) else {
            return Some(mangos_spells::defines::cast_result::FAILED_BAD_TARGETS);
        };
        // UNMODELED: fork additionally requires `GetEntry() == 13016`
        // (NPC_DEEPRUN_RAT); target entry lookup is not in the lane runtime, so a
        // wrong-entry unit target currently passes.
        None
    }

    // ===== spell_purify_and_place_food (spell_item.cpp:577) spells: 16072 (Filled Cleansing Bowl)

    // ===== spell_purify_and_place_food (spell_item.cpp:577) spells: 16072 (Filled Cleansing Bowl)

    /// `PurifyAndPlaceFoodScript::OnCheckCast` (`spell_item.cpp:579-587`): quest
    /// The Blackwood Corrupted — refuse while event 3938 already runs.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn purify_and_place_food_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // `spell->m_spellInfo->Id == 16072` is readable via `spell.id` at the arm
        // site; the map-event gate below is not in the lane runtime.
        // UNMODELED: `GetMap()->GetScriptedMapEvent(3938)` -> `SPELL_FAILED_NOT_READY`.
        None
    }

    // ===== spell_summon_black_qiraji_battle_tank (spell_item.cpp:592) spells: 26656

    // ===== spell_summon_black_qiraji_battle_tank (spell_item.cpp:592) spells: 26656

    /// `SummonBlackQirajiBattleTankScript::OnCheckCast` (`spell_item.cpp:594-621`,
    /// Black Qiraji Resonating Crystal): mount-only rules, dismounting first.
    #[allow(clippy::unused_self)] // UNMODELED body
    fn summon_black_qiraji_battle_tank_on_check_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
    ) -> Option<u8> {
        let _ = (caster, target);
        // UNMODELED: the whole body keys off state with no lane API —
        // `IsMounted()` (true branch also calls `RemoveSpellsCausingAura(MOUNTED)`
        // then returns `SPELL_FAILED_DONT_REPORT` (23)); `IsInWater()` (+
        // `IsPlayer`/`IsInHighLiquid`) -> `SPELL_FAILED_ONLY_ABOVEWATER` (80);
        // `GetTransport()` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `GetMapId() != MAP_AHN_QIRAJ_TEMPLE && !IsMountAllowed() &&
        // !IsTriggered()` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `GetAreaId() == 35` -> `SPELL_FAILED_NO_MOUNTS_ALLOWED`;
        // `IsInDisallowedMountForm()` -> `SPELL_FAILED_NOT_SHAPESHIFT` (61).
        // Only the mount-presence half is lane-readable (aura-type scan, as in
        // `reindeer_transformation_on_check_cast`), but the dismount side effect
        // has no lane API, so even that is left to the host wiring.
        None
    }
}
