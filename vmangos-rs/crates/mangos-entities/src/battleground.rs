//! Per-player battleground runtime state (`Player.h`
//! `m_bgBattleGroundQueueID` + `BGData`, Tier 3 Slice 3).

use mangos_db::character_load::BgDataRow;

use crate::unit::Team;

/// `PLAYER_MAX_BATTLEGROUND_QUEUES` (`SharedDefines.h:113`).
pub const MAX_BATTLEGROUND_QUEUES: u32 = 3;

/// `MAX_BATTLEGROUND_BRACKETS` (`BattleGroundDefines.h`).
pub const MAX_BATTLEGROUND_BRACKETS: u32 = 6;
/// `BG_BRACKET_ID_LAST`.
pub const BRACKET_ID_LAST: u32 = 5;

/// `enum BattleGroundQueueTypeId`.
pub mod queue_type {
    pub const NONE: u32 = 0;
    pub const AV: u32 = 1;
    pub const WS: u32 = 2;
    pub const AB: u32 = 3;
}

/// `BattleGroundMgr::BgQueueTypeId` (type ids: AV 1,
/// WS 2, AB 3 — queue ids numerically match).
#[must_use]
pub fn queue_type_of(bg_type_id: u32) -> u32 {
    match bg_type_id {
        1..=3 => bg_type_id,
        _ => queue_type::NONE,
    }
}

/// `BattleGroundMgr::BgTemplateId`.
#[must_use]
pub fn bg_type_of_queue(bg_queue_type_id: u32) -> u32 {
    match bg_queue_type_id {
        1..=3 => bg_queue_type_id,
        _ => 0,
    }
}

/// Fork `Team` DB value (`SharedDefines.h:266`: NONE 0,
/// HORDE 67, ALLIANCE 469).
#[must_use]
pub fn team_db_value(team: Team) -> u32 {
    match team {
        Team::None => 0,
        Team::Horde => 67,
        Team::Alliance => 469,
    }
}

/// Inverse of [`team_db_value`].
#[must_use]
pub fn team_from_db(value: u32) -> Team {
    match value {
        67 => Team::Horde,
        469 => Team::Alliance,
        _ => Team::None,
    }
}

/// `Player::GetBattleGroundBracketIdFromLevel(type, level)`
/// against the template min level; `None` is
/// `BG_BRACKET_ID_NONE`.
#[must_use]
pub fn bracket_id_from_level(min_level: u32, level: u32) -> Option<u32> {
    if level < min_level {
        return None;
    }
    let bracket = (level - min_level) / 10;
    // Fork compares `> MAX_BATTLEGROUND_BRACKETS` (6), so
    // bracket 6 itself passes the compare and is returned
    // as-is only when it equals... it returns
    // `BattleGroundBracketId(bracket_id)`; bracket 6 is
    // out of the 0..=5 queue array range in the fork too
    // (never reached: max level 60, min 10 -> max 5).
    if bracket > MAX_BATTLEGROUND_BRACKETS {
        return Some(BRACKET_ID_LAST);
    }
    Some(bracket.min(BRACKET_ID_LAST))
}

/// `Player::GetMinLevelForBattleGroundBracketId`.
#[must_use]
pub fn min_level_for_bracket(bg_type_id: u32, template_min_level: u32, bracket_id: u32) -> u32 {
    if bg_type_id == 1 {
        return 51;
    }
    let mut bracket = bracket_id;
    if bg_type_id == 3 {
        bracket += 1;
    }
    if bracket > BRACKET_ID_LAST {
        bracket = BRACKET_ID_LAST;
    }
    if bg_type_id == 3 {
        10 * bracket + template_min_level - 10
    } else {
        10 * bracket + template_min_level
    }
}

/// `Player::GetMaxLevelForBattleGroundBracketId`.
#[must_use]
pub fn max_level_for_bracket(bg_type_id: u32, template_min_level: u32, bracket_id: u32) -> u32 {
    if bg_type_id == 1 {
        return 61;
    }
    if bracket_id >= BRACKET_ID_LAST || (bg_type_id == 3 && bracket_id == BRACKET_ID_LAST - 1) {
        return min_level_for_bracket(bg_type_id, template_min_level, BRACKET_ID_LAST) + 1;
    }
    min_level_for_bracket(bg_type_id, template_min_level, bracket_id) + 10
}

/// One `BgBattleGroundQueueID_Rec`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BgQueueSlot {
    pub queue_type_id: u32,
    pub invited_to_instance: u32,
}

/// `m_bgBattleGroundQueueID` + `BGData` runtime state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BgPlayerState {
    pub queue_slots: [BgQueueSlot; MAX_BATTLEGROUND_QUEUES as usize],
    /// `BGData::bgInstanceID`.
    pub bg_instance_id: u32,
    /// `BGData::bgTypeID`.
    pub bg_type_id: u32,
    /// `BGData::bgTeam`.
    pub bg_team: Team,
    /// `BGData::joinPos` — (map, x, y, z, o).
    pub join_pos: (u32, f32, f32, f32, f32),
    /// `BGData::m_needSave`.
    pub need_save: bool,
}

