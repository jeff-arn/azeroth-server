//! Fork dummy-effect script hosts (Tier 3 Slice 2 step 6):
//! `Script::pEffectDummyCreature` / `pEffectDummyGameObj`
//! / `pEffectAuraDummy` (`ScriptMgr::OnEffectDummy` /
//! `OnAuraDummy`, `ScriptMgr.cpp:2019-2047`). Dispatch is
//! by the TARGET's script name (creature template / GO
//! template / aura wearer creature), never the spell's.
//! Reachability: the creature arm is called at the tail
//! of `Spell::EffectDummy` (fork `SpellEffects.cpp:1447`)
//! and from periodic-dummy ticks on creature targets
//! (`SpellAuras.cpp:1520`); the aura arm at
//! `Aura::HandleAuraDummy` end for creature wearers
//! (`SpellAuras.cpp:2148`). The GO arm has no call site
//! yet — spell casting at gameobjects is unmodeled
//! wholesale in Rust (recorded remainder).

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;

use crate::map::Map;

#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::match_same_arms,
    clippy::match_single_binding
)]
impl Map {
    /// Creature `script_name` for dummy dispatch
    /// (`Creature::GetScriptId`).
    fn dummy_script_name(&self, target: ObjectGuid) -> Option<String> {
        self.creature(target).map(|c| c.info.script_name.clone())
    }

    /// `ScriptMgr::OnEffectDummy` (creature target):
    /// `true` when a script handles the dummy effect.
    pub(crate) fn effect_dummy_creature(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        match self.dummy_script_name(target).as_deref() {
            Some("LazyPeons") => self.peon_wake_up(caster, spell_id, eff_index, target),
            Some("npc_grark_lorkrub") => self.dm_capture_grark(caster, spell_id, eff_index, target),
            Some("boss_thermaplugg") => {
                self.dm_thermaplugg_bomb(caster, spell_id, eff_index, target)
            }
            Some("spell_anchor") => self.dm_gothik_anchor(caster, spell_id, eff_index, target),
            Some("npc_rabid_thistle_bear") => {
                self.dm_rabid_thistle_bear(caster, spell_id, eff_index, target)
            }
            Some("npc_shay_leafrunner") => {
                self.dm_shay_leafrunner(caster, spell_id, eff_index, target)
            }
            Some("npc_keeper_remulos") => self.dm_conjure_rift(caster, spell_id, eff_index, target),
            Some("npc_snufflenose_gopher") => {
                self.dm_snufflenose_gopher(caster, spell_id, eff_index, target)
            }
            _ => false,
        }
    }

    /// `ScriptMgr::OnEffectDummy` (gameobject target).
    /// No call site yet (GO spell targets unmodeled);
    /// kept so lane bodies land against a stable host.
    #[allow(dead_code)] // NO-CALLSITE-YET (step-6 remainder)
    pub(crate) fn effect_dummy_gameobject(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        match self
            .gameobject(target)
            .map(|g| g.info.script_name.clone())
            .as_deref()
        {
            Some("go_mark_of_detonation") => {
                self.dm_mark_of_detonation(caster, spell_id, eff_index, target)
            }
            _ => false,
        }
    }

    /// `ScriptMgr::OnAuraDummy`: per dummy-aura effect
    /// index, on real apply/remove for creature wearers.
    pub(crate) fn effect_aura_dummy(
        &mut self,
        target: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        apply: bool,
    ) -> bool {
        match self.dummy_script_name(target).as_deref() {
            Some("boss_viscidus") => {
                self.viscidus_freeze_aura_dummy(target, spell_id, eff_index, apply)
            }
            _ => false,
        }
    }

    /// `Aura::HandleAuraDummy` tail fan-out: for each
    /// `SPELL_AURA_DUMMY` effect of the holder spell on
    /// a creature wearer, run `OnAuraDummy` (fork makes
    /// one `Aura` per effect; Rust holders bundle).
    pub(crate) fn aura_dummy_for_spell(&mut self, victim: ObjectGuid, spell_id: u32, apply: bool) {
        if !victim.is_creature() {
            return;
        }
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        for (i, effect) in spell.effects.iter().enumerate() {
            if effect.apply_aura_name == mangos_spells::defines::aura::DUMMY {
                self.effect_aura_dummy(victim, spell_id, i, apply);
            }
        }
    }

