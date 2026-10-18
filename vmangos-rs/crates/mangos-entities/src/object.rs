//! `Object` core state and client-update marking, ported from
//! `vmangos/src/game/Objects/Object.h:102-175,420-448` and
//! `Object.cpp:175-232,942-954,1355-1391,2806-2810`.
//!
//! Layering: the C++ virtuals `AddToClientUpdateList` /
//! `RemoveFromClientUpdateList` / `AddObjectToRemoveList` touch
//! the map (or the owning player, for items), which does not
//! exist in this slice. The marking methods here therefore
//! return the **decision** the C++ would act on
//! ([`MarkEffect`] / [`DelayedEffects`]); the map layer performs
//! the list operation. All flag/timing semantics (items mark
//! immediately, everything else goes through the delayed bit,
//! `m_objectUpdated` set exactly where C++ sets it) are kept.

use mangos_guid::{HighGuid, ObjectGuid, TypeId};
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::update_fields::TypeMask;

use crate::values::Values;

/// `DEFAULT_OBJECT_SCALE` fallback in `GetObjectScale`
/// (`Object.h:144`; the constant itself is 1.0f).
pub const DEFAULT_OBJECT_SCALE: f32 = 1.0;

/// `enum ObjectDelayedAction` (`Object.h:102-107`).
pub const DELAYED_MARK_CLIENT_UPDATE: u32 = 0x1;
/// `OBJECT_DELAYED_ADD_TO_RELOCATED_LIST` (map-side, Slice 3).
pub const DELAYED_ADD_TO_RELOCATED_LIST: u32 = 0x2;
/// `OBJECT_DELAYED_ADD_TO_REMOVE_LIST`.
pub const DELAYED_ADD_TO_REMOVE_LIST: u32 = 0x4;

/// What a marking call decided; the map/player layer performs the
/// named list operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkEffect {
    /// Value did not change.
    Unchanged,
    /// Changed, but the object is not in the world: C++ drops the
    /// mark (`MarkForClientUpdate` early return).
    NotInWorld,
    /// Changed and queued via the delayed-action bit; the list
    /// add happens in `execute_delayed_actions`.
    Delayed,
    /// Item/container fast path: add to the client update list
    /// now (only returned when this call actually queues it —
    /// repeat marks while already queued return [`Self::Queued`]).
    EnqueueNow,
    /// Changed, but already queued for update.
    Queued,
}

/// List operations owed after [`ObjectBase::execute_delayed_actions`]
/// (`Object.cpp:1374-1391`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DelayedEffects {
    pub add_to_update_list: bool,
    pub add_to_remove_list: bool,
}

/// The `Object` base every entity carries: identity, update flag,
/// values store, and update-list lifecycle flags. Leaf types
/// construct it with their type id/mask/update-flag/word count
/// exactly as the C++ leaf constructors stamp them
/// (`Object.cpp:175-189`, `Unit.cpp:80-83`, …).
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)] // mirrors Object's independent m_* state flags 1:1
pub struct ObjectBase {
    type_id: TypeId,
    type_mask: TypeMask,
    update_flag: ObjectUpdateFlags,
    values: Values,
    pack_guid: Vec<u8>,
    in_world: bool,
    object_updated: bool,
    is_new_object: bool,
    deleted: bool,
    delayed_actions: u32,
}

impl ObjectBase {
    #[must_use]
    pub fn new(
        type_id: TypeId,
        type_mask: TypeMask,
        update_flag: ObjectUpdateFlags,
        values_count: u16,
    ) -> Self {
        Self {
            type_id,
            type_mask,
            update_flag,
            values: Values::new(values_count),
            pack_guid: Vec::new(),
            in_world: false,
            object_updated: false,
            is_new_object: false,
            deleted: false,
            delayed_actions: 0,
        }
    }

    /// `_Create` (`Object.cpp:223-232`): write GUID + TYPE words
    /// and cache the packed GUID. `OBJECT_FIELD_ENTRY` is **not**
    /// written here (each `Create` does it via `SetEntry`).
    /// The returned mark effect covers both setter marks merged:
    /// pre-world objects report [`MarkEffect::NotInWorld`].
    pub fn create(&mut self, guid_low: u32, entry: u32, high: HighGuid) -> MarkEffect {
        let guid = ObjectGuid::with_entry(high, entry, guid_low);
        let mut effect = self.set_guid(offsets::OBJECT_FIELD_GUID, guid);
        effect = merge_effect(
            effect,
            self.set_u32(offsets::OBJECT_FIELD_TYPE, u32::from(self.type_mask.raw())),
        );
        self.pack_guid.clear();
        guid.write_packed(&mut self.pack_guid);
        effect
    }

