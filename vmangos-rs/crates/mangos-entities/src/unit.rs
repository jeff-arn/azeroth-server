//! Shared `WorldObject`/`Unit` state for the aggregates
//! (`vmangos/src/game/Objects/{Creature,Unit}...` bases):
//! position, map location, phasing mask, movement info,
//! speeds, create stats/health/mana, and the value-word
//! accessors every unit needs. Player/Creature compose this;
//! nothing here knows about maps or sessions.

use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::{MovementInfo, Position};
use mangos_protocol::update_field_table::offsets;

use crate::object::ObjectBase;

/// `MAX_MOVE_TYPE` (baseMoveSpeed table, `Unit.cpp:66-73`).
pub const MAX_MOVE_TYPE: usize = 6;
/// `MOVE_WALK`.
pub const MOVE_WALK: usize = 0;
/// `MOVE_RUN`.
pub const MOVE_RUN: usize = 1;
/// `MOVE_TURN_RATE`.
pub const MOVE_TURN_RATE: usize = 5;
/// `UNIT_STATE_*` bits used by movement/combat slices
/// (`UnitDefines.h:392-460`).
pub const UNIT_STATE_MELEE_ATTACKING: u32 = 0x0000_0001;
pub const UNIT_STATE_FEIGN_DEATH: u32 = 0x0000_0004;
pub const UNIT_STATE_STUNNED: u32 = 0x0000_0008;
pub const UNIT_STATE_ROOT: u32 = 0x0000_0010;
pub const UNIT_STATE_DISTRACTED: u32 = 0x0000_0100;
pub const UNIT_STATE_CONFUSED: u32 = 0x0000_0200;
pub const UNIT_STATE_ROAMING: u32 = 0x0000_0800;
pub const UNIT_STATE_ROAMING_MOVE: u32 = 0x0000_1000;
pub const UNIT_STATE_CHASE: u32 = 0x0000_2000;
pub const UNIT_STATE_CHASE_MOVE: u32 = 0x0000_4000;
pub const UNIT_STATE_FOLLOW: u32 = 0x0000_8000;
pub const UNIT_STATE_FOLLOW_MOVE: u32 = 0x0001_0000;
pub const UNIT_STATE_FLEEING: u32 = 0x0002_0000;
pub const UNIT_STATE_FLEEING_MOVE: u32 = 0x0004_0000;
pub const UNIT_STATE_IGNORE_PATHFINDING: u32 = 0x0008_0000;
pub const UNIT_STATE_RUNNING: u32 = 0x0100_0000;
pub const UNIT_STATE_CAN_NOT_MOVE: u32 =
    UNIT_STATE_ROOT | UNIT_STATE_STUNNED | UNIT_STATE_FEIGN_DEATH;
pub const UNIT_STATE_MOVING: u32 = UNIT_STATE_ROAMING_MOVE
    | UNIT_STATE_CHASE_MOVE
    | UNIT_STATE_FOLLOW_MOVE
    | UNIT_STATE_FLEEING_MOVE;
pub const UNIT_STATE_ALL_DYN_STATES: u32 = !(UNIT_STATE_RUNNING
    | UNIT_STATE_IGNORE_PATHFINDING
    | 0x0400_0000
    | 0x0800_0000
    | 0x1000_0000
    | 0x2000_0000);

/// `enum MovementChangeType` (`UnitDefines.h:38-56`).
pub const MOVEMENT_CHANGE_INVALID: u8 = 0;
pub const MOVEMENT_CHANGE_ROOT: u8 = 1;
pub const MOVEMENT_CHANGE_WATER_WALK: u8 = 2;
pub const MOVEMENT_CHANGE_SET_HOVER: u8 = 3;
pub const MOVEMENT_CHANGE_FEATHER_FALL: u8 = 4;
pub const MOVEMENT_CHANGE_SPEED_WALK: u8 = 5;
pub const MOVEMENT_CHANGE_SPEED_RUN: u8 = 6;
pub const MOVEMENT_CHANGE_SPEED_RUN_BACK: u8 = 7;
pub const MOVEMENT_CHANGE_SPEED_SWIM: u8 = 8;
pub const MOVEMENT_CHANGE_SPEED_SWIM_BACK: u8 = 9;
pub const MOVEMENT_CHANGE_RATE_TURN: u8 = 10;
pub const MOVEMENT_CHANGE_TELEPORT: u8 = 11;
pub const MOVEMENT_CHANGE_KNOCK_BACK: u8 = 12;

