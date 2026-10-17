//! Player-login packet builders and the `LoadFromDB` gate
//! (`CharacterHandler.cpp:386-577`, `Player.cpp:14470+`).
//!
//! Phase 3 stops the login train after the map-independent
//! packets (verify-world, account-data MD5, friend/ignore
//! lists, MOTD): `SendInitialPacketsBeforeAddToMap` and the
//! map add need the simulated world. The `characters.online=1`
//! / `account.current_realm` presence writes are deliberately
//! skipped (no map presence exists to publish); recorded in
//! PHASE3-PROGRESS.

use mangos_db::character_load::{CharacterLoad, CharacterRow, SocialRow};
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `LoginFailureReason` byte used by every early refusal in
/// `HandlePlayerLoginOpcode` (`uint8(1)` verbatim at
/// `CharacterHandler.cpp:393`).
pub const CHARACTER_LOGIN_FAILED: u8 = 1;

/// `CHARACTER_FLAG_LOCKED_FOR_TRANSFER` (`Player.h:290`).
const FLAG_LOCKED_FOR_TRANSFER: u32 = 0x0000_0004;
/// `CHARACTER_FLAG_DELETED_BY_TRANSFER` (`Player.h:306`).
const FLAG_DELETED_BY_TRANSFER: u32 = 0x0004_0000;

/// `SOCIAL_FLAG_FRIEND` / `SOCIAL_FLAG_IGNORED` (`SocialMgr.h:47`).
const SOCIAL_FLAG_FRIEND: u8 = 0x01;
const SOCIAL_FLAG_IGNORED: u8 = 0x02;

/// `FRIEND_STATUS_OFFLINE` (`SocialMgr.h`).
const FRIEND_STATUS_OFFLINE: u8 = 0;

/// Why a loaded character may not enter (all roads lead to
/// `KickPlayer` in C++).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginGate {
    Proceed,
    KickNotFound,
    KickAccountMismatch,
    KickLocked,
}

/// The `LoadFromDB` pre-checks Phase 3 reproduces
/// (`Player.cpp:14480-14510`): row exists, belongs to the
/// session account, not transfer-locked. Deferred with record:
/// the bad-name check (its failure path WRITES the rename flag
/// in C++, a Phase 6 write path).
#[must_use]
pub fn evaluate_load(load: Option<&CharacterLoad>, session_account: u32) -> LoginGate {
    let Some(load) = load else {
        return LoginGate::KickNotFound;
    };
    if load.row.account != session_account {
        return LoginGate::KickAccountMismatch;
    }
    let flags = load.row.character_flags;
    if flags & FLAG_LOCKED_FOR_TRANSFER != 0 || flags & FLAG_DELETED_BY_TRANSFER != 0 {
        return LoginGate::KickLocked;
    }
    LoginGate::Proceed
}

#[must_use]
pub fn build_character_login_failed() -> WorldPacket {
    WorldPacket::with_body(
        opcode_id::SMSG_CHARACTER_LOGIN_FAILED,
        [CHARACTER_LOGIN_FAILED],
    )
}

/// `SMSG_LOGIN_VERIFY_WORLD` (`CharacterHandler.cpp:535-543`):
/// map, position, orientation from the loaded row. Transport
/// relocation (a character logged out mid-flight) is applied
/// by `LoadFromDB` through transport state — deferred with
/// movement; the row position is what the empty world knows.
#[must_use]
pub fn build_login_verify_world(row: &CharacterRow) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(row.map);
    writer.write_f32_le(row.position_x);
    writer.write_f32_le(row.position_y);
    writer.write_f32_le(row.position_z);
    writer.write_f32_le(row.orientation);
    let mut packet = WorldPacket::new(opcode_id::SMSG_LOGIN_VERIFY_WORLD);
    packet.take_writer(writer);
    packet
}

/// `SMSG_FRIEND_LIST` (`SocialMgr.cpp:100-148`): everyone is
/// offline in the empty world, so entries are guid + status 0.
#[must_use]
pub fn build_friend_list(social: &[SocialRow]) -> WorldPacket {
    let friends: Vec<&SocialRow> = social
        .iter()
        .filter(|s| s.flags & SOCIAL_FLAG_FRIEND != 0)
        .collect();
    let mut writer = PacketWriter::new();
    writer.write_u8(friends.len() as u8);
    for friend in friends {
        writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, friend.friend).raw());
        writer.write_u8(FRIEND_STATUS_OFFLINE);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_FRIEND_LIST);
    packet.take_writer(writer);
    packet
}