    #[must_use]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    #[must_use]
    pub fn type_mask(&self) -> TypeMask {
        self.type_mask
    }

    /// `IsType` (`Object.h:149`).
    #[must_use]
    pub fn is_type(&self, mask: TypeMask) -> bool {
        self.type_mask.intersects(mask)
    }

    #[must_use]
    pub fn update_flag(&self) -> ObjectUpdateFlags {
        self.update_flag
    }

    pub fn set_update_flag(&mut self, flags: ObjectUpdateFlags) {
        self.update_flag = flags;
    }

    #[must_use]
    pub fn values(&self) -> &Values {
        &self.values
    }

    pub fn values_mut(&mut self) -> &mut Values {
        &mut self.values
    }

    /// `GetObjectGuid` (`Object.h:135`): read from the values words.
    ///
    /// # Panics
    ///
    /// Panics if the values block is smaller than 3 words (calling
    /// before `create` on a 0-word base, as in C++).
    #[must_use]
    pub fn object_guid(&self) -> ObjectGuid {
        self.values.get_guid(offsets::OBJECT_FIELD_GUID)
    }

    /// `GetGUIDLow` (`Object.h:137`).
    #[must_use]
    pub fn guid_low(&self) -> u32 {
        self.object_guid().counter()
    }

    /// `GetPackGUID` (`Object.h:138`): cached packed bytes, set by
    /// [`Self::create`]. Empty until then.
    #[must_use]
    pub fn pack_guid(&self) -> &[u8] {
        &self.pack_guid
    }

    /// `GetEntry` (`Object.h:141`).
    #[must_use]
    pub fn entry(&self) -> u32 {
        self.values.get_u32(offsets::OBJECT_FIELD_ENTRY)
    }

    /// `SetEntry` (`Object.h:142`).
    pub fn set_entry(&mut self, entry: u32) -> MarkEffect {
        self.set_u32(offsets::OBJECT_FIELD_ENTRY, entry)
    }

    /// `GetObjectScale` (`Object.h:144`): a zero word means 1.0
    /// (and is omitted from CREATE blocks, so the client agrees).
    #[allow(clippy::float_cmp)] // C++ tests the float word for zero exactly
    #[must_use]
    pub fn object_scale(&self) -> f32 {
        let scale = self.values.get_f32(offsets::OBJECT_FIELD_SCALE_X);
        if scale == 0.0 {
            DEFAULT_OBJECT_SCALE
        } else {
            scale
        }
    }

    /// `SetObjectScale` (`Object.cpp:234`).
    pub fn set_object_scale(&mut self, scale: f32) -> MarkEffect {
        self.set_f32(offsets::OBJECT_FIELD_SCALE_X, scale)
    }

    #[must_use]
    pub fn is_in_world(&self) -> bool {
        self.in_world
    }

    #[must_use]
    pub fn is_new_object(&self) -> bool {
        self.is_new_object
    }

    /// `SetIsNewObject` (`Object.h:115`).
    pub fn set_is_new_object(&mut self, state: bool) {
        self.is_new_object = state;
    }

    #[must_use]
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub fn set_deleted(&mut self) {
        self.deleted = true;
    }

    #[must_use]
    pub fn is_object_updated(&self) -> bool {
        self.object_updated
    }

    /// `AddToWorld` (`Object.h:117-127`): idempotent; syncs the
    /// mirror (changes ride the CREATE block anyway).
    pub fn add_to_world(&mut self) {
        if self.in_world {
            return;
        }
        self.in_world = true;
        self.clear_update_mask(false);
    }

    /// `RemoveFromWorld` (`Object.h:128-133`). Returns `true` when
    /// the caller must remove the object from its client update
    /// list (the C++ `ClearUpdateMask(true)` side effect).
    pub fn remove_from_world(&mut self) -> bool {
        let remove = self.clear_update_mask(true);
        self.in_world = false;
        remove
    }

