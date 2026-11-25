//! Census + spot checks for the generated 5875 update-field table.
//! Expected values are read directly from
//! `UpdateFields_1_12_1.cpp` / `UpdateFields.h` /
//! `UpdateFields.cpp`; the .h offset comments are known-stale and
//! are never used here (that trap is what `end_offsets` pins down).

use mangos_guid::TypeId;
use mangos_protocol::update_field_table::{
    CONTAINER_END, CORPSE_END, DYNAMICOBJECT_END, GAMEOBJECT_END, ITEM_END, OBJECT_END, PLAYER_END,
    UNIT_END, UPDATE_FIELDS_5875, offsets,
};
use mangos_protocol::update_fields::{
    TypeMask, UpdateFieldFlags, UpdateFieldType, field_by_mask_and_offset, field_by_name,
    index_of_update_field_for_current_build, update_field_flags,
};

#[test]
fn census_and_end_offsets() {
    assert_eq!(UPDATE_FIELDS_5875.len(), 324, "316 fields + 8 END rows");
    // The header comments claim CONTAINER_END=0x74 and (by addition
    // from a wrong base) a smaller PLAYER_END; the .cpp rows are
    // authoritative.
    assert_eq!(OBJECT_END, 6);
    assert_eq!(ITEM_END, 48);
    assert_eq!(CONTAINER_END, 122);
    assert_eq!(UNIT_END, 188);
    assert_eq!(PLAYER_END, 1282);
    assert_eq!(GAMEOBJECT_END, 26);
    assert_eq!(DYNAMICOBJECT_END, 16);
    assert_eq!(CORPSE_END, 38);
}

#[test]
fn spot_rows_match_cpp() {
    let guid = field_by_name("OBJECT_FIELD_GUID").expect("row exists");
    assert_eq!(guid.object_type_mask, TypeMask::OBJECT);
    assert_eq!(guid.offset, 0);
    assert_eq!(guid.size, 2);
    assert_eq!(guid.value_type, UpdateFieldType::Guid);
    assert_eq!(guid.flags.raw(), UpdateFieldFlags::PUBLIC);

    let health = field_by_name("UNIT_FIELD_HEALTH").expect("row exists");
    assert_eq!(health.offset, 0x16);
    assert_eq!(health.flags.raw(), UpdateFieldFlags::DYNAMIC);

    let aura = field_by_name("UNIT_FIELD_AURA").expect("row exists");
    assert_eq!((aura.offset, aura.size), (0x2F, 48));

    let duel = field_by_name("PLAYER_DUEL_ARBITER").expect("row exists");
    assert_eq!(duel.offset, UNIT_END, "player block starts at UNIT_END");

    let go_dyn = field_by_name("GAMEOBJECT_DYN_FLAGS").expect("row exists");
    assert_eq!(go_dyn.offset, 0x13);
    assert_eq!(go_dyn.flags.raw(), UpdateFieldFlags::DYNAMIC);

    let mindmg = field_by_name("UNIT_FIELD_MINDAMAGE").expect("row exists");
    assert_eq!(mindmg.value_type, UpdateFieldType::Float);
    assert_eq!(
        mindmg.flags.raw(),
        UpdateFieldFlags::PRIVATE | UpdateFieldFlags::OWNER_ONLY | UpdateFieldFlags::SPECIAL_INFO
    );

    assert!(field_by_name("NOT_A_FIELD").is_none());
}

#[test]
fn offsets_agree_with_table_rows() {
    assert_eq!(offsets::OBJECT_FIELD_GUID, 0);
    assert_eq!(offsets::UNIT_FIELD_HEALTH, 0x16);
    assert_eq!(offsets::UNIT_FIELD_AURA, 0x2F);
    assert_eq!(offsets::PLAYER_FLAGS, 190);
    assert_eq!(offsets::PLAYER_END, 1282);
    assert_eq!(offsets::CONTAINER_FIELD_SLOT_1, 0x32);
}

