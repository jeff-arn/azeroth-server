//! Fork C++ areatrigger script host (`Script::pAreaTrigger`,
//! Tier 3 Slice 2 step 6): dispatch by the trigger's
//! `script_name` (`ScriptMgr::OnAreaTrigger`,
//! `ScriptMgr.cpp:1998`) from the session-side
//! `HandleAreaTriggerOpcode` gate. The bool mirrors the
//! fork: `true` suppresses the DB `areatrigger_scripts`
//! arm in `Map::StartAreaTriggerScript`; quest/tavern/
//! teleport processing in the session handler continues
//! regardless (several fork scripts act and still return
//! `false`).

use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;

use crate::content::{DONE, IN_PROGRESS};
use mangos_movement::movement_info::Position;

use crate::map::Map;

/// `PLAYER_EXTRA_GM_ON` (`AbstractPlayer.h:12`).
const PLAYER_EXTRA_GM_ON: u32 = 0x0001;
/// `TEMPSUMMON_TIMED_OR_DEAD_DESPAWN` (`ObjectDefines.h:61`).
const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;

#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::approx_constant // fork coordinate literals
)]
impl Map {
    /// `Player::IsGameMaster` (`Player.h:804`).
    fn player_is_game_master(&self, player: ObjectGuid) -> bool {
        self.player(player)
            .is_some_and(|p| p.extra_flags & PLAYER_EXTRA_GM_ON != 0)
    }

    /// `ScriptMgr::OnAreaTrigger` dispatch: `true` when a
    /// script handles the trigger (DB script suppressed).
    pub fn area_trigger_script(
        &mut self,
        player: ObjectGuid,
        trigger_id: u32,
        script_name: &str,
    ) -> bool {
        match script_name {
            "at_murloc_camp" => self.at_murloc_camp(player, trigger_id),
            "at_ring_of_law" => self.at_ring_of_law(player, trigger_id),
            "at_shadowforge_bridge" => self.at_shadowforge_bridge(player, trigger_id),
            "at_blackrock_spire" => self.at_blackrock_spire(player, trigger_id),
            "at_ubrs_the_beast" => self.at_ubrs_the_beast(player, trigger_id),
            "at_orb_of_command" => self.at_orb_of_command(player, trigger_id),
            "at_enter_vael_room" => self.at_enter_vael_room(player, trigger_id),
            "at_naxxramas" => self.at_naxxramas(player, trigger_id),
            "at_shade_of_eranikus" => self.at_shade_of_eranikus(player, trigger_id),
            "at_cathedral_entrance" => self.at_cathedral_entrance(player, trigger_id),
            "at_dmf_chest_dm" => self.at_dmf_chest_dm(player, trigger_id),
            "at_temple_ahnqiraj" => self.at_temple_ahnqiraj(player, trigger_id),
            "at_zumrah" => self.at_zumrah(player, trigger_id),
            "at_antusul" => self.at_antusul(player, trigger_id),
            "at_dmf_chest_wc" => self.at_dmf_chest_wc(player, trigger_id),
            "at_twilight_grove" => self.at_twilight_grove(player, trigger_id),
            "at_huldar_miran" => self.at_huldar_miran(player, trigger_id),
            "at_stormwind_gates" => self.at_stormwind_gates(player, trigger_id),
            "at_sentry_point" => self.at_sentry_point(player, trigger_id),
            "at_irontree_wood" => self.at_irontree_wood(player, trigger_id),
            "at_twiggy_flathead" => self.at_twiggy_flathead(player, trigger_id),
            "at_scent_larkorwi" => self.at_scent_larkorwi(player, trigger_id),
            "at_childrens_week_spot" => self.at_childrens_week_spot(player, trigger_id),
            "at_ravenholdt" => self.at_ravenholdt(player, trigger_id),
            _ => false,
        }
    }

    /// `at_murloc_camp` (`darkshore.cpp:1134-1158`): on the
    /// Murkdeep trigger (1966), a living non-GM player with
    /// quest 4740 incomplete summons Murkdeep (10323) at the
    /// first summon point facing it, timed-or-dead 30 min,
    /// and starts his event — unless he is already alive
    /// within visibility distance. Always returns `false`
    /// (fork falls through to DB scripts / teleport arms).
    fn at_murloc_camp(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_MURKDEEP: u32 = 1966;
        const NPC_MURKDEEP: u32 = 10323;
        const QUEST_WANTED_MURKDEEP: u32 = 4740;
        // `m_fSummonPoints[0]` (cpp 936); points 1/2 are
        // the AI's add spawns.
        const SUMMON_X: f32 = 4984.772;
        const SUMMON_Y: f32 = 596.975;
        const SUMMON_Z: f32 = -1.172;
        if trigger_id != AREATRIGGER_MURKDEEP {
            return false;
        }
        // Fork: `IsGameMaster() || IsDead()`.
        if self.player_is_game_master(player) || !self.unit_is_alive(player) {
            return false;
        }
        if !self.player_quest_incomplete(player, QUEST_WANTED_MURKDEEP) {
            return false;
        }
        // `GetClosestCreatureWithEntry(..., DEFAULT_VISIBILITY_DISTANCE)`
        // + alive check (the finder already filters alive).
        if self
            .nearest_creature_with_entry(player, NPC_MURKDEEP, self.config.visibility_distance)
            .is_some()
        {
            return false;
        }
        // `pPlayer->GetAngle(x, y)`: facing from the player
        // toward the summon point.
        let orientation = self.object_position(player).map_or(0.0, |pp| {
            let angle = (SUMMON_Y - pp.y).atan2(SUMMON_X - pp.x);
            if angle < 0.0 {
                angle + 2.0 * std::f32::consts::PI
            } else {
                angle
            }
        });
        if let Some(murkdeep) = self.summon_creature(
            NPC_MURKDEEP,
            Position {
                x: SUMMON_X,
                y: SUMMON_Y,
                z: SUMMON_Z,
                o: orientation,
            },
            player,
            TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
            30 * 60 * 1000,
        ) {
            self.murkdeep_begin_event(murkdeep, player);
        }
        false
    }

