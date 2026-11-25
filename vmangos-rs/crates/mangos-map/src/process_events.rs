//! Fork process-event script host (`Script::pProcessEventId`,
//! Tier 3 Slice 2 step 6): `ScriptMgr::OnProcessEvent`
//! (`ScriptMgr.cpp:2008`) dispatch by `scripted_event_id`
//! name. Callers mirror the fork: GO goober/camera event
//! ids (source = player, target = gameobject). The taxi
//! waypoint-event caller (`WaypointMovementGenerator.cpp:449`)
//! is unreached — taxi flight has no node-event runtime in
//! Rust yet (recorded remainder). `true` suppresses the
//! DB `event_scripts` arm.

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;

use crate::content::{
    CreatureScriptAi, DREADSTEED_GO_PEDESTAL, GoAiState, IN_PROGRESS,
    ULD_ULDAMAN_ENCOUNTER_ARCHAEDAS, ULD_ULDAMAN_ENCOUNTER_STONE_KEEPERS, UROK_GO_CHALLENGE,
    ZG_DATA_GAHZRANKA, ZG_NOT_STARTED, ZG_TYPE_GAHZRANKA,
};
use crate::map::Map;
use crate::spells::{CastOptions, CastTarget};

/// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`).
const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;

#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::items_after_statements
)]
impl Map {
    /// `ScriptMgr::OnProcessEvent` dispatch.
    pub fn process_event(
        &mut self,
        event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        is_start: bool,
    ) -> bool {
        let Some(name) = self
            .stores
            .scripts
            .scripted_event_ids
            .get(&event_id)
            .cloned()
        else {
            return false;
        };
        match name.as_str() {
            "event_test_of_endurance" => {
                self.process_event_test_of_endurance(event_id, source, target, is_start)
            }
            "event_awaken_archaedas" => {
                self.process_event_awaken_archaedas(event_id, source, target, is_start)
            }
            "event_awaken_stone_keeper" => {
                self.process_event_awaken_stone_keeper(event_id, source, target, is_start)
            }
            "event_king_of_the_foulweald" => {
                self.process_event_king_of_the_foulweald(event_id, source, target, is_start)
            }
            "event_dreadsteed_ritual_start" => {
                self.process_event_dreadsteed_ritual_start(event_id, source, target, is_start)
            }
            "event_dreadsteed_ritual_second_part" => {
                self.process_event_dreadsteed_ritual_second_part(event_id, source, target, is_start)
            }
            "event_banner_destroyed" => {
                self.process_event_banner_destroyed(event_id, source, target, is_start)
            }
            "event_summon_gahzranka" => {
                self.process_event_summon_gahzranka(event_id, source, target, is_start)
            }
            "event_the_conterattack" => {
                self.process_event_the_conterattack(event_id, source, target, is_start)
            }
            "event_the_principle_source" => {
                self.process_event_the_principle_source(event_id, source, target, is_start)
            }
            _ => false,
        }
    }

    /// `ProcessEventId_event_test_of_endurance`
    /// (`thousand_needles.cpp:437`): from the Harpy
    /// Foodstuffs goober — if Grenka (4490) is not
    /// already within 100yd of the GO, summon her at
    /// `Harpies[4]` timed-or-dead 1 min and record the
    /// player on her AI. Null source/target, wrong
    /// types, and the already-present case all return
    /// `true` (fork).
    fn process_event_test_of_endurance(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        const NPC_GRENKA: u32 = 4490;
        let (Some(player), Some(go)) = (source, target) else {
            return true;
        };
        if !player.is_player() || !go.is_game_object() {
            return true;
        }
        if self
            .nearest_creature_with_entry(go, NPC_GRENKA, 100.0)
            .is_some()
        {
            return true;
        }
        if let Some(grenka) = self.summon_creature(
            NPC_GRENKA,
            Position::new(-5589.63, -1575.89, 11.75, 6.02),
            go,
            TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            60 * 1000,
        ) {
            if let Some(s) = self.grenka_state_mut(grenka) {
                s.player_guid = player;
            }
        }
        true
    }
    // ---- lane pe_events.rs ----

