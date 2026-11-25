//! `MovementInfo` wire codec + sanitisation, ported from
//! `vmangos/src/game/Objects/MovementInfo.h` (flags: `:27-70`, class:
//! `:157-247`) and `Read`/`CorrectData`/`Write` in
//! `vmangos/src/game/Objects/Object.cpp:61-172`, plus the packet-level
//! bounds check `WorldSession::VerifyMovementInfo`
//! (`vmangos/src/game/Handlers/MovementHandler.cpp:1078-1095`, with
//! `MaNGOS::IsValidMapCoord` from
//! `vmangos/src/game/Maps/GridDefines.h:176-183`).
//!
//! Wire facts a port must not lose:
//!
//! - [`MovementInfo::read`] consumes a **client** time (`ctime`) where
//!   [`MovementInfo::write`] emits the **server** time (`stime`) —
//!   the same 4-byte slot, different field (`Object.cpp:65` vs `:141`).
//! - Conditional blocks in wire order: transport (raw `u64` GUID + 4
//!   floats), swimming pitch, then unconditional `fallTime`, then jump
//!   (`zspeed`, **cos before sin**, `xyspeed`), then spline elevation.
//!   Base size 28 bytes, all-conditionals size 76 bytes.
//! - `read` mutates a caller-seeded value: the C++ handler
//!   copy-initialises it from the mover's current info
//!   (`MovementHandler.cpp:311`), which is what makes the jump latch
//!   (`jump.startClientTime`) stateful across packets.
//! - [`MovementInfo::correct_data`] is mandatory sanitisation, not
//!   decoration: contradictory flag pairs freeze receiving clients.
//!
//! Layering deviation (same rule as `mangos-guid`): the C++ `Read`
//! stamps `stime` from the global `WorldTimer` and
//! `SetAsServerSide` reads it too; here both take the current server
//! time as a parameter so the crate stays clock-free. The HOVER aura
//! check stays disabled exactly as in C++ (`Object.cpp:119-121` is
//! commented out) pending the recorded owner parity decision.

use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::{PacketReader, PacketWriter, ReadError};

/// `enum MovementFlags` (`MovementInfo.h:27-70`) — bitfield wrapper
/// around the raw `u32`, following the `MoveSplineFlag` shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MovementFlags(pub u32);

impl MovementFlags {
    pub const NONE: u32 = 0x0000_0000;
    pub const FORWARD: u32 = 0x0000_0001;
    pub const BACKWARD: u32 = 0x0000_0002;
    pub const STRAFE_LEFT: u32 = 0x0000_0004;
    pub const STRAFE_RIGHT: u32 = 0x0000_0008;
    pub const TURN_LEFT: u32 = 0x0000_0010;
    pub const TURN_RIGHT: u32 = 0x0000_0020;
    pub const PITCH_UP: u32 = 0x0000_0040;
    pub const PITCH_DOWN: u32 = 0x0000_0080;
    pub const WALK_MODE: u32 = 0x0000_0100;
    pub const UNUSED10: u32 = 0x0000_0200;
    pub const LEVITATING: u32 = 0x0000_0400;
    pub const FIXED_Z: u32 = 0x0000_0800;
    pub const ROOT: u32 = 0x0000_1000;
    pub const JUMPING: u32 = 0x0000_2000;
    pub const FALLING_FAR: u32 = 0x0000_4000;
    pub const PENDING_STOP: u32 = 0x0000_8000;
    pub const PENDING_UNSTRAFE: u32 = 0x0001_0000;
    pub const PENDING_FORWARD: u32 = 0x0002_0000;
    pub const PENDING_BACKWARD: u32 = 0x0004_0000;
    pub const PENDING_STR_LEFT: u32 = 0x0008_0000;
    pub const PENDING_STR_RGHT: u32 = 0x0010_0000;
    pub const SWIMMING: u32 = 0x0020_0000;
    pub const SPLINE_ENABLED: u32 = 0x0040_0000;
    pub const MOVED: u32 = 0x0080_0000;
    pub const FLYING: u32 = 0x0100_0000;
    pub const ON_TRANSPORT: u32 = 0x0200_0000;
    pub const SPLINE_ELEVATION: u32 = 0x0400_0000;
    pub const UNUSED28: u32 = 0x0800_0000;
    pub const WATER_WALKING: u32 = 0x1000_0000;
    pub const SAFE_FALL: u32 = 0x2000_0000;
    pub const HOVER: u32 = 0x4000_0000;
    pub const UNUSED32: u32 = 0x8000_0000;

