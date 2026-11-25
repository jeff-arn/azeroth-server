//! `SMSG_CHAR_ENUM` construction:
//! `WorldSession::HandleCharEnum` (`CharacterHandler.cpp:135-162`)
//! over `Player::BuildEnumData` (`Player.cpp:1627-1727`).

use mangos_db::characters::CharEnumRow;
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::world::{WorldPacket, opcode};
use mangos_world_data::enum_stores::EnumStores;

/// `CHARACTER_FLAG_GHOST` (`Player.h:301`).
const CHARACTER_FLAG_GHOST: u32 = 0x0000_2000;
const CLASS_HUNTER: u8 = 3;
const CLASS_WARLOCK: u8 = 9;
/// `INVENTORY_SLOT_BAG_START + 1` visual slots are written
/// (`Player.h:508`, loop at `Player.cpp:1710`).
const ENUM_VISUAL_SLOTS: usize = 20;

pub struct BuiltCharEnum {
    pub packet: WorldPacket,
    /// Characters actually written (invalid race/class rows are
    /// skipped, `BuildEnumData` false).
    pub count: u8,
    /// Highest level across *all* rows, skipped or not
    /// (`m_characterMaxLevel` updates before the skip check).
    pub max_level: u32,
}

/// Build the enum packet exactly as the C++ does, in row order.
#[must_use]
pub fn build_char_enum(rows: &[CharEnumRow], stores: &EnumStores) -> BuiltCharEnum {
    let mut writer = PacketWriter::new();
    writer.write_u8(0); // count placeholder, patched at the end
    let mut count: u8 = 0;
    let mut max_level: u32 = 0;

    for row in rows {
        max_level = max_level.max(u32::from(row.level));
        if build_one(&mut writer, row, stores) {
            count += 1;
        }
    }

    let mut packet = WorldPacket::new(opcode::SMSG_CHAR_ENUM);
    packet.take_writer(writer);
    packet.body_mut()[0] = count;
    BuiltCharEnum {
        packet,
        count,
        max_level,
    }
}

fn build_one(writer: &mut PacketWriter, row: &CharEnumRow, stores: &EnumStores) -> bool {
    if !stores.has_player_info(row.race, row.class) {
        return false;
    }

    writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, row.guid).raw());
    writer.write_cstring(&row.name);
    writer.write_u8(row.race);
    writer.write_u8(row.class);
    writer.write_u8(row.gender);
    writer.write_u8(row.skin);
    writer.write_u8(row.face);
    writer.write_u8(row.hair_style);
    writer.write_u8(row.hair_color);
    writer.write_u8(row.facial_hair);
    writer.write_u8(row.level);
    writer.write_u32_le(row.zone);
    writer.write_u32_le(row.map);
    writer.write_f32_le(row.position_x);
    writer.write_f32_le(row.position_y);
    writer.write_f32_le(row.position_z);
    writer.write_u32_le(row.guild_id.unwrap_or(0));
    writer.write_u32_le(row.character_flags);
    writer.write_u8(u8::from(row.played_time_total == 0));

    // Pet display: only non-ghost warlocks/hunters whose pet
    // template exists.
    let (mut pet_display, mut pet_level, mut pet_family) = (0u32, 0u32, 0u32);
    if row.character_flags & CHARACTER_FLAG_GHOST == 0
        && (row.class == CLASS_WARLOCK || row.class == CLASS_HUNTER)
        && let Some(entry) = row.pet_entry
        && let Some(family) = stores.pet_family(entry)
    {
        pet_display = row.pet_display_id.unwrap_or(0);
        pet_level = row.pet_level.unwrap_or(0);
        pet_family = family;
    }
    writer.write_u32_le(pet_display);
    writer.write_u32_le(pet_level);
    writer.write_u32_le(pet_family);

    // Equipment visuals from the space-separated cache
    // (entry, enchant pairs per slot).
    let cache = row.equipment_cache.as_deref().unwrap_or("");
    let tokens: Vec<&str> = cache.split(' ').filter(|t| !t.is_empty()).collect();
    for slot in 0..ENUM_VISUAL_SLOTS {
        let item_id = tokens
            .get(slot * 2)
            .map_or(0, |t| t.parse::<u32>().unwrap_or(0));
        if let Some(display) = stores.item_display(item_id) {
            writer.write_u32_le(display.display_id);
            writer.write_u8(display.inventory_type);
        } else {
            writer.write_u32_le(0);
            writer.write_u8(0);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_world_data::enum_stores::EnumStores;

    fn row(guid: u32, race: u8, class: u8, level: u8) -> CharEnumRow {
        CharEnumRow {
            guid,
            name: format!("Char{guid}"),
            race,
            class,
            gender: 0,
            skin: 1,
            face: 2,
            hair_style: 3,
            hair_color: 4,
            facial_hair: 5,
            level,
            zone: 12,
            map: 0,
            position_x: -8949.95,
            position_y: -132.493,
            position_z: 83.5312,
            guild_id: None,
            character_flags: 0,
            played_time_total: 3600,
            pet_entry: None,
            pet_display_id: None,
            pet_level: None,
            equipment_cache: Some(String::new()),
        }
    }

    #[test]
    fn empty_enum_is_single_zero_byte() {
        let built = build_char_enum(&[], &EnumStores::default());
        assert_eq!(built.count, 0);
        assert_eq!(built.packet.body(), &[0]);
    }

    #[test]
    fn invalid_pair_skipped_but_max_level_counts_it() {
        // Default stores know no pairs: everything is skipped.
        let rows = vec![row(1, 1, 1, 19), row(2, 1, 1, 60)];
        let built = build_char_enum(&rows, &EnumStores::default());
        assert_eq!(built.count, 0);
        assert_eq!(built.max_level, 60);
        assert_eq!(built.packet.body(), &[0]);
    }
}
