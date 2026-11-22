//! Item object projection (Tier 4 step 4b1): rows
//! stay the inventory model; this builds the
//! client-facing `ObjectBase` for one row exactly
//! as the fork's `Item::LoadFromDB` + `_Create`
//! fills it (`Item.cpp`, `UpdateFields_1_12_1.h`).
//! There is no persistent Item entity — bases are
//! rebuilt from rows for create/value packets.

use mangos_db::character_load::InventoryRow;
use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::update_fields::TypeMask;

use crate::object::ObjectBase;

/// Prototype facts the projection needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemProtoFacts {
    pub max_durability: u32,
    /// `ContainerSlots`; >0 makes a container.
    pub container_slots: u32,
}

/// Item object GUID for an instance low.
#[must_use]
pub fn item_guid(item_low: u32) -> ObjectGuid {
    ObjectGuid::with_entry(HighGuid::ITEM, 0, item_low)
}

fn parse_i32_words(text: &str, count: usize) -> Vec<i32> {
    let mut out: Vec<i32> = text
        .split_whitespace()
        .map(|t| t.parse().unwrap_or(0))
        .collect();
    out.resize(count, 0);
    out.truncate(count);
    out
}

/// Build the object base for one inventory row.
/// `contents` = `(slot, item guid)` pairs when the
/// row is a container (its contained rows).
#[must_use]
pub fn item_object(
    row: &InventoryRow,
    owner: ObjectGuid,
    facts: ItemProtoFacts,
    contents: &[(u8, ObjectGuid)],
) -> ObjectBase {
    let inst = &row.item;
    let is_container = facts.container_slots > 0;
    let (type_id, mask, count) = if is_container {
        (
            TypeId::Container,
            TypeMask::OBJECT
                .union(TypeMask::ITEM)
                .union(TypeMask::CONTAINER),
            offsets::CONTAINER_END,
        )
    } else {
        (
            TypeId::Item,
            TypeMask::OBJECT.union(TypeMask::ITEM),
            offsets::ITEM_END,
        )
    };
    let mut object = ObjectBase::new(
        type_id,
        mask,
        ObjectUpdateFlags(ObjectUpdateFlags::ALL),
        count,
    );
    object.create(inst.item_guid, 0, HighGuid::ITEM);
    object.set_entry(inst.item_id);
    object.set_object_scale(1.0);
    fill_item_values(&mut object, row, owner, facts, contents);
    object
}

/// Write every item field from the row (create and
/// values-update paths share this; the caller
/// clears the dirty mask between the pre- and
/// post-mutation fills for updates). Container
/// slots are zeroed first so vacated slots clear.
pub fn fill_item_values(
    object: &mut ObjectBase,
    row: &InventoryRow,
    owner: ObjectGuid,
    facts: ItemProtoFacts,
    contents: &[(u8, ObjectGuid)],
) {
    let inst = &row.item;
    let contained = if row.bag == 0 {
        owner
    } else {
        item_guid(row.bag)
    };
    let creator = if inst.creator_guid == 0 {
        ObjectGuid::EMPTY
    } else {
        ObjectGuid::with_entry(HighGuid::PLAYER, 0, inst.creator_guid)
    };
    let gift_creator = if inst.gift_creator_guid == 0 {
        ObjectGuid::EMPTY
    } else {
        ObjectGuid::with_entry(HighGuid::PLAYER, 0, inst.gift_creator_guid)
    };
    let v = object.values_mut();
    v.set_guid(offsets::ITEM_FIELD_OWNER, owner);
    v.set_guid(offsets::ITEM_FIELD_CONTAINED, contained);
    v.set_guid(offsets::ITEM_FIELD_CREATOR, creator);
    v.set_guid(offsets::ITEM_FIELD_GIFTCREATOR, gift_creator);
    v.set_u32(offsets::ITEM_FIELD_STACK_COUNT, inst.count);
    v.set_i32(offsets::ITEM_FIELD_DURATION, inst.duration);
    for (i, charge) in parse_i32_words(&inst.charges, 5).into_iter().enumerate() {
        v.set_i32(
            offsets::ITEM_FIELD_SPELL_CHARGES + u16::try_from(i).unwrap_or(0),
            charge,
        );
    }
    v.set_u32(offsets::ITEM_FIELD_FLAGS, inst.flags);
    for (i, word) in parse_i32_words(&inst.enchantments, 21)
        .into_iter()
        .enumerate()
    {
        v.set_i32(
            offsets::ITEM_FIELD_ENCHANTMENT + u16::try_from(i).unwrap_or(0),
            word,
        );
    }
    // `ITEM_FIELD_PROPERTY_SEED` is never written by
    // this fork (stays 0).
    v.set_i32(
        offsets::ITEM_FIELD_RANDOM_PROPERTIES_ID,
        i32::from(inst.random_property_id),
    );
    v.set_u32(offsets::ITEM_FIELD_ITEM_TEXT_ID, inst.text);
    v.set_u32(offsets::ITEM_FIELD_DURABILITY, u32::from(inst.durability));
    v.set_u32(offsets::ITEM_FIELD_MAXDURABILITY, facts.max_durability);
    if facts.container_slots > 0 {
        v.set_u32(offsets::CONTAINER_FIELD_NUM_SLOTS, facts.container_slots);
        for slot in 0..36u16 {
            v.set_guid(
                offsets::CONTAINER_FIELD_SLOT_1 + slot * 2,
                ObjectGuid::EMPTY,
            );
        }
        for (slot, guid) in contents {
            v.set_guid(
                offsets::CONTAINER_FIELD_SLOT_1 + u16::from(*slot) * 2,
                *guid,
            );
        }
    }
}