    // ---- lane at_instances.rs ----

    /// `AreaTrigger_at_ring_of_law`
    /// (`blackrock_depths.cpp:551-574`): while the ring event is idle,
    /// start it and summon Grimstone; while it runs, route the player
    /// into the arena AI. Always returns `false`.
    fn at_ring_of_law(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const TYPE_RING_OF_LAW: u32 = 0;
        const NPC_GRIMSTONE: u32 = 10_096;
        const TEMPSUMMON_DEAD_DESPAWN: u32 = 7;
        let _ = trigger_id;
        // Fork gates the whole body on `GetInstanceData`; a missing
        // BRD state stands in for a missing instance.
        if self.brd_get_data(TYPE_RING_OF_LAW).is_none() {
            return false;
        }
        if self.brd_get_data(TYPE_RING_OF_LAW) == Some(IN_PROGRESS)
            || self.brd_get_data(TYPE_RING_OF_LAW) == Some(DONE)
        {
            if let Some(grimstone) = self.brd_get_data64(NPC_GRIMSTONE) {
                // UNMODELED: npc_grimstoneAI::PlayerEnteredArena (no Map helper).
                let _ = grimstone;
                let _ = player;
            }
            return false;
        }
        self.brd_set_data(TYPE_RING_OF_LAW, IN_PROGRESS);
        let _ = self.summon_creature(
            NPC_GRIMSTONE,
            Position::new(625.559, -205.618, -52.735, 2.609),
            player,
            TEMPSUMMON_DEAD_DESPAWN,
            0,
        );
        false
    }

    /// `AreaTrigger_at_shadowforge_bridge`
    /// (`blackrock_depths.cpp:1455-1484`): on the first crossing by a
    /// living non-GM player, summon the two Anvilrage guards and latch
    /// the bridge event DONE. Always returns `false`.
    fn at_shadowforge_bridge(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const TYPE_BRIDGE: u32 = 33;
        const NPC_ANVILRAGE_GUARDMAN: u32 = 8891;
        const SAY_GUARD_AGGRO: i32 = 5271;
        const TEMPSUMMON_DEAD_DESPAWN: u32 = 7;
        let _ = trigger_id;
        // Fork gates the whole body on `GetInstanceData`.
        if self.brd_get_data(TYPE_BRIDGE).is_none() {
            return false;
        }
        if self.player_is_game_master(player)
            || !self.unit_is_alive(player)
            || self.brd_get_data(TYPE_BRIDGE) == Some(DONE)
        {
            return false;
        }
        if let Some(master) = self.summon_creature(
            NPC_ANVILRAGE_GUARDMAN,
            Position::new(642.3660, -274.5155, -43.10918, 0.4712389),
            player,
            TEMPSUMMON_DEAD_DESPAWN,
            0,
        ) {
            // UNMODELED: SetWalk(false) + MoveWaypoint + GetContactPoint /
            // MovePoint(1) (no movement model).
            self.do_script_text(SAY_GUARD_AGGRO, master, None, -1);
            if let Some(slave) = self.summon_creature(
                NPC_ANVILRAGE_GUARDMAN,
                Position::new(740.1137, -283.3448, -42.75082, 2.86234),
                player,
                TEMPSUMMON_DEAD_DESPAWN,
                0,
            ) {
                // UNMODELED: MoveFollow(master, 2.0) (no movement model).
                let _ = slave;
            }
        }
        self.brd_set_data(TYPE_BRIDGE, DONE);
        false
    }

    /// `AreaTrigger_at_blackrock_spire`
    /// (`instance_blackrock_spire.cpp:814-852`): UBRS seal-door arm
    /// (trigger 2046) and stadium-event arm (trigger 2026). Always
    /// returns `false`.
    fn at_blackrock_spire(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_ENTER_UBRS: u32 = 2046;
        const AREATRIGGER_STADIUM: u32 = 2026;
        const TYPE_STADIUM: u32 = 3;
        const TYPE_EVENT_DOOR_UBRS: u32 = 5;
        const ITEM_SEAL_OF_ASCENSION: u32 = 12_344;
        match trigger_id {
            AREATRIGGER_ENTER_UBRS => {
                self.brs_sort_room_event_mobs();
                // `HasItemCount(12344, 1)` over bag inventory rows.
                let has_seal = self.player(player).is_some_and(|p| {
                    p.saved
                        .inventory
                        .iter()
                        .filter(|r| r.item.item_id == ITEM_SEAL_OF_ASCENSION)
                        .map(|r| r.item.count)
                        .sum::<u32>()
                        >= 1
                });
                if has_seal && self.brs_encounter(TYPE_EVENT_DOOR_UBRS as usize) != DONE {
                    self.brs_set_data(TYPE_EVENT_DOOR_UBRS, DONE);
                }
            }
            AREATRIGGER_STADIUM => {
                if self.brs_encounter(TYPE_STADIUM as usize) == IN_PROGRESS
                    || self.brs_encounter(TYPE_STADIUM as usize) == DONE
                {
                    return false;
                }
                // UNMODELED: Nefarius (10162) / Rend (10429) Respawn()
                // when not alive (no creature-respawn helper).
                self.brs_set_data(TYPE_STADIUM, IN_PROGRESS);
            }
            _ => {}
        }
        false
    }

