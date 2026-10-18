//! The `GameObject` aggregate and the Slice 3 spawn subset
//! of `GameObject::LoadFromDB` + `Create`
//! (`GameObject.cpp:173-261,918-986,1976-2005`).
//!
//! Deferred: use/loot/trap/door logic, transport motion
//! (transports create static here; their update is a map
//! system), despawn/respawn ticking (state recorded),
//! zone-script `OnObjectCreate` notification (Slice 6/7).

use mangos_guid::{HighGuid, TypeId};
use mangos_movement::movement_info::Position;
use mangos_protocol::update_data::ObjectUpdateFlags;
use mangos_protocol::update_field_table::{GAMEOBJECT_END, offsets};
use mangos_protocol::update_fields::TypeMask;
use mangos_world_data::gameobject_template::GameObjectInfo;
use mangos_world_data::spawns::{GameObjectSpawn, SPAWN_FLAG_ACTIVE, SPAWN_FLAG_DISABLED};

use crate::creature::SpawnRng;
use crate::object::ObjectBase;
use crate::unit::WorldObjectBase;

/// `MAX_GAMEOBJECT_TYPE` (`GameObjectDefines.h:71`).
const MAX_GAMEOBJECT_TYPE: u32 = 31;
/// `GAMEOBJECT_TYPE_TRANSPORT`.
const GO_TYPE_TRANSPORT: u32 = 11;
/// `GO_FLAG_NODESPAWN`.
const GO_FLAG_NODESPAWN: u32 = 0x0000_0002;
/// `GO_FLAG_TRANSPORT`.
const GO_FLAG_TRANSPORT: u32 = 0x0000_0100;
/// `VISIBILITY_DISTANCE_LARGE` (`ObjectDefines.h:37`).
pub const VISIBILITY_DISTANCE_LARGE: f32 = 200.0;
/// `MAX_VISIBILITY_DISTANCE` = `SIZE_OF_GRIDS`
/// (`ObjectDefines.h:29`).
pub const MAX_VISIBILITY_DISTANCE: f32 = 533.333_3;

/// `enum GOState` (`GameObject.h`): READY 0, ACTIVE 1,
/// `ACTIVE_ALTERNATIVE` 2 (values ride the spawn row).
pub const GO_STATE_READY: u32 = 0;
pub const GO_STATE_ACTIVE: u32 = 1;

/// Why a spawn was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoSpawnError {
    Disabled,
    UnknownEntry,
    InvalidType,
    InvalidPosition,
}

/// One gameobject.
#[derive(Clone, Debug)]
pub struct GameObject {
    pub world: WorldObjectBase,
    pub info: GameObjectInfo,
    pub data: Option<GameObjectSpawn>,
    /// `m_stationaryPosition`.
    pub stationary_position: Position,
    /// `m_rotation` packed orientation word
    /// (`UpdateRotationFields`).
    pub packed_rotation: i64,
    /// `m_spawnedByDefault`.
    pub spawned_by_default: bool,
    /// `m_respawnDelayTime` (signed in C++).
    pub respawn_delay_time: i32,
    /// `m_respawnTime`.
    pub respawn_time: u64,
    /// Expired persisted row consumed (persist 0).
    pub respawn_row_expired: bool,
    pub is_active_object: bool,
    pub visibility_modifier: f32,
}

