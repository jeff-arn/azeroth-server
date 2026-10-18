//! Block-builder tests with expected bytes hand-assembled from
//! `Object.cpp:236-940` (never from the Rust builders).

use mangos_entities::object::{MarkEffect, ObjectBase};
use mangos_entities::update::{
    BuildEffects, MovementSection, ObjectView, Viewer, build_create_block, build_movement_block,
    build_out_of_range, build_values_block, destroy_packet, direct_public_value_update_range,
};
use mangos_guid::{HighGuid, TypeId};
use mangos_movement::movement_info::{MovementFlags, MovementInfo, Position};
use mangos_protocol::update_data::{
    CompressionConfig, ObjectUpdateFlags, ObjectUpdateType, SMSG_DESTROY_OBJECT, UpdateData,
};
use mangos_protocol::update_field_table::{GAMEOBJECT_END, PLAYER_END, UNIT_END, offsets};
use mangos_protocol::update_fields::TypeMask;

fn go_base() -> ObjectBase {
    let mut base = ObjectBase::new(
        TypeId::GameObject,
        TypeMask::OBJECT.union(TypeMask::GAMEOBJECT),
        ObjectUpdateFlags(ObjectUpdateFlags::ALL | ObjectUpdateFlags::HAS_POSITION),
        GAMEOBJECT_END,
    );
    base.create(0x123, 55, HighGuid::GAMEOBJECT);
    base.set_entry(55);
    base
}

#[test]
fn go_create_block_layout() {
    let base = go_base();
    let view = ObjectView {
        movement: MovementSection::HasPosition(Position::new(10.0, 20.0, 30.0, 1.5)),
        ..ObjectView::default()
    };
    let mut data = UpdateData::new();
    let mut effects = BuildEffects::default();
    build_create_block(
        &base,
        &view,
        &Viewer::default(),
        &mut data,
        5000,
        &mut effects,
    );

    let mut want = Vec::new();
    want.push(ObjectUpdateType::CreateObject as u8);
    base.object_guid().write_packed(&mut want);
    want.push(TypeId::GameObject as u8);
    // Movement: flags ALL|HAS_POSITION, position, ALL -> u32 1.
    want.push(ObjectUpdateFlags::ALL | ObjectUpdateFlags::HAS_POSITION);
    for f in [10.0f32, 20.0, 30.0, 1.5] {
        want.extend_from_slice(&f.to_le_bytes());
    }
    want.extend_from_slice(&1u32.to_le_bytes());
    // Values: 1 mask block; bits guid(0,1) type(2) entry(3) dyn(19, forced).
    want.push(1);
    want.extend_from_slice(
        &((1u32 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 19)).to_le_bytes(),
    );
    want.extend_from_slice(&base.object_guid().raw().to_le_bytes()); // words 0,1
    want.extend_from_slice(&0x21u32.to_le_bytes()); // TYPE = OBJECT|GAMEOBJECT
    want.extend_from_slice(&55u32.to_le_bytes()); // ENTRY
    want.extend_from_slice(&0u32.to_le_bytes()); // DYN_FLAGS forced 0 (not quest-activated)

    assert_eq!(data.packets()[0].bytes(), want.as_slice());
    assert_eq!(
        effects.go_quest_activated,
        vec![(base.object_guid(), false)],
        "GO quest activation is recorded for the viewer"
    );
}

#[test]
fn go_quest_activated_dyn_flags() {
    let base = go_base();
    let mut view = ObjectView {
        movement: MovementSection::HasPosition(Position::default()),
        ..ObjectView::default()
    };
    view.rewrites.go_activate_to_quest = true;
    view.rewrites.go_type = 3; // CHEST
    let mut data = UpdateData::new();
    let mut effects = BuildEffects::default();
    build_create_block(&base, &view, &Viewer::default(), &mut data, 0, &mut effects);

    let bytes = data.packets()[0].bytes();
    // Last word = u16 LO_ACTIVATE=1 | u16 0.
    assert_eq!(&bytes[bytes.len() - 4..], &[1, 0, 0, 0]);
    assert_eq!(effects.go_quest_activated, vec![(base.object_guid(), true)]);

    // A non-activating GO type emits plain 0 even when "activated".
    view.rewrites.go_type = 0; // DOOR
    let mut data = UpdateData::new();
    build_create_block(&base, &view, &Viewer::default(), &mut data, 0, &mut effects);
    let bytes = data.packets()[0].bytes();
    assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
}