/// `GetChangeTypeByMoveType` for the six speed types.
#[must_use]
pub fn speed_change_type(move_type: usize) -> u8 {
    match move_type {
        0 => MOVEMENT_CHANGE_SPEED_WALK,
        1 => MOVEMENT_CHANGE_SPEED_RUN,
        2 => MOVEMENT_CHANGE_SPEED_RUN_BACK,
        3 => MOVEMENT_CHANGE_SPEED_SWIM,
        4 => MOVEMENT_CHANGE_SPEED_SWIM_BACK,
        _ => MOVEMENT_CHANGE_RATE_TURN,
    }
}

/// `GetMoveTypeByChangeType` inverse (speed changes only).
#[must_use]
pub fn move_type_for_change(change_type: u8) -> Option<usize> {
    match change_type {
        MOVEMENT_CHANGE_SPEED_WALK => Some(0),
        MOVEMENT_CHANGE_SPEED_RUN => Some(1),
        MOVEMENT_CHANGE_SPEED_RUN_BACK => Some(2),
        MOVEMENT_CHANGE_SPEED_SWIM => Some(3),
        MOVEMENT_CHANGE_SPEED_SWIM_BACK => Some(4),
        MOVEMENT_CHANGE_RATE_TURN => Some(5),
        _ => None,
    }
}

/// `struct PlayerMovementPendingChange` (`Unit.h:61-79`,
/// knockback payload deferred with knockback itself).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingMovementChange {
    pub movement_counter: u32,
    pub change_type: u8,
    /// Flat speed for speed changes.
    pub new_value: f32,
    /// Apply flag for movement-flag changes.
    pub apply: bool,
}

/// `baseMoveSpeed[MAX_MOVE_TYPE]` (`Unit.cpp:66-73`).
pub const BASE_MOVE_SPEED: [f32; MAX_MOVE_TYPE] = [2.5, 7.0, 4.5, 4.722_222, 2.5, 3.141_594];

/// `MAX_STATS` (`enum Stats`: strength, agility, stamina,
/// intellect, spirit).
pub const MAX_STATS: usize = 5;
/// `STAT_STRENGTH`.
pub const STAT_STRENGTH: usize = 0;
/// `STAT_AGILITY`.
pub const STAT_AGILITY: usize = 1;
/// `STAT_STAMINA`.
pub const STAT_STAMINA: usize = 2;
/// `STAT_INTELLECT`.
pub const STAT_INTELLECT: usize = 3;
/// `STAT_SPIRIT`.
pub const STAT_SPIRIT: usize = 4;

/// `MAX_POWERS` (`enum Powers`: mana, rage, focus, energy,
/// happiness).
pub const MAX_POWERS: usize = 5;
/// `POWER_MANA`.
pub const POWER_MANA: usize = 0;
/// `POWER_RAGE`.
pub const POWER_RAGE: usize = 1;
/// `POWER_FOCUS`.
pub const POWER_FOCUS: usize = 2;
/// `POWER_ENERGY`.
pub const POWER_ENERGY: usize = 3;
/// `POWER_HAPPINESS`.
pub const POWER_HAPPINESS: usize = 4;

/// `MAX_SPELL_SCHOOL` (normal + 6 magic schools).
pub const MAX_SPELL_SCHOOL: usize = 7;
/// `SPELL_SCHOOL_NORMAL`.
pub const SPELL_SCHOOL_NORMAL: usize = 0;

/// `DEFAULT_OBJECT_SCALE` (`ObjectDefines.h:43`).
pub const DEFAULT_OBJECT_SCALE: f32 = 1.0;
/// `DEFAULT_WORLD_OBJECT_SIZE` (`ObjectDefines.h:42`; same
/// f32 bits as the C++ literal `0.388999998569489f`).
pub const DEFAULT_WORLD_OBJECT_SIZE: f32 = 0.389;

/// `CheckValidScale` (`Unit.cpp:9393-9399`).
#[must_use]
pub fn check_valid_scale(scale: f32) -> f32 {
    if scale <= 0.0 {
        DEFAULT_OBJECT_SCALE
    } else {
        scale
    }
}

/// `enum Team` subset (`SharedDefines.h`): the values
/// `TeamForRace` can produce.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Team {
    #[default]
    None,
    Alliance,
    Horde,
}

/// `enum DeathState` (`UnitDefines.h:378-386`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeathState {
    #[default]
    Alive,
    JustDied,
    Corpse,
    Dead,
    JustAlived,
    CorpseFalling,
}