    /// `AreaTrigger_at_ubrs_the_beast`
    /// (`instance_blackrock_spire.cpp:868-879`): pull the Beast onto a
    /// living player when it idles outside combat. Always returns
    /// `false`.
    fn at_ubrs_the_beast(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const NPC_THE_BEAST: u32 = 10_430;
        let _ = trigger_id;
        if !self.unit_is_alive(player) {
            return false;
        }
        if let Some(beast) = self.brs_get_data64(NPC_THE_BEAST) {
            if self.unit_is_alive(beast) && !self.ai_in_combat(beast) {
                self.attack_start(beast, player);
            }
        }
        false
    }

    /// `AreaTrigger_at_orb_of_command`
    /// (`instance_blackwing_lair.cpp:1042-1056`): a dead player carrying
    /// the Blackhand's Command reward whose corpse lies in BWL is
    /// resurrected and placed at the orb. Always returns `false`.
    fn at_orb_of_command(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AT_ORB_OF_COMMAND: u32 = 3847;
        const QUEST_BLACKHANDS_COMMAND: u32 = 7761;
        if trigger_id != AT_ORB_OF_COMMAND {
            return false;
        }
        // UNMODELED: GetCorpse + corpse map check (no corpse model);
        // ResurrectPlayer(0.5) + SpawnCorpseBones (no resurrect helper).
        let rewarded = self
            .player(player)
            .and_then(|p| p.quest_row(QUEST_BLACKHANDS_COMMAND))
            .is_some_and(|r| r.rewarded != 0);
        if !self.unit_is_alive(player) && rewarded {
            self.teleport_unit_within_map(
                player,
                Position::new(-7672.32, -1107.05, 396.651, 0.785398),
            );
        }
        false
    }

    /// `AreaTrigger_at_enter_vael_room`
    /// (`instance_blackwing_lair.cpp:1058-1071`): latch the Vael event
    /// DONE for living non-GM players. Always returns `false`.
    fn at_enter_vael_room(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AT_ENTER_VAEL_ROOM: u32 = 3626;
        const TYPE_VAEL_EVENT: u32 = 8;
        if trigger_id != AT_ENTER_VAEL_ROOM {
            return false;
        }
        if self.player_is_game_master(player) {
            return false;
        }
        if self.bwl_get_data(TYPE_VAEL_EVENT) != Some(DONE) {
            self.bwl_set_data(TYPE_VAEL_EVENT, DONE);
        }
        false
    }

    /// `AreaTrigger_at_naxxramas`
    /// (`instance_naxxramas.cpp:1368-1415,1417-1431` with
    /// `boss_kelthuzad.cpp:1007-1019`): hub teleporter, KT intro, the
    /// Faerlina/Thaddius greets and the DK-wing taunt. Always returns
    /// `false`.
    fn at_naxxramas(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_HUB_TO_FROSTWYRM: u32 = 4156;
        const AREATRIGGER_KELTHUZAD: u32 = 4112;
        const AREATRIGGER_FAERLINA: u32 = 4115;
        const AREATRIGGER_THADDIUS_ENTRANCE: u32 = 4113;
        const AREATRIGGER_START_DK_WING: u32 = 4117;
        const TYPE_FOUR_HORSEMEN: u32 = 8;
        const TYPE_KELTHUZAD: u32 = 14;
        const NOT_STARTED: u32 = 0;
        const NPC_FAERLINA: u32 = 15_953;
        const NPC_THADDIUS: u32 = 15_928;
        const NPC_THANE: u32 = 16_064;
        const NPC_KELTHUZAD: u32 = 15_990;
        const SAY_FAERLINA_GREET: i32 = 12_852;
        const SAY_THADDIUS_GREET: i32 = 13_091;
        const SAY_KORT_TAUNT1: i32 = 13_038;
        if !self.unit_is_alive(player) {
            return false;
        }
        // GMs are barred everywhere except the hub teleporter.
        if self.player_is_game_master(player) && trigger_id != AREATRIGGER_HUB_TO_FROSTWYRM {
            return false;
        }
        match trigger_id {
            AREATRIGGER_HUB_TO_FROSTWYRM => {
                if self.naxx_wings_are_cleared() || self.player_is_game_master(player) {
                    self.teleport_unit_within_map(
                        player,
                        Position::new(3498.13, -5349.6, 144.967, 1.48353),
                    );
                }
            }
            AREATRIGGER_KELTHUZAD => {
                if self.naxx_encounter(TYPE_KELTHUZAD) != NOT_STARTED {
                    return false;
                }
                if let Some(kt) = self.find_creature_by_entry(NPC_KELTHUZAD) {
                    self.kelthuzad_start_encounter(kt);
                }
            }
            AREATRIGGER_FAERLINA => {
                // UNMODELED: m_faerlinaHaveGreeted once-flag (no
                // NaxxramasState field; greet repeats per entry).
                if let Some(faerlina) = self.find_creature_by_entry(NPC_FAERLINA) {
                    if self.unit_is_alive(faerlina) {
                        self.do_script_text(SAY_FAERLINA_GREET, faerlina, None, -1);
                    }
                }
            }
            AREATRIGGER_THADDIUS_ENTRANCE => {
                // UNMODELED: m_thaddiusHaveGreeted once-flag (no
                // NaxxramasState field; greet repeats per entry).
                if let Some(thaddius) = self.find_creature_by_entry(NPC_THADDIUS) {
                    if self.unit_is_alive(thaddius) {
                        self.do_script_text(SAY_THADDIUS_GREET, thaddius, None, -1);
                    }
                }
            }
            AREATRIGGER_START_DK_WING
                // UNMODELED: m_haveDoneDKWingIntro once-flag (no
                // NaxxramasState field) + EVENT_DKWING_INTRO_2 chain
                // (ids dropped with the area trigger, content.rs:17518).
                if self.naxx_encounter(TYPE_FOUR_HORSEMEN) != DONE => {
                    if let Some(thane) = self.find_creature_by_entry(NPC_THANE) {
                        self.do_script_text(SAY_KORT_TAUNT1, thane, None, -1);
                    }
                }
            _ => {}
        }
        false
    }