impl GameObject {
    /// `LoadFromDB(guid, map, force)` subset.
    ///
    /// # Errors
    ///
    /// [`GoSpawnError`] for the C++ false returns.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_from_db(
        guid_low: u32,
        data: &GameObjectSpawn,
        templates: &mangos_world_data::gameobject_template::GameObjectTemplates,
        respawn_time: u64,
        now_unix: u64,
        force: bool,
        force_active_objects: bool,
        rng: &mut dyn SpawnRng,
    ) -> Result<GameObject, GoSpawnError> {
        if !force && data.spawn_flags & SPAWN_FLAG_DISABLED != 0 {
            return Err(GoSpawnError::Disabled);
        }
        let info = templates
            .get(data.id)
            .cloned()
            .ok_or(GoSpawnError::UnknownEntry)?;
        let mut go = GameObject::create(
            guid_low,
            info,
            Position::new(
                data.position.x,
                data.position.y,
                data.position.z,
                data.position.o,
            ),
            u32::from(data.position.map_id),
            [
                data.rotation0,
                data.rotation1,
                data.rotation2,
                data.rotation3,
            ],
            data.animprogress,
            data.go_state,
            force_active_objects,
        )?;
        go.data = Some(data.clone());
        // LoadFromDB respawn bookkeeping.
        if !despawn_possibility(&go.info)
            && !despawn_at_action(&go.info)
            && data.spawntimesecsmin >= 0
        {
            go.world
                .object
                .values_mut()
                .set_flag(offsets::GAMEOBJECT_FLAGS, GO_FLAG_NODESPAWN);
            go.spawned_by_default = true;
            go.respawn_delay_time = 0;
            go.respawn_time = 0;
        } else if data.spawntimesecsmin >= 0 {
            go.spawned_by_default = true;
            let delay = data.random_respawn_time(|min, max| rng.urand(min, max));
            go.respawn_delay_time = i32::from_ne_bytes(delay.to_ne_bytes());
            go.respawn_time = respawn_time;
            if respawn_time != 0 && respawn_time <= now_unix {
                go.respawn_time = 0;
                go.respawn_row_expired = true;
            }
        } else {
            go.spawned_by_default = false;
            go.respawn_delay_time = -data.spawntimesecsmin;
            go.respawn_time = 0;
        }
        if data.spawn_flags & SPAWN_FLAG_ACTIVE != 0 {
            go.is_active_object = true;
        }
        if data.visibility_mod != 0.0 {
            go.visibility_modifier = data.visibility_mod;
        }
        Ok(go)
    }

    /// `Create(...)` subset (zone-script notify deferred).
    #[allow(clippy::too_many_arguments)]
    fn create(
        guid_low: u32,
        info: GameObjectInfo,
        position: Position,
        map_id: u32,
        rotation: [f32; 4],
        anim_progress: u32,
        go_state: u32,
        force_active_objects: bool,
    ) -> Result<GameObject, GoSpawnError> {
        if !mangos_movement::movement_info::is_valid_map_coord(
            position.x, position.y, position.z, position.o,
        ) {
            return Err(GoSpawnError::InvalidPosition);
        }
        if info.go_type >= MAX_GAMEOBJECT_TYPE {
            return Err(GoSpawnError::InvalidType);
        }
        let is_transport = info.go_type == GO_TYPE_TRANSPORT;
        let mut flags = ObjectUpdateFlags::ALL | ObjectUpdateFlags::HAS_POSITION;
        if is_transport {
            flags |= ObjectUpdateFlags::TRANSPORT;
        }
        let mut object = ObjectBase::new(
            TypeId::GameObject,
            TypeMask::OBJECT.union(TypeMask::GAMEOBJECT),
            ObjectUpdateFlags(flags),
            GAMEOBJECT_END,
        );
        object.create(
            guid_low,
            info.id,
            if is_transport {
                HighGuid::TRANSPORT
            } else {
                HighGuid::GAMEOBJECT
            },
        );
        object.set_object_scale(info.size);
        let mut world = WorldObjectBase::new(object);
        world.map_id = map_id;
        world.name.clone_from(&info.name);
        world.relocate(position);
        let mut go = GameObject {
            world,
            info,
            data: None,
            stationary_position: position,
            packed_rotation: 0,
            spawned_by_default: true,
            respawn_delay_time: 0,
            respawn_time: 0,
            respawn_row_expired: false,
            is_active_object: false,
            visibility_modifier: 0.0,
        };
        {
            let v = go.world.object.values_mut();
            v.set_f32(offsets::GAMEOBJECT_POS_X, position.x);
            v.set_f32(offsets::GAMEOBJECT_POS_Y, position.y);
            v.set_f32(offsets::GAMEOBJECT_POS_Z, position.z);
            v.set_f32(offsets::GAMEOBJECT_FACING, position.o);
            v.set_f32(offsets::GAMEOBJECT_ROTATION, rotation[0]);
            v.set_f32(offsets::GAMEOBJECT_ROTATION + 1, rotation[1]);
        }
        go.packed_rotation = go.update_rotation_fields(rotation[2], rotation[3]);
        {
            let v = go.world.object.values_mut();
            v.set_u32(offsets::GAMEOBJECT_FACTION, go.info.faction);
            v.set_u32(offsets::GAMEOBJECT_FLAGS, go.info.flags);
            v.set_u32(offsets::GAMEOBJECT_DISPLAYID, go.info.display_id);
            // At 5875 these are whole words, not bytes:
            // SetGoState/SetGoType/SetGoAnimProgress.
            v.set_u32(offsets::GAMEOBJECT_STATE, go_state);
            v.set_u32(offsets::GAMEOBJECT_TYPE_ID, go.info.go_type);
            v.set_u32(offsets::GAMEOBJECT_ANIMPROGRESS, anim_progress);
            if is_transport {
                v.set_u32(offsets::GAMEOBJECT_LEVEL, go.info.data_u32(0));
                let state = if go.info.data_u32(1) != 0 {
                    GO_STATE_ACTIVE
                } else {
                    GO_STATE_READY
                };
                v.set_u32(offsets::GAMEOBJECT_STATE, state);
                v.set_flag(
                    offsets::GAMEOBJECT_FLAGS,
                    GO_FLAG_TRANSPORT | GO_FLAG_NODESPAWN,
                );
            }
        }
        go.world.object.set_entry(go.info.id);
        if is_large_gameobject(&go.info) {
            go.visibility_modifier = VISIBILITY_DISTANCE_LARGE;
            if force_active_objects {
                go.is_active_object = true;
            }
        } else if is_infinite_gameobject(go.info.go_type) {
            go.visibility_modifier = MAX_VISIBILITY_DISTANCE;
            if force_active_objects {
                go.is_active_object = true;
            }
        }
        Ok(go)
    }

    /// `UpdateRotationFields` (`GameObject.cpp:1976-2005`):
    /// packs the orientation half-angle and defaults
    /// rotation2/3 from it when both are zero. The math is
    /// done in `f64` from `f32` inputs exactly as written
    /// (C++ promotes `float` to `double` here).
    fn update_rotation_fields(&mut self, rotation2: f32, rotation3: f32) -> i64 {
        let (rotation, r2, r3) = pack_rotation(self.world.position.o, rotation2, rotation3);
        let v = self.world.object.values_mut();
        v.set_f32(offsets::GAMEOBJECT_ROTATION + 2, r2);
        v.set_f32(offsets::GAMEOBJECT_ROTATION + 3, r3);
        rotation
    }

    #[must_use]
    pub fn is_spawned(&self, now_unix: u64) -> bool {
        self.respawn_time <= now_unix
    }
}

