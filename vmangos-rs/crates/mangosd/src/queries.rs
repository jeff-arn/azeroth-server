//! Static cache query replies at build 5875:
//! `WorldSession::HandleNameQueryOpcode`,
//! `HandleCreatureQueryOpcode`,
//! `HandleGameObjectQueryOpcode`
//! (`QueryHandler.cpp:34-296`), and
//! `HandleItemQuerySingleOpcode`
//! (`ItemHandler.cpp:287-439`).
//!
//! Creature and gameobject names currently use the template's
//! base text. The localized creature/gameobject stores the
//! C++ consults for a non-default session locale are not wired
//! into the Rust world actor yet.

use mangos_db::characters::NameQueryRow;
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_spells::SpellEntry;
use mangos_world_data::creature_template::CreatureInfo;
use mangos_world_data::gameobject_template::GameObjectInfo;
use mangos_world_data::item_prototype::ItemPrototype;
use std::collections::HashMap;
use std::hash::BuildHasher;

/// `ITEM_CLASS_CONSUMABLE`.
const ITEM_CLASS_CONSUMABLE: u32 = 0;

/// Entry high bit marking a template cache miss.
const QUERY_ENTRY_NOT_FOUND: u32 = 0x8000_0000;

fn finish(opcode: u16, writer: PacketWriter) -> WorldPacket {
    let mut packet = WorldPacket::new(opcode);
    packet.take_writer(writer);
    packet
}

fn entry_not_found(opcode: u16, entry: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry | QUERY_ENTRY_NOT_FOUND);
    finish(opcode, writer)
}

/// `SMSG_NAME_QUERY_RESPONSE` at build 5875: player GUID,
/// name, the empty cross-realm placeholder, then race, gender,
/// and class (`QueryHandler.cpp:41-54`).
#[must_use]
pub fn build_name_query_response(guid_low: u32, row: &NameQueryRow) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, guid_low).raw());
    writer.write_cstring(&row.name);
    writer.write_u8(0); // realm name, always empty here
    writer.write_u32_le(u32::from(row.race));
    writer.write_u32_le(u32::from(row.gender));
    writer.write_u32_le(u32::from(row.class));
    finish(opcode_id::SMSG_NAME_QUERY_RESPONSE, writer)
}

/// `SMSG_CREATURE_QUERY_RESPONSE` at build 5875
/// (`QueryHandler.cpp:202-231`). A missing template sends only
/// the entry with the not-found high bit.
#[must_use]
pub fn build_creature_query_response(entry: u32, info: Option<&CreatureInfo>) -> WorldPacket {
    let Some(info) = info else {
        return entry_not_found(opcode_id::SMSG_CREATURE_QUERY_RESPONSE, entry);
    };

    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry);
    writer.write_cstring(&info.name);
    writer.write_u8(0); // name2
    writer.write_u8(0); // name3
    writer.write_u8(0); // name4
    writer.write_cstring(&info.subname);
    writer.write_u32_le(info.type_flags());
    writer.write_u32_le(info.creature_type);
    writer.write_u32_le(info.pet_family);
    writer.write_u32_le(info.rank);
    writer.write_u32_le(0);
    writer.write_u32_le(info.pet_spell_list_id);
    writer.write_u32_le(info.display_id[0]);
    writer.write_u8(u8::from(info.civilian));
    writer.write_u8(u8::from(info.racial_leader));
    finish(opcode_id::SMSG_CREATURE_QUERY_RESPONSE, writer)
}

/// `SMSG_GAMEOBJECT_QUERY_RESPONSE` at build 5875
/// (`QueryHandler.cpp:274-294`). A missing template sends only
/// the entry with the not-found high bit.
#[must_use]
pub fn build_gameobject_query_response(entry: u32, info: Option<&GameObjectInfo>) -> WorldPacket {
    let Some(info) = info else {
        return entry_not_found(opcode_id::SMSG_GAMEOBJECT_QUERY_RESPONSE, entry);
    };

    let mut writer = PacketWriter::new();
    writer.write_u32_le(entry);
    writer.write_u32_le(info.go_type);
    writer.write_u32_le(info.display_id);
    writer.write_cstring(&info.name);
    writer.write_u8(0); // name2
    writer.write_u8(0); // name3
    writer.write_u8(0); // name4
    writer.write_cstring(&info.icon); // name5 at 1.12.1
    for index in 0..info.data.len() {
        writer.write_u32_le(info.data_u32(index));
    }
    finish(opcode_id::SMSG_GAMEOBJECT_QUERY_RESPONSE, writer)
}

