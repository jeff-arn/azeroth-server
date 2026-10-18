//! Movement splines, the `SMSG_MONSTER_MOVE` packet builder, and
//! the `MovementInfo` wire codec, ported from
//! `vmangos/src/game/Movement/spline/` (`spline.{h,cpp}` +
//! `spline.impl.h`, `MoveSpline.{h,cpp}`, `MoveSplineFlag.h`,
//! `MoveSplineInitArgs.h`, `util.cpp`, `packet_builder.cpp`) and
//! `vmangos/src/game/Objects/MovementInfo.h` + `Object.cpp:61-172`.
//!
//! All float arithmetic mirrors the C++ operation order (G3D
//! `Vector3`/`Vector4`/`Matrix4` semantics, f64 promotions in the
//! fall formulas), and the packet builder is byte-exact against
//! golden tests hand-derived from the C++ algorithm.
//!
//! Unit/world integration (`MoveSplineInit`, `MotionMaster`,
//! waypoint generators, transport handling) belongs to the Phase 4
//! host and is intentionally absent.

pub mod move_spline;
pub mod movement_info;
pub mod packet_builder;
pub mod spline;

pub use movement_info::{JumpInfo, MovementFlags, MovementInfo, Position};

pub use move_spline::{
    FacingInfo, Location, MoveSpline, MoveSplineFlag, MoveSplineInitArgs, UpdateResult,
    compute_fall_elevation, compute_fall_elevation_with_velocity, compute_fall_time,
};
pub use packet_builder::{
    DEFAULT_MAX_POINTS_PER_PACKET, write_common_monster_move_part, write_create, write_monster_move,
};
pub use spline::{EvaluationMode, Spline, Vec3};
