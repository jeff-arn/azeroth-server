//! Player chat core: `HandleMessagechatOpcode` parsing and
//! `ChatHandler::BuildChatPacket` shapes for the types
//! routed so far (SAY/YELL/EMOTE/WHISPER/AFK/DND plus,
//! since Slice B.3, party/raid leader/warning types,
//! channels from B.2b, guild/officer from B.4), from
//! `ChatHandler.cpp:74-105,149-724` and `Chat.cpp:2003-2055`.
//! Battleground types still parse but are dropped by
//! the handler until that service lands.
//! Deviations from the fork's
//! handler: the antispam/mute/level-restriction/telemetry
//! family is not ported, the unlearned-language reply drops
//! silently instead of sending `SMSG_NOTIFICATION`, and
//! whisper delivery follows the fork's default-config
//! observable behavior (`WhisperRestriction` off delivers
//! every whisper; the client filters its own ignore list).
//! Also unported: GM-mode universal language/tag (no GM
//! state yet), the AFK in-combat guard (no player combat
//! flag yet), chat `LogChat` lines, and message security
//! checks (link validation et al.).

use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::{PacketReader, PacketWriter, ReadError};
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `MAX_CHAT_MSG_TYPE` (`SharedDefines.h:1316`).
pub const MAX_CHAT_MSG_TYPE: u32 = 0x5E;

/// `ChatMsg` values this slice routes or builds
/// (`SharedDefines.h:1207-1223`).
pub mod chat_msg {
    pub const SAY: u32 = 0x00;
    pub const PARTY: u32 = 0x01;
    pub const RAID: u32 = 0x02;
    pub const GUILD: u32 = 0x03;
    pub const OFFICER: u32 = 0x04;
    pub const YELL: u32 = 0x05;
    pub const WHISPER: u32 = 0x06;
    pub const WHISPER_INFORM: u32 = 0x07;
    pub const EMOTE: u32 = 0x08;
    pub const SYSTEM: u32 = 0x0A;
    pub const CHANNEL: u32 = 0x0E;
    pub const AFK: u32 = 0x14;
    pub const DND: u32 = 0x15;
    pub const RAID_LEADER: u32 = 0x57;
    pub const RAID_WARNING: u32 = 0x58;
}

/// `Language` values (`SharedDefines.h:246-260`).
pub mod language {
    pub const UNIVERSAL: u32 = 0;
    pub const ORCISH: u32 = 1;
    pub const COMMON: u32 = 7;
    pub const ADDON: u32 = 0xFFFF_FFFF;
}

/// `lang_description` (`ObjectMgr.cpp:82-84`): language id
/// to the spell that teaches it (`KnowsLanguage` is a mask
/// built from known language spells).
const LANGUAGE_SPELLS: &[(u32, u32)] = &[
    (1, 669),    // Orcish
    (2, 671),    // Darnassian
    (3, 670),    // Taurahe
    (6, 672),    // Dwarvish
    (7, 668),    // Common
    (8, 815),    // Demonic
    (9, 816),    // Titan
    (10, 813),   // Thalassian
    (11, 814),   // Draconic
    (12, 817),   // Kalimag
    (13, 7340),  // Gnomish
    (14, 7341),  // Troll
    (33, 17737), // Gutterspeak
];

/// The spell teaching `lang`, when the language exists.
#[must_use]
pub fn language_spell(lang: u32) -> Option<u32> {
    LANGUAGE_SPELLS
        .iter()
        .find(|(id, _)| *id == lang)
        .map(|(_, spell)| *spell)
}

/// A parsed `CMSG_MESSAGECHAT` body. `target` carries the
/// whisper recipient or channel name for those types.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatRequest {
    pub msg_type: u32,
    pub lang: u32,
    pub target: String,
    pub text: String,
}

/// Parses a `CMSG_MESSAGECHAT` body. `Ok(None)` covers the
/// C++ silent returns (unknown/oversized type); `Err` is a
/// truncated fixed-size field.
///
/// # Errors
///
/// Returns [`ReadError`] on truncation.
pub fn parse_message_chat(reader: &mut PacketReader) -> Result<Option<ChatRequest>, ReadError> {
    let msg_type = reader.read_u32_le()?;
    let lang = reader.read_u32_le()?;
    if msg_type >= MAX_CHAT_MSG_TYPE {
        return Ok(None);
    }
    let target = match msg_type {
        chat_msg::CHANNEL | chat_msg::WHISPER => reader.read_cstring(),
        _ => String::new(),
    };
    let text = reader.read_cstring();
    Ok(Some(ChatRequest {
        msg_type,
        lang,
        target,
        text,
    }))
}

/// `IsLanguageAllowedForChatType`: the addon language is
/// only valid for group/guild/raid/channel types.
#[must_use]
pub fn language_allowed(lang: u32, msg_type: u32) -> bool {
    if lang != language::ADDON {
        return true;
    }
    matches!(msg_type, 0x01 | 0x02 | 0x03 | 0x04 | chat_msg::CHANNEL)
}

/// `Player::GetYellRange` (`Player.cpp:17174-17185`).
/// The u32→f32 casts mirror the C++ float math; levels
/// and scale caps are far below f32 precision limits.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn yell_range(
    base: f32,
    level: u32,
    linear_scale_max_level: u32,
    quadratic_scale_max_level: u32,
    min: u32,
) -> f32 {
    let mut range = base;
    if level < linear_scale_max_level {
        range *= level as f32 / linear_scale_max_level as f32;
    }
    if level < quadratic_scale_max_level {
        range *=
            (level * level) as f32 / (quadratic_scale_max_level * quadratic_scale_max_level) as f32;
    }
    if range < min as f32 {
        range = min as f32;
    }
    range
}

