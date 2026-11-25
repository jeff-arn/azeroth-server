//! `OutdoorPvP` Silithus (`OutdoorPvPSI.h/.cpp`:
//! map 1, zones 1377/3428/3429, no capture-point
//! slider — `Update()` is empty in the fork).
//!
//! Map-local only (append-only DB lock): gathered
//! counters, max, last controller and spawned dust-bag
//! guids live on [`Map`] and reset on map drop. The
//! fork persists only `m_MaxRessources` (2317, default
//! 200) across reboots via `SetSavedVariable` and
//! resets gathered to 0; here max is a plain field
//! (in-memory-only deviation, recorded).

use std::collections::HashMap;

use mangos_entities::unit::Team;
use mangos_guid::ObjectGuid;

use crate::battlegrounds::build_update_world_state_packet;
use crate::map::{Map, MapEvent};
use crate::spells::{AuraRemoveMode, CastOptions, CastTarget};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// Kalimdor (`OutdoorPvP_silithus::GetMapId`).
pub const SI_MAP_ID: u32 = 1;
/// Silithus proper (`OutdoorPvPSIBuffZones[0]`,
/// zone-text target and aura/drop zone).
pub const SI_ZONE_SILITHUS: u32 = 1377;
/// Buff zones (`OutdoorPvPSIBuffZones`).
pub const SI_BUFF_ZONES: [u32; 3] = [1377, 3428, 3429];
/// `WS_OPVP_SI_GATHERED_A` (`WorldStates.h:112`).
pub const SI_WS_GATHERED_A: u32 = 2313;
/// `WS_OPVP_SI_GATHERED_H` (`WorldStates.h:113`).
pub const SI_WS_GATHERED_H: u32 = 2314;
/// `WS_OPVP_SI_SILITHYST_MAX` (`WorldStates.h:114`).
pub const SI_WS_SILITHYST_MAX: u32 = 2317;
/// `SI_MAX_RESOURCES_DEFAULT` (cpp:40).
pub const SI_MAX_DEFAULT: u32 = 200;
/// Alliance turn-in trigger (`SI_AREATRIGGER_A`).
pub const SI_AREATRIGGER_A: u32 = 4162;
/// Horde turn-in trigger (`SI_AREATRIGGER_H`).
pub const SI_AREATRIGGER_H: u32 = 4168;
/// Carried flag (`SI_SILITHYST_FLAG`).
pub const SI_SPELL_FLAG: u32 = 29519;
/// Deposit reward spells (cpp:244-246).
pub const SI_SPELL_TRACES: u32 = 29534;
pub const SI_SPELL_HONOR_199: u32 = 31420;
pub const SI_SPELL_REP: u32 = 31247;
/// Winner buff (`SI_CENARION_FAVOR`).
pub const SI_SPELL_FAVOR: u32 = 30754;
/// Speed-limit debuffs (`spell_special.cpp:290-293`).
pub const SI_SPELL_SPEED_A: u32 = 29894;
pub const SI_SPELL_SPEED_H: u32 = 29895;
/// Drop spell (`SILLITHUS_FLAG_DROP`).
pub const SI_SPELL_DROP: u32 = 29533;
/// Quest credit entries (h:45-46).
pub const SI_QUEST_CM_A: u32 = 17090;
pub const SI_QUEST_CM_H: u32 = 18199;
/// Dust-bag gameobject entry (`SI_DUST_BAG`).
pub const SI_DUST_BAG_ENTRY: u32 = 181_962;
/// Yeller creature entries (cpp:132-135).
pub const SI_ANNOUNCER_H: u32 = 17079;
pub const SI_ANNOUNCER_A: u32 = 17080;
/// Horde milestone broadcast texts (cpp:137-140).
pub const SI_BCT_H: [u32; 4] = [13476, 13477, 13478, 13469];
/// Alliance milestone broadcast texts (cpp:141-144).
pub const SI_BCT_A: [u32; 4] = [13479, 13480, 13481, 13470];
/// Alliance dust-bag DB guids (cpp:87).
pub const SI_ALLIANCE_BAGS: [u32; 11] = [
    9496, 9497, 9498, 9499, 9500, 9501, 9502, 9503, 9504, 9507, 9508,
];
/// Horde dust-bag DB guids (cpp:88).
pub const SI_HORDE_BAGS: [u32; 12] = [
    13566, 13567, 13568, 13582, 13583, 13584, 13585, 13587, 13588, 13589, 13590, 13591,
];
/// Capture zone-text `mangos_string` ids
/// (`LANG_OPVP_SI_CAPTURE_H/A`, `Language.h:1151-1152`).
pub const SI_LANG_CAPTURE_H: i32 = 10049;
pub const SI_LANG_CAPTURE_A: i32 = 10050;