    /// `ProcessEventId_event_awaken_archaedas`
    /// (`eastern_kingdoms/uldaman/boss_archaedas.cpp:45-58`): altar GO event —
    /// non-player source or missing altar (target) both return `true` (fork).
    /// Otherwise flags `ULDAMAN_ENCOUNTER_ARCHAEDAS` (2) `IN_PROGRESS`.
    fn process_event_awaken_archaedas(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        let Some(player) = source else {
            return true;
        };
        if !player.is_player() {
            return true;
        }
        // Couldn't get Altar, block event (fork comment, cpp 51).
        if target.is_none() {
            return true;
        }
        // `((Player*)source)->GetInstanceData()` null outside instances;
        // `instance_set_data` covers that (returns false, no write).
        self.instance_set_data(ULD_ULDAMAN_ENCOUNTER_ARCHAEDAS, IN_PROGRESS);
        true
    }

    /// `ProcessEventId_event_awaken_stone_keeper`
    /// (`eastern_kingdoms/uldaman/uldaman.cpp:55-67`): same shape as
    /// `event_awaken_archaedas`; flags `ULDAMAN_ENCOUNTER_STONE_KEEPERS` (1)
    /// `IN_PROGRESS`. Null/non-player source and null target return `true`.
    fn process_event_awaken_stone_keeper(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        let Some(player) = source else {
            return true;
        };
        if !player.is_player() {
            return true;
        }
        if target.is_none() {
            return true;
        }
        self.instance_set_data(ULD_ULDAMAN_ENCOUNTER_STONE_KEEPERS, IN_PROGRESS);
        true
    }

    /// `ProcessEventId_event_king_of_the_foulweald`
    /// (`kalimdor/ashenvale/ashenvale.cpp:629-637`): null source/target
    /// returns `true`; otherwise `go_foulweald_totem_moundAI::EventStart`
    /// with the source guid. `EventStart`'s own bool is discarded — the fork
    /// always returns `true` ("otherwise it'll get the event in the DB").
    fn process_event_king_of_the_foulweald(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        let (Some(player), Some(go)) = (source, target) else {
            return true;
        };
        // Fork blind-casts target to `GameObject*`; a non-GO target has no
        // `AI()` to drive, so skip (still `true`).
        if !go.is_game_object() {
            return true;
        }
        let Some(mut state) = self.go_ai.remove(&go) else {
            return true;
        };
        if let GoAiState::FoulwealdTotemMound(s) = &mut state {
            // `go_ai_foulweald_totem_mound_event_start` (content.rs).
            let _ = self.go_ai_foulweald_totem_mound_event_start(go, s, player);
        }
        if self.gameobjects.contains_key(&go) {
            self.go_ai.insert(go, state);
        }
        true
    }

    /// `ProcessEventId_event_dreadsteed_ritual_start`
    /// (`kalimdor/feralas/dire_maul/dreadsteed_ritual.cpp:707-715`): null
    /// source/target returns `true`; otherwise
    /// `go_pedestal_of_immol_tharAI::EventStart` with the source (player)
    /// guid. Return value discarded — always `true` ("to always override
    /// what could be in DB").
    fn process_event_dreadsteed_ritual_start(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        let (Some(player), Some(go)) = (source, target) else {
            return true;
        };
        if !go.is_game_object() {
            return true;
        }
        let Some(mut state) = self.go_ai.remove(&go) else {
            return true;
        };
        if let GoAiState::PedestalOfImmolThar(s) = &mut state {
            // `go_ai_pedestal_event_start` (content.rs).
            let _ = self.go_ai_pedestal_event_start(player, go, s);
        }
        if self.gameobjects.contains_key(&go) {
            self.go_ai.insert(go, state);
        }
        true
    }

