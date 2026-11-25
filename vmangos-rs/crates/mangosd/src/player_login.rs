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

use std::collections::BTreeMap;

use mangos_db::character_load::{
    ActionRow, CharacterLoad, CharacterRow, SocialRow, SpellCooldownRow, SpellRow,
};
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::reputations::INITIAL_FACTION_SLOTS;
use mangos_world_data::spawns::WorldLocation;
use time::OffsetDateTime;

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
    build_login_verify_world_at(
        row.map,
        mangos_movement::movement_info::Position::new(
            row.position_x,
            row.position_y,
            row.position_z,
            row.orientation,
        ),
    )
}

/// Same packet at a materialized position (homebind
/// fallback may differ from the saved row).
#[must_use]
pub fn build_login_verify_world_at(
    map_id: u32,
    position: mangos_movement::movement_info::Position,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(map_id);
    writer.write_f32_le(position.x);
    writer.write_f32_le(position.y);
    writer.write_f32_le(position.z);
    writer.write_f32_le(position.o);
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

/// `SMSG_TUTORIAL_FLAGS` (`WorldSession::SendTutorialsData`,
/// `WorldSession.cpp:997-1003`): the account's eight tutorial
/// flag words verbatim. The client copies these 32 bytes into
/// its 256-bit tutorial bitsets; until the packet arrives the
/// bitset buffers are null and any per-frame tutorial-bit test
/// dereferences them, so the packet must precede the first
/// update burst.
#[must_use]
pub fn build_tutorial_flags(flags: &[u32; 8]) -> WorldPacket {
    let mut writer = PacketWriter::new();
    for flag in flags {
        writer.write_u32_le(*flag);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_TUTORIAL_FLAGS);
    packet.take_writer(writer);
    packet
}

/// One remaining cooldown as sent in the second half of
/// `SMSG_INITIAL_SPELLS` (`Player::SendInitialSpells`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialSpellCooldown {
    pub spell_id: u32,
    pub item_id: u32,
    pub category: u16,
    pub cooldown_ms: u32,
    pub category_cooldown_ms: u32,
}

/// `Player::_LoadSpellCooldowns` (`Player.cpp:3985-4024`)
/// reduced to the values `SendInitialSpells` can emit:
/// invalid spell/item references and fully expired rows are
/// dropped; expiry timestamps become remaining milliseconds.
/// Rows are sorted by spell id like the C++ cooldown map.
#[must_use]
pub fn initial_spell_cooldowns(
    rows: &[SpellCooldownRow],
    now_unix: u64,
    valid_spell: impl Fn(u32) -> bool,
    valid_item: impl Fn(u32) -> bool,
) -> Vec<InitialSpellCooldown> {
    fn remaining_ms(expire_unix: i64, now_unix: u64) -> u32 {
        let remaining_seconds = i128::from(expire_unix) - i128::from(now_unix);
        if remaining_seconds <= 0 {
            0
        } else {
            u32::try_from(remaining_seconds.saturating_mul(1000)).unwrap_or(u32::MAX)
        }
    }

    let mut cooldowns: Vec<InitialSpellCooldown> = rows
        .iter()
        .filter(|row| valid_spell(row.spell) && (row.item_id == 0 || valid_item(row.item_id)))
        .filter_map(|row| {
            let cooldown_ms = remaining_ms(row.spell_expire_time, now_unix);
            let category_cooldown_ms = remaining_ms(row.category_expire_time, now_unix);
            (cooldown_ms != 0 || category_cooldown_ms != 0).then_some(InitialSpellCooldown {
                spell_id: row.spell,
                item_id: row.item_id,
                category: row.category as u16,
                cooldown_ms,
                category_cooldown_ms,
            })
        })
        .collect();
    cooldowns.sort_by_key(|cooldown| cooldown.spell_id);
    cooldowns
}