fn creature_base() -> ObjectBase {
    let mut base = ObjectBase::new(
        TypeId::Unit,
        TypeMask::OBJECT.union(TypeMask::UNIT),
        ObjectUpdateFlags(
            ObjectUpdateFlags::ALL | ObjectUpdateFlags::LIVING | ObjectUpdateFlags::HAS_POSITION,
        ),
        UNIT_END,
    );
    base.create(7, 69, HighGuid::UNIT);
    base.set_entry(69);
    base.set_u32(offsets::UNIT_FIELD_HEALTH, 50);
    base.set_u32(offsets::UNIT_FIELD_MAXHEALTH, 200);
    base.set_u32(offsets::UNIT_FIELD_LEVEL, 12);
    base.set_u32(offsets::UNIT_FIELD_DISPLAYID, 1000);
    base
}

#[test]
fn creature_create_living_block_and_health_percent() {
    let base = creature_base();
    let mut info = MovementInfo::default();
    info.flags.add(MovementFlags::ROOT); // must be stripped for creatures
    let view = ObjectView {
        movement: MovementSection::Living {
            info,
            position: Position::new(1.0, 2.0, 3.0, 0.5),
            speeds: [2.5, 7.0, 4.5, 4.722_222, 2.5, std::f32::consts::PI],
            is_creature: true,
            spline: None,
        },
        ..ObjectView::default()
    };
    // Stranger viewer: cannot see real health, ShowHealthValues off.
    let mut data = UpdateData::new();
    let mut effects = BuildEffects::default();
    build_create_block(
        &base,
        &view,
        &Viewer::default(),
        &mut data,
        4000,
        &mut effects,
    );
    let bytes = data.packets()[0].bytes();

    let mut want = Vec::new();
    want.push(ObjectUpdateType::CreateObject as u8);
    base.object_guid().write_packed(&mut want);
    want.push(TypeId::Unit as u8);
    want.push(ObjectUpdateFlags::ALL | ObjectUpdateFlags::LIVING | ObjectUpdateFlags::HAS_POSITION);
    // MovementInfo: flags 0 (ROOT stripped), stime = now + 1000,
    // authoritative position, fallTime 0.
    want.extend_from_slice(&0u32.to_le_bytes());
    want.extend_from_slice(&5000u32.to_le_bytes());
    for f in [1.0f32, 2.0, 3.0, 0.5] {
        want.extend_from_slice(&f.to_le_bytes());
    }
    want.extend_from_slice(&0u32.to_le_bytes());
    for f in [2.5f32, 7.0, 4.5, 4.722_222, 2.5, std::f32::consts::PI] {
        want.extend_from_slice(&f.to_le_bytes());
    }
    want.extend_from_slice(&1u32.to_le_bytes()); // ALL
    // Values mask/words are checked by parsing below instead of a
    // full golden (guid/type/entry + 4 set unit fields).
    let head_len = want.len();
    assert_eq!(&bytes[..head_len], want.as_slice(), "movement head");

    // Parse mask + words; health must arrive as percent 25 / 100.
    let rest = &bytes[head_len..];
    let block_count = rest[0] as usize;
    assert_eq!(block_count, (UNIT_END as usize).div_ceil(32));
    let mut words: Vec<(u16, u32)> = Vec::new();
    let mut cursor = 1 + block_count * 4;
    for index in 0..UNIT_END {
        let block = u32::from_le_bytes(
            rest[1 + (index as usize / 32) * 4..1 + (index as usize / 32) * 4 + 4]
                .try_into()
                .unwrap(),
        );
        if block >> (index % 32) & 1 == 1 {
            words.push((
                index,
                u32::from_le_bytes(rest[cursor..cursor + 4].try_into().unwrap()),
            ));
            cursor += 4;
        }
    }
    let get = |i: u16| words.iter().find(|(idx, _)| *idx == i).map(|(_, w)| *w);
    assert_eq!(
        get(offsets::UNIT_FIELD_HEALTH),
        Some(25),
        "hidden as percent"
    );
    assert_eq!(get(offsets::UNIT_FIELD_MAXHEALTH), Some(100));
    assert_eq!(get(offsets::UNIT_FIELD_LEVEL), Some(12));
    assert_eq!(get(offsets::UNIT_FIELD_DISPLAYID), Some(1000));
    assert_eq!(get(offsets::OBJECT_FIELD_ENTRY), Some(69));
}