    /// `MovementInfo.h:64`: flags that must not coexist with `ROOT`.
    pub const MASK_MOVING: u32 = Self::FORWARD
        | Self::BACKWARD
        | Self::STRAFE_LEFT
        | Self::STRAFE_RIGHT
        | Self::PITCH_UP
        | Self::PITCH_DOWN
        | Self::JUMPING
        | Self::FALLING_FAR
        | Self::SPLINE_ELEVATION;
    /// `MovementInfo.h:65`.
    pub const MASK_MOVING_OR_TURN: u32 = Self::MASK_MOVING | Self::TURN_LEFT | Self::TURN_RIGHT;
    /// `MovementInfo.h:69`: x/z translations only (melee leeway).
    pub const MASK_XZ: u32 =
        Self::FORWARD | Self::BACKWARD | Self::STRAFE_LEFT | Self::STRAFE_RIGHT;

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// `HasMovementFlag` (`MovementInfo.h:170`): any-bit test.
    #[must_use]
    pub const fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    /// `AddMovementFlag` (`MovementInfo.h:168`).
    pub fn add(&mut self, flag: u32) {
        self.0 |= flag;
    }

    /// `RemoveMovementFlag` (`MovementInfo.h:169`).
    pub fn remove(&mut self, flag: u32) {
        self.0 &= !flag;
    }
}

/// `struct Position` (`SharedDefines.h:1816-1826`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub o: f32,
}

impl Position {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32, o: f32) -> Self {
        Self { x, y, z, o }
    }

    /// `Position::IsEmpty` (`SharedDefines.h:1825`).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.z == 0.0 && self.o == 0.0
    }
}

/// `MovementInfo::JumpInfo` (`MovementInfo.h:219-225`). The wire
/// order is `zspeed, cosAngle, sinAngle, xyspeed` (`Object.cpp:86-89`)
/// — cos before sin, unlike the member declaration order here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JumpInfo {
    pub z_speed: f32,
    pub sin_angle: f32,
    pub cos_angle: f32,
    pub xy_speed: f32,
    /// Non-wire: latched start position of the current jump.
    pub start: Position,
    /// Non-wire: client time the current jump started; 0 = not jumping.
    pub start_client_time: u32,
}

/// `class MovementInfo` (`MovementInfo.h:157-247`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementInfo {
    /// `moveFlags` (`MovementInfo.h:230`).
    pub flags: MovementFlags,
    /// Server time (`stime`, `:231`): written to the wire.
    pub server_time: u32,
    /// Client time (`ctime`, `:232`): read from the wire, never written.
    pub client_time: u32,
    /// `pos` (`:233`).
    pub pos: Position,
    /// `t_guid` (`:235`); wire only under `ON_TRANSPORT`.
    pub transport_guid: ObjectGuid,
    /// `t_pos` (`:236`); wire only under `ON_TRANSPORT`.
    pub transport_pos: Position,
    /// `s_pitch` (`:238`); wire only under `SWIMMING`.
    pub swim_pitch: f32,
    /// `fallTime` (`:240`); always on the wire.
    pub fall_time: u32,
    /// `jump` (`:242`); wire only under `JUMPING`.
    pub jump: JumpInfo,
    /// `splineElevation` (`:244`); wire only under `SPLINE_ELEVATION`.
    pub spline_elevation: f32,
    /// `sourceSessionGuid` (`:246`); never on the wire.
    pub source_session_guid: u32,
}