impl BgPlayerState {
    /// Seeds from the loaded `character_battleground_data`
    /// row (`_LoadBGData`; queue slots are runtime-only
    /// in the fork and start empty, re-linked by
    /// `BattleGroundQueue::PlayerLoggedIn`).
    #[must_use]
    pub fn from_row(row: Option<&BgDataRow>) -> Self {
        let mut state = Self::default();
        if let Some(row) = row {
            state.bg_instance_id = row.instance_id;
            state.bg_team = team_from_db(row.team);
            state.join_pos = (row.join_map, row.join_x, row.join_y, row.join_z, row.join_o);
        }
        state
    }

    /// The `_SaveBGData` row for the save bundle, when
    /// `m_needSave`; outer `None` = no write (fork
    /// early-out), inner `None` = delete only.
    #[must_use]
    pub fn save_row(&self) -> Option<Option<BgDataRow>> {
        if !self.need_save {
            return None;
        }
        if self.bg_instance_id != 0 || self.in_battleground_queue() {
            Some(Some(BgDataRow {
                instance_id: self.bg_instance_id,
                team: team_db_value(self.bg_team),
                join_x: self.join_pos.1,
                join_y: self.join_pos.2,
                join_z: self.join_pos.3,
                join_o: self.join_pos.4,
                join_map: self.join_pos.0,
            }))
        } else {
            Some(None)
        }
    }

    /// `InBattleGround`.
    #[must_use]
    pub fn in_battleground(&self) -> bool {
        self.bg_instance_id != 0
    }

    /// `SetBattleGroundId`.
    pub fn set_battleground_id(&mut self, instance_id: u32, bg_type_id: u32) {
        self.bg_instance_id = instance_id;
        self.bg_type_id = bg_type_id;
        self.need_save = true;
    }

    /// `SetBGTeam`.
    pub fn set_bg_team(&mut self, team: Team) {
        self.bg_team = team;
        self.need_save = true;
    }

    /// `GetBGTeam` against the player's racial team.
    #[must_use]
    pub fn bg_team_or(&self, own_team: Team) -> Team {
        if self.bg_team == Team::None {
            own_team
        } else {
            self.bg_team
        }
    }

    /// `SetBattleGroundEntryPoint(mapId, x, y, z, o)`.
    pub fn set_entry_point(&mut self, map_id: u32, x: f32, y: f32, z: f32, o: f32) {
        self.join_pos = (map_id, x, y, z, o);
        self.need_save = true;
    }

    /// `InBattleGroundQueue`.
    #[must_use]
    pub fn in_battleground_queue(&self) -> bool {
        self.queue_slots
            .iter()
            .any(|s| s.queue_type_id != queue_type::NONE)
    }

    /// `GetQueuedBattleground`.
    #[must_use]
    pub fn queued_battleground(&self) -> u32 {
        self.queue_slots
            .iter()
            .map(|s| s.queue_type_id)
            .find(|t| *t != queue_type::NONE)
            .unwrap_or(queue_type::NONE)
    }

    /// `GetBattleGroundQueueTypeId(index)` (out of range
    /// reads NONE).
    #[must_use]
    pub fn queue_type_at(&self, index: u32) -> u32 {
        self.queue_slots
            .get(index as usize)
            .map_or(queue_type::NONE, |s| s.queue_type_id)
    }

    /// `GetBattleGroundQueueIndex` (absent reads
    /// `PLAYER_MAX_BATTLEGROUND_QUEUES`).
    #[must_use]
    pub fn queue_index(&self, bg_queue_type_id: u32) -> u32 {
        for (i, slot) in self.queue_slots.iter().enumerate() {
            if slot.queue_type_id == bg_queue_type_id {
                return i as u32;
            }
        }
        MAX_BATTLEGROUND_QUEUES
    }

    /// `IsInvitedForBattleGroundQueueType`.
    #[must_use]
    pub fn is_invited_for_queue_type(&self, bg_queue_type_id: u32) -> bool {
        self.queue_slots
            .iter()
            .any(|s| s.queue_type_id == bg_queue_type_id && s.invited_to_instance != 0)
    }

    /// `IsInvitedForBattleGroundInstance`.
    #[must_use]
    pub fn is_invited_for_instance(&self, instance_id: u32) -> bool {
        self.queue_slots
            .iter()
            .any(|s| s.invited_to_instance == instance_id)
    }

    /// `InBattleGroundQueueForBattleGroundQueueType`.
    #[must_use]
    pub fn in_queue_for(&self, bg_queue_type_id: u32) -> bool {
        self.queue_index(bg_queue_type_id) < MAX_BATTLEGROUND_QUEUES
    }