#[test]
fn values_block_respects_private_and_empty_rules() {
    let mut base = creature_base();
    base.add_to_world(); // syncs mirror (create-time values are clean now)
    base.set_u32(offsets::UNIT_FIELD_HEALTH, 60);
    base.set_f32(offsets::UNIT_FIELD_RANGEDATTACKTIME, 2000.0); // PRIVATE, stored float

    let view = ObjectView::default();
    let stranger = Viewer {
        can_see_health: true,
        ..Viewer::default()
    };
    let mut data = UpdateData::new();
    let mut effects = BuildEffects::default();
    build_values_block(&base, &view, &stranger, &mut data, &mut effects);
    let bytes = data.packets()[0].bytes().to_vec();
    // type + packed guid, then mask: only the health word present,
    // so the block is header + 1 word.
    assert_eq!(bytes[0], ObjectUpdateType::Values as u8);
    let tail = &bytes[bytes.len() - 4..];
    assert_eq!(
        tail,
        &60u32.to_le_bytes(),
        "only health crosses to strangers"
    );

    // Self viewer also gets the private field.
    let self_viewer = Viewer {
        is_self: true,
        can_see_health: true,
        ..Viewer::default()
    };
    let mut data = UpdateData::new();
    build_values_block(&base, &view, &self_viewer, &mut data, &mut effects);
    let bytes = data.packets()[0].bytes();
    assert_eq!(
        &bytes[bytes.len() - 4..],
        &2000u32.to_le_bytes(),
        "ranged attack time (float-stored, sent as u32) is the last dirty word"
    );

    // Clean object: no block at all.
    base.clear_update_mask(false);
    let mut data = UpdateData::new();
    build_values_block(&base, &view, &self_viewer, &mut data, &mut effects);
    assert!(data.packets().is_empty());
}

#[test]
fn movement_block_uses_unpacked_guid() {
    let base = creature_base();
    let view = ObjectView {
        movement: MovementSection::HasPosition(Position::new(4.0, 5.0, 6.0, 0.0)),
        ..ObjectView::default()
    };
    let mut data = UpdateData::new();
    build_movement_block(
        &base,
        &view,
        &mut data,
        ObjectUpdateFlags(ObjectUpdateFlags::HAS_POSITION),
        0,
    );
    let bytes = data.packets()[0].bytes();
    assert_eq!(bytes[0], ObjectUpdateType::Movement as u8);
    assert_eq!(
        &bytes[1..9],
        &base.object_guid().raw().to_le_bytes(),
        "movement blocks carry the raw (unpacked) GUID"
    );
    assert_eq!(bytes[9], ObjectUpdateFlags::HAS_POSITION);
}

#[test]
fn destroy_and_out_of_range() {
    let base = creature_base();
    let packet = destroy_packet(&base);
    assert_eq!(packet.opcode(), SMSG_DESTROY_OBJECT);
    assert_eq!(packet.body(), &base.object_guid().raw().to_le_bytes());

    let mut data = UpdateData::new();
    build_out_of_range(&base, &mut data);
    assert!(data.out_of_range_guids().contains(&base.object_guid()));
}

