//! Who list service: `HandleWhoOpcode` request parsing and
//! the `WhoListClientQueryTask` filtering/response
//! (`MiscHandler.cpp:60-367`). The world actor collects the
//! online candidates and applies the viewer gates; this
//! module is the pure half. Deviations from C++: the query
//! runs synchronously in the world actor instead of as an
//! async world task (the one-request-in-flight flag is
//! therefore a no-op), and guild names are empty until the
//! guild service lands (Slice B continues).

use mangos_protocol::buffer::{PacketReader, PacketWriter, ReadError};
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `MAX_LEVEL` (`DBCEnums.h:34`): a `level_max` at or above
/// it becomes `PLAYER_STRONG_MAX_LEVEL`.
const MAX_LEVEL: u32 = 100;
/// `PLAYER_STRONG_MAX_LEVEL` (`DBCEnums.h:38`).
const STRONG_MAX_LEVEL: u32 = 255;

/// Client entries per who response (the C++ `clientCount`
/// break at 49).
pub const WHO_DISPLAY_CAP: usize = 49;

/// A parsed `CMSG_WHO` body (`WhoRequest` fields the task
/// consumes, with search strings already lower-cased).
#[derive(Clone, Debug, PartialEq)]
pub struct WhoRequest {
    pub level_min: u32,
    pub level_max: u32,
    pub player_name: String,
    pub guild_name: String,
    pub race_mask: u32,
    pub class_mask: u32,
    pub zones: Vec<u32>,
    /// Lower-cased search strings.
    pub strings: Vec<String>,
}

/// Parses a `CMSG_WHO` body (`HandleWhoOpcode` head).
/// `Ok(None)` is the C++ silent drop for over-limit zone
/// or string counts; `Err` is a truncated body.
///
/// # Errors
///
/// Returns [`ReadError`] on truncation.
pub fn parse_who_request(reader: &mut PacketReader) -> Result<Option<WhoRequest>, ReadError> {
    let level_min = reader.read_u32_le()?;
    let level_max = reader.read_u32_le()?;
    let player_name = reader.read_cstring();
    let guild_name = reader.read_cstring();
    let race_mask = reader.read_u32_le()?;
    let class_mask = reader.read_u32_le()?;
    let zones_count = reader.read_u32_le()?;
    if zones_count > 10 {
        return Ok(None);
    }
    let mut zones = Vec::with_capacity(zones_count as usize);
    for _ in 0..zones_count {
        zones.push(reader.read_u32_le()?);
    }
    let str_count = reader.read_u32_le()?;
    if str_count > 4 {
        return Ok(None);
    }
    let mut strings = Vec::with_capacity(str_count as usize);
    for _ in 0..str_count {
        strings.push(lower(&reader.read_cstring()));
    }
    Ok(Some(WhoRequest {
        level_min,
        level_max,
        player_name,
        guild_name,
        race_mask,
        class_mask,
        zones,
        strings,
    }))
}

/// One online player the who filter judges. `team_id` is
/// the `ChrRaces` `TeamID` (7 Alliance, 1 Horde, else 0).
#[derive(Clone, Debug, PartialEq)]
pub struct WhoCandidate {
    pub name: String,
    pub guild_name: String,
    pub level: u32,
    pub class: u8,
    pub race: u8,
    pub zone: u32,
    pub zone_name: String,
    pub team_id: u32,
    pub security: u8,
    pub instance_id: u32,
}

/// One listed row of `SMSG_WHO`.
#[derive(Clone, Debug, PartialEq)]
pub struct WhoEntry {
    pub name: String,
    pub guild_name: String,
    pub level: u32,
    pub class: u32,
    pub race: u32,
    pub zone: u32,
}

/// Viewer-side gates for the player-security arm of the
/// filter (`World::FindSession` loop pre-checks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WhoViewer {
    pub team_id: u32,
    pub zone: u32,
    pub instance_id: u32,
    pub security: u8,
    pub allow_two_side: bool,
    pub gm_level_in_who_list: u8,
}