    /// `ProcessEventId_event_dreadsteed_ritual_second_part`
    /// (`kalimdor/feralas/dire_maul/dreadsteed_ritual.cpp:779-789`): null
    /// target returns `true`; otherwise the `GOBJ_PEDESTAL` (179701) nearest
    /// the target GO within 10.0yd gets `EventSecondPartStart`. Always `true`.
    fn process_event_dreadsteed_ritual_second_part(
        &mut self,
        _event_id: u32,
        _source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        let Some(go) = target else {
            return true;
        };
        if !go.is_game_object() {
            return true;
        }
        // `((GameObject*)target)->FindNearestGameObject(GOBJ_PEDESTAL, 10.0)`.
        let Some(pedestal) = self.nearest_gameobject_with_entry(go, DREADSTEED_GO_PEDESTAL, 10.0)
        else {
            return true;
        };
        let Some(mut state) = self.go_ai.remove(&pedestal) else {
            return true;
        };
        if let GoAiState::PedestalOfImmolThar(s) = &mut state {
            // `go_ai_pedestal_event_second_part_start` (content.rs).
            self.go_ai_pedestal_event_second_part_start(pedestal, s);
        }
        if self.gameobjects.contains_key(&pedestal) {
            self.go_ai.insert(pedestal, state);
        }
        true
    }

    /// `ProcessEventId_event_banner_destroyed`
    /// (`eastern_kingdoms/burning_steppes/blackrock_spire/boss_urok.cpp:298-318`,
    /// event 4777): null target goes through the source creature
    /// (`urokUnderlingAI::BannerDestroyed`, cpp 227-231: nearest
    /// `GO_CHALLENGE_UROK` (175584) within 40yd gets `EventBannerDestroyed`);
    /// non-null target that is a gameobject gets `EventBannerDestroyed`
    /// with the source guid directly. Always `true`.
    fn process_event_banner_destroyed(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        // Null-target arm: source must be a creature with underling AI.
        let Some(go) = target else {
            let Some(attacker) = source else {
                return true;
            };
            if !attacker.is_creature() {
                return true;
            }
            let is_underling = self.ai.get(&attacker).is_some_and(|ai| {
                matches!(
                    &ai.script,
                    Some(CreatureScriptAi::UrokEnforcer(_) | CreatureScriptAi::UrokOgreMagus(_))
                )
            });
            if !is_underling {
                return true;
            }
            // `m_creature->FindNearestGameObject(GO_CHALLENGE_UROK, 40)`.
            let Some(banner) =
                self.nearest_gameobject_with_entry(attacker, UROK_GO_CHALLENGE, 40.0)
            else {
                return true;
            };
            let Some(mut state) = self.go_ai.remove(&banner) else {
                return true;
            };
            if let GoAiState::UrokChallenge(s) = &mut state {
                // `go_ai_urok_challenge_banner_destroyed` (content.rs).
                self.go_ai_urok_challenge_banner_destroyed(banner, s);
            }
            if self.gameobjects.contains_key(&banner) {
                self.go_ai.insert(banner, state);
            }
            return true;
        };
        // `else if (ObjectGuid targetGuid = ...)` — non-null target that
        // `IsGameObject()`; the fork passes `source->GetGUID()` unchecked.
        if !go.is_game_object() {
            return true;
        }
        let Some(mut state) = self.go_ai.remove(&go) else {
            return true;
        };
        if let GoAiState::UrokChallenge(s) = &mut state {
            self.go_ai_urok_challenge_banner_destroyed(go, s);
        }
        if self.gameobjects.contains_key(&go) {
            self.go_ai.insert(go, state);
        }
        true
    }

