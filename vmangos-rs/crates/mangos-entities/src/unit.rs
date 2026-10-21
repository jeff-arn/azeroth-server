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

/// One `m_auraModifiersGroup` row: the four
/// `UnitModifierType` registers (pct rows start at 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatModGroup {
    pub base_value: f32,
    pub base_pct: f32,
    pub total_value: f32,
    pub total_pct: f32,
}

impl Default for StatModGroup {
    fn default() -> Self {
        Self {
            base_value: 0.0,
            base_pct: 1.0,
            total_value: 0.0,
            total_pct: 1.0,
        }
    }
}

/// `UnitModifierType` subset for stat groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatModKind {
    BaseValue,
    BasePct,
    TotalValue,
    TotalPct,
}

/// `ApplyPercentModFloatVar` (`Util.h:176`).
pub fn apply_percent_mod_float_var(var: &mut f32, val: f32, apply: bool) {
    #[allow(clippy::float_cmp)]
    let val = if val == -100.0 { -99.99 } else { val };
    if apply {
        *var *= (100.0 + val) / 100.0;
    } else {
        *var *= 100.0 / (100.0 + val);
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
    /// Aura-free speed rate (creature template/display
    /// rates; 1.0 for players). `UpdateSpeed` aura
    /// factors multiply this base.
    pub base_speed_rate: [f32; MAX_MOVE_TYPE],
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
    /// Aura-fed `m_auraModifiersGroup[UNIT_MOD_STAT_*]`
    /// rows (item stats compose separately as their own
    /// `BASE_VALUE` aggregate on players).
    pub stat_mods: [StatModGroup; MAX_STATS],
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
    /// `m_lastManaUseTimer` (five-second rule countdown).
    pub last_mana_use_timer_ms: u32,
    /// `m_lastManaUseSpellId`.
    pub last_mana_use_spell: u32,
    /// Class-level base attack power before aura mods
    /// (creature `pCLS->attack_power`; player: the
    /// materialized `UNIT_FIELD_ATTACK_POWER`). Drives
    /// the `UpdateDamagePhysical` AP factor.
    pub base_attack_power: f32,
    /// `m_unitState` bits (`UnitDefines.h` `UnitState`).
    pub unit_state: u32,
    /// Base weapon damage ranges per attack type
    /// ([mainhand, offhand, ranged]; `m_weaponDamage`
    /// damage row 0 — rows 1+ join multi-damage swing
    /// summation, the update fields only carry row 0
    /// in C++ too). Fists (`BASE_MINDAMAGE`/
    /// `BASE_MAXDAMAGE`) until item mods apply.
    /// Creatures keep their template ranges on the
    /// `Creature` aggregate; this field serves players.
    pub base_weapon_damage: [(f32, f32); 3],
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
            base_speed_rate: [1.0; MAX_MOVE_TYPE],
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
            stat_mods: [StatModGroup::default(); MAX_STATS],
            cast_counter: 0,
            gcd_until_ms: 0,
            attack_timers: [0, 0],
            melee_victim: None,
            last_combat_ms: 0,
            last_mana_use_timer_ms: 0,
            last_mana_use_spell: 0,
            base_attack_power: 0.0,
            unit_state: 0,
            base_weapon_damage: [(1.0, 2.0); 3],
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

    /// `SetHealth` (caps at max health, as in C++).
    pub fn set_health(&mut self, value: u32) {
        let value = value.min(self.max_health());
        self.values().set_u32(offsets::UNIT_FIELD_HEALTH, value);
    }

    /// `SetMaxHealth` (clamps current health down to a
    /// shrunken max, as the C++ tail does).
    pub fn set_max_health(&mut self, value: u32) {
        self.values().set_u32(offsets::UNIT_FIELD_MAXHEALTH, value);
        if self.health() > value {
            self.set_health(value);
        }
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

    /// `SetMaxPower` (clamps current power down to a
    /// shrunken max, as the C++ tail does).
    pub fn set_max_power(&mut self, power: usize, value: u32) {
        self.values()
            .set_u32(offsets::UNIT_FIELD_MAXPOWER1 + power as u16, value);
        if self.power(power) > value {
            self.set_power(power, value);
        }
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

    /// The register half of `HandleStatModifier`
    /// (`Unit.cpp:8022`) for one stat group; the
    /// `Update*` dispatch is the caller's derived-stat
    /// refresh (map-side, mirroring the attack-power
    /// mod pattern).
    pub fn handle_stat_modifier(
        &mut self,
        stat: usize,
        kind: StatModKind,
        amount: f32,
        apply: bool,
    ) {
        let group = &mut self.stat_mods[stat];
        match kind {
            StatModKind::BaseValue => group.base_value += if apply { amount } else { -amount },
            StatModKind::TotalValue => group.total_value += if apply { amount } else { -amount },
            StatModKind::BasePct => apply_percent_mod_float_var(&mut group.base_pct, amount, apply),
            StatModKind::TotalPct => {
                apply_percent_mod_float_var(&mut group.total_pct, amount, apply);
            }
        }
    }

    /// `GetTotalStatValue` (`Unit.cpp:8118`) with the
    /// item-fed `BASE_VALUE` aggregate passed separately
    /// (players; creatures pass 0):
    /// `((create + base) * base_pct + total) * total_pct`,
    /// 0 when the total pct register is non-positive.
    #[must_use]
    pub fn total_stat_value(&self, stat: usize, item_base: f32) -> f32 {
        let group = &self.stat_mods[stat];
        if group.total_pct <= 0.0 {
            return 0.0;
        }
        let mut value = self.create_stats[stat] + item_base + group.base_value;
        value *= group.base_pct;
        value += group.total_value;
        value *= group.total_pct;
        value
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

    /// `GetStandState`: `UNIT_FIELD_BYTES_1` byte 0.
    #[must_use]
    pub fn stand_state(&self) -> u8 {
        self.world
            .object
            .values()
            .get_byte(offsets::UNIT_FIELD_BYTES_1, 0)
    }

    /// `IsStandingUp` (`Unit.cpp:9351`): STAND (0) or
    /// DEAD (7).
    #[must_use]
    pub fn is_standing_up(&self) -> bool {
        matches!(self.stand_state(), 0 | 7)
    }

    /// `IsInCombat`: `UNIT_FLAG_IN_COMBAT` (0x00080000)
    /// in `UNIT_FIELD_FLAGS`.
    #[must_use]
    pub fn is_in_combat(&self) -> bool {
        self.world
            .object
            .values()
            .get_u32(offsets::UNIT_FIELD_FLAGS)
            & 0x0008_0000
            != 0
    }

    /// `SetLastManaUse` (`Unit.h:874`): arms the
    /// five-second timer.
    pub fn set_last_mana_use(&mut self, spell_id: u32) {
        self.last_mana_use_timer_ms = 5000;
        self.last_mana_use_spell = spell_id;
    }

    /// `IsUnderLastManaUseEffect` (`Unit.h:879`).
    #[must_use]
    pub fn is_under_last_mana_use_effect(&self) -> bool {
        self.last_mana_use_timer_ms != 0
    }

    /// The `m_lastManaUseTimer` countdown in
    /// `Unit::Update` (`Unit.cpp:229-248`). The
    /// still-channeling exception (patch 1.7.0 arm)
    /// joins channeled casts; no channeled spell can
    /// be active in the current spell engine, so the
    /// timer clears on expiry exactly as the fork's
    /// non-channeling path does.
    pub fn update_last_mana_use(&mut self, diff_ms: u32) {
        if self.last_mana_use_timer_ms == 0 {
            return;
        }
        if diff_ms >= self.last_mana_use_timer_ms {
            self.last_mana_use_timer_ms = 0;
            self.last_mana_use_spell = 0;
        } else {
            self.last_mana_use_timer_ms -= diff_ms;
        }
    }

    /// `GetRegenHPPerSpirit` (`Unit.cpp:6403-6442`):
    /// health per 2-second tick by class. Class ids:
    /// warrior 1, paladin 2, hunter 3, rogue 4, priest
    /// 5, shaman 7, mage 8, warlock 9, druid 11.
    #[must_use]
    pub fn regen_hp_per_spirit(&self) -> f32 {
        let spirit = self.stat(STAT_SPIRIT);
        let regen = match self.class() {
            11 | 8 => spirit * 0.11 + 1.0,
            3 => spirit * 0.43 - 5.5,
            2 => spirit * 0.25,
            5 => spirit * 0.15 + 1.4,
            4 => spirit * 0.84 - 13.0,
            7 => spirit * 0.28 - 3.6,
            9 => spirit * 0.12 + 1.5,
            1 => spirit * 1.26 - 22.6,
            _ => 0.0,
        };
        regen.max(0.0)
    }

    /// `GetRegenMPPerSpirit` (`Unit.cpp:6444-6478`):
    /// mana per second (the per-tick values are halved
    /// in C++). Classes without a case return 0.
    #[must_use]
    pub fn regen_mp_per_spirit(&self) -> f32 {
        let spirit = self.stat(STAT_SPIRIT);
        let addvalue = match self.class() {
            11 | 3 | 2 | 9 => spirit / 5.0 + 15.0,
            8 | 5 => spirit / 4.0 + 12.5,
            7 => spirit / 5.0 + 17.0,
            _ => 0.0,
        };
        addvalue / 2.0
    }

    /// `GetAttackPowerFromStrengthAndAgility`
    /// (`StatSystem.cpp:195-306`) over the current stat
    /// fields. Shapeshift-form arms take their default
    /// (caster-form) branches: no form state exists in
    /// the Rust aggregates yet.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // C++ converts identically
    pub fn attack_power_from_stats(&self, ranged: bool) -> f32 {
        let strength = self.stat(STAT_STRENGTH);
        let agility = self.stat(STAT_AGILITY);
        let level = self.level() as f32;
        if ranged {
            match self.class() {
                // hunter
                3 => level * 2.0 + agility * 2.0 - 10.0,
                // rogue, warrior
                4 | 1 => level + agility - 10.0,
                // druid caster form and everyone else
                _ => agility - 10.0,
            }
        } else {
            match self.class() {
                // warrior, paladin
                1 | 2 => level * 3.0 + strength * 2.0 - 20.0,
                // rogue, hunter
                4 | 3 => level * 2.0 + strength + agility - 20.0,
                // shaman
                7 => level * 2.0 + strength * 2.0 - 20.0,
                // druid caster form
                11 => strength * 2.0 - 20.0,
                // mage, priest, warlock
                8 | 5 | 9 => strength - 10.0,
                _ => 0.0,
            }
        }
    }

    /// `GetTotalAttackPowerValue` (`Unit.cpp:8137-8168`):
    /// base field + signed mod halves, floored at 0,
    /// scaled by `1 + multiplier`. `att` indexes
    /// `WeaponAttackType` (0 main, 1 off, 2 ranged);
    /// offhand shares the melee fields as in C++.
    #[must_use]
    pub fn total_attack_power(&self, att: usize) -> f32 {
        let v = self.world.object.values();
        let (index, index_mod, index_mult) = if att == 2 {
            (
                offsets::UNIT_FIELD_RANGED_ATTACK_POWER,
                offsets::UNIT_FIELD_RANGED_ATTACK_POWER_MODS,
                offsets::UNIT_FIELD_RANGED_ATTACK_POWER_MULTIPLIER,
            )
        } else {
            (
                offsets::UNIT_FIELD_ATTACK_POWER,
                offsets::UNIT_FIELD_ATTACK_POWER_MODS,
                offsets::UNIT_FIELD_ATTACK_POWER_MULTIPLIER,
            )
        };
        let ap = v.get_i32(index)
            + i32::from(v.get_i16(index_mod, 0))
            + i32::from(v.get_i16(index_mod, 1));
        if ap < 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)] // C++ converts identically
        let ap = ap as f32;
        ap * (1.0 + v.get_f32(index_mult))
    }

    /// `CalculateMinMaxDamage(att, normalized=false)`
    /// at identity damage modifier groups
    /// (`StatSystem.cpp:359-446`): the aura-fed pct/
    /// flat groups, shapeshift weapon override, and
    /// the ranged ammo-DPS term (`_ApplyAmmoBonuses`
    /// unported) contribute their no-aura identities.
    /// `att` as in [`Self::total_attack_power`].
    #[must_use]
    pub fn calculate_min_max_damage(&self, att: usize) -> (f32, f32) {
        let v = self.world.object.values();
        let attack_time_ms = match att {
            1 => v.get_f32(offsets::UNIT_FIELD_BASEATTACKTIME + 1),
            2 => v.get_f32(offsets::UNIT_FIELD_RANGEDATTACKTIME),
            _ => v.get_f32(offsets::UNIT_FIELD_BASEATTACKTIME),
        };
        // `GetAPMultiplier` non-normalized arm.
        let att_speed = attack_time_ms / 1000.0;
        let base_value = self.total_attack_power(att) / 14.0 * att_speed;
        let (weapon_min, weapon_max) = self.base_weapon_damage[att.min(2)];
        (base_value + weapon_min, base_value + weapon_max)
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

#[cfg(test)]
mod tests {
    use super::{STAT_SPIRIT, UnitBase, WorldObjectBase};
    use crate::object::ObjectBase;
    use mangos_guid::TypeId;
    use mangos_protocol::update_data::ObjectUpdateFlags;
    use mangos_protocol::update_field_table::{UNIT_END, offsets};
    use mangos_protocol::update_fields::TypeMask;

    fn unit_with(class: u8, spirit: i32) -> UnitBase {
        let object = ObjectBase::new(
            TypeId::Player,
            TypeMask::OBJECT.union(TypeMask::UNIT),
            ObjectUpdateFlags(ObjectUpdateFlags::ALL),
            UNIT_END,
        );
        let mut unit = UnitBase::new(WorldObjectBase::new(object));
        unit.values()
            .set_byte(offsets::UNIT_FIELD_BYTES_0, 1, class);
        unit.set_stat(STAT_SPIRIT, spirit);
        unit
    }

    #[test]
    fn spirit_regen_matches_unit_cpp_tables() {
        // Priest (5): HP 0.15*spirit+1.4 per tick;
        // MP (spirit/4+12.5)/2 per second.
        let priest = unit_with(5, 100);
        assert!((priest.regen_hp_per_spirit() - 16.4).abs() < 1e-5);
        assert!((priest.regen_mp_per_spirit() - 18.75).abs() < 1e-5);
        // Warrior (1): HP 1.26*spirit-22.6, no MP case.
        let warrior = unit_with(1, 100);
        assert!((warrior.regen_hp_per_spirit() - 103.4).abs() < 1e-4);
        assert!(warrior.regen_mp_per_spirit().abs() < 1e-6);
        // Rogue (4) HP floors at 0 for low spirit.
        let rogue = unit_with(4, 10);
        assert!(rogue.regen_hp_per_spirit().abs() < 1e-6);
        // Shaman (7) MP uses +17: (100/5+17)/2.
        let shaman = unit_with(7, 100);
        assert!((shaman.regen_mp_per_spirit() - 18.5).abs() < 1e-5);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn stat_mod_registers_compose_like_fork() {
        use super::{STAT_STAMINA, StatModKind, apply_percent_mod_float_var};
        let mut unit = unit_with(1, 0);
        unit.create_stats[STAT_STAMINA] = 42.0;
        // Identity groups: create + item base only.
        assert_eq!(unit.total_stat_value(STAT_STAMINA, 6.0), 48.0);
        // MOD_STAT flat (TOTAL_VALUE) adds after base pct.
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::TotalValue, 10.0, true);
        assert_eq!(unit.total_stat_value(STAT_STAMINA, 6.0), 58.0);
        // MOD_PERCENT_STAT (BASE_PCT +10%): (48)*1.1 + 10.
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::BasePct, 10.0, true);
        assert!((unit.total_stat_value(STAT_STAMINA, 6.0) - 62.8).abs() < 1e-4);
        // Resurrection Sickness shape replaces the buffs:
        // TOTAL_PCT -75% over the naked (42+6).
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::TotalValue, 10.0, false);
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::BasePct, 10.0, false);
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::TotalPct, -75.0, true);
        assert_eq!(unit.total_stat_value(STAT_STAMINA, 6.0), 12.0);
        unit.handle_stat_modifier(STAT_STAMINA, StatModKind::TotalPct, -75.0, false);
        assert_eq!(unit.total_stat_value(STAT_STAMINA, 6.0), 48.0);
        // TOTAL_PCT <= 0 reads 0; -100 clamps to -99.99
        // in the percent helper, so the register stays > 0.
        let mut v = 200.0f32;
        apply_percent_mod_float_var(&mut v, -100.0, true);
        assert!((v - 0.02).abs() < 1e-4);
        apply_percent_mod_float_var(&mut v, -100.0, false);
        assert!((v - 200.0).abs() < 1e-2);
    }

    #[test]
    fn five_second_timer_arms_counts_and_clears() {
        let mut unit = unit_with(8, 50);
        assert!(!unit.is_under_last_mana_use_effect());
        unit.set_last_mana_use(12345);
        assert!(unit.is_under_last_mana_use_effect());
        assert_eq!(unit.last_mana_use_spell, 12345);
        unit.update_last_mana_use(4999);
        assert!(unit.is_under_last_mana_use_effect());
        assert_eq!(unit.last_mana_use_timer_ms, 1);
        unit.update_last_mana_use(1);
        assert!(!unit.is_under_last_mana_use_effect());
        assert_eq!(unit.last_mana_use_spell, 0);
        // A diff overshooting the remainder also clears.
        unit.set_last_mana_use(7);
        unit.update_last_mana_use(6000);
        assert_eq!(unit.last_mana_use_timer_ms, 0);
    }
}
