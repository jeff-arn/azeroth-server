//! Friend/ignore social services: `HandleAddFriendOpcode`,
//! `HandleDelFriendOpcode`, `HandleAddIgnoreOpcode`,
//! `HandleDelIgnoreOpcode` (`MiscHandler.cpp:488-599`) over
//! `PlayerSocial` state and `SocialMgr::SendFriendStatus`
//! (`SocialMgr.cpp:50-107,225-261`). The handlers live in
//! the world actor; this module is the pure half (results,
//! list mutation, packet shape) so it is unit-testable
//! without a session. This is the Phase 5 Slice A exemplar
//! for the service pattern: parse, in-memory state decision,
//! one `character_social` write, one status reply.

use mangos_db::character_load::SocialRow;
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `SOCIAL_FLAG_FRIEND` (`SocialMgr.h:47`).
pub const SOCIAL_FLAG_FRIEND: u8 = 0x01;
/// `SOCIAL_FLAG_IGNORED` (`SocialMgr.h:48`).
pub const SOCIAL_FLAG_IGNORED: u8 = 0x02;

/// `SOCIALMGR_FRIEND_LIMIT` (`SocialMgr.h:114`).
pub const FRIEND_LIMIT: usize = 50;
/// `SOCIALMGR_IGNORE_LIMIT` (`SocialMgr.h:115`, checked max
/// for 1.12.1).
pub const IGNORE_LIMIT: usize = 25;

/// `FRIEND_STATUS_ONLINE` (`SocialMgr.h:39`). AFK/DND
/// statuses have no Rust session state yet, so an online
/// friend always reports plain online.
pub const FRIEND_STATUS_ONLINE: u8 = 1;

/// `MAX_INTERNAL_PLAYER_NAME` (`ObjectMgr.h:396`).
const MAX_INTERNAL_PLAYER_NAME: usize = 15;

/// `FriendsResult` subset these handlers can produce
/// (`SocialMgr.h:83-111`). The `NOT_FOUND` results are
/// unreachable here exactly as in C++: an unknown or
/// unnormalizable name returns before any packet is sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FriendsResult {
    ListFull = 0x01,
    Removed = 0x05,
    AddedOnline = 0x06,
    AddedOffline = 0x07,
    Already = 0x08,
    Self_ = 0x09,
    Enemy = 0x0A,
    IgnoreFull = 0x0B,
    IgnoreSelf = 0x0C,
    IgnoreAlready = 0x0E,
    IgnoreAdded = 0x0F,
    IgnoreRemoved = 0x10,
}

/// The add-handler early returns, in C++ check order
/// (`HandleAddFriendOpcode` / `HandleAddIgnoreOpcode`):
/// self, then enemy (friend adds only; callers pass
/// `is_enemy == false` for ignores), then already-listed.
/// `None` proceeds to the list mutation.
#[must_use]
#[allow(clippy::fn_params_excessive_bools)]
pub fn early_add_result(
    ignore: bool,
    is_self: bool,
    is_enemy: bool,
    already: bool,
) -> Option<FriendsResult> {
    if is_self {
        Some(if ignore {
            FriendsResult::IgnoreSelf
        } else {
            FriendsResult::Self_
        })
    } else if is_enemy {
        Some(FriendsResult::Enemy)
    } else if already {
        Some(if ignore {
            FriendsResult::IgnoreAlready
        } else {
            FriendsResult::Already
        })
    } else {
        None
    }
}

/// The `character_social` write a list mutation implies,
/// mirroring the statement `PlayerSocial` picks between
/// INSERT / flag-UPDATE / DELETE from its in-memory map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocialWrite {
    Insert(u8),
    AddFlag(u8),
    ClearFlag(u8),
    DeleteRow,
}

/// Online tail of `SMSG_FRIEND_STATUS` for
/// `FRIEND_ADDED_ONLINE` (`GetFriendInfo` output).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FriendOnlineInfo {
    pub status: u8,
    pub area: u32,
    pub level: u32,
    pub class: u32,
}

/// `SMSG_FRIEND_STATUS` (`MakeFriendStatusPacket` +
/// `SendFriendStatus`): result byte, friend guid, and for
/// the online result a status byte plus area/level/class.
#[must_use]
pub fn build_friend_status(
    result: FriendsResult,
    friend_low: u32,
    online: Option<FriendOnlineInfo>,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(result as u8);
    writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, friend_low).raw());
    if let Some(info) = online {
        writer.write_u8(info.status);
        writer.write_u32_le(info.area);
        writer.write_u32_le(info.level);
        writer.write_u32_le(info.class);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_FRIEND_STATUS);
    packet.take_writer(writer);
    packet
}

/// `normalizePlayerName` (`ObjectMgr.cpp:62-80`): nonempty,
/// at most `MAX_INTERNAL_PLAYER_NAME` wide chars, first
/// char upper-cased and the rest lower-cased. `None` is the
/// C++ `false` (handler returns without a packet).
#[must_use]
pub fn normalize_player_name(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if name.chars().count() > MAX_INTERNAL_PLAYER_NAME {
        return None;
    }
    let mut normalized: String = first.to_uppercase().collect();
    normalized.extend(chars.flat_map(char::to_lowercase));
    Some(normalized)
}

fn flag_for(ignore: bool) -> u8 {
    if ignore {
        SOCIAL_FLAG_IGNORED
    } else {
        SOCIAL_FLAG_FRIEND
    }
}

