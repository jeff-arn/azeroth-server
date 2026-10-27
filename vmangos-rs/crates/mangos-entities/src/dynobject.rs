//! Dynamic object entity (`DynamicObject.cpp`
//! Create): a ground spell visual with caster,
//! spell, radius, and position fields.

use crate::object::ObjectBase;
use crate::unit::WorldObjectBase;
use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_movement::movement_info::Position;
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{DYNAMICOBJECT_END, offsets};
use mangos_protocol::update_fields::TypeMask;

/// `DynamicObject` world object plus the map-layer
/// lifetime state (`m_aliveDuration` et al).
pub struct DynObject {
    pub world: WorldObjectBase,
    pub caster: ObjectGuid,
    pub spell_id: u32,
    pub effect_index: usize,
    pub radius: f32,
    pub alive_ms: i32,
    pub positive: bool,
}

impl DynObject {
    /// `DynamicObject::Create` field writes (AREA
    /// SPELL type; the 1543 flare diameter quirk
    /// included).
    #[must_use]
    #[allow(clippy::too_many_arguments)] // mirrors the C++ Create signature
    pub fn create(
        guid_low: u32,
        map_id: u32,
        caster: ObjectGuid,
        spell_id: u32,
        effect_index: usize,
        position: Position,
        duration_ms: i32,
        radius: f32,
        positive: bool,
    ) -> DynObject {
        let mut object = ObjectBase::new(
            TypeId::DynamicObject,
            TypeMask::OBJECT.union(TypeMask::DYNAMICOBJECT),
            ObjectUpdateFlags(ObjectUpdateFlags::ALL | ObjectUpdateFlags::HAS_POSITION),
            DYNAMICOBJECT_END,
        );
        object.create(guid_low, spell_id, HighGuid::DYNAMIC_OBJECT);
        object.set_object_scale(1.0);
        let mut world = WorldObjectBase::new(object);
        world.map_id = map_id;
        world.relocate(position);
        let mut dynobj = DynObject {
            world,
            caster,
            spell_id,
            effect_index,
            radius,
            alive_ms: duration_ms,
            positive,
        };
        let bytes: u32 = if spell_id == 1543 { 0x10 } else { 0x1 };
        let field_radius = if bytes == 0x10 { radius * 2.0 } else { radius };
        let v = dynobj.world.object.values_mut();
        v.set_guid(offsets::DYNAMICOBJECT_CASTER, caster);
        v.set_u32(offsets::DYNAMICOBJECT_BYTES, bytes);
        v.set_u32(offsets::DYNAMICOBJECT_SPELLID, spell_id);
        v.set_f32(offsets::DYNAMICOBJECT_RADIUS, field_radius);
        v.set_f32(offsets::DYNAMICOBJECT_POS_X, position.x);
        v.set_f32(offsets::DYNAMICOBJECT_POS_Y, position.y);
        v.set_f32(offsets::DYNAMICOBJECT_POS_Z, position.z);
        dynobj
    }

    #[must_use]
    pub fn guid(&self) -> ObjectGuid {
        self.world.object.object_guid()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn create_writes_fork_fields() {
        let caster = ObjectGuid::new(HighGuid::PLAYER, 7);
        let dynobj = DynObject::create(
            0x00F0_0001,
            0,
            caster,
            2120,
            1,
            Position::new(10.0, 20.0, 30.0, 0.0),
            8000,
            8.0,
            false,
        );
        let v = dynobj.world.object.values();
        assert_eq!(
            dynobj.guid(),
            ObjectGuid::with_entry(HighGuid::DYNAMIC_OBJECT, 2120, 0x00F0_0001)
        );
        assert_eq!(v.get_u32(offsets::DYNAMICOBJECT_BYTES), 1);
        assert_eq!(v.get_u32(offsets::DYNAMICOBJECT_SPELLID), 2120);
        assert!((v.get_f32(offsets::DYNAMICOBJECT_RADIUS) - 8.0).abs() < f32::EPSILON);
        assert!((v.get_f32(offsets::DYNAMICOBJECT_POS_Z) - 30.0).abs() < f32::EPSILON);
    }
}
