//! DB script engine interpreter (Slice 6b):
//! `Map::ScriptCommandStartDirect` + the command
//! subset, `ScriptMgr` target resolution
//! (`GetTargetByType`), the Conditions evaluator, and
//! the delayed generic-script schedule
//! (`Map::ScriptsStart`/`ScriptsProcess`). `EventAI`
//! drives the direct path from `ai.rs`.
//!
//! Recorded subset deviations (full list in
//! `docs/rust-migration-evidence/PHASE4-PROGRESS.md`):
//! quest/item/taxi/gossip/game-event/map-event/
//! creature-group/guardian commands and temp summons
//! have no backing systems yet and fail with the C++
//! error-log shape; friendly-target selection uses the
//! slice hostility approximation (creature<->player);
//! saved variables and instance data are in-memory
//! only; `CONDITION_LOCAL_TIME` uses UTC instead of
//! the host local zone; line-of-sight is always true
//! (no vmaps, as in Slice 5).

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_scripts_engine::{
    CONDITION_FLAG_REVERSE_RESULT, CONDITION_FLAG_SWAP_TARGETS, ScriptInfo, chat_type, command,
    condition_type, script_target,
};

use mangos_entities::creature::SpawnRng;

use crate::map::{Map, MapEvent};
use crate::spells::{CastOptions, CastOutcome, CastTarget};

/// One scheduled generic-script step (`ScriptAction`).
pub(crate) struct ScheduledScript {
    pub(crate) info: ScriptInfo,
    pub(crate) source: ObjectGuid,
    pub(crate) target: ObjectGuid,
}

/// `CHAT_MSG_MONSTER_SAY`.
const CHAT_MSG_MONSTER_SAY: u8 = 0x0B;
/// `CHAT_MSG_MONSTER_YELL`.
const CHAT_MSG_MONSTER_YELL: u8 = 0x0C;
/// `CHAT_MSG_MONSTER_EMOTE`.
const CHAT_MSG_MONSTER_EMOTE: u8 = 0x0D;
/// `CHAT_MSG_MONSTER_WHISPER`.
const CHAT_MSG_MONSTER_WHISPER: u8 = 0x1A;
/// `CONFIG_FLOAT_LISTEN_RANGE_SAY` default.
const LISTEN_RANGE_SAY: f32 = 25.0;
/// `CONFIG_FLOAT_LISTEN_RANGE_YELL` default.
const LISTEN_RANGE_YELL: f32 = 300.0;
/// `CONFIG_FLOAT_LISTEN_RANGE_TEXTEMOTE` default.
const LISTEN_RANGE_TEXTEMOTE: f32 = 25.0;
/// `POWER_MANA`.
const POWER_MANA: u32 = 0;
/// `QUEST_STATUS_COMPLETE`.
const QUEST_STATUS_COMPLETE: i32 = 1;
/// `QUEST_STATUS_INCOMPLETE`.
const QUEST_STATUS_INCOMPLETE: i32 = 3;
/// `ALLIANCE` team faction id (`enum Team`).
const TEAM_ALLIANCE: u32 = 469;
/// `HORDE` team faction id.
const TEAM_HORDE: u32 = 67;
/// `sWorld.GetWowPatch()` for the 1.12 content patch
/// (progression index 10).
const WOW_PATCH: i32 = 10;
/// `GO_STATE_READY`.
const GO_STATE_READY: u32 = 1;
/// `UNIT_FLAG_IN_COMBAT` (also defined in ai/spells).
const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
/// Script cast flags (`eCastSpellFlags`).
const CF_INTERRUPT_PREVIOUS: u32 = 0x001;
const CF_TRIGGERED: u32 = 0x002;
const CF_AURA_NOT_PRESENT: u32 = 0x020;
const CF_ONLY_IN_MELEE: u32 = 0x040;
const CF_NOT_IN_MELEE: u32 = 0x080;
const CF_TARGET_CASTING: u32 = 0x100;

impl Map {
    // ------------------------------------------------ object helpers

    /// Position of any world object on this map.
    pub(crate) fn object_position(&self, guid: ObjectGuid) -> Option<Position> {
        if let Some(pos) = self.unit_position(guid) {
            return Some(pos);
        }
        self.gameobjects.get(&guid).map(|go| go.world.position())
    }

    /// Entry of any world object (`GetEntry`).
    pub(crate) fn object_entry(&self, guid: ObjectGuid) -> Option<u32> {
        if let Some(c) = self.creatures.get(&guid) {
            return Some(c.info.entry);
        }
        self.gameobjects.get(&guid).map(|go| {
            go.world
                .object
                .values()
                .get_u32(offsets::OBJECT_FIELD_ENTRY)
        })
    }