/// Entries carrying `flag` (`GetNumberOfSocialsWithFlag`).
#[must_use]
pub fn count_with_flag(social: &[SocialRow], flag: u8) -> usize {
    social.iter().filter(|row| row.flags & flag != 0).count()
}

/// `PlayerSocial::HasFriend` / `HasIgnore`.
#[must_use]
pub fn has_flag(social: &[SocialRow], friend_low: u32, flag: u8) -> bool {
    social
        .iter()
        .any(|row| row.friend == friend_low && row.flags & flag != 0)
}

/// `PlayerSocial::AddToSocialList`: limit check first, then
/// set the flag in memory and report the DB write. `None`
/// is the C++ `false` (list full, nothing changed).
#[must_use]
pub fn add_to_social_list(
    social: &mut Vec<SocialRow>,
    friend_low: u32,
    ignore: bool,
) -> Option<SocialWrite> {
    let flag = flag_for(ignore);
    let limit = if ignore { IGNORE_LIMIT } else { FRIEND_LIMIT };
    if count_with_flag(social, flag) >= limit {
        return None;
    }
    if let Some(row) = social.iter_mut().find(|row| row.friend == friend_low) {
        row.flags |= flag;
        Some(SocialWrite::AddFlag(flag))
    } else {
        social.push(SocialRow {
            friend: friend_low,
            flags: flag,
        });
        Some(SocialWrite::Insert(flag))
    }
}

/// `PlayerSocial::RemoveFromSocialList`: clears the flag;
/// the row is deleted when no flags remain. `None` when the
/// pair is not in the list (no DB write, exactly as C++).
#[must_use]
pub fn remove_from_social_list(
    social: &mut Vec<SocialRow>,
    friend_low: u32,
    ignore: bool,
) -> Option<SocialWrite> {
    let flag = flag_for(ignore);
    let index = social.iter().position(|row| row.friend == friend_low)?;
    social[index].flags &= !flag;
    if social[index].flags == 0 {
        social.remove(index);
        Some(SocialWrite::DeleteRow)
    } else {
        Some(SocialWrite::ClearFlag(flag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(friend: u32, flags: u8) -> SocialRow {
        SocialRow { friend, flags }
    }

    #[test]
    fn normalize_matches_cpp_case_folding() {
        assert_eq!(normalize_player_name(""), None);
        assert_eq!(normalize_player_name("gROKTAR").as_deref(), Some("Groktar"));
        assert_eq!(normalize_player_name("a").as_deref(), Some("A"));
        assert!(normalize_player_name(&"a".repeat(15)).is_some());
        assert_eq!(normalize_player_name(&"a".repeat(16)), None);
    }

    #[test]
    fn add_insert_then_flag_merge_then_limit() {
        let mut social = Vec::new();
        assert_eq!(
            add_to_social_list(&mut social, 9, false),
            Some(SocialWrite::Insert(SOCIAL_FLAG_FRIEND))
        );
        assert_eq!(
            add_to_social_list(&mut social, 9, true),
            Some(SocialWrite::AddFlag(SOCIAL_FLAG_IGNORED))
        );
        assert_eq!(
            social,
            vec![row(9, SOCIAL_FLAG_FRIEND | SOCIAL_FLAG_IGNORED)]
        );
        assert!(has_flag(&social, 9, SOCIAL_FLAG_FRIEND));
        assert!(has_flag(&social, 9, SOCIAL_FLAG_IGNORED));

        let mut full: Vec<SocialRow> = (1..=25u32)
            .map(|guid| row(guid, SOCIAL_FLAG_IGNORED))
            .collect();
        assert_eq!(add_to_social_list(&mut full, 99, true), None);
        assert_eq!(full.len(), IGNORE_LIMIT);
    }

    #[test]
    fn remove_clears_flag_then_deletes_row() {
        let mut social = vec![row(9, SOCIAL_FLAG_FRIEND | SOCIAL_FLAG_IGNORED)];
        assert_eq!(
            remove_from_social_list(&mut social, 9, false),
            Some(SocialWrite::ClearFlag(SOCIAL_FLAG_FRIEND))
        );
        assert_eq!(social, vec![row(9, SOCIAL_FLAG_IGNORED)]);
        assert_eq!(
            remove_from_social_list(&mut social, 9, true),
            Some(SocialWrite::DeleteRow)
        );
        assert!(social.is_empty());
        assert_eq!(remove_from_social_list(&mut social, 9, true), None);
    }

    #[test]
    fn friend_status_packet_shape() {
        let offline = build_friend_status(FriendsResult::Already, 9, None);
        assert_eq!(offline.opcode(), opcode_id::SMSG_FRIEND_STATUS);
        let mut expected = vec![FriendsResult::Already as u8];
        expected.extend_from_slice(&ObjectGuid::new(HighGuid::PLAYER, 9).raw().to_le_bytes());
        assert_eq!(offline.body(), expected.as_slice());

        let online = build_friend_status(
            FriendsResult::AddedOnline,
            9,
            Some(FriendOnlineInfo {
                status: FRIEND_STATUS_ONLINE,
                area: 14,
                level: 35,
                class: 1,
            }),
        );
        let mut expected = vec![FriendsResult::AddedOnline as u8];
        expected.extend_from_slice(&ObjectGuid::new(HighGuid::PLAYER, 9).raw().to_le_bytes());
        expected.push(FRIEND_STATUS_ONLINE);
        expected.extend_from_slice(&14u32.to_le_bytes());
        expected.extend_from_slice(&35u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(online.body(), expected.as_slice());
    }
}