    /// `AreaTrigger_at_shade_of_eranikus` (`sunken_temple.cpp:143-166`):
    /// Malfurion summon for the AQ-scepter opener. Returns `true` when
    /// Malfurion is already present or is summoned, `false` otherwise.
    fn at_shade_of_eranikus(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_MALFURION: u32 = 4016;
        const NPC_MALFURION: u32 = 15_362;
        const QUEST_THE_CHARGE_OF_DRAGONFLIGHTS: u32 = 8555;
        const QUEST_ERANIKUS_TYRANT_OF_DREAMS: u32 = 8733;
        const QUEST_STATUS_NONE: i32 = 0;
        const TEMPSUMMON_CORPSE_DESPAWN: u32 = 5;
        if !self.unit_is_alive(player) {
            return false;
        }
        if trigger_id != AREATRIGGER_MALFURION {
            return false;
        }
        let charge_rewarded = self
            .player(player)
            .and_then(|p| p.quest_row(QUEST_THE_CHARGE_OF_DRAGONFLIGHTS))
            .is_some_and(|r| r.rewarded != 0);
        let tyrant_status = self.player(player).map_or(QUEST_STATUS_NONE, |p| {
            p.quest_status(QUEST_ERANIKUS_TYRANT_OF_DREAMS)
        });
        let tyrant_rewarded = self
            .player(player)
            .and_then(|p| p.quest_row(QUEST_ERANIKUS_TYRANT_OF_DREAMS))
            .is_some_and(|r| r.rewarded != 0);
        if !charge_rewarded || tyrant_status != QUEST_STATUS_NONE || tyrant_rewarded {
            return false;
        }
        if self
            .nearest_creature_with_entry(player, NPC_MALFURION, 50.0)
            .is_some()
        {
            return true;
        }
        // UNMODELED: AreaTriggerEntry x/y/z (dispatcher passes only
        // trigger_id); fork summons at (pAt->x, pAt->y - 15, pAt->z, 1.52).
        if self
            .summon_creature(
                NPC_MALFURION,
                Position::new(0.0, -15.0, 0.0, 1.52),
                player,
                TEMPSUMMON_CORPSE_DESPAWN,
                0,
            )
            .is_some()
        {
            return true;
        }
        false
    }

    /// `AreaTrigger_at_cathedral_entrance`
    /// (`instance_scarlet_monastery.cpp:500-519`): aura-bearing players
    /// start the Ashbringer event with Mograine's yell. Returns `true`
    /// when the event starts, `false` otherwise.
    fn at_cathedral_entrance(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_CATHEDRAL: u32 = 4089;
        const SPELL_ASHBRINGER: u32 = 28_282;
        const TYPE_ASHBRINGER_EVENT: u32 = 2;
        const DATA_MOGRAINE: u32 = 2;
        const NOT_STARTED: u32 = 0;
        const YELL_COMMANDER: i32 = 12_389;
        if trigger_id != AREATRIGGER_CATHEDRAL {
            return false;
        }
        if !self.bot_has_aura(player, SPELL_ASHBRINGER) {
            return false;
        }
        if self.sm_get_data(TYPE_ASHBRINGER_EVENT) != Some(NOT_STARTED) {
            return false;
        }
        self.sm_set_data(TYPE_ASHBRINGER_EVENT, IN_PROGRESS);
        if let Some(mograine) = self.sm_get_data64(DATA_MOGRAINE) {
            if self.unit_is_alive(mograine) {
                self.do_script_text(YELL_COMMANDER, mograine, Some(player), -1);
            }
        }
        true
    }

    /// `AreaTrigger_at_dmf_chest_dm`
    /// (`instance_deadmines.cpp:289-299`): reveal the Darkmoon chest for
    /// players with Fortune Awaits complete. Always returns `false`.
    fn at_dmf_chest_dm(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_FORTUNE_AWAITS: u32 = 7938;
        const QUEST_STATUS_COMPLETE: i32 = 1;
        const GO_DMF_CHEST: u32 = 180_024;
        let _ = trigger_id;
        let complete = self
            .player(player)
            .is_some_and(|p| p.quest_status(QUEST_FORTUNE_AWAITS) == QUEST_STATUS_COMPLETE);
        if complete {
            if let Some(go) = self.nearest_gameobject_with_entry(player, GO_DMF_CHEST, 100.0) {
                self.hidden_objects.remove(&go);
            }
        }
        false
    }