/// `SMSG_INITIAL_SPELLS` (`Player::SendInitialSpells`,
/// `Player.cpp:3445-3518`): an intro byte, each active and
/// not-disabled spell as a `u16` id plus a zero `u16`, then
/// remaining cooldowns. The Rust player defers
/// `character_spell` rows without the C++ spell map, so only
/// rows with an existing spell entry are spelled here,
/// sorted like `PlayerSpellMap` iteration.
/// `default_spells` are the race/class
/// `playercreateinfo_spell` ids `LearnDefaultSpells` adds
/// at load: a missing default is learned active, a stored
/// not-disabled default is reactivated, and a stored
/// disabled default only has its disabled flag cleared
/// (the packet-relevant part of `AddSpell` before chains).
#[must_use]
pub fn build_initial_spells(
    spells: &[SpellRow],
    default_spells: &[u32],
    cooldowns: &[InitialSpellCooldown],
    valid_spell: impl Fn(u32) -> bool,
) -> WorldPacket {
    let mut by_spell: BTreeMap<u32, (bool, bool)> = BTreeMap::new();
    for row in spells {
        if row.spell != 0 && valid_spell(row.spell) {
            by_spell
                .entry(row.spell)
                .or_insert((row.active != 0, row.disabled != 0));
        }
    }
    for &spell in default_spells {
        if spell == 0 || !valid_spell(spell) {
            continue;
        }
        match by_spell.get_mut(&spell) {
            Some((active, disabled)) => {
                if *disabled {
                    *disabled = false;
                } else {
                    *active = true;
                }
            }
            None => {
                by_spell.insert(spell, (true, false));
            }
        }
    }
    let known: Vec<u32> = by_spell
        .into_iter()
        .filter(|(_, (active, disabled))| *active && !*disabled)
        .map(|(spell, _)| spell)
        .collect();

    let mut writer = PacketWriter::new();
    writer.write_u8(0);
    writer.write_u16_le(known.len() as u16);
    for spell in known {
        writer.write_u16_le(spell as u16);
        writer.write_u16_le(0);
    }
    writer.write_u16_le(cooldowns.len() as u16);
    for cooldown in cooldowns {
        writer.write_u16_le(cooldown.spell_id as u16);
        writer.write_u16_le(cooldown.item_id as u16);
        writer.write_u16_le(cooldown.category);
        writer.write_u32_le(cooldown.cooldown_ms);
        writer.write_u32_le(cooldown.category_cooldown_ms);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_INITIAL_SPELLS);
    packet.take_writer(writer);
    packet
}

/// `MAX_ACTION_BUTTONS` (`Player.h:133`).
pub const MAX_ACTION_BUTTONS: usize = 120;
const ACTION_BUTTON_SPELL: u8 = 0x00;
const ACTION_BUTTON_ITEM: u8 = 0x80;
const MAX_ACTION_BUTTON_ACTION_VALUE: u32 = 0x00FF_FFFF + 1;

/// `SMSG_ACTION_BUTTONS`
/// (`MasterPlayer::SendInitialActionButtons`,
/// `MasterPlayer.cpp:317-329`): all 120 packed button words,
/// zero for empty slots. Row validity follows the login
/// load path (`Player::IsActionButtonDataValid` with no
/// player): slot and action bounds, spell rows need a spell
/// entry, item rows need an item prototype, other types are
/// accepted unchecked. Invalid rows occupy no slot; a later
/// valid row for the same button replaces an earlier one,
/// as `addActionButton` does.
#[must_use]
pub fn build_action_buttons(
    actions: &[ActionRow],
    valid_spell: impl Fn(u32) -> bool,
    valid_item: impl Fn(u32) -> bool,
) -> WorldPacket {
    let mut button_words = [0u32; MAX_ACTION_BUTTONS];
    for row in actions {
        let valid = usize::from(row.button) < MAX_ACTION_BUTTONS
            && row.action < MAX_ACTION_BUTTON_ACTION_VALUE
            && match row.kind {
                ACTION_BUTTON_SPELL => valid_spell(row.action),
                ACTION_BUTTON_ITEM => valid_item(row.action),
                _ => true,
            };
        if valid {
            button_words[usize::from(row.button)] =
                (u32::from(row.kind) << 24) | (row.action & 0x00FF_FFFF);
        }
    }

    let mut writer = PacketWriter::new();
    for word in button_words {
        writer.write_u32_le(word);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_ACTION_BUTTONS);
    packet.take_writer(writer);
    packet
}