    /// `ClearUpdateMask` (`Object.cpp:942-954`): mirror sync plus
    /// update-list bookkeeping. Returns `true` when `remove` was
    /// requested for a queued object (caller performs the removal).
    pub fn clear_update_mask(&mut self, remove: bool) -> bool {
        self.values.clear_update_mask();
        let mut must_remove = false;
        if self.object_updated {
            must_remove = remove;
            self.object_updated = false;
        }
        self.delayed_actions &= !DELAYED_MARK_CLIENT_UPDATE;
        must_remove
    }

    /// `AddDelayedAction` (`Object.h:161`).
    pub fn add_delayed_action(&mut self, action: u32) {
        self.delayed_actions |= action;
    }

    /// `MarkForClientUpdate` (`Object.cpp:1355-1372`) as a
    /// decision: assumes a value just changed.
    fn mark_for_client_update(&mut self) -> MarkEffect {
        if !self.in_world {
            return MarkEffect::NotInWorld;
        }
        if matches!(self.type_id, TypeId::Item | TypeId::Container) {
            if self.object_updated {
                MarkEffect::Queued
            } else {
                self.object_updated = true;
                MarkEffect::EnqueueNow
            }
        } else {
            self.add_delayed_action(DELAYED_MARK_CLIENT_UPDATE);
            MarkEffect::Delayed
        }
    }

    /// `ExecuteDelayedActions` (`Object.cpp:1374-1391`) minus the
    /// list operations themselves. The relocated-list bit is
    /// map-internal (Slice 3) and is left untouched here.
    pub fn execute_delayed_actions(&mut self) -> DelayedEffects {
        let mut effects = DelayedEffects::default();
        if self.delayed_actions & DELAYED_MARK_CLIENT_UPDATE != 0 {
            if self.in_world && !self.deleted && !self.object_updated {
                self.object_updated = true;
                effects.add_to_update_list = true;
            }
            self.delayed_actions &= !DELAYED_MARK_CLIENT_UPDATE;
        }
        if self.delayed_actions & DELAYED_ADD_TO_REMOVE_LIST != 0 {
            if !self.deleted && self.in_world {
                effects.add_to_remove_list = true;
            }
            self.delayed_actions &= !DELAYED_ADD_TO_REMOVE_LIST;
        }
        effects
    }

    /// `ForceValuesUpdateAtIndex` (`Object.cpp:2806-2810`): poison
    /// + delayed mark.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn force_values_update_at(&mut self, index: u16) {
        self.values.force_update_at(index);
        self.add_delayed_action(DELAYED_MARK_CLIENT_UPDATE);
    }
}