    /// `AreaTrigger_at_temple_ahnqiraj`
    /// (`instance_temple_of_ahnqiraj.cpp:526-540`): twin/sartura intro
    /// arm plus the C'Thun stomach triggers. Always returns `false`.
    fn at_temple_ahnqiraj(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AREATRIGGER_TWIN_EMPERORS: u32 = 4047;
        const AREATRIGGER_SARTURA: u32 = 4052;
        // Stomach arms (4033 ground / 4034 air / 4036 knockback) ride the
        // same trigger; positions come from the entry (see UNMODELED).
        if trigger_id == AREATRIGGER_TWIN_EMPERORS || trigger_id == AREATRIGGER_SARTURA {
            if self.player_is_game_master(player) || !self.unit_is_alive(player) {
                return false;
            }
            self.aq40_handle_temple_area_trigger(trigger_id);
        }
        // UNMODELED: AreaTriggerEntry x/y/z (dispatcher passes only
        // trigger_id); the STOMACH_GROUND punt summon needs them.
        self.aq40_handle_stomach_trigger(player, trigger_id, 0.0, 0.0, 0.0);
        false
    }

    /// `OnTrigger_at_zumrah` (`zulfarrak.cpp:560-580`): turn Zumrah
    /// hostile with yell on first approach. Always returns `true`
    /// (fork falls through to DB scripts).
    fn at_zumrah(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const NPC_WITCH_DOCTOR_ZUMRAH: u32 = 7271;
        const ZUMRAH_HOSTILE_FACTION: u32 = 37;
        const EVENT_ZUMRAH: u32 = 4;
        const SAY_ZUMRAH_TRIGGER: i32 = 3622;
        const UNIT_FLAG_IMMUNE_TO_PLAYER: u32 = 0x100;
        let _ = trigger_id;
        let Some(zumrah) = self.nearest_creature_with_entry(player, NPC_WITCH_DOCTOR_ZUMRAH, 30.0)
        else {
            return false;
        };
        if self.unit_value_u32(zumrah, offsets::UNIT_FIELD_FACTIONTEMPLATE)
            != ZUMRAH_HOSTILE_FACTION
        {
            self.zf_set_data(EVENT_ZUMRAH, IN_PROGRESS);
            let flags = self.object_get_field(zumrah, offsets::UNIT_FIELD_FLAGS);
            let _ = self.object_set_field(
                zumrah,
                u32::from(offsets::UNIT_FIELD_FLAGS),
                flags & !UNIT_FLAG_IMMUNE_TO_PLAYER,
            );
            let _ = self.object_set_field(
                zumrah,
                u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
                ZUMRAH_HOSTILE_FACTION,
            );
            self.do_script_text(SAY_ZUMRAH_TRIGGER, zumrah, None, -1);
        }
        true
    }

    /// `OnTrigger_at_antusul` (`zulfarrak.cpp:591-640`): start the
    /// Antusul ambush with yell. Returns `true` when the ambush starts,
    /// `false` otherwise.
    fn at_antusul(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const NPC_ANTUSUL: u32 = 8127;
        const EVENT_ANTUSUL: u32 = 5;
        const NOT_STARTED: u32 = 0;
        const SAY_ANTUSUL_TRIGGER: i32 = 4166;
        let _ = trigger_id;
        let Some(antusul) = self.nearest_creature_with_entry(player, NPC_ANTUSUL, 100.0) else {
            return false;
        };
        if self.ai_in_combat(antusul) {
            return false;
        }
        if self.zf_get_data(EVENT_ANTUSUL) != Some(NOT_STARTED) {
            return false;
        }
        self.zf_set_data(EVENT_ANTUSUL, IN_PROGRESS);
        self.do_script_text(SAY_ANTUSUL_TRIGGER, antusul, None, -1);
        // UNMODELED: BATCHING_INTERVAL x3 delayed lambda (no timer
        // model): 4x Sul'lithuz Broodling (8138) summons at fixed points
        // + Antusul MovePoint to (1805.133667, 740.349304, 14.763382).
        true
    }

    /// `AreaTrigger_at_dmf_chest_wc`
    /// (`instance_wailing_caverns.cpp:235-246`): reveal the Darkmoon
    /// chest for players with Fortune Awaits complete. Always returns
    /// `false`.
    fn at_dmf_chest_wc(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_FORTUNE_AWAITS: u32 = 7944;
        const QUEST_STATUS_COMPLETE: i32 = 1;
        const GO_DMF_CHEST: u32 = 180_055;
        let _ = trigger_id;
        let complete = self
            .player(player)
            .is_some_and(|p| p.quest_status(QUEST_FORTUNE_AWAITS) == QUEST_STATUS_COMPLETE);
        if complete {
            if let Some(go) = self.nearest_gameobject_with_entry(player, GO_DMF_CHEST, 100.0) {
                self.hidden_objects.remove(&go);
            }
        }
        false
    }