    /// `ProcessEventId_event_summon_gahzranka`
    /// (`eastern_kingdoms/stranglethorn_vale/zulgurub/instance_zulgurub.cpp:497-523`):
    /// null source returns `true` (block); non-player source returns
    /// `false`; missing instance, already-started (`GetData(TYPE_GAHZRANKA=12)`
    /// `!= NOT_STARTED`), and missing Gahzranka creature all return `false`.
    /// Otherwise the player casts 12816 (triggered) on self, the instance
    /// records `IN_PROGRESS`, Gahzranka respawns, return `true`.
    fn process_event_summon_gahzranka(
        &mut self,
        _event_id: u32,
        source: Option<ObjectGuid>,
        _target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        // No target or source, block event (fork comment, cpp 499).
        let Some(player) = source else {
            return true;
        };
        // `pSource->ToPlayer()` null (fork cpp 503-505).
        if !player.is_player() {
            return false;
        }
        // `GetData(TYPE_GAHZRANKA) != NOT_STARTED` (cpp 510); a missing
        // instance reads as `None`, which also fails this gate (fork
        // `return false`, cpp 521).
        if self.instance_get_data(ZG_TYPE_GAHZRANKA) != Some(ZG_NOT_STARTED) {
            return false;
        }
        // `pPlayer->CastSpell(pPlayer, 12816, true)` (cpp 513): triggered,
        // origin-unknown spell so `skip_known`.
        let _ = self.cast_spell(
            player,
            12_816,
            CastTarget::Unit(player),
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
        // `m_pInstance->GetCreature(GetData64(DATA_GAHZRANKA=18))` null
        // (fork cpp 515).
        let Some(gahzranka) = self.instance_get_data64(ZG_DATA_GAHZRANKA) else {
            return false;
        };
        if !self.creatures.contains_key(&gahzranka) {
            return false;
        }
        self.instance_set_data(ZG_TYPE_GAHZRANKA, IN_PROGRESS);
        // `pCreature->Respawn()` (cpp 518): force re-materialization through
        // the normal spawn path (same shape as `uld_respawn_minion`).
        self.despawn_creature_now(gahzranka, 0);
        true
    }

    /// `event_the_conterattack` (`kalimdor/the_barrens/the_barrens.cpp:762-763`):
    /// fork registration is commented out and no `ProcessEventId` body was
    /// ever defined.
    fn process_event_the_conterattack(
        &mut self,
        _event_id: u32,
        _source: Option<ObjectGuid>,
        _target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        // UNMODELED: no fork handler body (registration commented out;
        // `ProcessEventId_event_the_conterattack` does not exist). `false`
        // so the DB `event_scripts` arm is not suppressed.
        false
    }

    /// `ProcessEventId_event_the_principle_source`
    /// (`kalimdor/the_barrens/the_barrens.cpp:705-728`, event 5246): a wrong
    /// `eventId`, null source, or non-player source all return `true`;
    /// otherwise summon the three `NPC_BURNING_BLADE_TOXICOLOGIST` (12319)
    /// at their fork coords (`TEMPSUMMON_TIMED_OR_DEAD_DESPAWN`,
    /// `2 * MINUTE * IN_MILLISECONDS` = 120000) and aggro each on the player.
    fn process_event_the_principle_source(
        &mut self,
        event_id: u32,
        source: Option<ObjectGuid>,
        _target: Option<ObjectGuid>,
        _is_start: bool,
    ) -> bool {
        // `EVENT_THE_PRINCIPLE_SOURCE` (the_barrens.cpp:687).
        if event_id != 5_246 {
            return true;
        }
        let Some(player) = source else {
            return true;
        };
        if !player.is_player() {
            return true;
        }
        // `Toxicologist[]` (cpp 698-702): (entry, x, y, z, o).
        const TOXICOLOGIST: [(u32, f32, f32, f32, f32); 3] = [
            (12_319, 331.52, -2_270.94, 242.21, 5.15),
            (12_319, 332.09, -2_291.26, 241.86, 1.05),
            (12_319, 345.97, -2_282.66, 241.77, 3.16),
        ];
        // `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`), kind 1.
        const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;
        for (entry, x, y, z, o) in TOXICOLOGIST {
            if let Some(toxicologist) = self.summon_creature(
                entry,
                Position::new(x, y, z, o),
                player,
                TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                120_000,
            ) {
                // `pToxicologist->AI()->AttackStart(pPlayer)` (cpp 724);
                // `false` falls through to the drive's chase sync, which is
                // the correct default for unported AI.
                let _ = self.creature_script_attack_start(toxicologist, player);
            }
        }
        true
    }
}
