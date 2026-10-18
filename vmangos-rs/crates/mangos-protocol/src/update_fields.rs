//! Update-field metadata for build 5875 (1.12.1): the
//! `UpdateFieldData` shape, value types, visibility flags, per-type
//! flags arrays, and name/offset lookups, ported from
//! `vmangos/src/game/Objects/UpdateFields.h` (`:111-153`) and
//! `UpdateFields.cpp` (`:46-121`). The row data itself lives in the
//! generated [`crate::update_field_table`] (324 rows from
//! `UpdateFields_1_12_1.cpp`; the sibling header's offset comments
//! are stale for CONTAINER/PLAYER and are never used as a source).
//!
//! Deviations forced by layering, same rule as `mangos-guid`:
//! `GetUpdateFieldFlagsArray` logs an error and returns null for
//! `TYPEID_OBJECT` in C++; [`update_field_flags`] returns `None`
//! and the caller decides the logging policy. The C++ flags arrays
//! are built into default-initialised `std::array`s (indeterminate
//! where uncovered); the 5875 rows tile every block completely
//! (generator-asserted), and the Rust arrays zero-initialise, which
//! agrees on every covered offset.

use mangos_guid::TypeId;

use crate::update_field_table::{
    CONTAINER_END, CORPSE_END, DYNAMICOBJECT_END, GAMEOBJECT_END, PLAYER_END, UPDATE_FIELDS_5875,
    offsets,
};

/// `enum TypeMask` (`ObjectGuid.h:47-57`). Distinct from
/// [`TypeId`]: rows are keyed by mask, objects carry a type id.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct TypeMask(pub u8);

impl TypeMask {
    pub const OBJECT: Self = Self(0x01);
    pub const ITEM: Self = Self(0x02);
    pub const CONTAINER: Self = Self(0x04);
    /// Players also have it (`ObjectGuid.h:52`).
    pub const UNIT: Self = Self(0x08);
    pub const PLAYER: Self = Self(0x10);
    pub const GAMEOBJECT: Self = Self(0x20);
    pub const DYNAMICOBJECT: Self = Self(0x40);
    pub const CORPSE: Self = Self(0x80);

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Any-bit test, matching the C++ `mask & other` checks.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Bitwise union (`BitOr` cannot be `const` on stable, and the
    /// flags-array statics below are built in const context).
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for TypeMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// `enum UpdateFieldValueTypes` (`UpdateFields.h:111-120`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum UpdateFieldType {
    #[default]
    None = 0,
    Int = 1,
    TwoShort = 2,
    Float = 3,
    Guid = 4,
    Bytes = 5,
    Bytes2 = 6,
}

/// `enum UpdateFieldFlags` (`UpdateFields.h:122-134`), a `u16` bitset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct UpdateFieldFlags(pub u16);

impl UpdateFieldFlags {
    pub const NONE: u16 = 0x000;
    pub const PUBLIC: u16 = 0x001;
    pub const PRIVATE: u16 = 0x002;
    pub const OWNER_ONLY: u16 = 0x004;
    pub const UNK1: u16 = 0x008;
    pub const UNK2: u16 = 0x010;
    pub const SPECIAL_INFO: u16 = 0x020;
    pub const GROUP_ONLY: u16 = 0x040;
    pub const UNK5: u16 = 0x080;
    pub const DYNAMIC: u16 = 0x100;

    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Any-bit test.
    #[must_use]
    pub const fn has(self, flag: u16) -> bool {
        self.0 & flag != 0
    }
}

/// `struct UpdateFieldData` (`UpdateFields.h:136-146`): one row of
/// the generated table. `offset`/`size` are in 32-bit words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateFieldData {
    pub object_type_mask: TypeMask,
    pub name: &'static str,
    pub offset: u16,
    pub size: u16,
    pub value_type: UpdateFieldType,
    pub flags: UpdateFieldFlags,
}

/// `SetupUpdateFieldFlagsArray` (`UpdateFields.cpp:46-61`): fill a
/// per-word flags array from every row whose mask intersects.
const fn setup_flags<const SIZE: usize>(mask: TypeMask) -> [u16; SIZE] {
    let mut flags = [0u16; SIZE];
    let mut row = 0;
    while row < UPDATE_FIELDS_5875.len() {
        let field = UPDATE_FIELDS_5875[row];
        row += 1;
        if !field.object_type_mask.intersects(mask) {
            continue;
        }
        let mut i = field.offset;
        while i < field.offset + field.size {
            flags[i as usize] = field.flags.raw();
            i += 1;
        }
    }
    flags
}