#[test]
fn flags_arrays_follow_cpp_setup() {
    let player = update_field_flags(TypeId::Player).expect("player array");
    assert_eq!(player.len(), PLAYER_END as usize);
    // OBJECT block is shared into the player array via mask union.
    assert_eq!(player[0], UpdateFieldFlags::PUBLIC, "guid word 0");
    assert_eq!(player[1], UpdateFieldFlags::PUBLIC, "guid word 1");
    assert_eq!(player[5], UpdateFieldFlags::NONE, "object padding");
    assert_eq!(player[0x16], UpdateFieldFlags::DYNAMIC, "health");
    assert_eq!(
        player[0x80],
        UpdateFieldFlags::PRIVATE,
        "ranged attack time"
    );
    assert_eq!(player[0xC6], UpdateFieldFlags::GROUP_ONLY, "quest log 1");
    // Every aura word inherits the row's flags.
    assert!(
        player[0x2F..0x2F + 48]
            .iter()
            .all(|&f| f == UpdateFieldFlags::PUBLIC)
    );

    // Unit shares the player array; Item/Container the container one.
    assert_eq!(
        update_field_flags(TypeId::Unit).expect("unit").len(),
        PLAYER_END as usize
    );
    let container = update_field_flags(TypeId::Item).expect("item->container");
    assert_eq!(container.len(), CONTAINER_END as usize);
    assert_eq!(
        container[0x32], // CONTAINER_FIELD_SLOT_1
        UpdateFieldFlags::PUBLIC
    );

    let go = update_field_flags(TypeId::GameObject).expect("go array");
    assert_eq!(go.len(), GAMEOBJECT_END as usize);
    assert_eq!(go[0x13], UpdateFieldFlags::DYNAMIC);

    assert!(update_field_flags(TypeId::Object).is_none());
}

#[test]
fn mask_and_offset_lookup_finds_covering_row() {
    // Interior word of the 48-word aura array.
    let row = field_by_mask_and_offset(TypeMask::UNIT, 0x2F + 20).expect("aura row");
    assert_eq!(row.name, "UNIT_FIELD_AURA");
    // Same numeric offset under the gameobject mask is out of block.
    assert!(field_by_mask_and_offset(TypeMask::GAMEOBJECT, 0x2F + 20).is_none());
    // Exact-offset hit.
    let row = field_by_mask_and_offset(TypeMask::PLAYER, 0xBC).expect("duel arbiter");
    assert_eq!(row.name, "PLAYER_DUEL_ARBITER");
}

#[test]
fn each_block_tiles_without_gaps() {
    // Re-derives the generator's invariant from the table itself:
    // per mask, sizes sum exactly over [start, END).
    let groups = [
        (TypeMask::OBJECT, 0, OBJECT_END),
        (TypeMask::ITEM, OBJECT_END, ITEM_END),
        (TypeMask::CONTAINER, ITEM_END, CONTAINER_END),
        (TypeMask::UNIT, OBJECT_END, UNIT_END),
        (TypeMask::PLAYER, UNIT_END, PLAYER_END),
        (TypeMask::GAMEOBJECT, OBJECT_END, GAMEOBJECT_END),
        (TypeMask::DYNAMICOBJECT, OBJECT_END, DYNAMICOBJECT_END),
        (TypeMask::CORPSE, OBJECT_END, CORPSE_END),
    ];
    for (mask, start, end) in groups {
        let tiled: u16 = UPDATE_FIELDS_5875
            .iter()
            .filter(|f| f.object_type_mask == mask)
            .map(|f| f.size)
            .sum();
        assert_eq!(start + tiled, end, "mask {mask:?} must tile its block");
    }
}

#[test]
fn db_index_mapping_is_identity_on_5875() {
    for db_index in [9, 19, 21, 35, 36, 46, 143, 147, 155, 161, 165, 190, 999] {
        assert_eq!(index_of_update_field_for_current_build(db_index), db_index);
    }
}