    /// `peon_wake_up` (`durotar.cpp:202` area): Awaken
    /// Peon (19938) on a sleeping Lazy Peon (10556) with
    /// the sleep buff (17743) grants the player monster
    /// credit. The wake itself is the AI `SpellHit`.
    /// Always returns `true`.
    fn peon_wake_up(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        _eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_BUFF_SLEEP: u32 = 17743;
        const SPELL_AWAKEN_PEON: u32 = 19938;
        const LAZY_PEON_ENTRY: u32 = 10556;
        let entry = self.creature(target).map_or(0, |c| c.info.entry);
        if spell_id == SPELL_AWAKEN_PEON
            && entry == LAZY_PEON_ENTRY
            && self.unit_has_aura(target, SPELL_BUFF_SLEEP)
            && caster.is_player()
        {
            // `KilledMonsterCredit(entry, guid)`.
            self.quest_kill_credit_one(caster, target, LAZY_PEON_ENTRY);
        }
        true
    }

    /// `EffectAuraDummy_spell_aura_dummy_viscidus_freeze`
    /// (`boss_viscidus.cpp:548`): on removal of Viscidus
    /// Freeze (25937) effect index 1, reset the frozen
    /// phase. Always returns `true`.
    fn viscidus_freeze_aura_dummy(
        &mut self,
        target: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        apply: bool,
    ) -> bool {
        const SPELL_VISCIDUS_FREEZE: u32 = 25937;
        if spell_id == SPELL_VISCIDUS_FREEZE && eff_index == 1 && !apply {
            self.viscidus_reset_frozen_phase(target);
        }
        true
    }
    // ---- lane dm_dummy.rs ----

    /// `EffectDummyCreature_spell_capture_grark`
    /// (`burning_steppes.cpp:378`, script `npc_grark_lorkrub`):
    /// Awaken-grark capture (14250) effect 0 on a Grark below 25% health:
    /// emote, friendly faction, evade. Returns `true` above 25% health too
    /// (fork always returns `true` once spell+effect match), `false` otherwise.
    fn dm_capture_grark(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_CAPTURE_GRARK: u32 = 14250;
        const EMOTE_SUBMIT: i32 = 4918;
        const FACTION_GUESS: u32 = 85;
        let _ = caster;
        if spell_id != SPELL_CAPTURE_GRARK || eff_index != 0 {
            return false;
        }
        if self.creature_health_percent(target) > 25.0 {
            return true;
        }
        self.do_script_text(EMOTE_SUBMIT, target, None, -1);
        let _ = self.object_set_field(
            target,
            u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
            FACTION_GUESS,
        );
        // UNMODELED: SetFactionTemporary restore-respawn timer (TEMPFACTION_RESTORE_RESPAWN).
        self.enter_evade_mode(target);
        true
    }

    /// `EffectDummyCreature_spell_boss_thermaplugg`
    /// (`boss_thermaplugg.cpp:224`, script `boss_thermaplugg`):
    /// Bomb-activation spells (11511 / 11795) effect 0 activate one random
    /// bomb face (0..=5). Returns `false` on spell/effect mismatch (fork
    /// returns `true` even with no instance).
    fn dm_thermaplugg_bomb(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_ACTIVATE_BOMB_A: u32 = 11511;
        const SPELL_ACTIVATE_BOMB_B: u32 = 11795;
        const MAX_GNOME_FACES: i32 = 6;
        let _ = (caster, target);
        if (spell_id != SPELL_ACTIVATE_BOMB_A && spell_id != SPELL_ACTIVATE_BOMB_B)
            || eff_index != 0
        {
            return false;
        }
        let index = self.spell_rng().irand(0, MAX_GNOME_FACES - 1) as u8;
        self.gno_activate_bomb_face(index);
        true
    }