macro_rules! marked_setter {
    ($(#[$doc:meta])* $name:ident, $inner:ident ( $($a:ident: $t:ty),* $(,)? )) => {
        $(#[$doc])*
        pub fn $name(&mut self, index: u16 $(, $a: $t)*) -> MarkEffect {
            if self.values.$inner(index $(, $a)*) {
                self.mark_for_client_update()
            } else {
                MarkEffect::Unchanged
            }
        }
    };
}

// Object-level setters: each forwards to the Values store and runs
// MarkForClientUpdate on change, exactly as the C++ setters do.
// Bounds/offset panics are those of the underlying Values methods.
impl ObjectBase {
    marked_setter!(
        /// `SetUInt32Value` (`Object.cpp:1101`).
        set_u32, set_u32(value: u32)
    );
    marked_setter!(
        /// `SetInt32Value` (`Object.cpp:1090`).
        set_i32, set_i32(value: i32)
    );
    marked_setter!(
        /// `SetUInt64Value` (`Object.cpp:1112`).
        set_u64, set_u64(value: u64)
    );
    marked_setter!(
        /// `SetGuidValue` (`Object.h:232`).
        set_guid, set_guid(value: ObjectGuid)
    );
    marked_setter!(
        /// `SetFloatValue` (`Object.cpp:1138`).
        set_f32, set_f32(value: f32)
    );
    marked_setter!(
        /// `SetByteValue` (`Object.cpp:1149`).
        set_byte, set_byte(offset: u8, value: u8)
    );
    marked_setter!(
        /// `SetUInt16Value` (`Object.cpp:1167`).
        set_u16, set_u16(offset: u8, value: u16)
    );
    marked_setter!(
        /// `SetInt16Value` (`Object.h:231`).
        set_i16, set_i16(offset: u8, value: i16)
    );
    marked_setter!(
        /// `SetStatFloatValue` (`Object.cpp:1185`).
        set_stat_f32, set_stat_f32(value: f32)
    );
    marked_setter!(
        /// `SetStatInt32Value` (`Object.cpp:1193`).
        set_stat_i32, set_stat_i32(value: i32)
    );
    marked_setter!(
        /// `ApplyModUInt32Value` (`Object.cpp:1201`).
        apply_mod_u32, apply_mod_u32(val: i32, apply: bool)
    );
    marked_setter!(
        /// `ApplyModInt32Value` (`Object.cpp:1210`).
        apply_mod_i32, apply_mod_i32(val: i32, apply: bool)
    );
    marked_setter!(
        /// `ApplyModSignedFloatValue` (`Object.cpp:1217`).
        apply_mod_signed_f32, apply_mod_signed_f32(val: f32, apply: bool)
    );
    marked_setter!(
        /// `ApplyModPositiveFloatValue` (`Object.cpp:1224`).
        apply_mod_positive_f32, apply_mod_positive_f32(val: f32, apply: bool)
    );
    marked_setter!(
        /// `ApplyPercentModFloatValue` (`Object.h:243`).
        apply_percent_mod_f32, apply_percent_mod_f32(val: f32, apply: bool)
    );
    marked_setter!(
        /// `SetFlag` (`Object.cpp:1233`).
        set_flag, set_flag(flag: u32)
    );
    marked_setter!(
        /// `RemoveFlag` (`Object.cpp:1246`).
        remove_flag, remove_flag(flag: u32)
    );
    marked_setter!(
        /// `ToggleFlag` (`Object.h:252`).
        toggle_flag, toggle_flag(flag: u32)
    );
    marked_setter!(
        /// `ApplyModFlag` (`Object.h:265`).
        apply_mod_flag, apply_mod_flag(flag: u32, apply: bool)
    );
    marked_setter!(
        /// `SetByteFlag` (`Object.cpp:1259`).
        set_byte_flag, set_byte_flag(offset: u8, flag: u8)
    );
    marked_setter!(
        /// `RemoveByteFlag` (`Object.cpp:1276`).
        remove_byte_flag, remove_byte_flag(offset: u8, flag: u8)
    );
    marked_setter!(
        /// `ToggleByteFlag` (`Object.h:276`).
        toggle_byte_flag, toggle_byte_flag(offset: u8, flag: u8)
    );
    marked_setter!(
        /// `ApplyModByteFlag` (`Object.h:290`).
        apply_mod_byte_flag, apply_mod_byte_flag(offset: u8, flag: u32, apply: bool)
    );
    marked_setter!(
        /// `SetShortFlag` (`Object.cpp:1293`).
        set_short_flag, set_short_flag(high_part: bool, flag: u16)
    );
    marked_setter!(
        /// `RemoveShortFlag` (`Object.cpp:1304`).
        remove_short_flag, remove_short_flag(high_part: bool, flag: u16)
    );
    marked_setter!(
        /// `ToggleShortFlag` (`Object.h:299`).
        toggle_short_flag, toggle_short_flag(high_part: bool, flag: u8)
    );
    marked_setter!(
        /// `ApplyModShortFlag` (`Object.h:312`).
        apply_mod_short_flag, apply_mod_short_flag(high_part: bool, flag: u32, apply: bool)
    );
    marked_setter!(
        /// `SetFlag64` (`Object.h:320`).
        set_flag64, set_flag64(flag: u64)
    );
    marked_setter!(
        /// `RemoveFlag64` (`Object.h:327`).
        remove_flag64, remove_flag64(flag: u64)
    );
    marked_setter!(
        /// `ToggleFlag64` (`Object.h:341`).
        toggle_flag64, toggle_flag64(flag: u64)
    );
    marked_setter!(
        /// `ApplyModFlag64` (`Object.h:349`).
        apply_mod_flag64, apply_mod_flag64(flag: u64, apply: bool)
    );
}

fn merge_effect(a: MarkEffect, b: MarkEffect) -> MarkEffect {
    use MarkEffect::{Delayed, EnqueueNow, NotInWorld, Queued, Unchanged};
    match (a, b) {
        (EnqueueNow, _) | (_, EnqueueNow) => EnqueueNow,
        (Delayed, _) | (_, Delayed) => Delayed,
        (Queued, _) | (_, Queued) => Queued,
        (NotInWorld, _) | (_, NotInWorld) => NotInWorld,
        (Unchanged, Unchanged) => Unchanged,
    }
}
