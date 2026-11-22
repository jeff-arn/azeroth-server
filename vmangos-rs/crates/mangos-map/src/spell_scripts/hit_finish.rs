//! Spell-script hit/finish-lane arms + dispatch (`OnHit`, `OnSuccessfulStart`,
//! `OnCast`, `OnSuccessfulDispel`, `OnSuccessfulFinish`, `OnAfterHit`).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `SpellScript::OnAfterHit` dispatch (fork
    /// `Spell.cpp` `DoAllEffectOnTarget`, after the
    /// effect pass for this unit target).
    /// `SpellScript::OnHit` dispatch (fork
    /// `Spell.cpp:1489/1558/1584`, per unit target
    /// with its miss info; gated on `m_canTrigger`
    /// by the caller).
    pub(crate) fn spell_script_on_hit(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        miss_none: bool,
    ) {
        if spell.script_name == "spell_priest_power_word_shield" {
            self.priest_power_word_shield_on_hit(caster, victim, miss_none);
        }
    }

    /// `SpellScript::OnSuccessfulStart` dispatch
    /// (fork `Spell::Prepare` success,
    /// `Spell.cpp:3585`, before `SendSpellStart`).
    /// The only live arm mutates the cast time
    /// (`spell_instant_cast`: `SetCastTime(0)`).
    pub(crate) fn spell_script_on_successful_start(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        cast_time_ms: &mut u32,
    ) {
        if spell.script_name == "spell_instant_cast" {
            self.instant_cast_on_successful_start(caster, spell, cast_time_ms);
        }
    }

    /// `SpellScript::OnCast` dispatch (fork
    /// `Spell::cast` after power/reagents,
    /// `Spell.cpp:3827`).
    pub(crate) fn spell_script_on_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) {
        if spell.script_name == "spell_elunes_candle" {
            self.elunes_candle_on_cast(caster, target, spell);
        }
    }

    /// `SpellScript::OnSuccessfulDispel` dispatch
    /// (fork `Spell::EffectDispel` success path,
    /// `SpellEffects.cpp:2521`).
    pub(crate) fn spell_script_on_successful_dispel(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
    ) {
        if spell.script_name == "spell_warlock_devour_magic" {
            self.warlock_devour_magic_on_successful_dispel(caster, spell.id, eff_index);
        }
    }

    /// `SpellScript::OnSuccessfulFinish` dispatch
    /// (fork `Spell::finish` success tail,
    /// `Spell.cpp:4519`; channeled spells fire it
    /// at channel end in the fork — only the
    /// immediate-finish path is wired here).
    pub(crate) fn spell_script_on_successful_finish(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) {
        match spell.script_name.as_str() {
            "spell_priest_holy_nova" => {
                self.priest_holy_nova_on_successful_finish(caster, spell.id);
            }
            "spell_cannibalize" => {
                self.cannibalize_on_successful_finish(
                    caster,
                    matches!(target, CastTarget::Unit(_)),
                );
            }
            "spell_wolfshead_helm" => self.wolfshead_helm_on_successful_finish(caster, spell),
            _ => {}
        }
    }

    pub(crate) fn spell_script_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
    ) {
        if spell.script_name == "spell_paladin_bubble" {
            self.paladin_bubble_on_after_hit(caster, spell, victim);
        }
        if spell.script_name == "spell_first_aid" {
            self.first_aid_on_after_hit(caster, victim, spell);
        }
    }

    // ===== spell_paladin_bubble (spell_paladin.cpp:196) spells: 642, 1020, 498, 5573, 1022, 5599, 10278

    /// `PaladinBubbleScript::OnAfterHit` (`spell_paladin.cpp:198-202`,
    /// `SPELL_FORBEARANCE = 25771`, fork line 190): the bubble target gains
    /// Forbearance via a triggered cast.
    fn paladin_bubble_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
    ) {
        let _ = spell;
        // `m_caster->CastSpell(GetUnitTarget(), SPELL_FORBEARANCE, true)`.
        self.cast_spell(
            caster,
            25_771,
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_warrior_intimidating_shout (spell_warrior.cpp:20) spells: 5246
    // OnCheckTarget seam: `Unit::CheckTarget` / `Spell::AddUnitTarget`
    // (ScriptMgr.h:155-156). Wired via `spell_script_on_check_target`;
    // `main_target` is `spell->m_targets.getUnitTarget()`.

    // ===== spell_warrior_execute_dummy (spell_warrior.cpp:83) spells: 5308, 20658, 20660, 20661, 20662

    /// `WarriorExecuteDummyScript::OnCast` (`spell_warrior.cpp:85-93`):
    /// forwards rage-scaled base points into 20647.
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    fn warrior_execute_dummy_on_cast(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        current_base_points0: i32,
    ) {
        // UNMODELED: rage read — `unit_power` (spells.rs) is private at
        // lane time; expose a `pub(crate)` seam for
        // `GetPower(POWER_RAGE)`.
        let rage = self.unit_power(caster, mangos_spells::defines::power::RAGE);
        #[allow(clippy::cast_precision_loss)]
        let rage_f = rage as f32;
        let base_points0 = current_base_points0
            + mangos_spells::entry::dither(
                rage_f * spell.effects[0].dmg_multiplier,
                self.spell_rng(),
            );
        // m_casterUnit->SetPower(POWER_RAGE, 0); // Done in spell 20647
        self.cast_custom_spell(caster, victim, 20647, [Some(base_points0), None, None]);
    }

    // ===== spell_warrior_wrath (spell_warrior.cpp:99) spells: 21977 (Warrior's Wrath)

    // ===== spell_warlock_devour_magic (spell_warlock.cpp:190) spells: 19505, 19731, 19734, 19736

    // NEEDS-HOST: OnSuccessfulDispel seam: fork `Spell::EffectDispel` success path
    // (`spell->m_casterUnit` + `m_spellInfo->Id` in scope).
    /// `WarlockDevourMagicScript::OnSuccessfulDispel`
    /// (`spell_warlock.cpp:192-217`): a dispelled debuff heals the felhunter.
    fn warlock_devour_magic_on_successful_dispel(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
    ) {
        if eff_index != 0 {
            return;
        }
        let heal_spell = match spell_id {
            19_505 => 19_658,
            19_731 => 19_732,
            19_734 => 19_733,
            19_736 => 19_735,
            // `sLog.Out(LOG_SCRIPTS, LOG_LVL_DEBUG, ...)`; no logger at this seam.
            _ => return,
        };
        self.cast_spell(
            caster,
            heal_spell,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_warlock_inferno (spell_warlock.cpp:223) spells: 1122, 24670
    // (Wired via `spell_script_on_summon`.)

    // ===== spell_priest_power_word_shield (spell_priest.cpp:60) spells: 17, 592, 600, 3747, 6065, 6066, 10898, 10899, 10900, 10901, 27607
    // Fork: `OnHit`: when the hit lands (`missInfo == SPELL_MISS_NONE`) on a unit
    // target, the caster applies Weakened Soul (6788) triggered on that target.
    // OnHit seam: fork calls it from `Spell::DoAllEffectOnTarget`
    // (`Spell.cpp:1489,1558,1584` for the unit case, `:1956,1970` for
    // gameobject targets). Wired via `spell_script_on_hit`.
    /// `PriestPowerWordShieldScript::OnHit` (`spell_priest.cpp:67-73`).
    /// `miss_none` mirrors `missInfo == SPELL_MISS_NONE`.
    fn priest_power_word_shield_on_hit(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        miss_none: bool,
    ) {
        const WEAKENED_SOUL: u32 = 6788;
        if !miss_none {
            return;
        }
        self.cast_spell(
            caster,
            WEAKENED_SOUL,
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
    }

    // ===== spell_priest_holy_nova (spell_priest.cpp:79) spells: 15237, 15430, 15431, 27799, 27800, 27801
    // Fork: `OnSuccessfulFinish`: fires the rank-matched heal sub-spell on the
    // original spell targets: 15237 -> 23455, 15430 -> 23458, 15431 -> 23459,
    // 27799 -> 27803, 27800 -> 27804, 27801 -> 27805. Unknown id logs and
    // returns; the fork builds a raw `new Spell(caster, entry, triggered,
    // originalCasterGuid)` and `prepare`s it with the original targets.
    // OnSuccessfulFinish seam: fork calls it from `Spell::finish`
    // (`Spell.cpp:4519`). Wired via `spell_script_on_successful_finish`.
    /// `PriestHolyNovaScript::OnSuccessfulFinish` (`spell_priest.cpp:81-118`).
    /// `caster` = `spell->m_casterUnit`; the heal must reuse the original
    /// targets (see UNMODELED note).
    fn priest_holy_nova_on_successful_finish(&mut self, caster: ObjectGuid, spell_id: u32) {
        let sub = match spell_id {
            15237 => 23455, // Holy Nova, rank 1
            15430 => 23458, // Holy Nova, rank 2
            15431 => 23459, // Holy Nova, rank 3
            27799 => 27803, // Holy Nova, rank 4
            27800 => 27804, // Holy Nova, rank 5
            27801 => 27805, // Holy Nova, rank 6
            _ => return,    // fork logs "assigned to unhandled spell id"
        };
        // Fork re-prepares a raw Spell on the
        // original targets; the sub-spells collect
        // friendlies around the caster themselves,
        // so one triggered self-cast re-derives the
        // same set (a target that left range between
        // the nova and here is the only divergence).
        self.cast_spell(
            caster,
            sub,
            CastTarget::Unit(caster),
            CastOptions {
                triggered: true,
                triggered_by_aura: false,
                skip_known: true,
                cast_item: None,
                custom_base_points: [None; 3],
                triggered_by_aura_spell: 0,
                triggered_by_spell: spell_id,
            },
        );
    }

    // ===== spell_opening_battleground_banner (spell_special.cpp:214) spells: 21651

    // OnSuccessfulStart seam (fork calls it on success during `Spell::Prepare`,
    // `Spell.cpp`): the `spell_script_on_successful_start` dispatch exists
    // but does not wire this arm (GO-target/lock-store runtime missing).
    #[allow(dead_code)] // NO-DISPATCHER-YET (step-4 remainder)
    #[allow(clippy::unused_self)] // UNMODELED body
    fn opening_battleground_banner_on_successful_start(&mut self, caster: ObjectGuid) {
        // Fork: with a valid GO target whose lock has `Index[1] ==
        // LOCKTYPE_SLOW_OPEN`, prepare a triggered visual spell 24390.
        // (Fork comment: `SPELL_EFFECT_OPEN_LOCK` can succeed with lockId 0,
        // so the lock check guards the visual.)
        // UNMODELED: GO targets (`m_targets.getGOTarget`), the lock store
        // (`sLockStore`), and `LOCKTYPE_SLOW_OPEN` have no Rust runtime;
        // the gated cast below is the reachable part.
        // self.cast_spell(caster, 24390, CastTarget::Unit(caster), CastOptions {
        //     triggered: true, triggered_by_aura: false, skip_known: true,
        //     cast_item: None, custom_base_points: [None; 3],
        //     triggered_by_aura_spell: 0, triggered_by_spell: 0,
        // });
        let _ = caster;
    }

    // ===== spell_cannibalize (spell_special.cpp:238) spells: 20577

    // ===== spell_cannibalize (spell_special.cpp:238) spells: 20577

    // OnSuccessfulFinish seam (fork calls it inside `Spell::finish`,
    // `Spell.cpp`; for channels only after a full channel). Wired via
    // `spell_script_on_successful_finish`.
    fn cannibalize_on_successful_finish(
        &mut self,
        caster: ObjectGuid,
        has_unit_or_corpse_target: bool,
    ) {
        // SPELL_CANNIBALIZE_AURA = 20578.
        if has_unit_or_corpse_target {
            self.cast_spell(
                caster,
                20578,
                CastTarget::Unit(caster),
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

    // ===== spell_cannibalize_aura (spell_special.cpp:255) spells: 20578

    // ===== spell_instant_cast (spell_item.cpp:430) spells: 4067, 13237
    // OnSuccessfulStart seam: fork `Spell::Prepare` success path
    // (`ScriptMgr.h:145`; `SetCastTime(0)` + `ReSetTimer`). Wired via
    // `spell_script_on_successful_start`.
    #[allow(clippy::unused_self)]
    fn instant_cast_on_successful_start(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        cast_time_ms: &mut u32,
    ) {
        let _ = (caster, spell);
        // Fork: `SetCastTime(0)` + `ReSetTimer` —
        // the Big Bronze Bomb / Goblin Mortar
        // fires instantly.
        *cast_time_ms = 0;
    }

    // ===== spell_digging_claw (spell_item.cpp:442) spells: 5166 (quest 868 Egg Hunt)

    // ===== spell_elunes_candle (spell_item.cpp:479) spells: 26374
    // OnCast seam: fork `Spell::cast` post-checks path
    // (`ScriptMgr.h:159`). Wired via `spell_script_on_cast`.
    fn elunes_candle_on_cast(
        &mut self,
        caster: ObjectGuid,
        target: CastTarget,
        spell: &SpellEntry,
    ) {
        // Omen spells for entry 15467.
        const OMEN: [u32; 5] = [26622, 26623, 26624, 26625, 26649];
        let _ = spell;
        let mut sub = 26636;
        // Fork keys off the unit target's entry:
        // 15467 (Omen) -> random OMEN spell,
        // 15466 -> 26624.
        if let CastTarget::Unit(victim) = target {
            match victim.entry() {
                15467 => sub = OMEN[self.spell_rng().irand(0, 4) as usize],
                15466 => sub = 26624,
                _ => {}
            }
        }
        self.cast_spell(
            caster,
            sub,
            target,
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_first_aid (spell_item.cpp:510) spells: 746, 1159, 3267, 3268, 7926, 7927, 10838, 10839, 18608, 18610, 20803, 23567, 23568, 23569, 23696, 24412, 24413, 24414, 30020
    // OnAfterHit seam: fork `DoAllEffectOnTarget` post damage/heal
    // path (`ScriptMgr.h:163`). Wired via `spell_script_on_after_hit`.
    fn first_aid_on_after_hit(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        spell: &SpellEntry,
    ) {
        let _ = spell;
        // Fork casts 11196 Recently Bandaged on the unit target, triggered with
        // `m_CastItem`; the effect seam carries no cast-item context here.
        // UNMODELED: `m_CastItem` forwarding (mapped to `None`).
        self.cast_spell(
            caster,
            11196, // SPELL_RECENTLY_BANDAGED
            CastTarget::Unit(victim),
            CastOptions {
                triggered: true,
                skip_known: true,
                ..CastOptions::default()
            },
        );
    }

    // ===== spell_wolfshead_helm (spell_item.cpp:527) spells: 17770
    // OnSuccessfulFinish seam: fork `Spell::finish` success path
    // (`ScriptMgr.h:147`). Wired via `spell_script_on_successful_finish`.
    fn wolfshead_helm_on_successful_finish(&mut self, caster: ObjectGuid, spell: &SpellEntry) {
        let _ = spell;
        // Fork guards on `m_casterUnit`; cast 29940 on self, triggered.
        if self.unit_base(caster).is_some() {
            self.cast_spell(
                caster,
                29940, // SPELL_WOLFSHEAD_HELM_ENERGY
                CastTarget::Unit(caster),
                CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..CastOptions::default()
                },
            );
        }
    }

    // ===== spell_kodo_kombobulator (spell_item.cpp:544) spells: 18153
}