    /// World-object display name (`GetName`).
    #[must_use]
    pub fn object_name(&self, guid: ObjectGuid) -> String {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.world.name.clone();
        }
        if let Some(c) = self.creatures.get(&guid) {
            return c.info.name.clone();
        }
        if let Some(go) = self.gameobjects.get(&guid) {
            return go.world.name.clone();
        }
        String::new()
    }

    pub(crate) fn is_unit(&self, guid: ObjectGuid) -> bool {
        self.players.contains_key(&guid) || self.creatures.contains_key(&guid)
    }

    /// `GetDistance2d` between two objects' positions.
    fn object_dist2d(&self, a: ObjectGuid, b: ObjectGuid) -> f32 {
        let (Some(pa), Some(pb)) = (self.object_position(a), self.object_position(b)) else {
            return f32::MAX;
        };
        crate::visibility::dist2d(pa, pb)
    }

    /// Write one values word on any object kind;
    /// returns false when the object or field is out of
    /// range (`FIELD_SET` bounds: above
    /// `OBJECT_FIELD_ENTRY`, below the values count).
    pub(crate) fn object_set_field(&mut self, guid: ObjectGuid, field: u32, value: u32) -> bool {
        let Ok(field_u16) = u16::try_from(field) else {
            return false;
        };
        if field <= u32::from(offsets::OBJECT_FIELD_ENTRY) {
            return false;
        }
        if let Some(u) = self.unit_base_mut(guid) {
            if field_u16 >= u.values().count() {
                return false;
            }
            u.values().set_u32(field_u16, value);
            return true;
        }
        if let Some(go) = self.gameobjects.get_mut(&guid) {
            let values = go.world.object.values_mut();
            if field_u16 >= values.count() {
                return false;
            }
            values.set_u32(field_u16, value);
            return true;
        }
        false
    }

    pub(crate) fn object_get_field(&self, guid: ObjectGuid, field: u16) -> u32 {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.world.object.values().get_u32(field);
        }
        if let Some(c) = self.creatures.get(&guid) {
            return c.unit.world.object.values().get_u32(field);
        }
        if let Some(go) = self.gameobjects.get(&guid) {
            return go.world.object.values().get_u32(field);
        }
        0
    }

    /// Set one byte of a unit values word.
    pub(crate) fn unit_set_byte(&mut self, guid: ObjectGuid, field: u16, byte: usize, value: u8) {
        if let Some(u) = self.unit_base_mut(guid) {
            let word = u.values().get_u32(field);
            let shift = (byte % 4) * 8;
            let next = (word & !(0xFFu32 << shift)) | (u32::from(value) << shift);
            u.values().set_u32(field, next);
        }
    }

    // ------------------------------------------------ conditions

    /// `IsConditionSatisfied` (target/source as guids;
    /// `None` = null object).
    /// `IsConditionSatisfied` (actor-facing wrapper).
    #[must_use]
    pub fn condition_satisfied(
        &self,
        condition_id: u32,
        target: Option<ObjectGuid>,
        source: Option<ObjectGuid>,
    ) -> bool {
        self.condition_meets(condition_id, target, source, 0)
    }

    fn condition_meets(
        &self,
        condition_id: u32,
        target: Option<ObjectGuid>,
        source: Option<ObjectGuid>,
        depth: u32,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        let Some(cond) = self.stores.scripts.conditions.get(&condition_id).copied() else {
            return false;
        };
        let (target, source) = if cond.flags & CONDITION_FLAG_SWAP_TARGETS != 0 {
            (source, target)
        } else {
            (target, source)
        };
        let mut result = self.condition_evaluate(&cond, target, source, depth);
        if cond.flags & CONDITION_FLAG_REVERSE_RESULT != 0 {
            result = !result;
        }
        result
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::too_many_lines
    )]
    fn condition_evaluate(
        &self,
        cond: &mangos_scripts_engine::Condition,
        target: Option<ObjectGuid>,
        source: Option<ObjectGuid>,
        depth: u32,
    ) -> bool {
        let v = cond.values;
        let sub = |id: i32, map: &Map| -> bool {
            id > 0 && map.condition_meets(id as u32, target, source, depth + 1)
        };
        match cond.cond_type {
            condition_type::NOT => v[0] > 0 && !sub(v[0], self),
            condition_type::OR => {
                sub(v[0], self) || sub(v[1], self) || sub(v[2], self) || sub(v[3], self)
            }
            condition_type::AND => {
                (v[0] == 0 || sub(v[0], self))
                    && (v[1] == 0 || sub(v[1], self))
                    && (v[2] == 0 || sub(v[2], self))
                    && (v[3] == 0 || sub(v[3], self))
            }
            condition_type::NONE => true,
            condition_type::AURA => {
                let Some(t) = target else { return false };
                v[0] > 0
                    && self
                        .aura_holders(t)
                        .iter()
                        .any(|h| h.spell_id == v[0] as u32)
            }
            condition_type::TEAM => {
                let Some(t) = target.filter(|g| g.is_player()) else {
                    return false;
                };
                self.player_team_id(t) == v[0] as u32
            }
            condition_type::QUESTREWARDED => self.player_quest_rewarded(target, v[0]),
            condition_type::QUESTTAKEN => self.player_quest_taken(target, v[0], v[1]),
            condition_type::QUEST_NONE => {
                target.is_some_and(ObjectGuid::is_player)
                    && !self.player_quest_taken(target, v[0], 0)
                    && !self.player_quest_rewarded(target, v[0])
            }
            condition_type::SAVED_VARIABLE => {
                if v[0] < 0 {
                    return false;
                }
                let data = self
                    .saved_variables
                    .get(&(v[0] as u32))
                    .copied()
                    .unwrap_or(0) as i32;
                match v[2] {
                    0 => data == v[1],
                    1 => data >= v[1],
                    2 => data <= v[1],
                    _ => false,
                }
            }
            condition_type::SOURCE_ENTRY => {
                let Some(s) = source else { return false };
                let Some(entry) = self.object_entry(s) else {
                    return false;
                };
                v.iter().any(|&x| x > 0 && entry == x as u32)
            }
            condition_type::NEARBY_CREATURE => {
                let Some(t) = target else { return false };
                let Some(tp) = self.object_position(t) else {
                    return false;
                };
                let radius = v[1] as f32;
                self.creatures.values().any(|c| {
                    c.info.entry == v[0] as u32
                        && (v[2] != 0 || c.unit.is_alive())
                        && (v[3] == 0 || c.unit.guid() != t)
                        && crate::visibility::dist2d(tp, c.unit.world.position()) <= radius
                })
            }
            condition_type::NEARBY_GAMEOBJECT => {
                let Some(t) = target else { return false };
                let Some(tp) = self.object_position(t) else {
                    return false;
                };
                let radius = v[1] as f32;
                self.gameobjects.values().any(|go| {
                    go.world
                        .object
                        .values()
                        .get_u32(offsets::OBJECT_FIELD_ENTRY)
                        == v[0] as u32
                        && crate::visibility::dist2d(tp, go.world.position()) <= radius
                })
            }
            condition_type::WOW_PATCH => match v[1] {
                0 => WOW_PATCH == v[0],
                1 => WOW_PATCH >= v[0],
                2 => WOW_PATCH <= v[0],
                _ => false,
            },
            condition_type::IS_PLAYER => {
                target.is_some_and(|t| t.is_player() && self.players.contains_key(&t))
            }
            condition_type::MAP_ID => v[0] >= 0 && self.map_id == v[0] as u32,
            34 => {
                // CONDITION_INSTANCE_DATA
                if v[0] < 0 {
                    return false;
                }
                let value = self
                    .instance_get_data(v[0] as u32)
                    .or_else(|| self.instance_data.get(&(v[0] as u32)).copied());
                let Some(value) = value else { return false };
                let value = i64::from(value);
                match v[2] {
                    0 => value == i64::from(v[1]),
                    1 => value >= i64::from(v[1]),
                    2 => value <= i64::from(v[1]),
                    _ => false,
                }
            }
            condition_type::LINE_OF_SIGHT => target.is_some() && source.is_some(),
            condition_type::DISTANCE_TO_TARGET => {
                let (Some(s), Some(t)) = (source, target) else {
                    return false;
                };
                let distance = self.object_dist2d(s, t) as i64;
                match v[1] {
                    0 => distance == i64::from(v[0]),
                    1 => distance >= i64::from(v[0]),
                    2 => distance <= i64::from(v[0]),
                    _ => false,
                }
            }
            condition_type::IS_MOVING => target.is_some_and(|t| self.unit_is_moving(t)),
            condition_type::HEALTH_PERCENT | condition_type::MANA_PERCENT => {
                let Some(t) = target else { return false };
                let (cur, max) = if cond.cond_type == condition_type::HEALTH_PERCENT {
                    (
                        self.object_get_field(t, offsets::UNIT_FIELD_HEALTH),
                        self.object_get_field(t, offsets::UNIT_FIELD_MAXHEALTH),
                    )
                } else {
                    (
                        self.unit_power_of(t, POWER_MANA),
                        self.unit_max_power_of(t, POWER_MANA),
                    )
                };
                if max == 0 {
                    return false;
                }
                let percent = i64::from(cur) * 100 / i64::from(max);
                match v[1] {
                    0 => percent == i64::from(v[0]),
                    1 => percent >= i64::from(v[0]),
                    2 => percent <= i64::from(v[0]),
                    _ => false,
                }
            }
            condition_type::IS_IN_COMBAT => target.is_some_and(|t| {
                self.object_get_field(t, offsets::UNIT_FIELD_FLAGS) & UNIT_FLAG_IN_COMBAT != 0
            }),
            condition_type::IS_ALIVE => target.is_some_and(|t| self.unit_is_alive(t)),
            condition_type::DB_GUID => {
                let Some(s) = source else { return false };
                if !s.is_creature() && !s.is_game_object() {
                    return false;
                }
                let db_guid = s.counter();
                db_guid != 0 && v.iter().any(|&x| x > 0 && db_guid == x as u32)
            }
            condition_type::LOCAL_TIME => {
                // UTC hour/min from the map unix clock
                // (C++ uses host localtime; recorded).
                let secs = self.now_unix % 86_400;
                let hour = (secs / 3600) as i32;
                let min = ((secs % 3600) / 60) as i32;
                hour >= v[0]
                    && (hour > v[0] || min >= v[1])
                    && hour <= v[2]
                    && (hour < v[2] || min <= v[3])
            }
            condition_type::DISTANCE_TO_POSITION => {
                let Some(t) = target else { return false };
                let Some(tp) = self.object_position(t) else {
                    return false;
                };
                let dx = tp.x - v[0] as f32;
                let dy = tp.y - v[1] as f32;
                let dz = tp.z - v[2] as f32;
                (dx * dx + dy * dy + dz * dz).sqrt() <= v[3] as f32
            }
            _ => false,
        }
    }

    /// `Player::GetTeam()` as the numeric team id.
    fn player_team_id(&self, guid: ObjectGuid) -> u32 {
        let Some(entry) = self.players.get(&guid) else {
            return 0;
        };
        match entry.player.unit.race() {
            // Human, Dwarf, Night Elf, Gnome.
            1 | 3 | 4 | 7 => TEAM_ALLIANCE,
            // Orc, Undead, Tauren, Troll.
            2 | 5 | 6 | 8 => TEAM_HORDE,
            _ => 0,
        }
    }

    fn player_quest_rewarded(&self, target: Option<ObjectGuid>, quest: i32) -> bool {
        if quest <= 0 {
            return false;
        }
        target
            .and_then(|t| self.players.get(&t))
            .is_some_and(|entry| {
                entry
                    .player
                    .deferred
                    .quest_status
                    .iter()
                    .any(|q| q.quest == quest as u32 && q.rewarded != 0)
            })
    }

    /// `Player::IsCurrentQuest(quest, completedOrNot)`.
    fn player_quest_taken(&self, target: Option<ObjectGuid>, quest: i32, mode: i32) -> bool {
        if quest <= 0 {
            return false;
        }
        target
            .and_then(|t| self.players.get(&t))
            .and_then(|entry| {
                entry
                    .player
                    .deferred
                    .quest_status
                    .iter()
                    .find(|q| q.quest == quest as u32)
            })
            .is_some_and(|q| match mode {
                1 => q.status == QUEST_STATUS_INCOMPLETE,
                2 => q.status == QUEST_STATUS_COMPLETE && q.rewarded == 0,
                _ => {
                    q.status == QUEST_STATUS_INCOMPLETE
                        || (q.status == QUEST_STATUS_COMPLETE && q.rewarded == 0)
                }
            })
    }

    pub(crate) fn unit_is_moving(&self, guid: ObjectGuid) -> bool {
        use mangos_movement::movement_info::MovementFlags;
        const MOVE_MASK: u32 = MovementFlags::FORWARD
            | MovementFlags::BACKWARD
            | MovementFlags::STRAFE_LEFT
            | MovementFlags::STRAFE_RIGHT
            | MovementFlags::JUMPING
            | MovementFlags::FALLING_FAR;
        if guid.is_creature() {
            // Generator-active approximation of
            // `IsMoving` (spline-flag test).
            return self.motion_type_of(guid) != crate::motion::IDLE_MOTION_TYPE;
        }
        self.players
            .get(&guid)
            .is_some_and(|entry| entry.player.unit.world.movement_info.flags.0 & MOVE_MASK != 0)
    }

    /// Mana-only power read (the conditions subset
    /// only ever asks for `POWER_MANA` = field pair 1).
    fn unit_power_of(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == POWER_MANA {
            self.object_get_field(guid, offsets::UNIT_FIELD_POWER1)
        } else {
            0
        }
    }

    fn unit_max_power_of(&self, guid: ObjectGuid, power_type: u32) -> u32 {
        if power_type == POWER_MANA {
            self.object_get_field(guid, offsets::UNIT_FIELD_MAXPOWER1)
        } else {
            0
        }
    }

    // ------------------------------------------------ target resolution

    /// `Unit::GetVictim` (current melee victim).
    pub(crate) fn unit_victim(&self, guid: ObjectGuid) -> Option<ObjectGuid> {
        if let Some(c) = self.creatures.get(&guid) {
            return c.unit.melee_victim;
        }
        self.players
            .get(&guid)
            .and_then(|e| e.player.unit.melee_victim)
    }

    /// Threat refs of a creature, highest first
    /// (`SelectAttackingTarget` works on the sorted
    /// list; offline/dead refs filtered like the host
    /// validity checks).
    pub(crate) fn threat_targets_sorted(&self, creature: ObjectGuid) -> Vec<ObjectGuid> {
        let Some(ai) = self.ai.get(&creature) else {
            return Vec::new();
        };
        let mut refs: Vec<(ObjectGuid, f32)> = ai
            .threat
            .threat_list()
            .iter()
            .filter(|r| r.online() && self.unit_is_alive(r.target()))
            .map(|r| (r.target(), r.threat()))
            .collect();
        refs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        refs.into_iter().map(|(g, _)| g).collect()
    }

    /// Nearest creature by entry around a searcher
    /// (`FindNearestCreature(entry, range, alive)`).
    pub(crate) fn nearest_creature_with_entry(
        &self,
        searcher: ObjectGuid,
        entry: u32,
        radius: f32,
    ) -> Option<ObjectGuid> {
        let sp = self.object_position(searcher)?;
        self.creatures
            .values()
            .filter(|c| c.info.entry == entry && c.unit.is_alive())
            .map(|c| {
                (
                    c.unit.guid(),
                    crate::visibility::dist2d(sp, c.unit.world.position()),
                )
            })
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(g, _)| g)
    }

    pub(crate) fn nearest_gameobject_with_entry(
        &self,
        searcher: ObjectGuid,
        entry: u32,
        radius: f32,
    ) -> Option<ObjectGuid> {
        let sp = self.object_position(searcher)?;
        self.gameobjects
            .iter()
            .filter(|(_, go)| {
                go.world
                    .object
                    .values()
                    .get_u32(offsets::OBJECT_FIELD_ENTRY)
                    == entry
            })
            .map(|(g, go)| (*g, crate::visibility::dist2d(sp, go.world.position())))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(g, _)| g)
    }

    /// Friendly units for the script target family:
    /// same-kind units (the slice hostility model is
    /// player<->creature, so a creature's friendlies
    /// are creatures — self included).
    fn friendly_candidates(&self, source: ObjectGuid, radius: f32) -> Vec<ObjectGuid> {
        let Some(sp) = self.object_position(source) else {
            return Vec::new();
        };
        if source.is_player() {
            return vec![source];
        }
        self.creatures
            .values()
            .filter(|c| {
                c.unit.is_alive()
                    && crate::visibility::dist2d(sp, c.unit.world.position()) <= radius
            })
            .map(|c| c.unit.guid())
            .collect()
    }

    /// `GetTargetByType` subset. `source`/`target` are
    /// the pre-resolution pair (either may be missing).
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn get_target_by_type(
        &mut self,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
        target_type: u8,
        param1: u32,
        param2: u32,
        spell_id: Option<u32>,
    ) -> Option<ObjectGuid> {
        let src = source;
        match target_type {
            script_target::PROVIDED_TARGET => target,
            script_target::HOSTILE => src.and_then(|s| self.unit_victim(s)),
            script_target::HOSTILE_SECOND_AGGRO => {
                let s = src.filter(|g| g.is_creature())?;
                self.threat_targets_sorted(s).get(1).copied()
            }
            script_target::HOSTILE_LAST_AGGRO => {
                let s = src.filter(|g| g.is_creature())?;
                self.threat_targets_sorted(s).last().copied()
            }
            script_target::HOSTILE_RANDOM | script_target::HOSTILE_RANDOM_NOT_TOP => {
                let s = src.filter(|g| g.is_creature())?;
                let mut list = self.threat_targets_sorted(s);
                if target_type == script_target::HOSTILE_RANDOM_NOT_TOP && list.len() > 1 {
                    list.remove(0);
                }
                self.pick_random(&list)
            }
            script_target::HOSTILE_NEAREST | script_target::HOSTILE_FARTHEST => {
                let s = src.filter(|g| g.is_creature())?;
                let list = self.threat_targets_sorted(s);
                let sp = self.object_position(s)?;
                let pick = if target_type == script_target::HOSTILE_NEAREST {
                    list.iter().min_by(|a, b| {
                        self.dist_from(sp, **a)
                            .partial_cmp(&self.dist_from(sp, **b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                } else {
                    list.iter().max_by(|a, b| {
                        self.dist_from(sp, **a)
                            .partial_cmp(&self.dist_from(sp, **b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                };
                pick.copied()
            }
            script_target::OWNER_OR_SELF => src,
            script_target::NEAREST_CREATURE_WITH_ENTRY => {
                let searcher = src.or(target)?;
                #[allow(clippy::cast_precision_loss)]
                let radius = param2 as f32;
                self.nearest_creature_with_entry(searcher, param1, radius)
            }
            script_target::RANDOM_CREATURE_WITH_ENTRY => {
                let searcher = src.or(target)?;
                let sp = self.object_position(searcher)?;
                #[allow(clippy::cast_precision_loss)]
                let radius = param2 as f32;
                let list: Vec<ObjectGuid> = self
                    .creatures
                    .values()
                    .filter(|c| {
                        c.info.entry == param1
                            && c.unit.is_alive()
                            && c.unit.guid() != searcher
                            && crate::visibility::dist2d(sp, c.unit.world.position()) <= radius
                    })
                    .map(|c| c.unit.guid())
                    .collect();
                self.pick_random(&list)
            }
            script_target::CREATURE_WITH_GUID => self
                .creatures
                .keys()
                .find(|g| g.counter() == param1)
                .copied(),
            script_target::CREATURE_FROM_INSTANCE_DATA => {
                if let Some(guid) = self.instance_get_data64(param1) {
                    return self.creatures.contains_key(&guid).then_some(guid);
                }
                let raw = self.instance_data64.get(&param1).copied()?;
                let guid = ObjectGuid::from_raw(raw);
                self.creatures.contains_key(&guid).then_some(guid)
            }
            script_target::NEAREST_GAMEOBJECT_WITH_ENTRY => {
                let searcher = src.or(target)?;
                #[allow(clippy::cast_precision_loss)]
                let radius = param2 as f32;
                self.nearest_gameobject_with_entry(searcher, param1, radius)
            }
            script_target::RANDOM_GAMEOBJECT_WITH_ENTRY => {
                let searcher = src.or(target)?;
                let sp = self.object_position(searcher)?;
                #[allow(clippy::cast_precision_loss)]
                let radius = param2 as f32;
                let list: Vec<ObjectGuid> = self
                    .gameobjects
                    .iter()
                    .filter(|(_, go)| {
                        go.world
                            .object
                            .values()
                            .get_u32(offsets::OBJECT_FIELD_ENTRY)
                            == param1
                            && crate::visibility::dist2d(sp, go.world.position()) <= radius
                    })
                    .map(|(g, _)| *g)
                    .collect();
                self.pick_random(&list)
            }
            script_target::GAMEOBJECT_WITH_GUID => self
                .gameobjects
                .keys()
                .find(|g| g.counter() == param1)
                .copied(),
            script_target::GAMEOBJECT_FROM_INSTANCE_DATA => {
                if let Some(guid) = self.instance_get_data64(param1) {
                    return self.gameobjects.contains_key(&guid).then_some(guid);
                }
                let raw = self.instance_data64.get(&param1).copied()?;
                let guid = ObjectGuid::from_raw(raw);
                self.gameobjects.contains_key(&guid).then_some(guid)
            }
            script_target::FRIENDLY => {
                let s = src?;
                let radius = self.friendly_radius(param1, spell_id);
                let mut list = self.friendly_candidates(s, radius);
                if param2 != 0 {
                    list.retain(|g| Some(*g) != target);
                }
                self.pick_random(&list)
            }
            script_target::FRIENDLY_INJURED | script_target::FRIENDLY_INJURED_EXCEPT => {
                let s = src?;
                let radius = self.friendly_radius(param1, spell_id);
                self.lowest_hp_friendly(s, radius, param2, target)
            }
            script_target::FRIENDLY_MISSING_BUFF | script_target::FRIENDLY_MISSING_BUFF_EXCEPT => {
                let s = src?;
                let radius = self.friendly_radius(param1, spell_id);
                let exclude =
                    (target_type == script_target::FRIENDLY_MISSING_BUFF_EXCEPT).then_some(target);
                let list: Vec<ObjectGuid> = self
                    .friendly_candidates(s, radius)
                    .into_iter()
                    .filter(|g| Some(*g) != exclude.flatten())
                    .filter(|g| !self.aura_holders(*g).iter().any(|h| h.spell_id == param2))
                    .collect();
                self.pick_random(&list)
            }
            script_target::FRIENDLY_CC => {
                // No CC auras are modeled (Slice 5), so
                // nobody qualifies.
                None
            }
            script_target::NEAREST_PLAYER
            | script_target::NEAREST_HOSTILE_PLAYER
            | script_target::NEAREST_FRIENDLY_PLAYER => {
                let searcher = src.or(target)?;
                let sp = self.object_position(searcher)?;
                #[allow(clippy::cast_precision_loss)]
                let radius = if param1 == 0 { 30.0 } else { param1 as f32 };
                self.players
                    .iter()
                    .filter(|(_, e)| e.player.unit.is_alive())
                    .map(|(g, e)| {
                        (
                            *g,
                            crate::visibility::dist2d(sp, e.player.unit.world.position()),
                        )
                    })
                    .filter(|(_, d)| *d <= radius)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(g, _)| g)
            }
            _ => None,
        }
    }

    fn dist_from(&self, from: Position, guid: ObjectGuid) -> f32 {
        self.object_position(guid)
            .map_or(f32::MAX, |p| crate::visibility::dist2d(from, p))
    }

    fn friendly_radius(&self, param1: u32, spell_id: Option<u32>) -> f32 {
        if param1 != 0 {
            #[allow(clippy::cast_precision_loss)]
            return param1 as f32;
        }
        if let Some(spell) = spell_id.and_then(|id| self.stores.spells.get(id)) {
            return spell.range_max(&self.stores.spells.tables);
        }
        30.0
    }

    /// `FindLowestHpFriendlyUnit(range, hpDeficit,
    /// alive, except)`.
    pub(crate) fn lowest_hp_friendly(
        &self,
        source: ObjectGuid,
        radius: f32,
        hp_deficit: u32,
        except: Option<ObjectGuid>,
    ) -> Option<ObjectGuid> {
        let deficit = if hp_deficit == 0 { 50 } else { hp_deficit };
        self.friendly_candidates(source, radius)
            .into_iter()
            .filter(|g| Some(*g) != except)
            .map(|g| {
                let hp = self.object_get_field(g, offsets::UNIT_FIELD_HEALTH);
                let max = self.object_get_field(g, offsets::UNIT_FIELD_MAXHEALTH);
                (g, max.saturating_sub(hp))
            })
            .filter(|(_, missing)| *missing >= deficit)
            .max_by_key(|(_, missing)| *missing)
            .map(|(g, _)| g)
    }

    fn pick_random(&mut self, list: &[ObjectGuid]) -> Option<ObjectGuid> {
        if list.is_empty() {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let idx = self.rng.urand(0, list.len() as u32 - 1) as usize;
        Some(list[idx])
    }

    /// `FindScriptFinalTargets`: initial swap, buddy
    /// (`target_type`) resolution, final swap, target-self.
    /// Returns None when a buddy search fails (command
    /// skipped, abort iff `SF_GENERAL_ABORT_ON_FAILURE`).
    fn resolve_final_targets(
        &mut self,
        info: &ScriptInfo,
        mut source: Option<ObjectGuid>,
        mut target: Option<ObjectGuid>,
    ) -> Option<(Option<ObjectGuid>, Option<ObjectGuid>)> {
        use mangos_scripts_engine::{
            SF_GENERAL_SWAP_FINAL_TARGETS, SF_GENERAL_SWAP_INITIAL_TARGETS, SF_GENERAL_TARGET_SELF,
        };
        if info.data_flags & SF_GENERAL_SWAP_INITIAL_TARGETS != 0 {
            std::mem::swap(&mut source, &mut target);
        }
        if info.target_type != 0 {
            let spell_id = (info.command == command::CAST_SPELL).then_some(info.datalong[0]);
            target = Some(self.get_target_by_type(
                source,
                target,
                info.target_type,
                info.target_param1,
                info.target_param2,
                spell_id,
            )?);
        }
        if info.data_flags & SF_GENERAL_SWAP_FINAL_TARGETS != 0 {
            std::mem::swap(&mut source, &mut target);
        }
        if info.data_flags & SF_GENERAL_TARGET_SELF != 0 {
            target = source;
        }
        Some((source, target))
    }

    /// `Map::ScriptCommandStartDirect`. Returns true
    /// when execution of the surrounding script must
    /// abort (command result or abort-on-failure).
    pub(crate) fn script_command_start_direct(
        &mut self,
        info: &ScriptInfo,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) -> bool {
        if info.command == command::DISABLED {
            return info.should_abort();
        }
        let Some((source, target)) = self.resolve_final_targets(info, source, target) else {
            return info.should_abort();
        };
        if info.condition_id != 0 && !self.condition_satisfied(info.condition_id, target, source) {
            return info.should_abort();
        }
        self.execute_script_command(info, source, target)
    }

    /// Run one script's steps directly (`EventAI`
    /// `ProcessAction`): stops at the first aborting
    /// step; returns whether any step failed/aborted
    /// (`scriptFailed` for `EFLAG_CHECK_RESULT`).
    pub(crate) fn run_script_direct(
        &mut self,
        script_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) -> bool {
        let Some(steps) = self
            .stores
            .scripts
            .creature_ai_scripts
            .get(&script_id)
            .cloned()
        else {
            return true;
        };
        self.run_steps_direct(&steps, source, target)
    }

    pub(crate) fn run_steps_direct(
        &mut self,
        steps: &[ScriptInfo],
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) -> bool {
        for step in steps {
            if self.script_command_start_direct(step, source, target) {
                return true;
            }
        }
        false
    }

    /// `Map::ScriptsStart` for generic scripts: every
    /// step schedules at now + delay seconds.
    pub(crate) fn scripts_start_generic(
        &mut self,
        script_id: u32,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) {
        let Some(steps) = self.stores.scripts.generic_scripts.get(&script_id).cloned() else {
            return;
        };
        self.schedule_steps(&steps, source, target);
    }

    pub(crate) fn schedule_steps(
        &mut self,
        steps: &[ScriptInfo],
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) {
        for step in steps {
            let due = u64::from(self.now_ms) + u64::from(step.delay) * 1000;
            self.script_seq += 1;
            self.script_schedule.insert(
                (due, self.script_seq),
                ScheduledScript {
                    info: step.clone(),
                    source: source.unwrap_or(ObjectGuid::EMPTY),
                    target: target.unwrap_or(ObjectGuid::EMPTY),
                },
            );
        }
    }

    /// `Map::ScriptsProcess`.
    pub(crate) fn scripts_process(&mut self) {
        loop {
            let Some((&key, _)) = self.script_schedule.iter().next() else {
                return;
            };
            if key.0 > u64::from(self.now_ms) {
                return;
            }
            let Some(step) = self.script_schedule.remove(&key) else {
                return;
            };
            // `FindScriptInitialTargets`: objects must
            // still be on this map.
            let present = |g: ObjectGuid, map: &Map| -> Option<ObjectGuid> {
                if g == ObjectGuid::EMPTY {
                    return None;
                }
                (map.players.contains_key(&g)
                    || map.creatures.contains_key(&g)
                    || map.gameobjects.contains_key(&g))
                .then_some(g)
            };
            let source = present(step.source, self);
            let target = present(step.target, self);
            if step.info.data_flags & mangos_scripts_engine::SF_GENERAL_SKIP_MISSING_TARGETS != 0
                && ((step.source != ObjectGuid::EMPTY && source.is_none())
                    || (step.target != ObjectGuid::EMPTY && target.is_none()))
            {
                continue;
            }
            if self.script_command_start_direct(&step.info.clone(), source, target) {
                // `TerminateScript`: drop remaining steps
                // of the same script id + guid pair.
                let (id, s, t) = (step.info.id, step.source, step.target);
                self.script_schedule
                    .retain(|_, v| !(v.info.id == id && v.source == s && v.target == t));
            }
        }
    }

    // ------------------------------------------------ command execution

    /// One `ScriptCommand_*` dispatch. True aborts the
    /// running script. Source/target are post-resolution.
    #[allow(
        clippy::too_many_lines,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::many_single_char_names,
        clippy::cast_possible_wrap
    )]
    fn execute_script_command(
        &mut self,
        info: &ScriptInfo,
        source: Option<ObjectGuid>,
        target: Option<ObjectGuid>,
    ) -> bool {
        let d = info.datalong;
        let ints = info.dataint;
        let abort = info.should_abort();
        let src_unit = source.filter(|g| self.is_unit(*g));
        let src_creature = source.filter(|g| self.creatures.contains_key(g));
        let tgt_unit = target.filter(|g| self.is_unit(*g));
        match info.command {
            command::TALK => {
                let Some(src) = source else { return abort };
                let mut text_id = ints[0];
                if ints[1] != 0 {
                    let mut count = 2;
                    while count < 4 && ints[count] != 0 {
                        count += 1;
                    }
                    let idx = self.rng.urand(0, count as u32 - 1) as usize;
                    text_id = ints[idx];
                }
                let override_type = if d[0] > 0 { d[0] as i32 } else { -1 };
                self.do_script_text(text_id, src, tgt_unit, override_type);
                false
            }
            command::EMOTE => {
                let Some(src) = src_unit else { return abort };
                let mut count = 1usize;
                while count < 4 && d[count] != 0 {
                    count += 1;
                }
                let emote_id = d[self.rng.urand(0, count as u32 - 1) as usize];
                // Targeted emotes: the 2 s face/pause
                // ceremony is not modeled; the emote
                // itself fires immediately (recorded).
                if emote_id == 0 {
                    let _ = self.object_set_field(src, u32::from(offsets::UNIT_NPC_EMOTESTATE), 0);
                } else {
                    self.handle_emote_command(src, emote_id);
                }
                false
            }
            command::FIELD_SET => {
                let Some(src) = source else { return abort };
                if self.object_set_field(src, d[0], d[1]) {
                    false
                } else {
                    abort
                }
            }
            command::MOVE_TO => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                let (mut x, mut y, mut z) = (info.x, info.y, info.z);
                match d[0] {
                    1 => {
                        let Some(tp) = target.and_then(|t| self.object_position(t)) else {
                            return abort;
                        };
                        x += tp.x;
                        y += tp.y;
                        z += tp.z;
                    }
                    2 => {
                        let Some(tp) = target.and_then(|t| self.object_position(t)) else {
                            return abort;
                        };
                        let sp = self.object_position(src).unwrap_or_default();
                        let angle = if info.o < 0.0 {
                            (sp.y - tp.y).atan2(sp.x - tp.x)
                        } else {
                            self.rng.frand(0.0, std::f32::consts::TAU)
                        };
                        x = tp.x + info.x * angle.cos();
                        y = tp.y + info.x * angle.sin();
                        z = tp.z;
                    }
                    3 => {
                        let center = Position { x, y, z, o: 0.0 };
                        let Some(p) = self.random_point(center, info.o) else {
                            return abort;
                        };
                        x = p.x;
                        y = p.y;
                        z = p.z;
                    }
                    _ => {}
                }
                let dest = Position {
                    x,
                    y,
                    z,
                    o: if info.o > 0.0 && d[0] != 3 {
                        info.o
                    } else {
                        0.0
                    },
                };
                let speed = if d[1] != 0 {
                    let sp = self.object_position(src).unwrap_or_default();
                    crate::visibility::dist2d(sp, dest) / (d[1] as f32 * 0.001)
                } else {
                    0.0
                };
                let orientation = if info.o > 0.0 && d[0] != 3 {
                    info.o
                } else {
                    -10.0
                };
                // Both the POINT_MOVEGEN and MonsterMove
                // arms ride the Point generator here
                // (transient splines are a recorded
                // deviation).
                self.creature_move_point(src, ints[0] as u32, dest, d[2], speed, orientation);
                false
            }
            command::MODIFY_FLAGS => {
                let Some(src) = source else { return abort };
                if d[0] <= u32::from(offsets::OBJECT_FIELD_ENTRY) || d[0] > u32::from(u16::MAX) {
                    return abort;
                }
                let field = d[0] as u16;
                let current = self.object_get_field(src, field);
                let next = match d[2] {
                    0 => current | d[1],
                    1 => current & !d[1],
                    _ => {
                        if current & d[1] != 0 {
                            current & !d[1]
                        } else {
                            current | d[1]
                        }
                    }
                };
                if self.object_set_field(src, d[0], next) {
                    false
                } else {
                    abort
                }
            }
            command::INTERRUPT_CASTS => {
                let Some(src) = src_unit else { return abort };
                let matches = self
                    .casts
                    .get(&src)
                    .is_some_and(|c| d[1] == 0 || c.spell_id == d[1]);
                if matches {
                    self.cancel_cast(src, true);
                }
                false
            }
            command::TELEPORT_TO => {
                // Same-map creature arm only; cross-map
                // and player teleports need the transfer
                // machinery (recorded).
                let Some(src) = src_creature else {
                    return abort;
                };
                if d[0] != self.map_id {
                    return abort;
                }
                self.stop_moving(src);
                self.relocate_unit(
                    src,
                    Position {
                        x: info.x,
                        y: info.y,
                        z: info.z,
                        o: info.o,
                    },
                );
                false
            }
            command::TEMP_SUMMON => {
                let Some(src) = source else { return abort };
                if d[0] == 0 {
                    return abort;
                }
                // Unique-limit gate (SF flags in dataint0).
                let flags = ints[0];
                if flags & 0x04 != 0 || flags & 0x08 != 0 {
                    let limit = if d[2] != 0 { d[2] } else { 1 };
                    let dist = if d[3] != 0 {
                        d[3] as f32
                    } else {
                        let dest = Position::new(info.x, info.y, info.z, 0.0);
                        self.object_position(src).map_or(100.0, |sp| {
                            (crate::visibility::dist2d(sp, dest) + 50.0) * 2.0
                        })
                    };
                    let sp = self.object_position(src).unwrap_or_default();
                    let existing = self
                        .creatures
                        .values()
                        .filter(|c| {
                            c.info.entry == d[0]
                                && c.unit.is_alive()
                                && crate::visibility::dist2d(sp, c.unit.world.position()) <= dist
                                && (flags & 0x04 != 0 || self.summons.contains_key(&c.unit.guid()))
                        })
                        .count() as u32;
                    if existing >= limit {
                        return abort;
                    }
                }
                let pos = Position::new(info.x, info.y, info.z, info.o);
                let Some(summon) = self.summon_creature(d[0], pos, src, ints[3] as u32, d[1])
                else {
                    return abort;
                };
                if flags & 0x01 != 0 {
                    self.set_walk(summon, false);
                }
                if flags & 0x10 != 0
                    && let Some(ai) = self.ai.get_mut(&summon)
                {
                    ai.kind = crate::ai::AiKind::Null;
                }
                if ints[2] >= 0
                    && let Some(attack_target) =
                        self.get_target_by_type(Some(src), tgt_unit, ints[2] as u8, 0, 0, None)
                    && self.is_unit(attack_target)
                {
                    self.enter_combat(summon, attack_target);
                }
                if ints[1] > 0 {
                    self.scripts_start_generic(ints[1] as u32, Some(summon), target);
                }
                false
            }
            command::RESPAWN_GAMEOBJECT | command::LOAD_GAMEOBJECT_SPAWN => {
                self.spawn_gameobject(
                    d[0],
                    &mangos_world_data::spawn_support::RespawnTimes::default(),
                );
                false
            }
            command::OPEN_DOOR | command::CLOSE_DOOR | command::RESET_DOOR_OR_BUTTON => {
                let go = self
                    .gameobjects
                    .keys()
                    .find(|g| d[0] != 0 && g.counter() == d[0])
                    .copied()
                    .or_else(|| source.filter(|g| self.gameobjects.contains_key(g)))
                    .or_else(|| target.filter(|g| self.gameobjects.contains_key(g)));
                let Some(go) = go else { return abort };
                let state = if info.command == command::OPEN_DOOR {
                    0
                } else {
                    GO_STATE_READY
                };
                let _ = self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), state);
                false
            }
            command::ACTIVATE_OBJECT => {
                let Some(go) = source.filter(|g| self.gameobjects.contains_key(g)) else {
                    return abort;
                };
                let state = self.object_get_field(go, offsets::GAMEOBJECT_STATE);
                let next = if state == GO_STATE_READY {
                    0
                } else {
                    GO_STATE_READY
                };
                let _ = self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), next);
                false
            }
            command::REMOVE_AURA => {
                let Some(src) = src_unit else { return abort };
                if d[0] != 0 {
                    self.cancel_aura(src, d[0]);
                } else {
                    self.remove_all_auras(src);
                }
                false
            }
            command::CAST_SPELL => {
                let (Some(src), Some(tgt)) = (src_unit, tgt_unit) else {
                    return abort;
                };
                if self.script_try_cast(src, tgt, d[0], d[1]) {
                    false
                } else {
                    abort
                }
            }
            command::PLAY_SOUND => {
                let Some(src) = source else { return abort };
                if d[1] & 0x1 != 0 {
                    let Some(player) = target.filter(|g| g.is_player()) else {
                        return abort;
                    };
                    self.play_direct_sound(src, d[0], Some(player));
                } else {
                    self.play_direct_sound(src, d[0], None);
                }
                false
            }
            command::DESPAWN_CREATURE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                let alive = self.unit_is_alive(src);
                let delay = if !alive && d[0] == 0 { 1 } else { d[0] };
                if delay == 0 {
                    self.despawn_creature_now(src, d[1]);
                } else {
                    self.despawns_due
                        .insert(src, (u64::from(self.now_ms) + u64::from(delay), d[1]));
                }
                false
            }
            command::SET_EQUIPMENT => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if d[0] != 0 {
                    let equip_id = self.creatures.get(&src).map_or(0, |c| c.info.equipment_id);
                    self.creature_load_equipment(src, equip_id);
                    return false;
                }
                for (slot, item) in ints.iter().take(3).enumerate() {
                    if *item >= 0 {
                        let proto = self.stores.items.get(*item as u32).map(|p| {
                            mangos_entities::creature::VirtualItemPrototype {
                                display_id: p.display_id,
                                class: p.class,
                                subclass: p.subclass,
                                material: p.material,
                                inventory_type: p.inventory_type,
                                sheath: p.sheath,
                            }
                        });
                        if let Some(c) = self.creatures.get_mut(&src) {
                            c.apply_virtual_item(slot as u16, proto);
                        }
                    }
                }
                false
            }
            command::MOVEMENT => {
                use crate::motion::{
                    CHASE_MOTION_TYPE, HOME_MOTION_TYPE, IDLE_MOTION_TYPE, RANDOM_MOTION_TYPE,
                    WAYPOINT_MOTION_TYPE,
                };
                let (src, other) = match (src_creature, target) {
                    (Some(c), t) => (c, t),
                    (None, Some(t)) if self.creatures.contains_key(&t) => (t, source),
                    _ => return abort,
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                match d[0] {
                    IDLE_MOTION_TYPE => self.creature_move_idle(src),
                    RANDOM_MOTION_TYPE => {
                        self.creature_move_random(src, d[1] != 0, info.x, 0);
                    }
                    WAYPOINT_MOTION_TYPE => {
                        self.creature_move_waypoint(src, d[2], d[1] != 0);
                    }
                    CHASE_MOTION_TYPE => {
                        let victim = if d[1] != 0 {
                            self.unit_victim(src)
                        } else {
                            other.filter(|g| self.is_unit(*g))
                        };
                        if let Some(v) = victim {
                            self.creature_move_chase(src, v, info.x, info.o);
                        }
                    }
                    HOME_MOTION_TYPE => self.creature_move_home(src),
                    10 => {
                        // FLEEING_MOTION_TYPE via the flee
                        // AI state.
                        let from = if d[1] != 0 {
                            self.unit_victim(src)
                        } else {
                            other.filter(|g| self.is_unit(*g))
                        };
                        if let Some(_v) = from {
                            self.do_flee(src, false);
                        }
                    }
                    _ => return abort,
                }
                false
            }
            command::SET_ACTIVEOBJECT => {
                let src =
                    src_creature.or_else(|| target.filter(|g| self.creatures.contains_key(g)));
                let Some(src) = src else { return abort };
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.is_active_object = d[0] != 0;
                }
                false
            }
            command::SET_FACTION => {
                let src =
                    src_creature.or_else(|| target.filter(|g| self.creatures.contains_key(g)));
                let Some(src) = src else { return abort };
                let faction = if d[0] != 0 {
                    d[0]
                } else {
                    self.creatures.get(&src).map_or(0, |c| c.info.faction)
                };
                let _ = self.object_set_field(
                    src,
                    u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
                    faction,
                );
                false
            }
            command::MORPH_TO_ENTRY_OR_MODEL => {
                let Some(src) = src_unit else { return abort };
                let display = if d[1] != 0 {
                    d[0]
                } else {
                    self.stores
                        .creature_templates
                        .get(d[0])
                        .map_or(0, |t| t.display_id[0])
                };
                if display == 0 {
                    return abort;
                }
                let _ =
                    self.object_set_field(src, u32::from(offsets::UNIT_FIELD_DISPLAYID), display);
                let _ = self.object_set_field(
                    src,
                    u32::from(offsets::UNIT_FIELD_NATIVEDISPLAYID),
                    display,
                );
                false
            }
            command::MOUNT_TO_ENTRY_OR_MODEL => {
                let Some(src) = src_creature else {
                    return abort;
                };
                let display = if d[1] != 0 {
                    d[0]
                } else {
                    self.stores
                        .creature_templates
                        .get(d[0])
                        .map_or(0, |t| t.display_id[0])
                };
                let _ = self.object_set_field(
                    src,
                    u32::from(offsets::UNIT_FIELD_MOUNTDISPLAYID),
                    display,
                );
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.mount_id = display;
                }
                false
            }
            command::SET_RUN => {
                let src =
                    src_creature.or_else(|| target.filter(|g| self.creatures.contains_key(g)));
                let Some(src) = src else { return abort };
                self.set_walk(src, d[0] == 0);
                false
            }
            command::ATTACK_START => {
                let (Some(src), Some(tgt)) = (src_creature, tgt_unit) else {
                    return abort;
                };
                if !self.unit_is_alive(src) || !self.unit_is_alive(tgt) {
                    return abort;
                }
                self.enter_combat(src, tgt);
                false
            }
            command::UPDATE_ENTRY => {
                let Some(src) = src_creature else {
                    return abort;
                };
                let Some(new_info) = self.stores.creature_templates.get(d[0]).cloned() else {
                    return abort;
                };
                let display = new_info.display_id[0];
                let faction = new_info.faction;
                if let Some(c) = self.creatures.get_mut(&src) {
                    if c.info.entry == d[0] {
                        return false;
                    }
                    c.info = new_info;
                }
                // OBJECT_FIELD_ENTRY is below the
                // FIELD_SET floor by design; write it
                // directly like UpdateEntry does.
                if let Some(u) = self.unit_base_mut(src) {
                    u.values().set_u32(offsets::OBJECT_FIELD_ENTRY, d[0]);
                    u.values().set_u32(offsets::UNIT_FIELD_DISPLAYID, display);
                    u.values()
                        .set_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID, display);
                    u.values()
                        .set_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE, faction);
                }
                self.ai.remove(&src);
                false
            }
            command::STAND_STATE => {
                let Some(src) = src_unit else { return abort };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                self.unit_set_byte(src, offsets::UNIT_FIELD_BYTES_1, 0, d[0] as u8);
                false
            }
            command::MODIFY_THREAT => {
                let Some(src) = src_creature else {
                    return abort;
                };
                #[allow(clippy::cast_possible_truncation)]
                let percent = info.x as i32;
                if d[0] == 8 {
                    let targets = self.threat_targets_sorted(src);
                    for t in targets {
                        self.modify_threat_percent_of(src, t, percent);
                    }
                } else if let Some(t) =
                    self.get_target_by_type(Some(src), target, d[0] as u8, 0, 0, None)
                {
                    self.modify_threat_percent_of(src, t, percent);
                }
                false
            }
            command::TERMINATE_SCRIPT => {
                if d[0] == 0 {
                    return true;
                }
                let Some(searcher) = source.or(target) else {
                    return abort;
                };
                let found = self
                    .nearest_creature_with_entry(searcher, d[0], d[1] as f32)
                    .is_some();
                if d[2] == 0 { !found } else { found }
            }
            command::TERMINATE_CONDITION => {
                let mut result = self.condition_satisfied(d[0], target, source);
                if d[2] & 0x1 != 0 {
                    result = !result;
                }
                result
            }
            command::ENTER_EVADE_MODE => {
                let src =
                    src_creature.or_else(|| target.filter(|g| self.creatures.contains_key(g)));
                let Some(src) = src else { return abort };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                self.enter_evade_mode(src);
                false
            }
            command::SET_HOME_POSITION => {
                let src =
                    src_creature.or_else(|| target.filter(|g| self.creatures.contains_key(g)));
                let Some(src) = src else { return abort };
                let current = self.object_position(src).unwrap_or_default();
                let default = self
                    .creatures
                    .get(&src)
                    .and_then(|c| c.data.as_ref())
                    .map(|data| data.position.position());
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.home_position = match d[0] {
                        1 => current,
                        2 => default.unwrap_or(c.home_position),
                        _ => Position {
                            x: info.x,
                            y: info.y,
                            z: info.z,
                            o: info.o,
                        },
                    };
                }
                false
            }
            command::TURN_TO => {
                let Some(src) = src_unit else { return abort };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                let mut pos = self.object_position(src).unwrap_or_default();
                if d[0] == 0 {
                    let Some(tp) = target.and_then(|t| self.object_position(t)) else {
                        return abort;
                    };
                    pos.o = (tp.y - pos.y).atan2(tp.x - pos.x);
                } else {
                    pos.o = info.o;
                }
                self.relocate_unit(src, pos);
                false
            }
            command::SET_INST_DATA => {
                if self.instance_id == 0 {
                    return abort;
                }
                let current = self
                    .instance_get_data(d[0])
                    .or_else(|| self.instance_data.get(&d[0]).copied())
                    .unwrap_or(0);
                let value = match d[2] {
                    1 => current + d[1],
                    2 => current.saturating_sub(d[1]),
                    _ => d[1],
                };
                self.instance_data.insert(d[0], value);
                self.instance_set_data(d[0], value);
                false
            }
            command::SET_INST_DATA64 => {
                if self.instance_id == 0 {
                    return abort;
                }
                let value = match d[2] {
                    1 => {
                        let Some(src) = source else { return abort };
                        src.raw()
                    }
                    _ => u64::from(d[1]),
                };
                self.instance_data64.insert(d[0], value);
                false
            }
            command::START_SCRIPT => {
                let chosen = self.choose_by_chance(&d, &ints, false);
                if chosen == 0 {
                    return abort;
                }
                self.scripts_start_generic(chosen, source, target);
                false
            }
            command::REMOVE_OBJECT => {
                if let Some(src) = src_creature {
                    self.despawn_creature_now(src, u32::MAX);
                    false
                } else if let Some(go) = source.filter(|g| self.gameobjects.contains_key(g)) {
                    self.despawn_gameobject_now(go);
                    false
                } else {
                    abort
                }
            }
            command::SET_MELEE_ATTACK => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.ensure_ai(src);
                if let Some(ai) = self.ai.get_mut(&src) {
                    ai.melee_enabled = d[0] != 0;
                }
                false
            }
            command::SET_COMBAT_MOVEMENT => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.ensure_ai(src);
                if let Some(ai) = self.ai.get_mut(&src) {
                    ai.combat_movement = d[0] != 0;
                }
                false
            }
            command::SET_PHASE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.ensure_ai(src);
                let Some(ai) = self.ai.get_mut(&src) else {
                    return abort;
                };
                if ai.kind != crate::ai::AiKind::EventAi {
                    return abort;
                }
                let mut phase = d[0];
                match d[1] {
                    1 => phase += u32::from(ai.phase),
                    2 => {
                        phase = if phase < u32::from(ai.phase) {
                            u32::from(ai.phase) - phase
                        } else {
                            0
                        };
                    }
                    _ => {}
                }
                if phase >= mangos_scripts_engine::MAX_PHASE {
                    return abort;
                }
                ai.phase = phase as u8;
                false
            }
            command::SET_PHASE_RANDOM => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.ensure_ai(src);
                let count = if d[3] != 0 {
                    4
                } else if d[2] != 0 {
                    3
                } else {
                    2
                };
                let phase = d[self.rng.urand(0, count - 1) as usize];
                if let Some(ai) = self.ai.get_mut(&src) {
                    if ai.kind != crate::ai::AiKind::EventAi {
                        return abort;
                    }
                    ai.phase = phase as u8;
                }
                false
            }
            command::SET_PHASE_RANGE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.ensure_ai(src);
                let phase = self.rng.urand(d[0], d[1].max(d[0]));
                if let Some(ai) = self.ai.get_mut(&src) {
                    if ai.kind != crate::ai::AiKind::EventAi {
                        return abort;
                    }
                    ai.phase = phase as u8;
                }
                false
            }
            command::FLEE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                if self.do_flee(src, d[0] != 0) {
                    false
                } else {
                    abort
                }
            }
            command::DEAL_DAMAGE => {
                // C++ quirk replicated: the target is
                // read from `source` (self-damage).
                let Some(src) = src_unit else { return abort };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                let damage = if d[1] != 0 {
                    let max = self.object_get_field(src, offsets::UNIT_FIELD_MAXHEALTH);
                    (f64::from(max) * (f64::from(d[0]) / 100.0)) as u32
                } else {
                    d[0]
                };
                self.deal_damage(src, src, damage);
                false
            }
            command::ZONE_COMBAT_PULSE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                let players: Vec<ObjectGuid> = self.players.keys().copied().collect();
                for p in players {
                    if self.unit_is_alive(p) {
                        self.enter_combat(src, p);
                    }
                }
                false
            }
            command::CALL_FOR_HELP => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                self.call_assistance(src, info.x);
                false
            }
            command::SET_SHEATH => {
                let Some(src) = src_unit else { return abort };
                self.unit_set_byte(src, offsets::UNIT_FIELD_BYTES_2, 0, d[0] as u8);
                false
            }
            command::INVINCIBILITY => {
                let Some(src) = src_creature else {
                    return abort;
                };
                let hp = if d[1] != 0 {
                    self.object_get_field(src, offsets::UNIT_FIELD_MAXHEALTH) * d[0] / 100
                } else {
                    d[0]
                };
                self.ensure_ai(src);
                if let Some(ai) = self.ai.get_mut(&src) {
                    ai.invincibility_hp = hp;
                }
                false
            }
            command::SET_SERVER_VARIABLE => {
                self.saved_variables.insert(d[0], d[1]);
                false
            }
            command::CREATURE_SPELLS => {
                let Some(src) = src_creature else {
                    return abort;
                };
                let chosen = self.choose_by_chance(&d, &ints, true);
                self.set_creature_spells_list(src, chosen);
                false
            }
            command::ADD_SPELL_COOLDOWN => {
                let Some(src) = src_unit else { return abort };
                self.cooldowns
                    .insert((src, d[0]), self.now_ms + d[1] * 1000);
                false
            }
            command::REMOVE_SPELL_COOLDOWN => {
                let Some(src) = src_unit else { return abort };
                if d[0] != 0 {
                    self.cooldowns.remove(&(src, d[0]));
                } else {
                    self.cooldowns.retain(|(g, _), _| *g != src);
                }
                false
            }
            command::SET_REACT_STATE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.react_state = d[0] as u8;
                }
                self.ensure_ai(src);
                if let Some(ai) = self.ai.get_mut(&src) {
                    ai.react_override = Some(d[0] as u8);
                }
                false
            }
            command::START_WAYPOINTS => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if !self.unit_is_alive(src) {
                    return abort;
                }
                // datalong2 = startPoint, datalong4 =
                // canRepeat (wpSource/overwrite arms
                // recorded).
                self.creature_move_waypoint(src, d[1], d[3] != 0);
                false
            }
            command::SET_DEFAULT_MOVEMENT => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.default_movement_type = d[0];
                    if d[0] == crate::motion::RANDOM_MOTION_TYPE {
                        c.wander_distance = d[2] as f32;
                    }
                }
                if self.unit_is_alive(src) && (d[1] != 0 || !self.ai_in_combat(src)) {
                    self.motion_initialize(src);
                }
                false
            }
            command::RESPAWN_CREATURE => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if self.unit_is_alive(src) && d[0] == 0 {
                    return abort;
                }
                self.respawns_due.remove(&src);
                self.spawn_creature(
                    src.counter(),
                    &mangos_world_data::spawn_support::RespawnTimes::default(),
                );
                self.events.push(MapEvent::ClearCreatureRespawn {
                    guid_low: src.counter(),
                    instance_id: self.creature_respawn_key(src.counter()),
                });
                false
            }
            command::ASSIST_UNIT => {
                let (Some(src), Some(tgt)) = (src_creature, tgt_unit) else {
                    return abort;
                };
                let attacker = self
                    .creatures
                    .keys()
                    .chain(self.players.keys())
                    .find(|g| self.unit_victim(**g) == Some(tgt))
                    .copied();
                if let Some(attacker) = attacker
                    && self.object_dist2d(src, attacker) <= 40.0
                {
                    self.enter_combat(src, attacker);
                }
                false
            }
            command::COMBAT_STOP => {
                let Some(src) = src_unit else { return abort };
                if src.is_creature() {
                    self.attack_stop(src);
                    if let Some(ai) = self.ai.get_mut(&src) {
                        ai.threat = mangos_threat::ThreatManager::new(src);
                        ai.in_combat = false;
                    }
                } else {
                    self.attack_stop(src);
                }
                if let Some(u) = self.unit_base_mut(src) {
                    let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
                    u.values()
                        .set_u32(offsets::UNIT_FIELD_FLAGS, flags & !UNIT_FLAG_IN_COMBAT);
                }
                false
            }
            command::ADD_AURA => {
                let Some(src) = src_unit else { return abort };
                let caster = tgt_unit.unwrap_or(src);
                self.apply_aura_by_id(caster, d[0], src);
                false
            }
            command::ADD_THREAT => {
                let (Some(src), Some(tgt)) = (src_creature, tgt_unit) else {
                    return abort;
                };
                if !self.ai_in_combat(src) || !self.unit_is_alive(src) {
                    return false;
                }
                self.add_threat(src, tgt, 0.0);
                false
            }
            command::SET_GO_STATE => {
                let Some(go) = source.filter(|g| self.gameobjects.contains_key(g)) else {
                    return abort;
                };
                let _ = self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), d[0]);
                false
            }
            command::DESPAWN_GAMEOBJECT => {
                let go = self
                    .gameobjects
                    .keys()
                    .find(|g| d[0] != 0 && g.counter() == d[0])
                    .copied()
                    .or_else(|| source.filter(|g| self.gameobjects.contains_key(g)))
                    .or_else(|| target.filter(|g| self.gameobjects.contains_key(g)));
                let Some(go) = go else { return abort };
                self.despawn_gameobject_now(go);
                false
            }
            command::SET_GOSSIP_MENU => {
                let Some(src) = src_creature else {
                    return abort;
                };
                if let Some(c) = self.creatures.get_mut(&src) {
                    c.gossip_menu_id = d[0];
                }
                false
            }
            command::SEND_SCRIPT_EVENT => {
                let Some(src) = src_creature else {
                    return abort;
                };
                self.eventai_on_script_event(src, d[0], d[1], target);
                false
            }
            command::SET_PVP => {
                let player = target
                    .filter(|g| g.is_player())
                    .or_else(|| source.filter(|g| g.is_player()));
                let Some(player) = player else { return abort };
                // `UNIT_FLAG_PVP` = 0x1000.
                if let Some(u) = self.unit_base_mut(player) {
                    let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
                    let next = if d[0] != 0 {
                        flags | 0x0000_1000
                    } else {
                        flags & !0x0000_1000
                    };
                    u.values().set_u32(offsets::UNIT_FIELD_FLAGS, next);
                }
                false
            }
            command::PLAY_CUSTOM_ANIM => {
                let Some(go) = source.filter(|g| self.gameobjects.contains_key(g)) else {
                    return abort;
                };
                let mut payload = PacketWriter::new();
                payload.write_u64_le(go.raw());
                payload.write_u32_le(d[0]);
                let packet =
                    WorldPacket::with_body(op::SMSG_GAMEOBJECT_CUSTOM_ANIM, payload.into_vec());
                self.broadcast_movement(go, &packet);
                false
            }
            command::LOAD_CREATURE_SPAWN => {
                self.spawn_creature(
                    d[0],
                    &mangos_world_data::spawn_support::RespawnTimes::default(),
                );
                false
            }
            _ => abort,
        }
    }

    /// `ChooseScriptIdToStart` / creature-spells chance
    /// pick over four (id, chance) slots.
    fn choose_by_chance(
        &mut self,
        ids: &[u32; 4],
        chances: &[i32; 4],
        skip_zero_chance: bool,
    ) -> u32 {
        let roll = self.rng.urand(1, 100);
        let mut sum = 0u32;
        for i in 0..4 {
            let chance = chances[i].max(0) as u32;
            if skip_zero_chance && chance == 0 {
                continue;
            }
            if ids[i] == 0 && !skip_zero_chance {
                continue;
            }
            if roll > sum && roll <= sum + chance {
                return ids[i];
            }
            sum += chance;
        }
        0
    }

    fn modify_threat_percent_of(&mut self, creature: ObjectGuid, victim: ObjectGuid, percent: i32) {
        let Some(mut ai) = self.ai.remove(&creature) else {
            return;
        };
        {
            let host = crate::ai::MapThreatHost { map: self };
            ai.threat.modify_threat_percent(&host, victim, percent);
        }
        self.ai.insert(creature, ai);
    }

    /// `Creature::CallAssistance` subset: nearby idle
    /// Basic/EventAI creatures join on the caller's
    /// victim (faction friendship is approximated by
    /// the creature<->player hostility model).
    pub(crate) fn call_assistance(&mut self, caller: ObjectGuid, radius: f32) {
        let radius = if radius <= 0.0 { 20.0 } else { radius };
        let Some(victim) = self.unit_victim(caller).filter(|v| v.is_player()) else {
            return;
        };
        let Some(cp) = self.object_position(caller) else {
            return;
        };
        let allies: Vec<ObjectGuid> = self
            .creatures
            .values()
            .filter(|c| {
                c.unit.guid() != caller
                    && c.unit.is_alive()
                    && crate::visibility::dist2d(cp, c.unit.world.position()) <= radius
            })
            .map(|c| c.unit.guid())
            .collect();
        for ally in allies {
            self.ensure_ai(ally);
            let joins = self
                .ai
                .get(&ally)
                .is_some_and(|ai| !ai.in_combat && ai.kind != crate::ai::AiKind::Null);
            if joins {
                self.enter_combat(ally, victim);
            }
        }
    }

    /// Delayed despawns whose time arrived.
    pub(crate) fn process_despawns_due(&mut self) {
        let now = u64::from(self.now_ms);
        let due: Vec<(ObjectGuid, u32)> = self
            .despawns_due
            .iter()
            .filter(|&(_, &(at, _))| at <= now)
            .map(|(&g, &(_, r))| (g, r))
            .collect();
        for (guid, respawn_sec) in due {
            self.despawns_due.remove(&guid);
            self.despawn_creature_now(guid, respawn_sec);
        }
    }

    /// Remove a creature from the world now
    /// (`DespawnOrUnsummon` immediate arm); viewers get
    /// the out-of-range destroy. `respawn_sec` of
    /// `u32::MAX` means no respawn; otherwise the
    /// creature returns via the normal respawn path.
    pub(crate) fn despawn_creature_now(&mut self, guid: ObjectGuid, respawn_sec: u32) {
        let Some(creature) = self.creatures.remove(&guid) else {
            return;
        };
        let cell = Self::cell_of(creature.unit.world.position());
        if let Some(c) = self.cells.get_mut(&cell) {
            c.creatures.remove(&guid);
        }
        self.motion.remove(&guid);
        self.ai.remove(&guid);
        self.casts.remove(&guid);
        self.hidden_objects.remove(&guid);
        if let Some(state) = self.summons.remove(&guid)
            && state.summoner.is_creature()
        {
            self.eventai_process_type(
                state.summoner,
                mangos_scripts_engine::event_type::SUMMONED_JUST_DESPAWN,
                Some(guid),
            );
        }
        self.notify_object_removed(guid);
        if respawn_sec != u32::MAX {
            let when = self.now_unix + u64::from(respawn_sec);
            self.respawns_due.insert(guid, when);
            self.events.push(MapEvent::StoreCreatureRespawn {
                guid_low: guid.counter(),
                respawn_time: when,
                map_id: self.map_id,
                instance_id: self.creature_respawn_key(guid.counter()),
            });
        }
    }

    pub(crate) fn despawn_gameobject_now(&mut self, guid: ObjectGuid) {
        let Some(go) = self.gameobjects.remove(&guid) else {
            return;
        };
        let cell = Self::cell_of(go.world.position());
        if let Some(c) = self.cells.get_mut(&cell) {
            c.gameobjects.remove(&guid);
        }
        self.hidden_objects.remove(&guid);
        self.notify_object_removed(guid);
    }

    fn notify_object_removed(&mut self, guid: ObjectGuid) {
        let viewers: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, e)| e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        for viewer in viewers {
            if let Some(e) = self.players.get_mut(&viewer) {
                e.visible.remove(&guid);
            }
            let mut data = mangos_protocol::update_data::UpdateData::new();
            data.add_out_of_range_guid(guid);
            self.send_update_data(viewer, &mut data);
        }
    }

    // ------------------------------------------------ chat / sound / emote

    /// Send a packet to every player within `range` of
    /// a position (`SendMessageToSetInRange` subset).
    fn send_in_range(&mut self, from: Position, range: f32, packet: &WorldPacket) {
        let targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, e)| {
                crate::visibility::dist2d(from, e.player.unit.world.position()) <= range
            })
            .map(|(g, _)| *g)
            .collect();
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `ChatHandler::BuildChatPacket` for the monster
    /// message types.
    fn build_monster_chat(
        &self,
        msg_type: u8,
        text: &str,
        language: u32,
        sender: ObjectGuid,
        target: Option<ObjectGuid>,
    ) -> WorldPacket {
        let sender_name = self.object_name(sender);
        let mut payload = PacketWriter::new();
        payload.write_u8(msg_type);
        payload.write_u32_le(language);
        match msg_type {
            CHAT_MSG_MONSTER_WHISPER | CHAT_MSG_MONSTER_EMOTE => {
                payload.write_u32_le(sender_name.len() as u32 + 1);
                payload.write_cstring(&sender_name);
                payload.write_u64_le(target.map_or(0, ObjectGuid::raw));
            }
            _ => {
                payload.write_u64_le(sender.raw());
                payload.write_u32_le(sender_name.len() as u32 + 1);
                payload.write_cstring(&sender_name);
                payload.write_u64_le(target.map_or(0, ObjectGuid::raw));
            }
        }
        payload.write_u32_le(text.len() as u32 + 1);
        payload.write_cstring(text);
        payload.write_u8(0); // CHAT_TAG_NONE
        WorldPacket::with_body(op::SMSG_MESSAGECHAT, payload.into_vec())
    }

    /// Text + metadata of a script text id (broadcast
    /// when positive, `script_texts` when negative).
    fn script_text_of(
        &self,
        text_id: i32,
        source: ObjectGuid,
    ) -> Option<(String, u32, u32, u32, u32)> {
        if text_id >= 0 {
            let bct = self.stores.scripts.broadcast_texts.get(&(text_id as u32))?;
            // `GetGender`: BYTES_0 byte 2 (1 = female).
            let gender = (self.object_get_field(source, offsets::UNIT_FIELD_BYTES_0) >> 16) & 0xFF;
            let text = if gender == 1 && !bct.female_text.is_empty() {
                bct.female_text.clone()
            } else {
                bct.male_text.clone()
            };
            Some((
                text,
                bct.chat_type,
                bct.language_id,
                bct.sound_id,
                bct.emote_ids[0],
            ))
        } else {
            let row = self.stores.scripts.script_texts.get(&text_id)?;
            Some((
                row.content.clone(),
                row.chat_type,
                row.language,
                row.sound_id,
                row.emote,
            ))
        }
    }

    /// `DoScriptText` (emote delays 2/3 of broadcast
    /// texts are not scheduled in the subset).
    pub(crate) fn do_script_text(
        &mut self,
        text_id: i32,
        source: ObjectGuid,
        target: Option<ObjectGuid>,
        chat_type_override: i32,
    ) {
        let Some((text, chat_type, language, sound_id, emote_id)) =
            self.script_text_of(text_id, source)
        else {
            return;
        };
        let chat_type = if chat_type_override >= 0 {
            chat_type_override as u32
        } else {
            chat_type
        };
        let Some(sp) = self.object_position(source) else {
            return;
        };
        if sound_id != 0 {
            self.play_direct_sound(source, sound_id, None);
        }
        if emote_id != 0 && self.is_unit(source) {
            self.handle_emote_command(source, emote_id);
        }
        let (msg_type, range) = match chat_type {
            chat_type::SAY => (CHAT_MSG_MONSTER_SAY, LISTEN_RANGE_SAY),
            chat_type::YELL => (CHAT_MSG_MONSTER_YELL, LISTEN_RANGE_YELL),
            chat_type::TEXT_EMOTE => (CHAT_MSG_MONSTER_EMOTE, LISTEN_RANGE_TEXTEMOTE),
            chat_type::BOSS_EMOTE => (CHAT_MSG_MONSTER_EMOTE, LISTEN_RANGE_YELL),
            chat_type::WHISPER | chat_type::BOSS_WHISPER => {
                if let Some(t) = target.filter(|g| g.is_player()) {
                    let packet = self.build_monster_chat(
                        CHAT_MSG_MONSTER_WHISPER,
                        &text,
                        0,
                        source,
                        Some(t),
                    );
                    self.events.push(MapEvent::Packet { to: t, packet });
                }
                return;
            }
            // Zone variants: zone ids are not computed,
            // so every player on the map hears them
            // (recorded deviation).
            chat_type::ZONE_YELL => (CHAT_MSG_MONSTER_YELL, f32::MAX),
            chat_type::ZONE_EMOTE => (CHAT_MSG_MONSTER_EMOTE, f32::MAX),
            _ => return,
        };
        let packet = self.build_monster_chat(msg_type, &text, language, source, target);
        self.send_in_range(sp, range, &packet);
    }

    /// `Unit::HandleEmoteCommand` (`SMSG_EMOTE`).
    pub fn handle_emote_command(&mut self, source: ObjectGuid, emote_id: u32) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(emote_id);
        payload.write_u64_le(source.raw());
        let packet = WorldPacket::with_body(op::SMSG_EMOTE, payload.into_vec());
        self.broadcast_movement(source, &packet);
    }

    /// `HandleTextEmoteOpcode` world half: the
    /// `SMSG_TEXT_EMOTE` broadcast plus the targeted
    /// creature's `ReceiveEmote` hook. The emote
    /// animation itself needs `EmotesText.dbc`
    /// (recorded deviation).
    pub fn player_text_emote(
        &mut self,
        player: ObjectGuid,
        text_emote: u32,
        emote_num: u32,
        target: ObjectGuid,
    ) {
        let Some(pp) = self.unit_position(player) else {
            return;
        };
        let target_name = if target != ObjectGuid::EMPTY && self.is_unit(target) {
            self.object_name(target)
        } else {
            String::new()
        };
        let mut payload = PacketWriter::new();
        payload.write_u64_le(player.raw());
        payload.write_u32_le(text_emote);
        payload.write_u32_le(emote_num);
        #[allow(clippy::cast_possible_truncation)]
        payload.write_u32_le(target_name.len() as u32 + 1);
        if target_name.is_empty() {
            payload.write_u8(0);
        } else {
            payload.write_cstring(&target_name);
        }
        let packet = WorldPacket::with_body(op::SMSG_TEXT_EMOTE, payload.into_vec());
        self.send_in_range(pp, LISTEN_RANGE_TEXTEMOTE, &packet);
        if target.is_creature() && self.creatures.contains_key(&target) {
            self.eventai_receive_emote(target, player, text_emote);
        }
    }

    /// `WorldObject::PlayDirectSound` subset: the
    /// source's viewers, or one player.
    fn play_direct_sound(&mut self, source: ObjectGuid, sound_id: u32, only: Option<ObjectGuid>) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(sound_id);
        let packet = WorldPacket::with_body(op::SMSG_PLAY_SOUND, payload.into_vec());
        if let Some(to) = only {
            self.events.push(MapEvent::Packet { to, packet });
            return;
        }
        let targets: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(g, e)| **g == source || e.visible.contains(&source))
            .map(|(g, _)| *g)
            .collect();
        for to in targets {
            self.events.push(MapEvent::Packet {
                to,
                packet: packet.clone(),
            });
        }
    }

    /// `TryToCast` subset for script `CAST_SPELL`:
    /// cast-flag gates then `cast_spell`; returns
    /// whether the cast started.
    fn script_try_cast(
        &mut self,
        source: ObjectGuid,
        target: ObjectGuid,
        spell_id: u32,
        flags: u32,
    ) -> bool {
        let triggered = flags & CF_TRIGGERED != 0;
        if flags & CF_INTERRUPT_PREVIOUS != 0 && self.casts.contains_key(&source) {
            self.cancel_cast(source, false);
        }
        if flags & CF_ONLY_IN_MELEE != 0 && !self.can_reach_melee(source, target) {
            return false;
        }
        if flags & CF_NOT_IN_MELEE != 0 && self.can_reach_melee(source, target) {
            return false;
        }
        if flags & CF_AURA_NOT_PRESENT != 0
            && self
                .aura_holders(target)
                .iter()
                .any(|h| h.spell_id == spell_id)
        {
            return false;
        }
        if flags & CF_TARGET_CASTING != 0 && !self.casts.contains_key(&target) {
            return false;
        }
        let cast_target = if target == source {
            CastTarget::SelfCast
        } else {
            CastTarget::Unit(target)
        };
        let outcome = self.cast_spell(
            source,
            spell_id,
            cast_target,
            CastOptions {
                triggered,
                skip_known: true,
            },
        );
        outcome == CastOutcome::Started
    }

    /// `Creature::DoFlee` / `DoFleeToGetAssistance`
    /// subset: marks the AI fleeing for
    /// `CreatureFamilyFleeDelay` (7000 ms default);
    /// `ai.rs` drives the away-points (the
    /// `FleeingMovementGenerator` approximation). The
    /// assistance-seeking variant falls back to plain
    /// flee (recorded).
    pub(crate) fn do_flee(&mut self, creature: ObjectGuid, _seek_assistance: bool) -> bool {
        let Some(victim) = self.unit_victim(creature) else {
            return false;
        };
        if !self.unit_is_alive(creature) {
            return false;
        }
        self.ensure_ai(creature);
        let until = self.now_ms + 7000;
        if let Some(ai) = self.ai.get_mut(&creature) {
            ai.flee_until_ms = until;
            ai.flee_from = victim;
            ai.flee_next_ms = 0;
        }
        self.cancel_cast(creature, false);
        // `MonsterTextEmote(CREATURE_FLEE_TEXT=1150)`.
        self.do_script_text(1150, creature, Some(victim), 2);
        true
    }
}