/// Unicode lower-casing for the `wstrToLower` call sites.
#[must_use]
pub fn lower(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// `Utf8FitTo`: case-insensitive substring test. `search`
/// must already be lower-cased.
#[must_use]
pub fn fit_to(text: &str, search_lower: &str) -> bool {
    lower(text).contains(search_lower)
}

/// Battleground zones whose who visibility is
/// instance-scoped (Alterac Valley, Warsong Gulch, Arathi
/// Basin).
fn is_battleground_zone(zone: u32) -> bool {
    matches!(zone, 2597 | 3277 | 3358)
}

/// `WhoListClientQueryTask::Execute` filtering, in C++
/// check order, capped at [`WHO_DISPLAY_CAP`] entries.
/// Candidates arrive in a stable order (guid order); C++
/// iterates its player holder in hash order, which the
/// client re-sorts for display either way.
#[must_use]
pub fn filter_who(
    request: &WhoRequest,
    viewer: &WhoViewer,
    candidates: Vec<WhoCandidate>,
) -> Vec<WhoEntry> {
    let level_max = if request.level_max >= MAX_LEVEL {
        STRONG_MAX_LEVEL
    } else {
        request.level_max
    };
    let mut entries = Vec::new();
    for candidate in candidates {
        if viewer.security == crate::security::PLAYER {
            if candidate.team_id != viewer.team_id && !viewer.allow_two_side {
                continue;
            }
            if candidate.security > viewer.gm_level_in_who_list {
                continue;
            }
        }
        if candidate.level < request.level_min || candidate.level > level_max {
            continue;
        }
        if request.race_mask & (1 << candidate.race) == 0 {
            continue;
        }
        if request.class_mask & (1 << candidate.class) == 0 {
            continue;
        }
        // Check if we're in a battleground - if so, only
        // return players in the same instance.
        if candidate.zone != viewer.zone && is_battleground_zone(viewer.zone) {
            continue;
        }
        if candidate.zone == viewer.zone
            && is_battleground_zone(viewer.zone)
            && candidate.instance_id != viewer.instance_id
        {
            continue;
        }
        if !request.zones.is_empty() && !request.zones.contains(&candidate.zone) {
            continue;
        }
        if !request.player_name.is_empty() && !fit_to(&candidate.name, &lower(&request.player_name))
        {
            continue;
        }
        if !request.guild_name.is_empty()
            && !fit_to(&candidate.guild_name, &lower(&request.guild_name))
        {
            continue;
        }
        if !request.strings.is_empty() {
            let matched = request.strings.iter().any(|search| {
                fit_to(&candidate.guild_name, search)
                    || fit_to(&candidate.name, search)
                    || fit_to(&candidate.zone_name, search)
            });
            if !matched {
                continue;
            }
        }
        entries.push(WhoEntry {
            name: candidate.name,
            guild_name: candidate.guild_name,
            level: candidate.level,
            class: u32::from(candidate.class),
            race: u32::from(candidate.race),
            zone: candidate.zone,
        });
        if entries.len() == WHO_DISPLAY_CAP {
            break;
        }
    }
    entries
}

/// `SMSG_WHO`: listed count, online count (the total
/// player count when it exceeds the display cap, else the
/// listed count — `MiscHandler.cpp:219-220`), then rows.
#[must_use]
pub fn build_who_response(entries: &[WhoEntry], total_online: usize) -> WorldPacket {
    let listed = entries.len() as u32;
    let online = if total_online > WHO_DISPLAY_CAP {
        u32::try_from(total_online).unwrap_or(u32::MAX)
    } else {
        listed
    };
    let mut writer = PacketWriter::new();
    writer.write_u32_le(listed);
    writer.write_u32_le(online);
    for entry in entries {
        writer.write_cstring(&entry.name);
        writer.write_cstring(&entry.guild_name);
        writer.write_u32_le(entry.level);
        writer.write_u32_le(entry.class);
        writer.write_u32_le(entry.race);
        writer.write_u32_le(entry.zone);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_WHO);
    packet.take_writer(writer);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str) -> WhoCandidate {
        WhoCandidate {
            name: name.to_string(),
            guild_name: String::new(),
            level: 20,
            class: 1,
            race: 1,
            zone: 14,
            zone_name: "Durotar".to_string(),
            team_id: 7,
            security: 0,
            instance_id: 0,
        }
    }

    fn request() -> WhoRequest {
        WhoRequest {
            level_min: 0,
            level_max: 100,
            player_name: String::new(),
            guild_name: String::new(),
            race_mask: u32::MAX,
            class_mask: u32::MAX,
            zones: Vec::new(),
            strings: Vec::new(),
        }
    }

    fn viewer() -> WhoViewer {
        WhoViewer {
            team_id: 7,
            zone: 14,
            instance_id: 0,
            security: 0,
            allow_two_side: false,
            gm_level_in_who_list: 6,
        }
    }

    #[test]
    fn team_and_level_and_masks_filter() {
        let mut orc = candidate("Groktar");
        orc.race = 2;
        orc.team_id = 1;
        let mut high = candidate("Toohigh");
        high.level = 60;
        let mut masked = candidate("Masked");
        masked.class = 8; // mage; request below masks warriors only
        let mut req = request();
        req.level_max = 30;
        req.class_mask = 1 << 1;
        let entries = filter_who(
            &req,
            &viewer(),
            vec![candidate("Ruikanis"), orc, high, masked],
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Ruikanis");
    }

    #[test]
    fn name_zone_and_string_search() {
        let mut req = request();
        req.player_name = "RUIK".to_string();
        let entries = filter_who(&req, &viewer(), vec![candidate("Ruikanis")]);
        assert_eq!(entries.len(), 1);

        req.player_name.clear();
        req.strings = vec!["durot".to_string()];
        let entries = filter_who(&req, &viewer(), vec![candidate("Ruikanis")]);
        assert_eq!(entries.len(), 1);

        req.strings = vec!["westfall".to_string()];
        assert!(filter_who(&req, &viewer(), vec![candidate("Ruikanis")]).is_empty());

        req.strings.clear();
        req.zones = vec![40];
        assert!(filter_who(&req, &viewer(), vec![candidate("Ruikanis")]).is_empty());
    }

    #[test]
    fn staff_hidden_from_players_but_not_from_staff() {
        let mut gm = candidate("Gamemaster");
        gm.security = 6;
        let mut v = viewer();
        v.gm_level_in_who_list = 3;
        assert!(filter_who(&request(), &v, vec![gm.clone()]).is_empty());
        v.security = 3;
        assert_eq!(filter_who(&request(), &v, vec![gm]).len(), 1);
    }

    #[test]
    fn response_shape_and_online_count_rule() {
        let entries = vec![WhoEntry {
            name: "Ruikanis".to_string(),
            guild_name: String::new(),
            level: 19,
            class: 1,
            race: 1,
            zone: 40,
        }];
        let packet = build_who_response(&entries, 1);
        assert_eq!(packet.opcode(), opcode_id::SMSG_WHO);
        let mut expected = 1u32.to_le_bytes().to_vec();
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(b"Ruikanis\0\0");
        expected.extend_from_slice(&19u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&40u32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());

        let packet = build_who_response(&entries, 80);
        assert_eq!(&packet.body()[4..8], &80u32.to_le_bytes());
    }
}