    /// `EffectDummyCreature_spell_anchor`
    /// (`boss_gothik.cpp:586`, script `spell_anchor`):
    /// Gothik anchor relay. Fork returns `true` on EVERY path (entry
    /// mismatch, missing instance, all spell arms, fallthrough).
    fn dm_gothik_anchor(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const NPC_SUB_BOSS_TRIGGER: u32 = 16137;
        const NPC_GOTHIK: u32 = 16060;
        const NPC_SPECT_TRAINEE: u32 = 16127;
        const NPC_SPECT_DEATH_KNIGTH: u32 = 16148;
        const NPC_SPECT_RIDER: u32 = 16150;
        const NPC_SPECT_HORSE: u32 = 16149;
        const SPELL_A_TO_ANCHOR_1: u32 = 27892;
        const SPELL_B_TO_ANCHOR_1: u32 = 27928;
        const SPELL_C_TO_ANCHOR_1: u32 = 27935;
        const SPELL_A_TO_ANCHOR_2: u32 = 27893;
        const SPELL_B_TO_ANCHOR_2: u32 = 27929;
        const SPELL_C_TO_ANCHOR_2: u32 = 27936;
        const SPELL_A_TO_SKULL: u32 = 27915;
        const SPELL_B_TO_SKULL: u32 = 27931;
        const SPELL_C_TO_SKULL: u32 = 27937;
        let _ = caster;
        let entry = self.creature(target).map_or(0, |c| c.info.entry);
        if eff_index != 0 || entry != NPC_SUB_BOSS_TRIGGER {
            return true;
        }
        match spell_id {
            SPELL_A_TO_ANCHOR_1 | SPELL_B_TO_ANCHOR_1 | SPELL_C_TO_ANCHOR_1 => {
                // UNMODELED: pInstance->GetClosestAnchorForGoth(target, false) anchor pick
                // plus triggered CastSpell of 27893 / 27929 / 27936 at that anchor.
                true
            }
            SPELL_A_TO_ANCHOR_2 | SPELL_B_TO_ANCHOR_2 | SPELL_C_TO_ANCHOR_2 => {
                // UNMODELED: pInstance->GetGothSummonPointCreatures random pick plus
                // triggered CastSpell of 27915 / 27931 / 27937 at the picked trigger.
                true
            }
            SPELL_A_TO_SKULL | SPELL_B_TO_SKULL | SPELL_C_TO_SKULL => {
                let mut npc_entry = NPC_SPECT_TRAINEE;
                if spell_id == SPELL_B_TO_SKULL {
                    npc_entry = NPC_SPECT_DEATH_KNIGTH;
                } else if spell_id == SPELL_C_TO_SKULL {
                    npc_entry = NPC_SPECT_RIDER;
                }
                // UNMODELED: pInstance->GetSingleCreatureFromStorage(NPC_GOTHIK);
                // nearest live Gothik stands in via the entry lookup.
                if let Some(goth) = self.find_creature_by_entry(NPC_GOTHIK) {
                    if let Some(pos) = self.object_position(target) {
                        self.gothik_summon_add(goth, npc_entry, pos.x, pos.y, pos.z, pos.o);
                        if npc_entry == NPC_SPECT_RIDER {
                            self.gothik_summon_add(
                                goth,
                                NPC_SPECT_HORSE,
                                pos.x,
                                pos.y,
                                pos.z,
                                pos.o,
                            );
                        }
                    }
                }
                true
            }
            _ => true,
        }
    }

    /// `EffectDummyCreature_npc_rabid_thistle_bear`
    /// (`darkshore.cpp:823`, script `npc_rabid_thistle_bear`):
    /// Trap-capture spell (9439) effect 0 from a player morphs the bear
    /// (11836), grants kill credit, and starts the follow. Returns `true`
    /// when handled, `false` otherwise.
    fn dm_rabid_thistle_bear(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_TRAPPED_BEAR: u32 = 9439;
        const NPC_CAPTURED_RABID_THISTLE_BEAR: u32 = 11836;
        if spell_id == SPELL_TRAPPED_BEAR && eff_index == 0 && caster.is_player() {
            // UNMODELED: pCreatureTarget->UpdateEntry(11836) (no Map equivalent).
            self.quest_kill_credit_one(caster, target, NPC_CAPTURED_RABID_THISTLE_BEAR);
            self.enter_evade_mode(target);
            // UNMODELED: npc_rabid_thistle_bearAI StartFollowing(pPlayer) with
            // 300s Captured_Timer (FollowerAI unmodeled).
            return true;
        }
        false
    }

    /// `EffectDummyCreature_npc_shay_leafrunner`
    /// (`feralas.cpp:226`, script `npc_shay_leafrunner`):
    /// Shay's Bell (11402) effect 0 resumes Shay's follow. Non-player
    /// casters return `true` without effect (fork). `false` otherwise.
    fn dm_shay_leafrunner(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_SHAYS_BELL: u32 = 11402;
        let _ = target;
        if spell_id == SPELL_SHAYS_BELL && eff_index == 0 {
            if !caster.is_player() {
                return true;
            }
            // UNMODELED: npc_shay_leafrunnerAI ResumeFollowing (FollowerAI unmodeled).
            return true;
        }
        false
    }