/// Silithus `OutdoorPvP` state on one Kalimdor map
/// (`OutdoorPvPSI` members, cpp:77-84).
#[derive(Clone, Debug)]
pub struct SiState {
    /// `m_Gathered_A`.
    pub gathered_a: u32,
    /// `m_Gathered_H`.
    pub gathered_h: u32,
    /// `m_MaxRessources` (default 200, never
    /// reloaded from a saved variable here).
    pub max_resources: u32,
    /// `m_LastController` (0 fork = none).
    pub last_controller: Team,
    /// Spawned dust-bag live guids by DB guid
    /// (`m_allianceDustBags`).
    pub alliance_bags: HashMap<u32, ObjectGuid>,
    /// (`m_hordeDustBags`).
    pub horde_bags: HashMap<u32, ObjectGuid>,
}

impl Default for SiState {
    fn default() -> Self {
        Self {
            gathered_a: 0,
            gathered_h: 0,
            max_resources: SI_MAX_DEFAULT,
            last_controller: Team::None,
            alliance_bags: HashMap::new(),
            horde_bags: HashMap::new(),
        }
    }
}

impl SiState {
    /// `SetupZoneScript` zone registration (cpp:66-67).
    #[must_use]
    pub fn is_si_zone(zone_id: u32) -> bool {
        SI_BUFF_ZONES.contains(&zone_id)
    }

    /// `FillInitialWorldStates` pairs (cpp:39-44).
    #[must_use]
    pub fn fill_initial(&self) -> [(u32, u32); 3] {
        [
            (SI_WS_GATHERED_A, self.gathered_a),
            (SI_WS_GATHERED_H, self.gathered_h),
            (SI_WS_SILITHYST_MAX, self.max_resources),
        ]
    }

    /// Milestone yell for a fresh counter value:
    /// exact `==` on Max/4, Max/2, Max-Max/4, Max
    /// (cpp:173-180), announcer entry + broadcast-text
    /// id for the depositing faction.
    #[must_use]
    pub fn milestone_yell(gathered: u32, max: u32, alliance: bool) -> Option<(u32, u32)> {
        if max == 0 {
            return None;
        }
        let quarter = max / 4;
        let half = max / 2;
        let three_quarter = max - quarter;
        let idx = if gathered == quarter {
            0
        } else if gathered == half {
            1
        } else if gathered == three_quarter {
            2
        } else if gathered == max {
            3
        } else {
            return None;
        };
        let (announcer, bcts) = if alliance {
            (SI_ANNOUNCER_A, SI_BCT_A)
        } else {
            (SI_ANNOUNCER_H, SI_BCT_H)
        };
        Some((announcer, bcts[idx]))
    }

    /// `SpawnDustBags` need (cpp:92).
    #[must_use]
    pub fn needed_bags(resource: u32) -> u32 {
        resource / 15
    }
}

/// Triggered cast options for the SI reward / buff /
/// drop casts (fork `CastSpell(player, ..., true)`,
/// unit-targeted here: the fixtures and the real
/// spells use `UNIT`-family implicit targets, which
/// reject `SelfCast`).
fn triggered_self() -> CastOptions {
    CastOptions {
        triggered: true,
        triggered_by_aura: false,
        skip_known: true,
        cast_item: None,
        custom_base_points: [None; 3],
        triggered_by_aura_spell: 0,
        triggered_by_spell: 0,
    }
}

impl Map {
    /// SI gate: Kalimdor map + `OutdoorPvP.SI.Enable`
    /// (`Register.cpp`, `World.cpp:787`, conf 2747).
    /// No client-patch gate: the tree targets 1.12+
    /// builds (fork gates `SUPPORTED_CLIENT_BUILD >
    /// 1.11.2 && wowPatch >= 112`; recorded deviation).
    #[must_use]
    pub(crate) fn si_enabled(&self) -> bool {
        self.map_id == SI_MAP_ID && self.config.outdoor_pvp_si_enable
    }

    /// `FillInitialWorldStates` packet when the zone is
    /// an SI zone on an enabled map, else `None` (the
    /// caller falls back to `build_init_world_states`).
    pub(crate) fn si_init_world_states(&self, zone_id: u32) -> Option<WorldPacket> {
        if !self.si_enabled() || !SiState::is_si_zone(zone_id) {
            return None;
        }
        let mut writer = PacketWriter::new();
        writer.write_u32_le(self.map_id);
        writer.write_u32_le(zone_id);
        let extra = self.outdoor_si.fill_initial();
        writer.write_u16_le((crate::map::DEFAULT_WORLD_STATES.len() + extra.len()) as u16 + 1);
        for &(state, value) in crate::map::DEFAULT_WORLD_STATES {
            writer.write_u32_le(state);
            writer.write_u32_le(value);
        }
        for &(state, value) in &extra {
            writer.write_u32_le(state);
            writer.write_u32_le(value);
        }
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let mut packet = WorldPacket::new(opcode_id::SMSG_INIT_WORLD_STATES);
        packet.take_writer(writer);
        Some(packet)
    }