    // ---- lane at_world.rs ----
    // `AreaTrigger_at_twilight_grove` (`eastern_kingdoms/duskwood/duskwood.cpp:56-60`
    // via `Handle_NightmareCorruption`, cpp 27-54): quest 8735 incomplete and a
    // living player with no Twilight Corrupter (15625) within 350 yd summons it at
    // the grove point (dead-despawn); always returns `false`.
    fn at_twilight_grove(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_NIGHTMARE_CORRUPTION: u32 = 8735;
        const NPC_TWILIGHT_CORRUPTER: u32 = 15625;
        const TEMPSUMMON_DEAD_DESPAWN: u32 = 7;
        let _ = trigger_id;
        if !self.unit_is_alive(player)
            || !self.player_quest_incomplete(player, QUEST_NIGHTMARE_CORRUPTION)
        {
            return false;
        }
        if self
            .nearest_creature_with_entry(player, NPC_TWILIGHT_CORRUPTER, 350.0)
            .is_none()
        {
            // UNMODELED: fork `sLog` error when the summon fails (cpp 44).
            self.summon_creature(
                NPC_TWILIGHT_CORRUPTER,
                Position {
                    x: -10335.9,
                    y: -489.051,
                    z: 50.6233,
                    o: 2.59373,
                },
                player,
                TEMPSUMMON_DEAD_DESPAWN,
                0,
            );
            // UNMODELED: `corrupter->MonsterWhisper("Come, %s. See what the
            // Nightmare brings...", ...)` (cpp 52-54).
        }
        false
    }

    // `AreaTrigger_at_huldar_miran` (`eastern_kingdoms/loch_modan/loch_modan.cpp:132-207`):
    // quest 273 incomplete (and not complete/none) completes the quest, then
    // requires living Huldar (2057) and Miran (1379) within 60 yd; Saean (1380)
    // turns hostile and summons two Dark Iron Ambushers (1981). Returns `true`
    // once the ambush runs (or the principals are busy/missing at the end),
    // `false` on early gates.
    fn at_huldar_miran(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_RESUPPLYING_THE_EXCAVATION: u32 = 273;
        const NPC_SAEAN: u32 = 1380;
        const NPC_MIRAN: u32 = 1379;
        const NPC_HULDAR: u32 = 2057;
        const NPC_DARK_IRON_AMBUSHER: u32 = 1981;
        const FACTION_HOSTILE: u32 = 54;
        const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;
        let _ = trigger_id;
        let _ = FACTION_HOSTILE;
        if !self.unit_is_alive(player)
            || !self.player_quest_incomplete(player, QUEST_RESUPPLYING_THE_EXCAVATION)
        {
            return false;
        }
        // UNMODELED: `pPlayer->CompleteQuest(273)` +
        // `pPlayer->SendQuestCompleteEvent(273)` (cpp 140-141, no Map equivalent).
        let Some(huldar) = self.nearest_creature_with_entry(player, NPC_HULDAR, 60.0) else {
            return false;
        };
        if !self.unit_is_alive(huldar) {
            return false;
        }
        let Some(miran) = self.nearest_creature_with_entry(player, NPC_MIRAN, 60.0) else {
            return false;
        };
        if !self.unit_is_alive(miran) {
            return false;
        }
        // UNMODELED: `miranAI->HasEscortState(STATE_ESCORT_ESCORTING)` screen
        // (cpp 162-166, no escort-state helper).
        if self.ai_in_combat(miran) || self.ai_in_combat(huldar) {
            return true;
        }
        if let Some(saean) = self.nearest_creature_with_entry(player, NPC_SAEAN, 60.0) {
            if self.unit_is_alive(saean) {
                // UNMODELED: `saean->SetFactionTemporary(FACTION_HOSTILE,
                // TEMPFACTION_RESTORE_RESPAWN)` (cpp 181).
                if self
                    .nearest_creature_with_entry(saean, NPC_DARK_IRON_AMBUSHER, 100.0)
                    .is_none()
                {
                    self.summon_creature(
                        NPC_DARK_IRON_AMBUSHER,
                        Position {
                            x: -5759.852051,
                            y: -3441.279053,
                            z: 305.573212,
                            o: 2.174024,
                        },
                        saean,
                        TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                        25_000,
                    );
                    self.summon_creature(
                        NPC_DARK_IRON_AMBUSHER,
                        Position {
                            x: -5757.629883,
                            y: -3437.680908,
                            z: 304.265106,
                            o: 2.610265,
                        },
                        saean,
                        TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                        25_000,
                    );
                    // UNMODELED: `saean->AI()->AttackStart(miran)` (cpp 189).
                } else {
                    return false;
                }
            }
        }
        true
    }