/// `SMSG_IGNORE_LIST` (`SocialMgr.cpp:150-169`).
#[must_use]
pub fn build_ignore_list(social: &[SocialRow]) -> WorldPacket {
    let ignored: Vec<&SocialRow> = social
        .iter()
        .filter(|s| s.flags & SOCIAL_FLAG_IGNORED != 0)
        .collect();
    let mut writer = PacketWriter::new();
    writer.write_u8(ignored.len() as u8);
    for entry in ignored {
        writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, entry.friend).raw());
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_IGNORE_LIST);
    packet.take_writer(writer);
    packet
}

/// One `CHAT_MSG_SYSTEM` line (`ChatHandler::BuildChatPacket`
/// default branch + `SendSysMessage`, `Chat.cpp:1068-1084,
/// 2003-2060`): type 0x0A, `LANG_UNIVERSAL`, zero sender,
/// length-prefixed message, zero tag.
#[must_use]
pub fn build_sys_message(line: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(0x0A); // CHAT_MSG_SYSTEM
    writer.write_u32_le(0); // LANG_UNIVERSAL
    writer.write_u64_le(0); // sender guid
    writer.write_u32_le(line.len() as u32 + 1);
    writer.write_cstring(line);
    writer.write_u8(0); // CHAT_TAG_NONE
    let mut packet = WorldPacket::new(opcode_id::SMSG_MESSAGECHAT);
    packet.take_writer(writer);
    packet
}

/// `World::GetPatchName` (`World.cpp:1252-1281`).
#[must_use]
pub fn patch_name(wow_patch: u32) -> &'static str {
    match wow_patch {
        0 => "Patch 1.2: Mysteries of Maraudon",
        1 => "Patch 1.3: Ruins of the Dire Maul",
        2 => "Patch 1.4: The Call to War",
        3 => "Patch 1.5: Battlegrounds",
        4 => "Patch 1.6: Assault on Blackwing Lair",
        5 => "Patch 1.7: Rise of the Blood God",
        6 => "Patch 1.8: Dragons of Nightmare",
        7 => "Patch 1.9: The Gates of Ahn'Qiraj",
        8 => "Patch 1.10: Storms of Azeroth",
        9 => "Patch 1.11: Shadow of the Necropolis",
        10 => "Patch 1.12: Drums of War",
        _ => "Invalid Patch!",
    }
}

/// The MOTD as `SetMotd` composes it (`World.cpp:468`), split
/// the way `LineFromMessage` (strtok on '\n') feeds packets.
#[must_use]
pub fn motd_lines(motd: &str, wow_patch: u32) -> Vec<String> {
    let full = format!("{motd}\n{} is now live!", patch_name(wow_patch));
    full.split('\n')
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_db::character_load::CharacterRow;

    fn row() -> CharacterRow {
        CharacterRow {
            guid: 1,
            account: 5,
            name: "Ruikanis".into(),
            race: 1,
            class: 1,
            gender: 0,
            level: 19,
            xp: 0,
            money: 0,
            skin: 0,
            face: 0,
            hair_style: 0,
            hair_color: 0,
            facial_hair: 0,
            bank_bag_slots: 0,
            character_flags: 0,
            position_x: -8949.95,
            position_y: -132.493,
            position_z: 83.5312,
            map: 0,
            orientation: 1.5,
            known_taxi_mask: String::new(),
            played_time_total: 0,
            played_time_level: 0,
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
            power1: 0,
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
        }
    }

    #[test]
    fn verify_world_bytes() {
        let packet = build_login_verify_world(&row());
        let mut expected = Vec::new();
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&(-8949.95f32).to_le_bytes());
        expected.extend_from_slice(&(-132.493f32).to_le_bytes());
        expected.extend_from_slice(&83.5312f32.to_le_bytes());
        expected.extend_from_slice(&1.5f32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn sys_message_bytes() {
        let packet = build_sys_message("hello");
        let mut expected = vec![0x0A];
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&0u64.to_le_bytes());
        expected.extend_from_slice(&6u32.to_le_bytes());
        expected.extend_from_slice(b"hello\0");
        expected.push(0);
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn gate_rules() {
        assert_eq!(evaluate_load(None, 5), LoginGate::KickNotFound);
        let load = CharacterLoad {
            row: row(),
            group_id: None,
            bound_instances: vec![],
            auras: vec![],
            spells: vec![],
            quest_status: vec![],
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
        };
        assert_eq!(evaluate_load(Some(&load), 5), LoginGate::Proceed);
        assert_eq!(
            evaluate_load(Some(&load), 6),
            LoginGate::KickAccountMismatch
        );
        let mut locked = load.clone();
        locked.row.character_flags = 0x4;
        assert_eq!(evaluate_load(Some(&locked), 5), LoginGate::KickLocked);
    }

    #[test]
    fn motd_composition() {
        let lines = motd_lines("Welcome.", 10);
        assert_eq!(lines, ["Welcome.", "Patch 1.12: Drums of War is now live!"]);
    }
}