/// `WorldObject` location state (`WorldObject.h`): position,
/// location map/instance, phasing world mask.
#[derive(Clone, Debug)]
pub struct WorldObjectBase {
    pub object: ObjectBase,
    /// `m_position` (authoritative; `movement_info.pos`
    /// mirrors it once the client drives).
    pub position: Position,
    pub map_id: u32,
    pub instance_id: u32,
    /// `m_worldMask` (phasing).
    pub world_mask: u32,
    pub name: String,
    /// `m_movementInfo`.
    pub movement_info: MovementInfo,
}

impl WorldObjectBase {
    #[must_use]
    pub fn new(object: ObjectBase) -> Self {
        Self {
            object,
            position: Position::default(),
            map_id: 0,
            instance_id: 0,
            world_mask: 0xFFFF_FFFF,
            name: String::new(),
            movement_info: MovementInfo::default(),
        }
    }

    /// `Relocate(x, y, z, o)`.
    pub fn relocate(&mut self, position: Position) {
        self.position = position;
        self.movement_info.pos = position;
    }

    #[must_use]
    pub fn position(&self) -> Position {
        self.position
    }
}

/// `Unit` state shared by players and creatures.
#[derive(Clone, Debug)]
pub struct UnitBase {
    pub world: WorldObjectBase,
    /// `m_createStats`.
    pub create_stats: [f32; MAX_STATS],
    /// `m_createHealth`.
    pub create_health: u32,
    /// `m_createMana`.
    pub create_mana: u32,
    /// `m_speed_rate[MAX_MOVE_TYPE]` (multiplies
    /// [`BASE_MOVE_SPEED`]).
    pub speed_rate: [f32; MAX_MOVE_TYPE],
    /// Display-addon walk/run speed rates for creatures
    /// (1.0 for players; `Creature::GetSpeedRate` inputs).
    pub display_speed_walk: f32,
    pub display_speed_run: f32,
    /// `m_modelCollisionHeight`.
    pub model_collision_height: f32,
    pub death_state: DeathState,
    /// Server-side spline (`Unit::movespline`).
    pub spline: mangos_movement::MoveSpline,
    /// `HasPendingSplineDone` (player-controlled splines).
    pub pending_spline_done: bool,
    /// `m_movementCounter` (ACK sequencing).
    pub movement_counter: u32,
    /// `m_pendingMovementChanges`.
    pub pending_movement_changes: std::collections::VecDeque<PendingMovementChange>,
    /// Applied aura holders (`m_spellAuraHolders`
    /// subset; slot fields are written by the map).
    pub auras: Vec<crate::aura::AuraHolder>,
    /// `m_attackPowerMods[MELEE]` flat pos/neg sums
    /// (multiplier lands with percentage auras).
    pub attack_power_mods: [f32; 2],
    /// `m_castCounter` (per-unit cast sequence byte).
    pub cast_counter: u8,
    /// Global cooldown expiry on the map ms clock
    /// (`m_GCD` end).
    pub gcd_until_ms: u32,
    /// `m_attackTimer[BASE_ATTACK/OFF_ATTACK]` (ms until
    /// the next swing is due; Slice 6 melee).
    pub attack_timers: [u32; 2],
    /// `m_attacking` (current melee victim).
    pub melee_victim: Option<ObjectGuid>,
    /// Map ms clock of the last damage dealt or taken
    /// (player combat-drop approximation of
    /// `m_combatTimer`, Slice 6).
    pub last_combat_ms: u32,
    /// Class-level base attack power before aura mods
    /// (creature `pCLS->attack_power`; player: the
    /// materialized `UNIT_FIELD_ATTACK_POWER`). Drives
    /// the `UpdateDamagePhysical` AP factor.
    pub base_attack_power: f32,
    /// `m_unitState` bits (`UnitDefines.h` `UnitState`).
    pub unit_state: u32,
}

impl UnitBase {
    #[must_use]
    pub fn new(world: WorldObjectBase) -> Self {
        Self {
            world,
            create_stats: [0.0; MAX_STATS],
            create_health: 0,
            create_mana: 0,
            speed_rate: [1.0; MAX_MOVE_TYPE],
            display_speed_walk: 1.0,
            display_speed_run: 1.0,
            model_collision_height: 2.0,
            death_state: DeathState::Alive,
            spline: mangos_movement::MoveSpline::new(),
            pending_spline_done: false,
            movement_counter: 0,
            pending_movement_changes: std::collections::VecDeque::new(),
            auras: Vec::new(),
            attack_power_mods: [0.0, 0.0],
            cast_counter: 0,
            gcd_until_ms: 0,
            attack_timers: [0, 0],
            melee_victim: None,
            last_combat_ms: 0,
            base_attack_power: 0.0,
            unit_state: 0,
        }
    }