impl MovementInfo {
    /// `Read` (`Object.cpp:61-103`). `server_time_ms` replaces the
    /// C++ `WorldTimer::getMSTime()` stamp at `:63`; handlers later
    /// overwrite it via [`Self::update_time`] with the packet time.
    ///
    /// Seeded-parse semantics are preserved: fields the packet does
    /// not carry keep their previous values, which is what latches
    /// the jump start across packets (`:90-94`); when `JUMPING` is
    /// absent only `start_client_time` resets (`:96-97`).
    ///
    /// # Errors
    ///
    /// [`ReadError`] on truncated input, leaving `self` partially
    /// updated exactly as the C++ stream operators would; callers
    /// treat any error as a dropped packet and discard the value.
    pub fn read(&mut self, data: &mut PacketReader, server_time_ms: u32) -> Result<(), ReadError> {
        self.server_time = server_time_ms;
        self.flags = MovementFlags(data.read_u32_le()?);
        self.client_time = data.read_u32_le()?;
        self.pos.x = data.read_f32_le()?;
        self.pos.y = data.read_f32_le()?;
        self.pos.z = data.read_f32_le()?;
        self.pos.o = data.read_f32_le()?;

        if self.flags.has(MovementFlags::ON_TRANSPORT) {
            // Raw u64 GUID with the player clamp,
            // `ObjectGuid.cpp:176-182`, via `from_wire_raw`.
            self.transport_guid = ObjectGuid::from_wire_raw(data.read_u64_le()?);
            self.transport_pos.x = data.read_f32_le()?;
            self.transport_pos.y = data.read_f32_le()?;
            self.transport_pos.z = data.read_f32_le()?;
            self.transport_pos.o = data.read_f32_le()?;
        }
        if self.flags.has(MovementFlags::SWIMMING) {
            self.swim_pitch = data.read_f32_le()?;
        }

        self.fall_time = data.read_u32_le()?;

        if self.flags.has(MovementFlags::JUMPING) {
            self.jump.z_speed = data.read_f32_le()?;
            self.jump.cos_angle = data.read_f32_le()?;
            self.jump.sin_angle = data.read_f32_le()?;
            self.jump.xy_speed = data.read_f32_le()?;
            if self.jump.start_client_time == 0 {
                self.jump.start_client_time = self.client_time;
                self.jump.start = self.pos;
            }
        } else {
            self.jump.start_client_time = 0;
        }

        if self.flags.has(MovementFlags::SPLINE_ELEVATION) {
            self.spline_elevation = data.read_f32_le()?;
        }
        Ok(())
    }

    /// `CorrectData` (`Object.cpp:105-136`): strip contradictory
    /// flag pairs; receiving clients freeze on `ROOT` combined with
    /// any moving flag. The HOVER-aura rule is commented out in C++
    /// (`:119-121`) and stays disabled here.
    pub fn correct_data(&mut self) {
        if self.flags.has(MovementFlags::ROOT) && self.flags.has(MovementFlags::MASK_MOVING) {
            self.flags.remove(MovementFlags::ROOT);
        }
        if self.flags.has(MovementFlags::TURN_LEFT) && self.flags.has(MovementFlags::TURN_RIGHT) {
            self.flags
                .remove(MovementFlags::TURN_LEFT | MovementFlags::TURN_RIGHT);
        }
        if self.flags.has(MovementFlags::STRAFE_LEFT) && self.flags.has(MovementFlags::STRAFE_RIGHT)
        {
            self.flags
                .remove(MovementFlags::STRAFE_LEFT | MovementFlags::STRAFE_RIGHT);
        }
        if self.flags.has(MovementFlags::PITCH_UP) && self.flags.has(MovementFlags::PITCH_DOWN) {
            self.flags
                .remove(MovementFlags::PITCH_UP | MovementFlags::PITCH_DOWN);
        }
        if self.flags.has(MovementFlags::FORWARD) && self.flags.has(MovementFlags::BACKWARD) {
            self.flags
                .remove(MovementFlags::FORWARD | MovementFlags::BACKWARD);
        }
    }

    /// `Write` (`Object.cpp:138-172`). Emits `server_time` in the
    /// slot [`Self::read`] filled from the client's time.
    pub fn write(&self, data: &mut PacketWriter) {
        data.write_u32_le(self.flags.raw())
            .write_u32_le(self.server_time)
            .write_f32_le(self.pos.x)
            .write_f32_le(self.pos.y)
            .write_f32_le(self.pos.z)
            .write_f32_le(self.pos.o);

        if self.flags.has(MovementFlags::ON_TRANSPORT) {
            data.write_u64_le(self.transport_guid.raw())
                .write_f32_le(self.transport_pos.x)
                .write_f32_le(self.transport_pos.y)
                .write_f32_le(self.transport_pos.z)
                .write_f32_le(self.transport_pos.o);
        }
        if self.flags.has(MovementFlags::SWIMMING) {
            data.write_f32_le(self.swim_pitch);
        }

        data.write_u32_le(self.fall_time);

        if self.flags.has(MovementFlags::JUMPING) {
            data.write_f32_le(self.jump.z_speed)
                .write_f32_le(self.jump.cos_angle)
                .write_f32_le(self.jump.sin_angle)
                .write_f32_le(self.jump.xy_speed);
        }

        if self.flags.has(MovementFlags::SPLINE_ELEVATION) {
            data.write_f32_le(self.spline_elevation);
        }
    }