    // `AreaTrigger_at_stormwind_gates`
    // (`eastern_kingdoms/stormwind_city/quest_stormwind_rendezvous.cpp:989-1031`):
    // pre-1.12 Windsor (12580) spawn at the Stormwind gates for quest 6402.
    // Returns `true` when Windsor is summoned, `false` otherwise.
    fn at_stormwind_gates(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_STORMWIND_RENDEZVOUS: u32 = 6402;
        const NPC_REGINALD_WINDSOR: u32 = 12580;
        const TEMPSUMMON_MANUAL_DESPAWN: u32 = 8;
        let _ = trigger_id;
        let _ = QUEST_STORMWIND_RENDEZVOUS;
        // UNMODELED: `sWorld.GetWowPatch() >= WOW_PATCH_112 &&
        // getConfig(CONFIG_BOOL_ACCURATE_PVE_EVENTS)` early-out (cpp 994).
        // `player_is_game_master` is private in area_triggers.rs; inlined here.
        if !self.unit_is_alive(player)
            || self
                .player(player)
                .is_some_and(|p| p.extra_flags & 0x1 != 0)
        {
            return false;
        }
        // UNMODELED: quest gate `IsCurrentQuest(6402) || (GetQuestRewardStatus(6402)
        // && !GetQuestRewardStatus(6403) && !IsCurrentQuest(6403))` (cpp 999, no
        // Map equivalent for current-quest/reward-status).
        // UNMODELED: `globalWindsorLastSpawnTime + 15 min` cooldown gate (cpp 1003).
        if self
            .nearest_creature_with_entry(player, NPC_REGINALD_WINDSOR, 200.0)
            .is_some()
        {
            return false;
        }
        if self
            .summon_creature(
                NPC_REGINALD_WINDSOR,
                Position {
                    x: -9148.40,
                    y: 371.32,
                    z: 91.0,
                    o: 0.70,
                },
                player,
                TEMPSUMMON_MANUAL_DESPAWN,
                5_400_000,
            )
            .is_some()
        {
            // UNMODELED: Windsor AI wiring (`playerGUID`, squire guid), `Mount`,
            // `SetWalk`, `SetSpeedRate`, `MovePoint` to WindsorWaypoints[0],
            // `RemoveFlag(UNIT_NPC_FLAGS, UNIT_NPC_FLAG_QUESTGIVER)` and the
            // cooldown store (cpp 1011-1027).
            return true;
        }
        false
    }

    // `AreaTrigger_at_sentry_point` (`kalimdor/dustwallow_marsh/dustwallow_marsh.cpp:205-262`):
    // quest 1265 incomplete completes the quest and drives the Tervosh (4967)
    // arrival event. Returns `true` on the handled path, `false` on gates/summon
    // failure.
    fn at_sentry_point(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_MISSING_DIPLO_PT14: u32 = 1265;
        const NPC_TERVOSH: u32 = 4967;
        const TEMPSUMMON_MANUAL_DESPAWN: u32 = 8;
        const MDQP_WAITING: u32 = 4;
        const MDQP_TELEPORT_BACK: u32 = 5;
        const MDQP_PREPARE_TO_ARRIVE: u32 = 0;
        let _ = trigger_id;
        let _ = MDQP_PREPARE_TO_ARRIVE;
        if !self.unit_is_alive(player)
            || self
                .player(player)
                .is_some_and(|p| p.extra_flags & 0x1 != 0)
            || !self.player_quest_incomplete(player, QUEST_MISSING_DIPLO_PT14)
        {
            return false;
        }
        // UNMODELED: `pPlayer->CompleteQuest(1265)` +
        // `pPlayer->SendQuestCompleteEvent(1265)` (cpp 212-213).
        match self.nearest_creature_with_entry(player, NPC_TERVOSH, 15.0) {
            None => {
                let Some(tervosh) = self.summon_creature(
                    NPC_TERVOSH,
                    Position {
                        x: -3476.860840,
                        y: -4106.740723,
                        z: 17.107151,
                        o: 5.420159,
                    },
                    player,
                    TEMPSUMMON_MANUAL_DESPAWN,
                    0,
                ) else {
                    return false;
                };
                // UNMODELED: `SetFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_ATTACKABLE_1 |
                // UNIT_FLAG_IMMUNE_TO_NPC)` (cpp 224).
                // UNMODELED: `tervoshAI->m_eventStarted = true` (cpp 227, no
                // begin-event helper; `tervosh_reset` only clears it).
                let _ = tervosh;
            }
            Some(tervosh) => {
                let phase = self.tervosh_current_phase(tervosh);
                if phase == MDQP_WAITING {
                    self.tervosh_reset_despawn_delay(tervosh);
                } else if phase == MDQP_TELEPORT_BACK {
                    let Some(new_tervosh) = self.summon_creature(
                        NPC_TERVOSH,
                        Position {
                            x: -3476.860840,
                            y: -4106.740723,
                            z: 17.107151,
                            o: 5.420159,
                        },
                        player,
                        TEMPSUMMON_MANUAL_DESPAWN,
                        0,
                    ) else {
                        return false;
                    };
                    // UNMODELED: `SetFlag(...)` + `SetVisibility(VISIBILITY_OFF)` +
                    // `m_nextPhaseDelayTimer = 3000`, `m_eventPhase =
                    // MDQP_PREPARE_TO_ARRIVE`, `m_eventStarted = true` (cpp 249-256).
                    let _ = new_tervosh;
                }
            }
        }
        true
    }