/// `SMSG_ITEM_QUERY_SINGLE_RESPONSE` at build 5875
/// (`ItemHandler.cpp:313-431`). A missing or undiscovered
/// prototype sends only the entry with the not-found high
/// bit; `allow_datamined` is the caller's
/// `GetSecurity() > SEC_PLAYER` test. Item names use the
/// template's base text until localized item stores are
/// wired into the world actor.
#[must_use]
pub fn build_item_query_response<S: BuildHasher>(
    entry: u32,
    proto: Option<&ItemPrototype>,
    allow_datamined: bool,
    spells: &HashMap<u32, SpellEntry, S>,
) -> WorldPacket {
    let Some(proto) = proto.filter(|proto| proto.discovered || allow_datamined) else {
        return entry_not_found(opcode_id::SMSG_ITEM_QUERY_SINGLE_RESPONSE, entry);
    };

    let mut writer = PacketWriter::new();
    writer.write_u32_le(proto.item_id);
    writer.write_u32_le(proto.class);
    writer.write_u32_le(if proto.class == ITEM_CLASS_CONSUMABLE {
        0
    } else {
        proto.subclass
    });
    writer.write_cstring(&proto.name);
    writer.write_u8(0); // name2
    writer.write_u8(0); // name3
    writer.write_u8(0); // name4
    writer.write_u32_le(proto.display_id);
    writer.write_u32_le(proto.quality);
    writer.write_u32_le(proto.flags);
    writer.write_u32_le(proto.buy_price);
    writer.write_u32_le(proto.sell_price);
    writer.write_u32_le(proto.inventory_type);
    writer.write_u32_le(proto.allowable_class);
    writer.write_u32_le(proto.allowable_race);
    writer.write_u32_le(proto.item_level);
    writer.write_u32_le(proto.required_level);
    writer.write_u32_le(proto.required_skill);
    writer.write_u32_le(proto.required_skill_rank);
    writer.write_u32_le(proto.required_spell);
    writer.write_u32_le(proto.required_honor_rank);
    writer.write_u32_le(proto.required_city_rank);
    writer.write_u32_le(proto.required_reputation_faction);
    writer.write_u32_le(if proto.required_reputation_faction > 0 {
        proto.required_reputation_rank
    } else {
        0
    });
    writer.write_u32_le(proto.max_count);
    writer.write_u32_le(proto.stackable);
    writer.write_u32_le(proto.container_slots);
    for stat in &proto.stats {
        writer.write_u32_le(stat.stat_type);
        writer.write_u32_le(stat.value as u32);
    }
    for damage in &proto.damages {
        writer.write_f32_le(damage.min);
        writer.write_f32_le(damage.max);
        writer.write_u32_le(damage.damage_type);
    }
    writer.write_u32_le(proto.armor as u32);
    writer.write_u32_le(proto.holy_res as u32);
    writer.write_u32_le(proto.fire_res as u32);
    writer.write_u32_le(proto.nature_res as u32);
    writer.write_u32_le(proto.frost_res as u32);
    writer.write_u32_le(proto.shadow_res as u32);
    writer.write_u32_le(proto.arcane_res as u32);
    writer.write_u32_le(proto.delay);
    writer.write_u32_le(proto.ammo_type);
    writer.write_f32_le(proto.ranged_mod_range);

    for item_spell in &proto.spells {
        if let Some(spell) = spells.get(&item_spell.spell_id) {
            writer.write_u32_le(item_spell.spell_id);
            writer.write_u32_le(item_spell.trigger);
            writer.write_u32_le(item_spell.charges as u32);
            if item_spell.cooldown >= 0 || item_spell.category_cooldown >= 0 {
                writer.write_u32_le(item_spell.cooldown as u32);
                writer.write_u32_le(item_spell.category);
                writer.write_u32_le(item_spell.category_cooldown as u32);
            } else {
                writer.write_u32_le(spell.recovery_time);
                writer.write_u32_le(spell.category);
                writer.write_u32_le(spell.category_recovery_time);
            }
        } else {
            writer.write_u32_le(0);
            writer.write_u32_le(0);
            writer.write_u32_le(0);
            writer.write_u32_le(u32::MAX);
            writer.write_u32_le(0);
            writer.write_u32_le(u32::MAX);
        }
    }

    writer.write_u32_le(proto.bonding);
    writer.write_cstring(&proto.description);
    writer.write_u32_le(proto.page_text);
    writer.write_u32_le(proto.language_id);
    writer.write_u32_le(proto.page_material);
    writer.write_u32_le(proto.start_quest);
    writer.write_u32_le(proto.lock_id);
    writer.write_u32_le(proto.material as u32);
    writer.write_u32_le(proto.sheath);
    writer.write_u32_le(proto.random_property);
    writer.write_u32_le(proto.block);
    writer.write_u32_le(proto.item_set);
    writer.write_u32_le(proto.max_durability);
    writer.write_u32_le(proto.area);
    writer.write_u32_le(proto.map as u32);
    writer.write_u32_le(proto.bag_family as u32);
    finish(opcode_id::SMSG_ITEM_QUERY_SINGLE_RESPONSE, writer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_query_response_matches_5875_shape() {
        let row = NameQueryRow {
            name: "Groktar".to_string(),
            race: 2,
            gender: 0,
            class: 1,
        };
        let packet = build_name_query_response(2, &row);
        assert_eq!(packet.opcode(), opcode_id::SMSG_NAME_QUERY_RESPONSE);

        let mut expected = Vec::new();
        expected.extend_from_slice(&2u64.to_le_bytes());
        expected.extend_from_slice(b"Groktar\0\0");
        expected.extend_from_slice(&2u32.to_le_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn creature_query_response_matches_5875_shape() {
        let info = CreatureInfo {
            name: "Mottled Boar".to_string(),
            subname: "Boar".to_string(),
            static_flags1: 0x0021_0010,
            static_flags2: 0x0000_0008,
            creature_type: 1,
            pet_family: 5,
            rank: 1,
            pet_spell_list_id: 9,
            display_id: [123, 456, 0, 0],
            civilian: true,
            racial_leader: true,
            ..CreatureInfo::default()
        };

        let packet = build_creature_query_response(3098, Some(&info));
        assert_eq!(packet.opcode(), opcode_id::SMSG_CREATURE_QUERY_RESPONSE);

        let mut expected = Vec::new();
        expected.extend_from_slice(&3098u32.to_le_bytes());
        expected.extend_from_slice(b"Mottled Boar\0\0\0\0Boar\0");
        expected.extend_from_slice(&0x47u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&5u32.to_le_bytes());
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&9u32.to_le_bytes());
        expected.extend_from_slice(&123u32.to_le_bytes());
        expected.extend_from_slice(&[1, 1]);
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn gameobject_query_response_matches_5875_shape() {
        let mut data = [0; 24];
        data[0] = -1;
        data[1] = 2;
        data[23] = 24;
        let info = GameObjectInfo {
            go_type: 3,
            display_id: 259,
            name: "Battered Chest".to_string(),
            icon: "ICON".to_string(),
            data,
            ..GameObjectInfo::default()
        };

        let packet = build_gameobject_query_response(2843, Some(&info));
        assert_eq!(packet.opcode(), opcode_id::SMSG_GAMEOBJECT_QUERY_RESPONSE);

        let mut expected = Vec::new();
        expected.extend_from_slice(&2843u32.to_le_bytes());
        expected.extend_from_slice(&3u32.to_le_bytes());
        expected.extend_from_slice(&259u32.to_le_bytes());
        expected.extend_from_slice(b"Battered Chest\0\0\0\0ICON\0");
        expected.extend_from_slice(&u32::MAX.to_le_bytes());
        expected.extend_from_slice(&2u32.to_le_bytes());
        expected.extend_from_slice(&[0; 21 * 4]);
        expected.extend_from_slice(&24u32.to_le_bytes());
        assert_eq!(packet.body(), expected.as_slice());
    }

    #[test]
    fn item_query_response_matches_5875_shape() {
        use mangos_world_data::item_prototype::{ItemDamage, ItemSpell, ItemStat};

        let mut proto = ItemPrototype {
            item_id: 6948,
            class: 15,
            subclass: 1,
            name: "Hearthstone".to_string(),
            description: "Returns you home.".to_string(),
            display_id: 6418,
            quality: 1,
            flags: 0x8,
            allowable_class: u32::MAX,
            allowable_race: u32::MAX,
            armor: -3,
            delay: 2000,
            ranged_mod_range: 15.0,
            bonding: 1,
            material: -1,
            block: 7,
            max_durability: 42,
            discovered: true,
            ..ItemPrototype::default()
        };
        proto.stats[0] = ItemStat {
            stat_type: 4,
            value: -2,
        };
        proto.damages[0] = ItemDamage {
            min: 1.5,
            max: 3.5,
            damage_type: 2,
        };
        proto.spells[0] = ItemSpell {
            spell_id: 8690,
            charges: -1,
            cooldown: -1,
            category: 44,
            category_cooldown: -1,
            ..ItemSpell::default()
        };
        proto.spells[1] = ItemSpell {
            spell_id: 123,
            trigger: 1,
            charges: 2,
            cooldown: 500,
            category: 9,
            category_cooldown: 600,
            ..ItemSpell::default()
        };
        proto.spells[2] = ItemSpell {
            spell_id: 999,
            ..ItemSpell::default()
        };

        let mut spells = HashMap::new();
        spells.insert(
            8690,
            SpellEntry {
                id: 8690,
                category: 45,
                recovery_time: 3_600_000,
                category_recovery_time: 7000,
                ..SpellEntry::default()
            },
        );
        spells.insert(
            123,
            SpellEntry {
                id: 123,
                category: 10,
                recovery_time: 8000,
                category_recovery_time: 9000,
                ..SpellEntry::default()
            },
        );

        let packet = build_item_query_response(6948, Some(&proto), false, &spells);
        assert_eq!(packet.opcode(), opcode_id::SMSG_ITEM_QUERY_SINGLE_RESPONSE);
        let body = packet.body();
        assert_eq!(body.len(), 485);
        let word = |offset: usize| u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap());
        assert_eq!(word(0), 6948);
        assert_eq!(word(4), 15);
        assert_eq!(word(8), 1);
        assert_eq!(&body[12..27], b"Hearthstone\0\0\0\0");
        assert_eq!(word(27), 6418);
        assert_eq!(word(107), 4);
        assert_eq!(word(111), (-2i32) as u32);
        assert_eq!(word(287), 8690);
        assert_eq!(word(295), (-1i32) as u32);
        assert_eq!(word(299), 3_600_000); // Spell.dbc fallback
        assert_eq!(word(303), 45);
        assert_eq!(word(307), 7000);
        assert_eq!(word(311), 123);
        assert_eq!(word(323), 500); // item_template cooldown wins
        assert_eq!(word(327), 9);
        assert_eq!(word(331), 600);
        assert_eq!(word(335), 0); // missing spell slot
        assert_eq!(word(347), u32::MAX);
        assert_eq!(word(407), 1); // bonding
        assert_eq!(word(449), u32::MAX); // material -1
        assert_eq!(word(461), 7); // block
        assert_eq!(word(469), 42); // max durability
    }

    #[test]
    fn missing_templates_send_only_the_not_found_entry() {
        let creature = build_creature_query_response(0x4d, None);
        assert_eq!(creature.body(), (0x4du32 | 0x8000_0000).to_le_bytes());

        let gameobject = build_gameobject_query_response(0x58, None);
        assert_eq!(gameobject.body(), (0x58u32 | 0x8000_0000).to_le_bytes());

        let undiscovered = ItemPrototype {
            item_id: 0x59,
            ..ItemPrototype::default()
        };
        let item = build_item_query_response(0x59, Some(&undiscovered), false, &HashMap::new());
        assert_eq!(item.body(), (0x59u32 | 0x8000_0000).to_le_bytes());
    }
}