    /// `VerifyMovementInfo` (`MovementHandler.cpp:1078-1095`):
    /// position must be a valid map coordinate; on a transport the
    /// local offset is box-limited (250/250/100) and the summed
    /// position must still be valid.
    #[must_use]
    pub fn verify(&self) -> bool {
        if !is_valid_map_coord(self.pos.x, self.pos.y, self.pos.z, self.pos.o) {
            return false;
        }

        if self.flags.has(MovementFlags::ON_TRANSPORT) {
            // Transports size limited (the zeppelin/lift absolute-
            // coordinate case is rejected here too, as in C++).
            if self.transport_pos.x.abs() > 250.0
                || self.transport_pos.y.abs() > 250.0
                || self.transport_pos.z.abs() > 100.0
            {
                return false;
            }

            if !is_valid_map_coord(
                self.pos.x + self.transport_pos.x,
                self.pos.y + self.transport_pos.y,
                self.pos.z + self.transport_pos.z,
                self.pos.o + self.transport_pos.o,
            ) {
                return false;
            }
        }

        true
    }

    /// `UpdateTime` (`MovementInfo.h:204`).
    pub fn update_time(&mut self, time_ms: u32) {
        self.server_time = time_ms;
    }

    /// `SetAsServerSide` (`MovementInfo.h:205-216`): `now_ms`
    /// replaces the C++ `WorldTimer::getMSTime()` read. Server-side
    /// packet ordering is preserved by bumping past the old time,
    /// and `client_time` is zeroed (pauses extrapolation).
    pub fn set_as_server_side(&mut self, now_ms: u32) {
        let old_time = self.server_time;
        self.server_time = now_ms;

        if old_time >= self.server_time {
            self.server_time = old_time + 1;
        }

        self.client_time = 0;
        self.source_session_guid = 0;
    }

    /// `WasSentBySession` (`MovementInfo.h:217`).
    #[must_use]
    pub fn was_sent_by_session(&self, session_guid: u32) -> bool {
        self.client_time != 0 && self.source_session_guid == session_guid
    }

    /// `SetTransportData` (`MovementInfo.h:176-183`).
    pub fn set_transport_data(&mut self, guid: ObjectGuid, pos: Position) {
        self.transport_guid = guid;
        self.transport_pos = pos;
    }

    /// `ClearTransportData` (`MovementInfo.h:184-191`).
    pub fn clear_transport_data(&mut self) {
        self.transport_guid = ObjectGuid::default();
        self.transport_pos = Position::default();
    }

    /// `ChangeOrientation` (`MovementInfo.h:196`).
    pub fn change_orientation(&mut self, orientation: f32) {
        self.pos.o = orientation;
    }

    /// `ChangePosition` (`MovementInfo.h:197-203`).
    pub fn change_position(&mut self, pos: Position) {
        self.pos = pos;
    }
}

/// `SIZE_OF_GRIDS` (`GridDefines.h:40`). The literal is spelled as
/// in C++ so the f32 rounding matches bit-for-bit.
#[allow(clippy::excessive_precision)]
pub const SIZE_OF_GRIDS: f32 = 533.33333;
/// `MAP_SIZE = SIZE_OF_GRIDS * MAX_NUMBER_OF_GRIDS` (`GridDefines.h:58`).
pub const MAP_SIZE: f32 = SIZE_OF_GRIDS * 64.0;
/// `MAP_HALFSIZE = MAP_SIZE / 2` (`GridDefines.h:59`).
pub const MAP_HALFSIZE: f32 = MAP_SIZE / 2.0;

/// `MaNGOS::IsValidZCoord` (`GridDefines.h:176`). As in C++, there is
/// no explicit finiteness check; NaN fails the comparison.
#[must_use]
pub fn is_valid_z_coord(z: f32) -> bool {
    z.abs() <= 400_000.0
}

/// `MaNGOS::IsValidMapCoord(x, y, z, o)` (`GridDefines.h:177-183`).
/// The C++ comparisons promote to `double` (`MAP_HALFSIZE - 0.5`,
/// `4 * M_PI`), mirrored here so boundary results match.
#[must_use]
pub fn is_valid_map_coord(x: f32, y: f32, z: f32, o: f32) -> bool {
    let limit = f64::from(MAP_HALFSIZE) - 0.5;
    let axis = |c: f32| c.is_finite() && f64::from(c.abs()) <= limit;
    axis(x)
        && axis(y)
        && is_valid_z_coord(z)
        && o.is_finite()
        && f64::from(o.abs()) <= 4.0 * std::f64::consts::PI
}