    /// `GetSpeed(type)` without aura modifiers:
    /// `baseMoveSpeed * m_speed_rate` (display rates fold
    /// into `speed_rate` for creatures at spawn).
    #[must_use]
    pub fn speed(&self, move_type: usize) -> f32 {
        BASE_MOVE_SPEED[move_type] * self.speed_rate[move_type]
    }

    /// `HasUnitState` (any-bit test, `Unit.h`).
    #[must_use]
    pub fn has_unit_state(&self, state: u32) -> bool {
        self.unit_state & state != 0
    }

    /// `AddUnitState`.
    pub fn add_unit_state(&mut self, state: u32) {
        self.unit_state |= state;
    }

    /// `ClearUnitState`.
    pub fn clear_unit_state(&mut self, state: u32) {
        self.unit_state &= !state;
    }

    /// `IsStopped()` (`Unit.h:1434`).
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        !self.has_unit_state(UNIT_STATE_MOVING)
    }

    /// `GetMovementCounterAndInc` (post-increment).
    pub fn movement_counter_and_inc(&mut self) -> u32 {
        let current = self.movement_counter;
        self.movement_counter = self.movement_counter.wrapping_add(1);
        current
    }

    /// `HasPendingMovementChange()` (any) and the per-type
    /// overload used by `SetSpeedRate`.
    #[must_use]
    pub fn has_pending_movement_change(&self, change_type: Option<u8>) -> bool {
        match change_type {
            None => !self.pending_movement_changes.is_empty(),
            Some(t) => self
                .pending_movement_changes
                .iter()
                .any(|c| c.change_type == t),
        }
    }

    /// `FindPendingMovementSpeedChange`: match counter,
    /// type, and flat speed within 0.01; erase on hit.
    pub fn find_pending_speed_change(
        &mut self,
        speed_received: f32,
        movement_counter: u32,
        move_type: usize,
    ) -> bool {
        let want = speed_change_type(move_type);
        let hit = self.pending_movement_changes.iter().position(|c| {
            c.movement_counter == movement_counter
                && c.change_type == want
                && (c.new_value - speed_received).abs() <= 0.01
        });
        match hit {
            Some(idx) => {
                self.pending_movement_changes.remove(idx);
                true
            }
            None => false,
        }
    }

    /// `FindPendingMovementFlagChange` analogue: match
    /// counter + change type + apply; erase on hit.
    pub fn find_pending_flag_change(
        &mut self,
        change_type: u8,
        movement_counter: u32,
        apply: bool,
    ) -> bool {
        let hit = self.pending_movement_changes.iter().position(|c| {
            c.movement_counter == movement_counter
                && c.change_type == change_type
                && c.apply == apply
        });
        match hit {
            Some(idx) => {
                self.pending_movement_changes.remove(idx);
                true
            }
            None => false,
        }
    }

    /// The six speeds a LIVING movement section writes.
    #[must_use]
    pub fn speeds(&self) -> [f32; 6] {
        let mut out = [0.0f32; 6];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.speed(i);
        }
        out
    }

    // -- values shorthand (marks flow through ObjectBase) ----

    /// Mutable update-values mirror (map-layer writes:
    /// aura slots, combat flags, attack power mods).
    pub fn values(&mut self) -> &mut crate::values::Values {
        self.world.object.values_mut()
    }

    #[must_use]
    pub fn race(&self) -> u8 {
        self.world
            .object
            .values()
            .get_byte(offsets::UNIT_FIELD_BYTES_0, 0)
    }

    #[must_use]
    pub fn class(&self) -> u8 {
        self.world
            .object
            .values()
            .get_byte(offsets::UNIT_FIELD_BYTES_0, 1)
    }

    #[must_use]
    pub fn gender(&self) -> u8 {
        self.world
            .object
            .values()
            .get_byte(offsets::UNIT_FIELD_BYTES_0, 2)
    }

    #[must_use]
    pub fn power_type(&self) -> u8 {
        self.world
            .object
            .values()
            .get_byte(offsets::UNIT_FIELD_BYTES_0, 3)
    }

    #[must_use]
    pub fn level(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_LEVEL)
    }

    #[must_use]
    pub fn health(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_HEALTH)
    }

    #[must_use]
    pub fn max_health(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_MAXHEALTH)
    }

    /// `SetHealth` (clamps nothing; callers clamp like C++).
    pub fn set_health(&mut self, value: u32) {
        self.values().set_u32(offsets::UNIT_FIELD_HEALTH, value);
    }

    /// `SetMaxHealth`.
    pub fn set_max_health(&mut self, value: u32) {
        self.values().set_u32(offsets::UNIT_FIELD_MAXHEALTH, value);
    }

    #[must_use]
    pub fn power(&self, power: usize) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_POWER1 + power as u16)
    }

    #[must_use]
    pub fn max_power(&self, power: usize) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_MAXPOWER1 + power as u16)
    }

    /// `SetPower`.
    pub fn set_power(&mut self, power: usize, value: u32) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_POWER1 + power as u16, value);
    }

    /// `SetMaxPower`.
    pub fn set_max_power(&mut self, power: usize, value: u32) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_MAXPOWER1 + power as u16, value);
    }

    /// `GetCreatePowers` for a player-shaped unit
    /// (`Unit.cpp:8345-8364`): rage 1000, energy 100, pet
    /// powers 0.
    #[must_use]
    pub fn create_power(&self, power: usize) -> u32 {
        match power {
            POWER_MANA => self.create_mana,
            POWER_RAGE => 1000,
            POWER_ENERGY => 100,
            _ => 0,
        }
    }

    /// `GetStat` (`Unit.h:390`): reads the word as
    /// **unsigned** and converts to float — a negative stat
    /// (written only through raw setters) reads back huge,
    /// exactly as in C++.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // C++ converts identically
    pub fn stat(&self, stat: usize) -> f32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_STAT0 + stat as u16) as f32
    }

    /// `SetStat` (`Unit.h:366`): `SetStatInt32Value`, i.e.
    /// negatives clamp to 0.
    pub fn set_stat(&mut self, stat: usize, value: i32) {
        self.values()
            .set_stat_i32(offsets::UNIT_FIELD_STAT0 + stat as u16, value);
    }

    /// `SetArmor` (`Unit.h:392`): resistance school 0 through
    /// the stat setter.
    pub fn set_armor(&mut self, value: i32) {
        self.values()
            .set_stat_i32(offsets::UNIT_FIELD_RESISTANCES, value);
    }

    /// `SetResistance` (`Unit.h:367`): plain signed write.
    pub fn set_resistance(&mut self, school: usize, value: i32) {
        self.values()
            .set_i32(offsets::UNIT_FIELD_RESISTANCES + school as u16, value);
    }

    #[must_use]
    pub fn display_id(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_DISPLAYID)
    }

    #[must_use]
    pub fn native_display_id(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID)
    }

    #[must_use]
    pub fn faction_template(&self) -> u32 {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE)
    }

    /// `SetFactionTemplateId` (value half; template-store
    /// lookups live in the map/spell layers).
    pub fn set_faction_template(&mut self, template_id: u32) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE, template_id);
    }

    #[must_use]
    pub fn guid(&self) -> ObjectGuid {
        self.world.object.object_guid()
    }

    #[must_use]
    pub fn is_alive(&self) -> bool {
        matches!(self.death_state, DeathState::Alive | DeathState::JustAlived)
    }
}