/// `g_containerUpdateFieldFlags` (`UpdateFields.cpp:63`).
pub static CONTAINER_FIELD_FLAGS: [u16; CONTAINER_END as usize] = setup_flags(
    TypeMask::OBJECT
        .union(TypeMask::ITEM)
        .union(TypeMask::CONTAINER),
);
/// `g_playerUpdateFieldFlags` (`UpdateFields.cpp:64`); also used for
/// plain units, as in C++.
pub static PLAYER_FIELD_FLAGS: [u16; PLAYER_END as usize] = setup_flags(
    TypeMask::OBJECT
        .union(TypeMask::UNIT)
        .union(TypeMask::PLAYER),
);
/// `g_gameObjectUpdateFieldFlags` (`UpdateFields.cpp:65`).
pub static GAMEOBJECT_FIELD_FLAGS: [u16; GAMEOBJECT_END as usize] =
    setup_flags(TypeMask::OBJECT.union(TypeMask::GAMEOBJECT));
/// `g_dynamicObjectUpdateFieldFlags` (`UpdateFields.cpp:66`).
pub static DYNAMICOBJECT_FIELD_FLAGS: [u16; DYNAMICOBJECT_END as usize] =
    setup_flags(TypeMask::OBJECT.union(TypeMask::DYNAMICOBJECT));
/// `g_corpseUpdateFieldFlags` (`UpdateFields.cpp:67`).
pub static CORPSE_FIELD_FLAGS: [u16; CORPSE_END as usize] =
    setup_flags(TypeMask::OBJECT.union(TypeMask::CORPSE));

/// `UpdateFields::GetUpdateFieldFlagsArray` (`UpdateFields.cpp:69-98`).
/// Returns `None` for [`TypeId::Object`], where C++ logs an error
/// and returns null.
#[must_use]
pub fn update_field_flags(type_id: TypeId) -> Option<&'static [u16]> {
    match type_id {
        TypeId::Item | TypeId::Container => Some(&CONTAINER_FIELD_FLAGS),
        TypeId::Unit | TypeId::Player => Some(&PLAYER_FIELD_FLAGS),
        TypeId::GameObject => Some(&GAMEOBJECT_FIELD_FLAGS),
        TypeId::DynamicObject => Some(&DYNAMICOBJECT_FIELD_FLAGS),
        TypeId::Corpse => Some(&CORPSE_FIELD_FLAGS),
        TypeId::Object => None,
    }
}

/// `UpdateFields::GetUpdateFieldDataByName` (`UpdateFields.cpp:100-108`):
/// first exact-name match in table order.
#[must_use]
pub fn field_by_name(name: &str) -> Option<&'static UpdateFieldData> {
    UPDATE_FIELDS_5875.iter().find(|f| f.name == name)
}

/// `UpdateFields::GetUpdateFieldDataByTypeMaskAndOffset`
/// (`UpdateFields.cpp:110-121`): first row whose mask intersects and
/// whose `[offset, offset + size)` covers `offset`.
#[must_use]
pub fn field_by_mask_and_offset(mask: TypeMask, offset: u16) -> Option<&'static UpdateFieldData> {
    UPDATE_FIELDS_5875.iter().find(|f| {
        f.object_type_mask.intersects(mask)
            && (offset == f.offset || (offset > f.offset && offset < f.offset + f.size))
    })
}

/// `GetIndexOfUpdateFieldForCurrentBuild` (`UpdateFields.h:74-109`):
/// DB script commands store field indexes in the fixed 5875
/// numbering (`enum UpdateFields5875`, `:51-70`); on any other build
/// they remap to that build's offsets. This port is 5875-only, so
/// the mapped values equal the inputs — the table-driven form is
/// kept so a future progression build changes one function.
#[must_use]
pub const fn index_of_update_field_for_current_build(db_index: u32) -> u32 {
    let mapped = match db_index {
        9 => offsets::GAMEOBJECT_FLAGS,
        19 => offsets::GAMEOBJECT_DYN_FLAGS,
        46 => offsets::UNIT_FIELD_FLAGS,
        143 => offsets::UNIT_DYNAMIC_FLAGS,
        147 => offsets::UNIT_NPC_FLAGS,
        155 => offsets::UNIT_FIELD_RESISTANCES,
        156 => offsets::UNIT_FIELD_RESISTANCES + 1,
        157 => offsets::UNIT_FIELD_RESISTANCES + 2,
        158 => offsets::UNIT_FIELD_RESISTANCES + 3,
        159 => offsets::UNIT_FIELD_RESISTANCES + 4,
        160 => offsets::UNIT_FIELD_RESISTANCES + 5,
        161 => offsets::UNIT_FIELD_RESISTANCES + 6,
        165 => offsets::UNIT_FIELD_ATTACK_POWER,
        190 => offsets::PLAYER_FLAGS,
        _ => return db_index,
    };
    mapped as u32
}