    /// `AddBattleGroundQueueId` (returns the slot, or
    /// `PLAYER_MAX_BATTLEGROUND_QUEUES` when full).
    pub fn add_queue_id(&mut self, bg_queue_type_id: u32) -> u32 {
        for (i, slot) in self.queue_slots.iter_mut().enumerate() {
            if slot.queue_type_id == queue_type::NONE || slot.queue_type_id == bg_queue_type_id {
                slot.queue_type_id = bg_queue_type_id;
                slot.invited_to_instance = 0;
                return i as u32;
            }
        }
        MAX_BATTLEGROUND_QUEUES
    }

    /// `HasFreeBattleGroundQueueId` (bounded by
    /// `BattleGround.QueuesCount`).
    #[must_use]
    pub fn has_free_queue_id(&self, queues_count: u32) -> bool {
        self.queue_slots
            .iter()
            .take(queues_count as usize)
            .any(|s| s.queue_type_id == queue_type::NONE)
    }

    /// `RemoveBattleGroundQueueId`.
    pub fn remove_queue_id(&mut self, bg_queue_type_id: u32) {
        for slot in &mut self.queue_slots {
            if slot.queue_type_id == bg_queue_type_id {
                slot.queue_type_id = queue_type::NONE;
                slot.invited_to_instance = 0;
                return;
            }
        }
    }

    /// `SetInviteForBattleGroundQueueType`.
    pub fn set_invite(&mut self, bg_queue_type_id: u32, instance_id: u32) {
        for slot in &mut self.queue_slots {
            if slot.queue_type_id == bg_queue_type_id {
                slot.invited_to_instance = instance_id;
            }
        }
    }
}

impl crate::player::Player {
    /// `HonorMgr::Add(cp, BONUS)` subset: appends a new
    /// `character_honor_cp` row (save-side multiset diff
    /// inserts it, fork `STATE_NEW`).
    pub fn honor_add_bonus(&mut self, cp: f32, game_day: u32) {
        self.deferred
            .honor_cp
            .push(mangos_db::character_load::HonorCpRow {
                // BONUS rows carry no victim; the fork
                // stores the owner guid (`Add` with
                // `victimGuid = GetGUIDLow()`).
                victim_type: 0,
                victim_id: self.unit.guid().counter(),
                cp,
                date: game_day,
                kind: 3,
            });
    }

    /// Row-delta subset of `ModifyFactionReputation`
    /// (no runtime reputation manager exists yet; same
    /// treatment as quest rewards actor-side).
    pub fn modify_reputation_row(&mut self, faction_id: u32, delta: i32) {
        match self
            .deferred
            .reputation
            .iter_mut()
            .find(|r| r.faction == faction_id)
        {
            Some(row) => row.standing += delta,
            None => self
                .deferred
                .reputation
                .push(mangos_db::character_load::ReputationRow {
                    faction: faction_id,
                    standing: delta,
                    flags: 0,
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets() {
        // WS min level 10 (patch-10 bench row: min 21? no —
        // bracket math is template-relative).
        assert_eq!(bracket_id_from_level(10, 9), None);
        assert_eq!(bracket_id_from_level(10, 10), Some(0));
        assert_eq!(bracket_id_from_level(10, 19), Some(0));
        assert_eq!(bracket_id_from_level(10, 20), Some(1));
        assert_eq!(bracket_id_from_level(10, 60), Some(5));
        assert_eq!(bracket_id_from_level(51, 60), Some(0));
        // AB min window shifts one bracket.
        assert_eq!(min_level_for_bracket(3, 20, 0), 20);
        assert_eq!(min_level_for_bracket(3, 20, 1), 30);
        assert_eq!(min_level_for_bracket(2, 10, 2), 30);
        assert_eq!(min_level_for_bracket(1, 51, 0), 51);
        assert_eq!(max_level_for_bracket(1, 51, 0), 61);
        assert_eq!(max_level_for_bracket(2, 10, 0), 20);
        assert_eq!(max_level_for_bracket(2, 10, 5), 61);
    }

    #[test]
    fn queue_slots() {
        let mut s = BgPlayerState::default();
        assert!(!s.in_battleground_queue());
        assert_eq!(s.add_queue_id(queue_type::WS), 0);
        assert_eq!(s.add_queue_id(queue_type::AB), 1);
        assert_eq!(s.add_queue_id(queue_type::WS), 0);
        assert_eq!(s.queue_index(queue_type::AV), MAX_BATTLEGROUND_QUEUES);
        s.set_invite(queue_type::WS, 77);
        assert!(s.is_invited_for_queue_type(queue_type::WS));
        assert!(s.is_invited_for_instance(77));
        s.remove_queue_id(queue_type::WS);
        assert!(!s.is_invited_for_instance(77));
        assert!(s.has_free_queue_id(3));
        assert_eq!(s.queued_battleground(), queue_type::AB);
    }
}