/// Model data for `UpdateModelData` (`Unit.cpp:9414-9445`),
/// precomputed by the caller from `CreatureDisplayInfo.dbc` +
/// `CreatureModelData.dbc` + `creature_display_info_addon`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelData {
    /// `modelScale * displayScale` (`CheckValidScale`'d).
    pub native_scale: f32,
    pub bounding_radius: f32,
    pub combat_reach: f32,
    /// `collisionHeight / modelScale`, if a model entry with
    /// positive values exists.
    pub collision_height: Option<f32>,
}

impl UnitBase {
    /// `SetDisplayId` value half + `UpdateModelData`. Without
    /// model data (or with zero bounding radius) the C++
    /// fallback writes 1.5/1.5.
    pub fn set_display_id(&mut self, display_id: u32, model: Option<ModelData>) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_DISPLAYID, display_id);
        let scale = self.world.object.object_scale();
        match model {
            Some(m) if m.bounding_radius != 0.0 => {
                let ratio = scale / m.native_scale;
                self.values().set_f32(
                    offsets::UNIT_FIELD_BOUNDINGRADIUS,
                    ratio * m.bounding_radius,
                );
                self.values()
                    .set_f32(offsets::UNIT_FIELD_COMBATREACH, ratio * m.combat_reach);
                if let Some(h) = m.collision_height {
                    self.model_collision_height = h * ratio;
                }
            }
            _ => {
                self.values().set_f32(offsets::UNIT_FIELD_COMBATREACH, 1.5);
                self.values()
                    .set_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS, 1.5);
            }
        }
    }

    /// `SetNativeDisplayId`.
    pub fn set_native_display_id(&mut self, display_id: u32) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID, display_id);
    }
}