#[test]
fn direct_public_update_range_mirrors_and_aborts_clean() {
    let mut base = creature_base();
    base.add_to_world();
    // Nothing changed since add_to_world synced the mirror.
    assert!(
        direct_public_value_update_range(
            &mut base,
            u32::from(offsets::UNIT_FIELD_HEALTH),
            1,
            CompressionConfig::default()
        )
        .is_none()
    );

    base.set_u32(offsets::UNIT_FIELD_HEALTH, 33);
    let packet = direct_public_value_update_range(
        &mut base,
        u32::from(offsets::UNIT_FIELD_HEALTH),
        1,
        CompressionConfig::default(),
    )
    .expect("changed range produces a packet");
    assert_eq!(
        &packet.body()[packet.body().len() - 4..],
        &33u32.to_le_bytes()
    );
    // Mirror synced inline: a second call aborts.
    assert!(
        direct_public_value_update_range(
            &mut base,
            u32::from(offsets::UNIT_FIELD_HEALTH),
            1,
            CompressionConfig::default()
        )
        .is_none()
    );
}

#[test]
fn marking_lifecycle_matches_cpp() {
    let mut base = ObjectBase::new(
        TypeId::Unit,
        TypeMask::OBJECT.union(TypeMask::UNIT),
        ObjectUpdateFlags(ObjectUpdateFlags::ALL),
        UNIT_END,
    );
    // Not in world: changes are dropped marks.
    assert_eq!(base.set_u32(22, 1), MarkEffect::NotInWorld);
    base.add_to_world();
    // Delayed path for non-items.
    assert_eq!(base.set_u32(22, 2), MarkEffect::Delayed);
    assert!(!base.is_object_updated());
    let effects = base.execute_delayed_actions();
    assert!(effects.add_to_update_list);
    assert!(base.is_object_updated());
    // Repeat execute: nothing owed.
    assert_eq!(
        base.execute_delayed_actions(),
        mangos_entities::object::DelayedEffects::default()
    );
    // Removal owes a list removal and clears the queued state.
    assert!(base.remove_from_world());
    assert!(!base.is_object_updated());

    // Items mark immediately.
    let mut item = ObjectBase::new(
        TypeId::Item,
        TypeMask::OBJECT.union(TypeMask::ITEM),
        ObjectUpdateFlags(ObjectUpdateFlags::ALL),
        48,
    );
    item.add_to_world();
    assert_eq!(item.set_u32(8, 1), MarkEffect::EnqueueNow);
    assert_eq!(item.set_u32(8, 2), MarkEffect::Queued);
}

#[test]
fn player_sized_values_block() {
    // Smoke: the 1282-word player store builds masks of 41 blocks.
    let mut base = ObjectBase::new(
        TypeId::Player,
        TypeMask::OBJECT
            .union(TypeMask::UNIT)
            .union(TypeMask::PLAYER),
        ObjectUpdateFlags(
            ObjectUpdateFlags::ALL | ObjectUpdateFlags::LIVING | ObjectUpdateFlags::HAS_POSITION,
        ),
        PLAYER_END,
    );
    base.create(1, 0, HighGuid::PLAYER);
    base.add_to_world();
    base.set_u32(offsets::PLAYER_FIELD_COINAGE, 12345);
    let self_viewer = Viewer {
        is_self: true,
        can_see_health: true,
        ..Viewer::default()
    };
    let mut data = UpdateData::new();
    let mut effects = BuildEffects::default();
    build_values_block(
        &base,
        &ObjectView::default(),
        &self_viewer,
        &mut data,
        &mut effects,
    );
    let bytes = data.packets()[0].bytes();
    let mask_count_index = 1 + base.pack_guid().len();
    assert_eq!(bytes[mask_count_index], 41, "ceil(1282/32) mask blocks");
    assert_eq!(
        &bytes[bytes.len() - 4..],
        &12345u32.to_le_bytes(),
        "coinage reaches its owner"
    );
}
