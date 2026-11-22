//! Player corpse entity (`Corpse.cpp` Create +
//! `Player::CreateCorpse` field fill): a stationary
//! world object with owner, appearance bytes, display,
//! equipment visuals, guild, and corpse flags.

use crate::object::ObjectBase;
use crate::unit::WorldObjectBase;
use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_movement::movement_info::Position;
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{CORPSE_END, offsets};
use mangos_protocol::update_fields::TypeMask;

/// `CORPSE_FLAG_BONES` (`Corpse.h:45`).
pub const CORPSE_FLAG_BONES: u32 = 0x01;
/// `CORPSE_FLAG_UNK2` (`Corpse.h:47`).
pub const CORPSE_FLAG_UNK2: u32 = 0x04;
/// `CORPSE_FLAG_HIDE_HELM` (`Corpse.h:48`).
pub const CORPSE_FLAG_HIDE_HELM: u32 = 0x08;
/// `CORPSE_FLAG_HIDE_CLOAK` (`Corpse.h:49`).
pub const CORPSE_FLAG_HIDE_CLOAK: u32 = 0x10;
/// `CORPSE_FLAG_LOOTABLE` (`Corpse.h:50`).
pub const CORPSE_FLAG_LOOTABLE: u32 = 0x20;

/// `CorpseType` (`Corpse.h`): bones are a separate
/// type-0 object spawned at conversion; resurrectable
/// corpses carry the death's `PvP` flavor (reclaim-delay
/// config gate).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorpseType {
    Bones = 0,
    ResurrectablePvE = 1,
    ResurrectablePvP = 2,
}

/// Appearance inputs copied from the dead player
/// (`Player::CreateCorpse` byte reads).
#[derive(Clone, Copy, Debug, Default)]
pub struct CorpseAppearance {
    pub race: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub display_id: u32,
    pub guild_id: u32,
    pub flags: u32,
    /// `display | inventory_type << 24` per equipment
    /// slot (`EQUIPMENT_SLOT_END` = 19).
    pub items: [u32; 19],
}

/// Persistence snapshot for one resurrectable
/// corpse (map → actor → `corpse` table; the
/// `SavePet` event pattern).
#[derive(Clone, Debug, PartialEq)]
pub struct CorpseSaveRow {
    pub guid: u32,
    pub player_guid: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation: f32,
    pub map: u32,
    pub time: u64,
    pub corpse_type: u32,
    pub instance: u32,
}

/// A resurrectable corpse or its bones successor.
pub struct Corpse {
    pub world: WorldObjectBase,
    pub owner: ObjectGuid,
    pub corpse_type: CorpseType,
    /// `m_time`: creation (ghost) time, unix seconds.
    pub ghost_time: u64,
    pub instance_id: u32,
}

impl Corpse {
    /// `Corpse::Create(guidlow, owner)` + the
    /// `CreateCorpse` field fill.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // mirrors the C++ field fill inputs
    pub fn create(
        guid_low: u32,
        map_id: u32,
        instance_id: u32,
        owner: ObjectGuid,
        corpse_type: CorpseType,
        position: Position,
        appearance: &CorpseAppearance,
        ghost_time: u64,
    ) -> Corpse {
        let mut object = ObjectBase::new(
            TypeId::Corpse,
            TypeMask::OBJECT.union(TypeMask::CORPSE),
            ObjectUpdateFlags(ObjectUpdateFlags::ALL | ObjectUpdateFlags::HAS_POSITION),
            CORPSE_END,
        );
        object.create(guid_low, 0, HighGuid::CORPSE);
        object.set_object_scale(1.0);
        let mut world = WorldObjectBase::new(object);
        world.map_id = map_id;
        world.relocate(position);
        let mut corpse = Corpse {
            world,
            owner,
            corpse_type,
            ghost_time,
            instance_id,
        };
        let v = corpse.world.object.values_mut();
        v.set_f32(offsets::CORPSE_FIELD_POS_X, position.x);
        v.set_f32(offsets::CORPSE_FIELD_POS_Y, position.y);
        v.set_f32(offsets::CORPSE_FIELD_POS_Z, position.z);
        v.set_f32(offsets::CORPSE_FIELD_FACING, position.o);
        v.set_guid(offsets::CORPSE_FIELD_OWNER, owner);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_1, 1, appearance.race);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_1, 2, appearance.gender);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_1, 3, appearance.skin);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_2, 0, appearance.face);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_2, 1, appearance.hair_style);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_2, 2, appearance.hair_color);
        v.set_byte(offsets::CORPSE_FIELD_BYTES_2, 3, appearance.facial_hair);
        v.set_u32(offsets::CORPSE_FIELD_FLAGS, appearance.flags);
        v.set_u32(offsets::CORPSE_FIELD_DISPLAY_ID, appearance.display_id);
        v.set_u32(offsets::CORPSE_FIELD_GUILD, appearance.guild_id);
        for (slot, item) in appearance.items.iter().enumerate() {
            if *item != 0 {
                v.set_u32(offsets::CORPSE_FIELD_ITEM + slot as u16, *item);
            }
        }
        corpse
    }

    /// The bones successor (`Map::RemoveCorpses`): same
    /// guid and fields from offset 3, bones flags,
    /// equipment visuals cleared.
    #[must_use]
    pub fn to_bones(&self) -> Corpse {
        let mut bones = Corpse::create(
            self.guid().counter(),
            self.world.map_id,
            self.instance_id,
            self.owner,
            CorpseType::Bones,
            self.world.position(),
            &CorpseAppearance::default(),
            self.ghost_time,
        );
        // Copy every field past guid/type, then override
        // flags/owner/items like the fork.
        let src = self.world.object.values();
        let dst = bones.world.object.values_mut();
        for i in 3..CORPSE_END {
            dst.set_u32(i, src.get_u32(i));
        }
        dst.set_u32(
            offsets::CORPSE_FIELD_FLAGS,
            CORPSE_FLAG_UNK2 | CORPSE_FLAG_BONES,
        );
        dst.set_guid(offsets::CORPSE_FIELD_OWNER, self.owner);
        for slot in 0..19u16 {
            if dst.get_u32(offsets::CORPSE_FIELD_ITEM + slot) != 0 {
                dst.set_u32(offsets::CORPSE_FIELD_ITEM + slot, 0);
            }
        }
        bones
    }

    #[must_use]
    pub fn guid(&self) -> ObjectGuid {
        self.world.object.object_guid()
    }
}