/// `SMSG_INITIALIZE_FACTIONS`
/// (`ReputationMgr::SendInitialReputations`): slot count,
/// then each slot's flags byte and standing.
#[must_use]
pub fn build_initialize_factions(slots: &[(u8, i32); INITIAL_FACTION_SLOTS]) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(INITIAL_FACTION_SLOTS as u32);
    for &(flags, standing) in slots {
        writer.write_u8(flags);
        writer.write_u32_le(standing as u32);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_INITIALIZE_FACTIONS);
    packet.take_writer(writer);
    packet
}

/// `SMSG_SET_REST_START` (`Player.cpp:18906-18909`): rest
/// state time, always zero on this path.
#[must_use]
pub fn build_set_rest_start() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_SET_REST_START);
    packet.take_writer(writer);
    packet
}

/// `SMSG_BINDPOINTUPDATE` (`Player.cpp:18911-18916`):
/// homebind x, y, z, map, area.
#[must_use]
pub fn build_bindpoint_update(location: WorldLocation, area_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_f32_le(location.x);
    writer.write_f32_le(location.y);
    writer.write_f32_le(location.z);
    writer.write_u32_le(u32::from(location.map_id));
    writer.write_u32_le(area_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_BINDPOINTUPDATE);
    packet.take_writer(writer);
    packet
}

/// `secsToTimeBitFields` (`Util.h:73-77`) over local time:
/// `(year-2000)<<24 | mon<<20 | (mday-1)<<14 | wday<<11 |
/// hour<<6 | min`, with C `tm` conventions (month 0-based,
/// Sunday-first weekday).
#[must_use]
pub fn time_bit_fields(now: OffsetDateTime) -> u32 {
    let year = u32::try_from(now.year() - 2000).unwrap_or(0);
    let month = u32::from(now.month() as u8 - 1);
    let mday = u32::from(now.day() - 1);
    let wday = u32::from(now.weekday().number_days_from_sunday());
    year << 24
        | month << 20
        | mday << 14
        | wday << 11
        | u32::from(now.hour()) << 6
        | u32::from(now.minute())
}

/// The current wall-clock time as game-time bitfields
/// (`sWorld.GetGameTime()` tracks real time).
#[must_use]
pub fn time_bit_fields_now() -> u32 {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    time_bit_fields(now)
}

/// `SMSG_LOGIN_SETTIMESPEED` (`Player.cpp:18934-18938`):
/// game-time bitfields + 1/60 speed. Until this arrives the
/// client's game-time clock is stopped and the local mover
/// never advances, so the character cannot move.
#[must_use]
pub fn build_login_set_time_speed(bitfields: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(bitfields);
    writer.write_f32_le(0.016_666_67);
    let mut packet = WorldPacket::new(opcode_id::SMSG_LOGIN_SETTIMESPEED);
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

/// `SMSG_TRANSFER_PENDING` (`Player::ExecuteTeleportFar`).
#[must_use]
pub fn build_transfer_pending(map_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(map_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_TRANSFER_PENDING);
    packet.take_writer(writer);
    packet
}

/// `SMSG_NEW_WORLD` (`Player::ExecuteTeleportFar`).
#[must_use]
pub fn build_new_world(
    map_id: u32,
    position: mangos_movement::movement_info::Position,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(map_id);
    writer.write_f32_le(position.x);
    writer.write_f32_le(position.y);
    writer.write_f32_le(position.z);
    writer.write_f32_le(position.o);
    let mut packet = WorldPacket::new(opcode_id::SMSG_NEW_WORLD);
    packet.take_writer(writer);
    packet
}

/// `SMSG_INSTANCE_SAVE_CREATED` (always 0 in the fork).
#[must_use]
pub fn build_instance_save_created() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_INSTANCE_SAVE_CREATED);
    packet.take_writer(writer);
    packet
}

/// `SMSG_UPDATE_INSTANCE_OWNERSHIP` (has any permanent bind).
#[must_use]
pub fn build_update_instance_ownership(has_permanent: bool) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(u32::from(has_permanent));
    let mut packet = WorldPacket::new(opcode_id::SMSG_UPDATE_INSTANCE_OWNERSHIP);
    packet.take_writer(writer);
    packet
}