/// `BuildChatPacket` for the player-sourced types: SAY,
/// PARTY, and YELL repeat the sender guid, every other
/// type writes it once, then the length-prefixed message
/// and chat tag. Channel/monster variants join with their
/// services.
#[must_use]
pub fn build_chat_packet(
    msg_type: u32,
    message: &str,
    language: u32,
    chat_tag: u8,
    sender: ObjectGuid,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(msg_type as u8);
    writer.write_u32_le(language);
    writer.write_u64_le(sender.raw());
    if matches!(msg_type, chat_msg::SAY | chat_msg::PARTY | chat_msg::YELL) {
        writer.write_u64_le(sender.raw());
    }
    writer.write_u32_le(message.len() as u32 + 1);
    writer.write_cstring(message);
    writer.write_u8(chat_tag);
    let mut packet = WorldPacket::new(opcode_id::SMSG_MESSAGECHAT);
    packet.take_writer(writer);
    packet
}

/// `BuildChatPacket` `CHAT_MSG_CHANNEL` variant: channel
/// name, player honor rank, sender guid, then the shared
/// length-prefixed message and chat tag.
#[must_use]
pub fn build_channel_chat_packet(
    message: &str,
    language: u32,
    chat_tag: u8,
    sender: ObjectGuid,
    channel_name: &str,
    player_rank: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(chat_msg::CHANNEL as u8);
    writer.write_u32_le(language);
    writer.write_cstring(channel_name);
    writer.write_u32_le(player_rank);
    writer.write_u64_le(sender.raw());
    writer.write_u32_le(message.len() as u32 + 1);
    writer.write_cstring(message);
    writer.write_u8(chat_tag);
    let mut packet = WorldPacket::new(opcode_id::SMSG_MESSAGECHAT);
    packet.take_writer(writer);
    packet
}

/// `SMSG_CHAT_PLAYER_NOT_FOUND` (`SendPlayerNotFoundNotice`).
#[must_use]
pub fn build_player_not_found(name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_CHAT_PLAYER_NOT_FOUND);
    packet.take_writer(writer);
    packet
}

/// `SMSG_CHAT_WRONG_FACTION` (`SendWrongFactionNotice`,
/// empty body).
#[must_use]
pub fn build_wrong_faction() -> WorldPacket {
    WorldPacket::new(opcode_id::SMSG_CHAT_WRONG_FACTION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_guid::HighGuid;
    use mangos_protocol::buffer::PacketReader as Reader;

    #[test]
    fn say_packet_repeats_sender_guid() {
        let guid = ObjectGuid::new(HighGuid::PLAYER, 7);
        let packet = build_chat_packet(chat_msg::SAY, "hello", language::ORCISH, 0, guid);
        assert_eq!(packet.opcode(), opcode_id::SMSG_MESSAGECHAT);
        let mut expected = vec![chat_msg::SAY as u8];
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&guid.raw().to_le_bytes());
        expected.extend_from_slice(&guid.raw().to_le_bytes());
        expected.extend_from_slice(&6u32.to_le_bytes());
        expected.extend_from_slice(b"hello\0");
        expected.push(0);
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn whisper_packet_writes_sender_once() {
        let guid = ObjectGuid::new(HighGuid::PLAYER, 7);
        let packet = build_chat_packet(chat_msg::WHISPER, "psst", language::UNIVERSAL, 1, guid);
        let mut expected = vec![chat_msg::WHISPER as u8];
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&guid.raw().to_le_bytes());
        expected.extend_from_slice(&5u32.to_le_bytes());
        expected.extend_from_slice(b"psst\0");
        expected.push(1);
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn parse_whisper_and_reject_oversize_type() {
        let mut body = Vec::new();
        body.extend_from_slice(&chat_msg::WHISPER.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(b"Morka\0hello\0");
        let request = parse_message_chat(&mut Reader::new(&body))
            .expect("parse")
            .expect("some");
        assert_eq!(request.target, "Morka");
        assert_eq!(request.text, "hello");

        let mut body = Vec::new();
        body.extend_from_slice(&MAX_CHAT_MSG_TYPE.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_message_chat(&mut Reader::new(&body)), Ok(None));
    }

    #[test]
    fn addon_language_gated_to_group_types() {
        assert!(!language_allowed(language::ADDON, chat_msg::SAY));
        assert!(language_allowed(language::ADDON, chat_msg::CHANNEL));
        assert!(language_allowed(language::ORCISH, chat_msg::SAY));
    }

    #[test]
    fn yell_range_scales_and_floors() {
        assert!((yell_range(300.0, 60, 0, 0, 0) - 300.0).abs() < f32::EPSILON);
        assert!((yell_range(300.0, 5, 10, 0, 0) - 150.0).abs() < f32::EPSILON);
        assert!((yell_range(300.0, 1, 0, 0, 25) - 300.0).abs() < f32::EPSILON);
        assert!((yell_range(300.0, 1, 10, 0, 25) - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn language_spell_table() {
        assert_eq!(language_spell(language::ORCISH), Some(669));
        assert_eq!(language_spell(language::COMMON), Some(668));
        assert_eq!(language_spell(5), None);
    }
}