    /// Zone-change hook (`OnPlayerEnter/Leave`,
    /// cpp:78-85 + `ZoneScript::OnPlayerLeave`): sends
    /// the three zeroes when leaving SI zones (fork
    /// skips them on logout, which this layer cannot
    /// see — harmless extra send, recorded) and casts
    /// Cenarion Favor when entering under the last
    /// controller's team.
    pub(crate) fn si_on_zone_change(&mut self, guid: ObjectGuid, old_zone: u32, new_zone: u32) {
        if !self.si_enabled() {
            return;
        }
        let was_in = SiState::is_si_zone(old_zone);
        let is_in = SiState::is_si_zone(new_zone);
        if was_in && !is_in {
            for (field, _) in self.outdoor_si.fill_initial() {
                let packet = build_update_world_state_packet(field, 0);
                self.events.push(MapEvent::Packet { to: guid, packet });
            }
        }
        if is_in {
            let team = self.player_team_of(guid).unwrap_or(Team::None);
            if team != Team::None && team == self.outdoor_si.last_controller {
                let _ = self.cast_spell(
                    guid,
                    SI_SPELL_FAVOR,
                    CastTarget::Unit(guid),
                    triggered_self(),
                );
            }
        }
    }

    /// `SendUpdateWorldState` fan-out to every player
    /// in an SI zone (`ZoneScript.cpp:568`).
    fn si_broadcast_ws(&mut self, field: u32, value: u32) {
        let packet = build_update_world_state_packet(field, value);
        let targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, entry)| SiState::is_si_zone(entry.zone_id))
            .map(|(guid, _)| *guid)
            .collect();
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// Current world-state triple broadcast
    /// (`UpdateWorldState`, cpp:54-58, minus the
    /// `SetSavedVariable` DB writes per the lock).
    fn si_broadcast_all(&mut self) {
        let states = self.outdoor_si.fill_initial();
        for (field, value) in states {
            self.si_broadcast_ws(field, value);
        }
    }

    /// `TeamApplyBuff` (cpp:184, `ZoneScript.cpp:605`):
    /// casts the favor on winner-team zone players,
    /// removes it from the losers.
    fn si_team_apply_buff(&mut self, winner: Team, spell_id: u32) {
        let members: Vec<(ObjectGuid, Team)> = self
            .players
            .iter()
            .filter(|(_, entry)| SiState::is_si_zone(entry.zone_id))
            .map(|(guid, entry)| (*guid, entry.player.team))
            .collect();
        for (guid, team) in members {
            if team == winner {
                let _ = self.cast_spell(guid, spell_id, CastTarget::Unit(guid), triggered_self());
            } else {
                self.remove_auras_due_to_spell(guid, spell_id);
            }
        }
    }

    /// `DoSilithystYell` (cpp:148-153): nearest
    /// announcer of the faction yells the milestone
    /// text; missing announcer is a silent skip (fork
    /// logs `LOG_SCRIPTS`; recorded).
    fn si_do_yell(&mut self, player: ObjectGuid, announcer: u32, text_id: u32) {
        let radius = self.config.visibility_distance;
        if let Some(source) = self.nearest_creature_with_entry(player, announcer, radius) {
            self.do_script_text(i32::try_from(text_id).unwrap_or(0), source, None, -1);
        }
    }

    /// `SpawnDustBags` (cpp:90-108): spawns static
    /// dust-bag spawns (entry 181962) by DB guid until
    /// `resource / 15` are up, tracking live guids.
    /// Unknown DB guids are skipped like the fork's
    /// failed `LoadGameObjectSpawn`.
    fn si_spawn_dust_bags(&mut self, summoner: ObjectGuid, resource: u32, alliance: bool) {
        let needed = SiState::needed_bags(resource) as usize;
        let db_guids: &[u32] = if alliance {
            &SI_ALLIANCE_BAGS
        } else {
            &SI_HORDE_BAGS
        };
        for &db_guid in db_guids {
            let tracked = if alliance {
                self.outdoor_si.alliance_bags.len()
            } else {
                self.outdoor_si.horde_bags.len()
            };
            if tracked >= needed {
                break;
            }
            let already = if alliance {
                self.outdoor_si.alliance_bags.contains_key(&db_guid)
            } else {
                self.outdoor_si.horde_bags.contains_key(&db_guid)
            };
            if already {
                continue;
            }
            let Some(spawn) = self.stores.spawns.gameobject(db_guid).cloned() else {
                continue;
            };
            let rotation = [
                spawn.rotation0,
                spawn.rotation1,
                spawn.rotation2,
                spawn.rotation3,
            ];
            let Some(live) = self.summon_gameobject(
                summoner,
                SI_DUST_BAG_ENTRY,
                spawn.position.position(),
                rotation,
                0,
                false,
            ) else {
                continue;
            };
            if alliance {
                self.outdoor_si.alliance_bags.insert(db_guid, live);
            } else {
                self.outdoor_si.horde_bags.insert(db_guid, live);
            }
        }
    }

    /// `ResetResourceCount` (cpp:110-130).
    fn si_reset_resources(&mut self) {
        self.outdoor_si.gathered_a = 0;
        self.outdoor_si.gathered_h = 0;
        let live: Vec<ObjectGuid> = self
            .outdoor_si
            .alliance_bags
            .values()
            .chain(self.outdoor_si.horde_bags.values())
            .copied()
            .collect();
        for guid in live {
            self.despawn_gameobject_now(guid);
        }
        self.outdoor_si.alliance_bags.clear();
        self.outdoor_si.horde_bags.clear();
    }

    /// `HandleAreaTrigger` (cpp:156-253). Returns true
    /// when the trigger was consumed (aura present),
    /// false when other dispatch may continue.
    pub fn si_handle_area_trigger(&mut self, player: ObjectGuid, trigger_id: u32) -> bool {
        if !self.si_enabled() {
            return false;
        }
        if trigger_id != SI_AREATRIGGER_A && trigger_id != SI_AREATRIGGER_H {
            return false;
        }
        if !self
            .aura_holders(player)
            .iter()
            .any(|h| h.spell_id == SI_SPELL_FLAG)
        {
            return false;
        }
        let team = self.player_team_of(player).unwrap_or(Team::None);
        let alliance_side = trigger_id == SI_AREATRIGGER_A;
        if (alliance_side && team != Team::Alliance) || (!alliance_side && team != Team::Horde) {
            // Wrong-team turn-in: swallowed, no credit
            // (cpp:200-201, 235-236).
            return true;
        }
        let max = self.outdoor_si.max_resources;
        if alliance_side {
            self.outdoor_si.gathered_a += 1;
        } else {
            self.outdoor_si.gathered_h += 1;
        }
        let gathered = if alliance_side {
            self.outdoor_si.gathered_a
        } else {
            self.outdoor_si.gathered_h
        };
        if let Some((announcer, text)) = SiState::milestone_yell(gathered, max, alliance_side) {
            self.si_do_yell(player, announcer, text);
        }
        if gathered >= max {
            let winner = if alliance_side {
                Team::Alliance
            } else {
                Team::Horde
            };
            let entry = if alliance_side {
                SI_LANG_CAPTURE_A
            } else {
                SI_LANG_CAPTURE_H
            };
            self.si_team_apply_buff(winner, SI_SPELL_FAVOR);
            self.events.push(MapEvent::OutdoorZoneText {
                zone_id: SI_ZONE_SILITHUS,
                entry,
                defense: false,
            });
            self.outdoor_si.last_controller = winner;
            self.si_reset_resources();
        } else {
            self.si_spawn_dust_bags(player, gathered, alliance_side);
        }
        let quest = if alliance_side {
            SI_QUEST_CM_A
        } else {
            SI_QUEST_CM_H
        };
        self.quest_kill_credit_one(player, ObjectGuid::EMPTY, quest);
        self.remove_auras_due_to_spell(player, SI_SPELL_FLAG);
        let _ = self.cast_spell(
            player,
            SI_SPELL_TRACES,
            CastTarget::Unit(player),
            triggered_self(),
        );
        let _ = self.cast_spell(
            player,
            SI_SPELL_HONOR_199,
            CastTarget::Unit(player),
            triggered_self(),
        );
        let _ = self.cast_spell(
            player,
            SI_SPELL_REP,
            CastTarget::Unit(player),
            triggered_self(),
        );
        self.si_broadcast_all();
        true
    }

    /// `SilithystAuraScript::OnAfterApply`
    /// (`spell_special.cpp:295-333`): team speed buff on
    /// apply, removal + drop-flag notify on cancel /
    /// dispel removal. No Death remove-mode, mount or
    /// stealth flag-drop paths exist in this tree
    /// (named follow-up); deposit-time removal rides
    /// `RemoveAurasDueToSpell` (mode Default) and never
    /// drops, fork-exact.
    pub(crate) fn si_silithyst_aura(
        &mut self,
        victim: ObjectGuid,
        spell_id: u32,
        apply: bool,
        remove_mode: AuraRemoveMode,
    ) {
        if spell_id != SI_SPELL_FLAG || !self.si_enabled() {
            return;
        }
        if !victim.is_player() {
            return;
        }
        if self.player_zone_id(victim) != SI_ZONE_SILITHUS {
            return;
        }
        let team = self.player_team_of(victim).unwrap_or(Team::None);
        let buff = match team {
            Team::Alliance => SI_SPELL_SPEED_A,
            Team::Horde => SI_SPELL_SPEED_H,
            Team::None => return,
        };
        if apply {
            let _ = self.cast_spell(victim, buff, CastTarget::Unit(victim), triggered_self());
        } else {
            self.remove_auras_due_to_spell(victim, buff);
            if matches!(remove_mode, AuraRemoveMode::Cancel | AuraRemoveMode::Dispel) {
                self.si_handle_drop_flag(victim, spell_id);
            }
        }
    }

    /// `HandleDropFlag` (cpp:255-270). Returns true
    /// when the flag was dropped.
    pub(crate) fn si_handle_drop_flag(&mut self, player: ObjectGuid, spell_id: u32) -> bool {
        if spell_id != SI_SPELL_FLAG || !self.si_enabled() {
            return false;
        }
        if self.player_zone_id(player) != SI_ZONE_SILITHUS {
            return false;
        }
        // No flag-aura gate here (fork-exact): the
        // cancel/dispel remove path fires after the
        // holder is gone.
        let team = self.player_team_of(player).unwrap_or(Team::None);
        let own_trigger = match team {
            Team::Alliance => SI_AREATRIGGER_A,
            Team::Horde => SI_AREATRIGGER_H,
            Team::None => return false,
        };
        // Within 5m (3D) of the own turn-in: no drop
        // (cpp:260-262).
        if let Some(at) = self.stores.area_triggers.get(own_trigger).cloned()
            && let Some(pos) = self.object_position(player)
        {
            let dx = f64::from(pos.x - at.x);
            let dy = f64::from(pos.y - at.y);
            let dz = f64::from(pos.z - at.z);
            if dx * dx + dy * dy + dz * dz <= 25.0 {
                return false;
            }
        }
        let _ = self.cast_spell(
            player,
            SI_SPELL_DROP,
            CastTarget::Unit(player),
            triggered_self(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    use mangos_db::character_load::{CharacterLoad, CharacterRow, QuestStatusRow};
    use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player, RestRates};
    use mangos_spells::defines::{aura as aura_type, effect, target as tgt};
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::area_trigger::AreaTriggerEntry;
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };
    use mangos_world_data::quest_template::{QuestTemplate, QuestTemplates};

    use crate::WorldRng;
    use crate::map::MapConfig;
    use crate::stores::WorldStores;

    /// Aura spell fixture: `APPLY_AURA`/`DUMMY` on the
    /// unit target (positive, cancelable, lands a
    /// holder).
    /// The flag carries the fork script name so the
    /// `spell_silithyst` dispatch arm fires.
    fn aura_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            script_name: if id == SI_SPELL_FLAG {
                "spell_silithyst".to_string()
            } else {
                String::new()
            },
            effects: [
                SpellEffectData {
                    effect: effect::APPLY_AURA,
                    implicit_target_a: tgt::UNIT,
                    apply_aura_name: aura_type::DUMMY,
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        }
    }

    /// Non-aura spell fixture: present in the store so
    /// triggered casts resolve, no effects to model.
    fn plain_spell(id: u32) -> SpellEntry {
        SpellEntry {
            id,
            ..Default::default()
        }
    }

    fn si_spells() -> Vec<SpellEntry> {
        [
            SI_SPELL_FLAG,
            SI_SPELL_SPEED_A,
            SI_SPELL_SPEED_H,
            SI_SPELL_DROP,
            SI_SPELL_FAVOR,
            SI_SPELL_TRACES,
            SI_SPELL_HONOR_199,
            SI_SPELL_REP,
        ]
        .into_iter()
        .map(|id| {
            if matches!(
                id,
                SI_SPELL_FLAG
                    | SI_SPELL_SPEED_A
                    | SI_SPELL_SPEED_H
                    | SI_SPELL_DROP
                    | SI_SPELL_FAVOR
            ) {
                aura_spell(id)
            } else {
                plain_spell(id)
            }
        })
        .collect()
    }

    fn test_infos() -> PlayerInfos {
        PlayerInfos::from_maps_for_tests(
            60,
            HashMap::from([(
                (1, 1),
                PlayerCreateInfo {
                    map_id: 0,
                    area_id: 14,
                    position_x: 100.0,
                    position_y: 200.0,
                    position_z: 10.0,
                    orientation: 0.0,
                    display_id_m: 51,
                    display_id_f: 52,
                },
            )]),
            HashMap::from([(
                1,
                vec![
                    PlayerClassLevelInfo {
                        basehealth: 100,
                        basemana: 100
                    };
                    60
                ],
            )]),
            HashMap::from([((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
            vec![400; 61],
        )
    }

    fn test_load(guid: u32, quest: Option<u32>) -> CharacterLoad {
        CharacterLoad {
            row: CharacterRow {
                guid,
                account: 42,
                name: format!("Si{guid}"),
                race: 1,
                class: 1,
                gender: 0,
                level: 60,
                xp: 0,
                money: 0,
                skin: 1,
                face: 2,
                hair_style: 3,
                hair_color: 4,
                facial_hair: 5,
                bank_bag_slots: 0,
                character_flags: 0,
                position_x: 100.0,
                position_y: 200.0,
                position_z: 10.0,
                map: 1,
                orientation: 0.0,
                known_taxi_mask: String::new(),
                played_time_total: 60,
                played_time_level: 60,
                rest_bonus: 0.0,
                logout_time: 0,
                reset_talents_multiplier: 0,
                reset_talents_time: 0,
                transport_guid: 0,
                transport_x: 0.0,
                transport_y: 0.0,
                transport_z: 0.0,
                transport_o: 0.0,
                extra_flags: 0,
                stable_slots: 0,
                death_expire_time: 0,
                current_taxi_path: String::new(),
                honor_rank_points: 0.0,
                honor_highest_rank: 0,
                honor_standing: 0,
                honor_last_week_hk: 0,
                honor_last_week_cp: 0.0,
                honor_stored_hk: 0,
                honor_stored_dk: 0,
                watched_faction: -1,
                drunk: 0,
                health: 100,
                power1: 100,
                power2: 0,
                power3: 0,
                power4: 0,
                power5: 0,
                explored_zones: String::new(),
                ammo_id: 0,
                action_bars: 0,
                world_phase_mask: 0,
                create_time: 0,
                instance: 0,
                zone: SI_ZONE_SILITHUS,
            },
            group_id: None,
            bound_instances: vec![],
            auras: vec![],
            spells: vec![],
            quest_status: quest
                .map(|q| {
                    vec![QuestStatusRow {
                        quest: q,
                        status: 3,
                        rewarded: 0,
                        explored: 0,
                        timer: 0,
                        mob_count: [0; 4],
                        item_count: [0; 4],
                        reward_choice: 0,
                    }]
                })
                .unwrap_or_default(),
            honor_cp: vec![],
            reputation: vec![],
            inventory: vec![],
            item_loot: vec![],
            actions: vec![],
            social: vec![],
            homebind: None,
            spell_cooldowns: vec![],
            guild: None,
            bg_data: None,
            account_data: vec![],
            skills: vec![],
            mails: vec![],
            mailed_items: vec![],
            forgotten_skills: vec![],
            pets: Vec::new(),
        }
    }

    fn si_quest() -> QuestTemplate {
        QuestTemplate {
            entry: SI_QUEST_CM_A,
            method: 2,
            req_creature_or_go_ids: [i32::try_from(SI_QUEST_CM_A).unwrap_or(0), 0, 0, 0],
            req_creature_or_go_counts: [100, 0, 0, 0],
            ..Default::default()
        }
    }

    /// Map 1 with SI spells, the Alliance turn-in quest
    /// and (optionally) the own-faction trigger row.
    fn si_map(trigger: Option<AreaTriggerEntry>, quest: bool) -> Map {
        let mut stores = WorldStores::empty();
        for spell in si_spells() {
            stores.spells.spells.insert(spell.id, spell);
        }
        if quest {
            stores.quest_templates = QuestTemplates::from_entries([si_quest()]);
        }
        if let Some(entry) = trigger {
            stores.area_triggers.insert_trigger_for_tests(entry);
        }
        Map::new(
            SI_MAP_ID,
            0,
            MapConfig::default(),
            Arc::new(stores),
            WorldRng::from_seed(7),
        )
    }

    fn trigger_row(id: u32, x: f32, y: f32, z: f32) -> AreaTriggerEntry {
        AreaTriggerEntry {
            id,
            name: format!("si-{id}"),
            map_id: SI_MAP_ID,
            x,
            y,
            z,
            radius: 5.0,
            box_x: 0.0,
            box_y: 0.0,
            box_z: 0.0,
            box_orientation: 0.0,
            cooldown: 0,
            condition_id: 0,
            script_id: 0,
            script_name: String::new(),
        }
    }

    /// Materialized level-60 player inserted at the SI
    /// zone with an explicit team.
    fn add_si_player(map: &mut Map, guid_low: u32, team: Team, quest: bool) -> ObjectGuid {
        let infos = test_infos();
        let inputs = MaterializeInputs {
            infos: &infos,
            race: None,
            power_type: 1,
            display_scale: 1.0,
            model: None,
            saved_map_known: true,
            saved_map_is_instance: false,
            max_money: MAX_MONEY_AMOUNT,
            now_unix: 1_800_000_000,
            rest_rates: RestRates::default(),
            skills: None,
            item_prototypes: None,
            quest_templates: None,
            taxi: None,
        };
        let mut player =
            Player::materialize(test_load(guid_low, quest.then_some(SI_QUEST_CM_A)), &inputs)
                .expect("player");
        player.team = team;
        let guid = player.unit.guid();
        let position = player.unit.world.position();
        map.players.insert(
            guid,
            crate::map::PlayerEntry {
                player,
                visible: HashSet::new(),
                last_notified: position,
                scan_due: false,
                zone_id: SI_ZONE_SILITHUS,
                area_id: 0,
            },
        );
        guid
    }

    fn give_flag(map: &mut Map, player: ObjectGuid) {
        map.apply_aura_by_id(player, SI_SPELL_FLAG, player);
        assert!(
            map.aura_holders(player)
                .iter()
                .any(|h| h.spell_id == SI_SPELL_FLAG),
            "silithyst flag lands"
        );
    }

    fn has_aura(map: &Map, player: ObjectGuid, spell: u32) -> bool {
        map.aura_holders(player).iter().any(|h| h.spell_id == spell)
    }

    /// (field, value) of every `SMSG_UPDATE_WORLD_STATE`
    /// in drained events.
    fn update_states(map: &mut Map) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for event in map.drain_events() {
            if let MapEvent::Packet { packet, .. } = event
                && packet.opcode()
                    == mangos_protocol::opcode_table::opcode_id::SMSG_UPDATE_WORLD_STATE
            {
                let mut reader = packet.reader();
                let field = reader.read_u32_le().expect("field");
                let value = reader.read_u32_le().expect("value");
                out.push((field, value));
            }
        }
        out
    }

    #[test]
    fn si_zone_sets_and_fill_initial_match_fork() {
        assert!(SiState::is_si_zone(1377));
        assert!(SiState::is_si_zone(3428));
        assert!(SiState::is_si_zone(3429));
        assert!(!SiState::is_si_zone(139));
        assert!(!SiState::is_si_zone(0));
        let state = SiState::default();
        assert_eq!(state.max_resources, SI_MAX_DEFAULT);
        assert_eq!(state.last_controller, Team::None);
        assert_eq!(state.fill_initial(), [(2313, 0), (2314, 0), (2317, 200)]);
    }

    #[test]
    fn milestone_yells_match_fork_bct_ids() {
        assert_eq!(
            SiState::milestone_yell(50, 200, true),
            Some((SI_ANNOUNCER_A, 13479))
        );
        assert_eq!(
            SiState::milestone_yell(100, 200, true),
            Some((SI_ANNOUNCER_A, 13480))
        );
        assert_eq!(
            SiState::milestone_yell(150, 200, true),
            Some((SI_ANNOUNCER_A, 13481))
        );
        assert_eq!(
            SiState::milestone_yell(200, 200, true),
            Some((SI_ANNOUNCER_A, 13470))
        );
        assert_eq!(
            SiState::milestone_yell(50, 200, false),
            Some((SI_ANNOUNCER_H, 13476))
        );
        assert_eq!(
            SiState::milestone_yell(100, 200, false),
            Some((SI_ANNOUNCER_H, 13477))
        );
        assert_eq!(
            SiState::milestone_yell(150, 200, false),
            Some((SI_ANNOUNCER_H, 13478))
        );
        assert_eq!(
            SiState::milestone_yell(200, 200, false),
            Some((SI_ANNOUNCER_H, 13469))
        );
        assert_eq!(SiState::milestone_yell(49, 200, true), None);
        assert_eq!(SiState::milestone_yell(201, 200, true), None);
        assert_eq!(SiState::milestone_yell(1, 0, true), None);
        assert_eq!(SiState::needed_bags(14), 0);
        assert_eq!(SiState::needed_bags(15), 1);
        assert_eq!(SiState::needed_bags(200), 13);
    }

    #[test]
    fn deposit_without_flag_returns_false() {
        let mut map = si_map(None, false);
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        assert!(!map.si_handle_area_trigger(player, SI_AREATRIGGER_A));
        assert_eq!(map.outdoor_si.gathered_a, 0);
        assert!(map.drain_events().is_empty());
    }

    #[test]
    fn unknown_trigger_returns_false() {
        let mut map = si_map(None, false);
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        give_flag(&mut map, player);
        assert!(!map.si_handle_area_trigger(player, 9999));
        assert!(has_aura(&map, player, SI_SPELL_FLAG));
    }

    #[test]
    fn wrong_team_turnin_is_swallowed() {
        let mut map = si_map(None, false);
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        give_flag(&mut map, player);
        assert!(map.si_handle_area_trigger(player, SI_AREATRIGGER_H));
        assert_eq!(map.outdoor_si.gathered_h, 0);
        assert!(has_aura(&map, player, SI_SPELL_FLAG));
        assert!(update_states(&mut map).is_empty());
    }

    #[test]
    fn deposit_increments_rewards_and_broadcasts() {
        let mut map = si_map(None, true);
        let player = add_si_player(&mut map, 11, Team::Alliance, true);
        give_flag(&mut map, player);
        assert!(map.si_handle_area_trigger(player, SI_AREATRIGGER_A));
        assert_eq!(map.outdoor_si.gathered_a, 1);
        assert_eq!(map.outdoor_si.gathered_h, 0);
        assert!(!has_aura(&map, player, SI_SPELL_FLAG));
        // Rewards: traces + honor + rep resolve (plain
        // fixtures), quest credit lands on the SI quest.
        let row = map
            .players
            .get(&player)
            .expect("player")
            .player
            .quest_row(SI_QUEST_CM_A)
            .expect("quest row");
        assert_eq!(row.mob_count[0], 1);
        let states = update_states(&mut map);
        assert!(states.contains(&(2313, 1)));
        assert!(states.contains(&(2314, 0)));
        assert!(states.contains(&(2317, 200)));
    }

    #[test]
    fn capture_resets_buffs_winner_and_zone_text() {
        let mut map = si_map(None, false);
        let alliance = add_si_player(&mut map, 11, Team::Alliance, false);
        let horde = add_si_player(&mut map, 12, Team::Horde, false);
        give_flag(&mut map, alliance);
        map.outdoor_si.gathered_a = 199;
        assert!(map.si_handle_area_trigger(alliance, SI_AREATRIGGER_A));
        assert_eq!(map.outdoor_si.gathered_a, 0);
        assert_eq!(map.outdoor_si.gathered_h, 0);
        assert_eq!(map.outdoor_si.last_controller, Team::Alliance);
        assert!(has_aura(&map, alliance, SI_SPELL_FAVOR));
        assert!(!has_aura(&map, horde, SI_SPELL_FAVOR));
        let zone_text: Vec<(u32, i32)> = map
            .drain_events()
            .into_iter()
            .filter_map(|event| match event {
                MapEvent::OutdoorZoneText { zone_id, entry, .. } => Some((zone_id, entry)),
                _ => None,
            })
            .collect();
        assert_eq!(zone_text, [(SI_ZONE_SILITHUS, SI_LANG_CAPTURE_A)]);
    }

    #[test]
    fn leave_sends_zeroes_and_enter_buffs_controller() {
        let mut map = si_map(None, false);
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        map.outdoor_si.last_controller = Team::Alliance;
        map.si_on_zone_change(player, SI_ZONE_SILITHUS, 0);
        let states = update_states(&mut map);
        assert!(states.contains(&(2313, 0)));
        assert!(states.contains(&(2314, 0)));
        assert!(states.contains(&(2317, 0)));
        map.si_on_zone_change(player, 0, SI_ZONE_SILITHUS);
        assert!(has_aura(&map, player, SI_SPELL_FAVOR));
    }

    #[test]
    fn init_states_carry_si_triple_in_si_zone() {
        let map = si_map(None, false);
        let packet = map.si_init_world_states(SI_ZONE_SILITHUS).expect("si init");
        assert_eq!(
            packet.opcode(),
            mangos_protocol::opcode_table::opcode_id::SMSG_INIT_WORLD_STATES
        );
        let mut reader = packet.reader();
        assert_eq!(reader.read_u32_le().expect("map"), SI_MAP_ID);
        assert_eq!(reader.read_u32_le().expect("zone"), SI_ZONE_SILITHUS);
        let count = reader.read_u16_le().expect("count") as usize;
        let mut pairs = Vec::with_capacity(count);
        for _ in 0..count {
            pairs.push((
                reader.read_u32_le().expect("state"),
                reader.read_u32_le().expect("value"),
            ));
        }
        assert!(pairs.contains(&(2313, 0)));
        assert!(pairs.contains(&(2314, 0)));
        assert!(pairs.contains(&(2317, 200)));
        assert!(map.si_init_world_states(139).is_none());
    }

    #[test]
    fn drop_flag_far_from_trigger_casts_drop() {
        let mut map = si_map(
            Some(trigger_row(SI_AREATRIGGER_A, 5000.0, 5000.0, 0.0)),
            false,
        );
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        give_flag(&mut map, player);
        assert!(map.si_handle_drop_flag(player, SI_SPELL_FLAG));
        assert!(has_aura(&map, player, SI_SPELL_DROP));
    }

    #[test]
    fn drop_flag_near_own_trigger_is_suppressed() {
        let mut map = si_map(
            Some(trigger_row(SI_AREATRIGGER_A, 100.0, 200.0, 10.0)),
            false,
        );
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        give_flag(&mut map, player);
        assert!(!map.si_handle_drop_flag(player, SI_SPELL_FLAG));
        assert!(!has_aura(&map, player, SI_SPELL_DROP));
    }

    #[test]
    fn silithyst_aura_apply_buffs_and_cancel_drops() {
        let mut map = si_map(
            Some(trigger_row(SI_AREATRIGGER_A, 5000.0, 5000.0, 0.0)),
            false,
        );
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        map.si_silithyst_aura(
            player,
            SI_SPELL_FLAG,
            true,
            crate::spells::AuraRemoveMode::Default,
        );
        assert!(has_aura(&map, player, SI_SPELL_SPEED_A));
        assert!(!has_aura(&map, player, SI_SPELL_SPEED_H));
        give_flag(&mut map, player);
        map.cancel_aura(player, SI_SPELL_FLAG);
        assert!(!has_aura(&map, player, SI_SPELL_FLAG));
        assert!(!has_aura(&map, player, SI_SPELL_SPEED_A));
        assert!(has_aura(&map, player, SI_SPELL_DROP));
    }

    #[test]
    fn silithyst_aura_deposit_removal_does_not_drop() {
        let mut map = si_map(
            Some(trigger_row(SI_AREATRIGGER_A, 5000.0, 5000.0, 0.0)),
            false,
        );
        let player = add_si_player(&mut map, 11, Team::Alliance, false);
        give_flag(&mut map, player);
        map.remove_auras_due_to_spell(player, SI_SPELL_FLAG);
        assert!(!has_aura(&map, player, SI_SPELL_DROP));
    }
}