/// `SMSG_UPDATE_LAST_INSTANCE` (one per permanent bind).
#[must_use]
pub fn build_update_last_instance(map_id: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(map_id);
    let mut packet = WorldPacket::new(opcode_id::SMSG_UPDATE_LAST_INSTANCE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_AREA_TRIGGER_MESSAGE`: u32 length + text + NUL.
#[must_use]
pub fn build_area_trigger_message(text: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(text.len() as u32 + 1);
    writer.write_bytes(text.as_bytes());
    writer.write_u8(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_AREA_TRIGGER_MESSAGE);
    packet.take_writer(writer);
    packet
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
            zone: 0,
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
            pets: Vec::new(),
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

    #[test]
    fn tutorial_flags_packet_is_eight_words_verbatim() {
        let flags = [
            0x0000_0002,
            0x8000_0000,
            0,
            0,
            0xDEAD_BEEF,
            0,
            1,
            0xFFFF_FFFF,
        ];
        let packet = build_tutorial_flags(&flags);
        assert_eq!(packet.opcode(), opcode_id::SMSG_TUTORIAL_FLAGS);
        let mut expected = Vec::new();
        for flag in flags {
            expected.extend_from_slice(&flag.to_le_bytes());
        }
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn bindpoint_update_is_position_map_area() {
        let location = WorldLocation {
            map_id: 1,
            x: -618.518,
            y: -4251.67,
            z: 38.718,
            o: 0.0,
        };
        let packet = build_bindpoint_update(location, 14);
        assert_eq!(packet.opcode(), opcode_id::SMSG_BINDPOINTUPDATE);
        let mut expected = Vec::new();
        expected.extend_from_slice(&(-618.518f32).to_le_bytes());
        expected.extend_from_slice(&(-4251.67f32).to_le_bytes());
        expected.extend_from_slice(&38.718f32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&14u32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn time_bit_fields_match_tm_packing() {
        // 2026-10-18 is a Sunday (wday 0); tm_mon for October is 9.
        let now = time::macros::datetime!(2026-10-18 13:19 UTC);
        assert_eq!(time_bit_fields(now), 0x1A94_4353);
    }

    #[test]
    fn login_set_time_speed_is_bitfields_plus_sixtieth() {
        let packet = build_login_set_time_speed(0x1A94_4353);
        assert_eq!(packet.opcode(), opcode_id::SMSG_LOGIN_SETTIMESPEED);
        let mut expected = 0x1A94_4353u32.to_le_bytes().to_vec();
        expected.extend_from_slice(&0.016_666_67f32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn initial_spells_are_sorted_filtered_and_include_cooldowns() {
        let spells = [
            SpellRow {
                spell: 78,
                active: 1,
                disabled: 0,
            },
            SpellRow {
                spell: 6603,
                active: 1,
                disabled: 0,
            },
            SpellRow {
                spell: 999_999,
                active: 1,
                disabled: 0,
            },
            SpellRow {
                spell: 78,
                active: 0,
                disabled: 0,
            },
            SpellRow {
                spell: 100,
                active: 0,
                disabled: 0,
            },
            SpellRow {
                spell: 284,
                active: 0,
                disabled: 1,
            },
        ];
        let cooldowns = [InitialSpellCooldown {
            spell_id: 78,
            item_id: 6948,
            category: 11,
            cooldown_ms: 12_000,
            category_cooldown_ms: 3_000,
        }];
        let packet =
            build_initial_spells(&spells, &[100, 284, 6603, 999_999], &cooldowns, |spell| {
                spell != 999_999
            });
        assert_eq!(packet.opcode(), opcode_id::SMSG_INITIAL_SPELLS);

        let mut expected = vec![0];
        expected.extend_from_slice(&3u16.to_le_bytes());
        expected.extend_from_slice(&78u16.to_le_bytes());
        expected.extend_from_slice(&0u16.to_le_bytes());
        expected.extend_from_slice(&100u16.to_le_bytes());
        expected.extend_from_slice(&0u16.to_le_bytes());
        expected.extend_from_slice(&6603u16.to_le_bytes());
        expected.extend_from_slice(&0u16.to_le_bytes());
        expected.extend_from_slice(&1u16.to_le_bytes());
        expected.extend_from_slice(&78u16.to_le_bytes());
        expected.extend_from_slice(&6948u16.to_le_bytes());
        expected.extend_from_slice(&11u16.to_le_bytes());
        expected.extend_from_slice(&12_000u32.to_le_bytes());
        expected.extend_from_slice(&3_000u32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn initial_spell_cooldowns_drop_invalid_and_expired_rows() {
        let rows = [
            SpellCooldownRow {
                spell: 78,
                item_id: 6948,
                category: 11,
                spell_expire_time: 1_012,
                category_expire_time: 900,
            },
            SpellCooldownRow {
                spell: 999_999,
                item_id: 0,
                category: 0,
                spell_expire_time: 2_000,
                category_expire_time: 0,
            },
            SpellCooldownRow {
                spell: 6603,
                item_id: 123_456,
                category: 0,
                spell_expire_time: 2_000,
                category_expire_time: 0,
            },
            SpellCooldownRow {
                spell: 6603,
                item_id: 0,
                category: 0,
                spell_expire_time: 900,
                category_expire_time: 900,
            },
        ];
        let cooldowns =
            initial_spell_cooldowns(&rows, 1_000, |spell| spell != 999_999, |item| item == 6948);
        assert_eq!(
            cooldowns,
            [InitialSpellCooldown {
                spell_id: 78,
                item_id: 6948,
                category: 11,
                cooldown_ms: 12_000,
                category_cooldown_ms: 0,
            }]
        );
    }

    #[test]
    fn action_buttons_pack_valid_rows_into_120_words() {
        let actions = [
            ActionRow {
                button: 0,
                action: 6603,
                kind: 0,
            },
            ActionRow {
                button: 119,
                action: 6948,
                kind: 0x80,
            },
            ActionRow {
                button: 120,
                action: 6603,
                kind: 0,
            },
            ActionRow {
                button: 1,
                action: 999_999,
                kind: 0,
            },
            ActionRow {
                button: 2,
                action: 123_456,
                kind: 0x80,
            },
            ActionRow {
                button: 3,
                action: 0x0100_0000,
                kind: 0x40,
            },
            ActionRow {
                button: 4,
                action: 42,
                kind: 0x40,
            },
        ];
        let packet = build_action_buttons(&actions, |spell| spell == 6603, |item| item == 6948);
        assert_eq!(packet.opcode(), opcode_id::SMSG_ACTION_BUTTONS);
        assert_eq!(packet.body().len(), MAX_ACTION_BUTTONS * 4);
        let word = |button: usize| {
            u32::from_le_bytes(
                packet.body()[button * 4..button * 4 + 4]
                    .try_into()
                    .expect("word"),
            )
        };
        assert_eq!(word(0), 6603);
        assert_eq!(word(1), 0);
        assert_eq!(word(2), 0);
        assert_eq!(word(3), 0);
        assert_eq!(word(4), 0x4000_002A);
        assert_eq!(word(119), 0x8000_1B24);
    }

    #[test]
    fn initialize_factions_writes_64_flag_standing_pairs() {
        let mut slots = [(0u8, 0i32); INITIAL_FACTION_SLOTS];
        slots[6] = (0x03, -125);
        slots[63] = (0x21, 42_999);
        let packet = build_initialize_factions(&slots);
        assert_eq!(packet.opcode(), opcode_id::SMSG_INITIALIZE_FACTIONS);
        assert_eq!(packet.body().len(), 4 + INITIAL_FACTION_SLOTS * 5);
        assert_eq!(&packet.body()[..4], &64u32.to_le_bytes());
        assert_eq!(packet.body()[4 + 6 * 5], 0x03);
        assert_eq!(
            &packet.body()[4 + 6 * 5 + 1..4 + 6 * 5 + 5],
            &(-125i32 as u32).to_le_bytes()
        );
        assert_eq!(packet.body()[4 + 63 * 5], 0x21);
    }

    #[test]
    fn set_rest_start_is_zero_word() {
        let packet = build_set_rest_start();
        assert_eq!(packet.opcode(), opcode_id::SMSG_SET_REST_START);
        assert_eq!(packet.body(), [0, 0, 0, 0]);
    }
}