    // `AreaTrigger_at_irontree_wood` (`kalimdor/felwood/felwood.cpp:558-577`):
    // trigger 3587, hunter with quest 7632 complete summons the three Ancients
    // (14524/14525/14526) unless Hastat is already near. Returns `true` on the
    // summon arm, `false` otherwise.
    fn at_irontree_wood(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const AT_IRONTREE_WOOD: u32 = 3587;
        const QUEST_THE_ANCIENT_LEAF: u32 = 7632;
        const NPC_VARTRUS_THE_ANCIENT: u32 = 14524;
        const NPC_STOME_THE_ANCIENT: u32 = 14525;
        const NPC_HASTAT_THE_ANCIENT: u32 = 14526;
        const TEMPSUMMON_TIMED_DESPAWN: u32 = 3;
        if trigger_id != AT_IRONTREE_WOOD {
            return false;
        }
        // UNMODELED: `pPlayer->GetClass() == CLASS_HUNTER` (cpp 562, no class
        // helper; cf. NEEDS-MAP-HELPER precedent at content.rs:45249).
        if self
            .player(player)
            .is_some_and(|p| p.quest_status(QUEST_THE_ANCIENT_LEAF) == 1)
        {
            if self
                .nearest_creature_with_entry(player, NPC_HASTAT_THE_ANCIENT, 100.0)
                .is_none()
            {
                self.summon_creature(
                    NPC_VARTRUS_THE_ANCIENT,
                    Position {
                        x: 6194.55,
                        y: -1176.35,
                        z: 369.056,
                        o: 1.1098,
                    },
                    player,
                    TEMPSUMMON_TIMED_DESPAWN,
                    600_000,
                );
                self.summon_creature(
                    NPC_STOME_THE_ANCIENT,
                    Position {
                        x: 6197.12,
                        y: -1135.42,
                        z: 366.31,
                        o: 5.28025,
                    },
                    player,
                    TEMPSUMMON_TIMED_DESPAWN,
                    600_000,
                );
                self.summon_creature(
                    NPC_HASTAT_THE_ANCIENT,
                    Position {
                        x: 6245.91,
                        y: -1165.98,
                        z: 366.325,
                        o: 2.60598,
                    },
                    player,
                    TEMPSUMMON_TIMED_DESPAWN,
                    600_000,
                );
            }
            return true;
        }
        false
    }

    // `AreaTrigger_at_twiggy_flathead` (`kalimdor/the_barrens/the_barrens.cpp:391-404`):
    // living player with quest 1719 incomplete near Twiggy (6248); defers to the
    // event state. Returns `false` (let mangos process further) only when the
    // event can start, `true` otherwise.
    fn at_twiggy_flathead(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_AFFRAY: u32 = 1719;
        const NPC_TWIGGY: u32 = 6248;
        let _ = trigger_id;
        if self.unit_is_alive(player) && self.player_quest_incomplete(player, QUEST_AFFRAY) {
            let Some(twiggy) = self.nearest_creature_with_entry(player, NPC_TWIGGY, 30.0) else {
                return true;
            };
            return !self.twiggy_can_start_event(twiggy, player);
        }
        true
    }

    // `AreaTrigger_at_scent_larkorwi` (`kalimdor/ungoro_crater/ungoro_crater.cpp:333-348`):
    // living non-GM player with quest 4291 incomplete summons Larkorwi's mate
    // (9683) at the trigger position (2 min, 60 s per-trigger cooldown). Always
    // returns `false`.
    fn at_scent_larkorwi(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_SCENT_OF_LARKORWI: u32 = 4291;
        const NPC_LARKORWI_MATE: u32 = 9683;
        const TEMPSUMMON_TIMED_OR_DEAD_DESPAWN: u32 = 1;
        let _ = trigger_id;
        if self.unit_is_alive(player)
            && self
                .player(player)
                .is_some_and(|p| p.extra_flags & 0x1 == 0)
            && self.player_quest_incomplete(player, QUEST_SCENT_OF_LARKORWI)
            && self
                .nearest_creature_with_entry(player, NPC_LARKORWI_MATE, 25.0)
                .is_none()
        {
            // UNMODELED: static per-trigger 60 s cooldown map
            // (`cooldown[pAt->id]`, cpp 338-343).
            // UNMODELED: AreaTriggerEntry x/y/z (fork summons at pAt->x/y/z facing
            // 3.3); the dispatcher passes only trigger_id, so summon at the
            // triggering player instead.
            let at = self.object_position(player).unwrap_or_default();
            self.summon_creature(
                NPC_LARKORWI_MATE,
                Position {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    o: 3.3,
                },
                player,
                TEMPSUMMON_TIMED_OR_DEAD_DESPAWN,
                120_000,
            );
        }
        false
    }

    // `AreaTrigger_at_childrens_week_spot` (`world/areatrigger_scripts.cpp:43-54`):
    // orphan-pet-percentile credit table. The minipet and credit arms have no Map
    // model, so no arm can fire; always returns `false` here.
    fn at_childrens_week_spot(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const TRIGGER_ORPHAN_SPELL: [[u32; 3]; 6] = [
            [3546, 14305, 1479],
            [3547, 14444, 1800],
            [3548, 14305, 1558],
            [3549, 14444, 911],
            [3550, 14444, 910],
            [3552, 14305, 1687],
        ];
        let _ = player;
        for row in TRIGGER_ORPHAN_SPELL {
            if trigger_id == row[0] {
                // UNMODELED: `pPlayer->GetMiniPet()->GetEntry() == row[1]` (no
                // minipet model; cf. customs.rs:511 precedent, treated as absent).
                // UNMODELED: `pPlayer->AreaExploredOrEventHappens(row[2])` (no Map
                // equivalent; NEEDS-MAP-HELPER precedent at content.rs:34318).
                break;
            }
        }
        false
    }

    // `AreaTrigger_at_ravenholdt` (`world/areatrigger_scripts.cpp:66-72`): quest
    // 6681 incomplete grants the Ravenholdt (13936) kill credit. Always returns
    // `false`.
    fn at_ravenholdt(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        const QUEST_MANOR_RAVENHOLDT: u32 = 6681;
        const NPC_RAVENHOLDT: u32 = 13936;
        let _ = trigger_id;
        if self.player_quest_incomplete(player, QUEST_MANOR_RAVENHOLDT) {
            // Fork `KilledMonsterCredit(entry)` defaults the packet victim guid to
            // empty; mirrored here.
            self.quest_kill_credit_one(player, ObjectGuid::EMPTY, NPC_RAVENHOLDT);
        }
        false
    }
}