/// `GameObjectInfo::IsLargeGameObject` (data indices from
/// `GameObjectDefines.h` unions).
#[must_use]
pub fn is_large_gameobject(info: &GameObjectInfo) -> bool {
    let large_at = match info.go_type {
        1 | 8 => 5, // button, spell focus
        2 => 9,     // questgiver
        5 => 3,     // generic
        6 => 10,    // trap
        10 => 13,   // goober
        22 => 4,    // spellcaster
        29 => 18,   // capture point (> 1.11.2)
        _ => return false,
    };
    info.data_u32(large_at) != 0
}

/// `GameObjectInfo::IsInfiniteGameObject` at 5875.
#[must_use]
pub fn is_infinite_gameobject(go_type: u32) -> bool {
    matches!(go_type, 0 | 24 | 26) // door, flagstand, flagdrop
}

/// `GameObjectInfo::GetDespawnPossibility` at 5875.
#[must_use]
pub fn despawn_possibility(info: &GameObjectInfo) -> bool {
    let no_damage_immune_at = match info.go_type {
        0 | 26 => 3, // door, flagdrop
        1 => 4,      // button
        2 | 24 => 5, // questgiver, flagstand
        10 => 11,    // goober
        _ => return true,
    };
    info.data_u32(no_damage_immune_at) != 0
}

/// `GameObjectInfo::IsServerOnly` at 5875.
#[must_use]
pub fn is_server_only(info: &GameObjectInfo) -> bool {
    let at = match info.go_type {
        5 => 2,  // generic
        6 => 8,  // trap
        8 => 3,  // spell focus
        30 => 6, // aura generator (> 1.11.2)
        _ => return false,
    };
    info.data_u32(at) != 0
}

/// `GameObjectInfo::IsDespawnAtAction`.
#[must_use]
pub fn despawn_at_action(info: &GameObjectInfo) -> bool {
    match info.go_type {
        3 => info.data_u32(3) != 0,  // chest.consumable
        10 => info.data_u32(5) != 0, // goober.consumable
        _ => false,
    }
}

/// `UpdateRotationFields` math, pure half: returns
/// `(packed, rotation2, rotation3)`.
fn pack_rotation(orientation: f32, mut rotation2: f32, mut rotation3: f32) -> (i64, f32, f32) {
    // `static double const atan_pow = atan(pow(2.0f, -20.0f))`:
    // pow result is float, the atan is double.
    let atan_pow = (2.0f32.powf(-20.0) as f64).atan();
    // C++ `sin(float)` resolves to the double overload.
    let f_rot1 = f64::from(orientation / 2.0f32).sin();
    let f_rot2 = f64::from(orientation / 2.0f32).cos();
    let sign = if f_rot2 >= 0.0 { 1.0f32 } else { -1.0f32 };
    let i_rot1 = (f_rot1 / atan_pow * f64::from(sign)) as i64;
    let rotation = i_rot1 & 0x0000_0000_001F_FFFF;
    if rotation2 == 0.0 && rotation3 == 0.0 {
        rotation2 = f_rot1 as f32;
        rotation3 = f_rot2 as f32;
    }
    (rotation, rotation2, rotation3)
}

#[cfg(test)]
mod tests {
    use super::pack_rotation;

    /// Goldens from an independent f64 computation
    /// (Python `math` over f32-rounded inputs).
    #[test]
    #[allow(clippy::approx_constant)] // 3.14159 is the oracle input, not PI
    fn packed_rotation_matches_oracle() {
        for (o, want) in [
            (0.0f32, 0i64),
            (0.139_626, 73_144),
            (1.5, 714_750),
            (3.141_59, 1_048_575),
            (6.0, 1_949_177),
            (-2.0, 1_214_806),
        ] {
            assert_eq!(pack_rotation(o, 0.0, 0.0).0, want, "o={o}");
        }
        // Zero pair defaults to sin/cos half-angle as f32.
        let (_, r2, r3) = pack_rotation(1.5, 0.0, 0.0);
        assert_eq!(r2.to_bits(), 0.681_638_8_f32.to_bits());
        assert_eq!(r3.to_bits(), 0.731_688_86f32.to_bits());
        // A given pair passes through untouched.
        let (_, r2, r3) = pack_rotation(1.5, 0.25, 0.75);
        assert_eq!((r2, r3), (0.25, 0.75));
    }
}