    /// `EffectDummyCreature_conjure_rift`
    /// (`moonglade.cpp:919`, script `npc_keeper_remulos`):
    /// Conjure Rift (25813) effect 0 summons Eranikus the Tyrant (15491)
    /// at (7881.72, -2651.23, 493.29, 0.40) for 600s. `true`/`false`
    /// exactly as fork.
    fn dm_conjure_rift(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_CONJURE_RIFT: u32 = 25813;
        const NPC_ERANIKUS_TYRANT: u32 = 15491;
        const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;
        let _ = target;
        if spell_id == SPELL_CONJURE_RIFT && eff_index == 0 {
            self.summon_creature(
                NPC_ERANIKUS_TYRANT,
                Position::new(7881.72, -2651.23, 493.29, 0.40),
                caster,
                TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                600_000,
            );
            return true;
        }
        false
    }

    /// `EffectDummyCreature_npc_snufflenose_gopher`
    /// (`razorfen_kraul.cpp:291`, script `npc_snufflenose_gopher`):
    /// Snufflenose Command (8283) effect 0 on the targeted gopher (4781):
    /// emote text plus follow-pause toggle / tuber search. Returns `true`
    /// when handled, `false` otherwise (fork returns `false` + cast-fail
    /// when the gopher is not the caster's target).
    fn dm_snufflenose_gopher(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_SNUFFLENOSE_COMMAND: u32 = 8283;
        const NPC_SNUFFLENOSE_GOPHER: u32 = 4781;
        const SAY_GOPHER_COMMAND: i32 = 1591;
        if spell_id != SPELL_SNUFFLENOSE_COMMAND || eff_index != 0 {
            return false;
        }
        let entry = self.creature(target).map_or(0, |c| c.info.entry);
        if entry == NPC_SNUFFLENOSE_GOPHER {
            // UNMODELED: pUnit->GetTargetGuid() selected-target check plus
            // SendPetCastFail(0x0A) + `false` return when the gopher is untargeted.
            self.do_script_text(SAY_GOPHER_COMMAND, target, Some(caster), -1);
            // UNMODELED: FollowerAI pause toggle (SetFollowPaused(false) clearing
            // m_bIsMovementActive/m_targetTuberGuid) vs DoFindNewTuber().
            return true;
        }
        true
    }

    /// `EffectDummyGameObj_go_mark_of_detonation`
    /// (`eastern_plaguelands.cpp:1692`, script `go_mark_of_detonation`):
    /// Placing Smokey's Explosives (19250) effect 0 from a player: kill
    /// credit + suicide for the nearby scourge-structure trigger (12247,
    /// 8.0 yd), respawn of its fire objects, GO despawn. `true`/`false`
    /// exactly as fork.
    fn dm_mark_of_detonation(
        &mut self,
        caster: ObjectGuid,
        spell_id: u32,
        eff_index: usize,
        target: ObjectGuid,
    ) -> bool {
        const SPELL_PLACING_SMOKEY_S_EXPLOSIVES: u32 = 19250;
        const TRIGGER_SCOURGE_STRUCTURE: u32 = 12247;
        if spell_id == SPELL_PLACING_SMOKEY_S_EXPLOSIVES && eff_index == 0 && caster.is_player() {
            if let Some(found) =
                self.nearest_creature_with_entry(target, TRIGGER_SCOURGE_STRUCTURE, 8.0)
            {
                let entry = self
                    .creature(found)
                    .map_or(TRIGGER_SCOURGE_STRUCTURE, |c| c.info.entry);
                self.quest_kill_credit_one(caster, found, entry);
                // Fork deals the trigger's exact current health (DIRECT_DAMAGE,
                // NORMAL school); overkill damage stands in (no health reader).
                self.deal_damage(found, found, u32::MAX, false);
                // UNMODELED: SCRIPT_COMMAND_RESPAWN_GAMEOBJECT fire-object map
                // (8 npc-guid keys x 10 go guids, 180s despawn delay).
            }
            self.despawn_gameobject_now(target);
            return true;
        }
        false
    }
}
