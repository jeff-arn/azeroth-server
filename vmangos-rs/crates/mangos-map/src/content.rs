//! Hand-ported C++ content scripts (Slice 7, owner
//! decision 2026-10-19): the Deadmines vertical slice
//! — `instance_deadmines`, the gunpowder/cannon/lever
//! gameobjects from `deadmines.cpp`, and
//! `boss_mr_smite`. Everything else in Deadmines is
//! DB-driven (EventAI/spell lists) in this fork and
//! already runs on the Slice 6 engines.
//!
//! Recorded deviations: instance objects are resolved
//! lazily by entry instead of create-hook GUID caches;
//! the Darkmoon chest area trigger has no backing
//! system (the chest stays hidden); Mr. Smite's
//! random-attack-point reposition quirk is left to
//! the chase generator; spell effect subsets from
//! Slice 5 apply to his casts (stomp stun/AoE are
//! inert there).

use mangos_entities::creature::{SpawnRng, VirtualItemPrototype};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::update_field_table::offsets;

use crate::map::Map;
use crate::spells::{CastOptions, CastOutcome, CastTarget};

// deadmines.h
const TYPE_DEFIAS_ENDDOOR: u32 = 1;
const DATA_DEFIAS_DOOR: u32 = 2;
const INST_SAY_ALARM1: i32 = 1148;
const INST_SAY_ALARM2: i32 = 1149;
const GO_DOOR1: u32 = 13965;
const GO_DOOR2: u32 = 16400;
const GO_DOOR3: u32 = 16399;
const GUN_POWDER_EVENT: u32 = 5000;
const GO_IRON_CLAD: u32 = 16397;
const GO_DMF_CHEST: u32 = 180_024;
const NPC_MR_SMITE: u32 = 646;
const NPC_SNEED: u32 = 643;
const NPC_RHAHKZOR: u32 = 644;
const NPC_GILDNID: u32 = 1763;
// EncounterState
const IN_PROGRESS: u32 = 1;
const DONE: u32 = 3;
/// `GO_STATE_ACTIVE` (open).
const GO_STATE_ACTIVE: u32 = 0;
/// `GO_STATE_READY` (closed).
const GO_STATE_READY: u32 = 1;
/// `TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT`.
const TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT: u32 = 4;

// boss_mr_smite.cpp
const SAY_PHASE_2: i32 = 1344;
const SAY_PHASE_3: i32 = 1345;
const EQUIP_ID_AXE: u32 = 2183;
const EQUIP_ID_HAMMER: u32 = 10756;
const SPELL_NIBLE_REFLEXES: u32 = 6433;
const SPELL_SMITE_SLAM: u32 = 6435;
const SPELL_SMITE_STOMP: u32 = 6432;
const SPELL_SMITE_HAMMER: u32 = 6436;
const SPELL_THRASH: u32 = 3391;
const GO_SMITE_CHEST: u32 = 144_111;
/// `UNIT_STAND_STATE_STAND`.
const STAND_STATE_STAND: u8 = 0;
/// `UNIT_STAND_STATE_KNEEL`.
const STAND_STATE_KNEEL: u8 = 8;
/// `SHEATH_STATE_UNARMED`.
const SHEATH_UNARMED: u8 = 0;
/// `SHEATH_STATE_MELEE`.
const SHEATH_MELEE: u8 = 1;

/// Mr. Smite phase machine (`m_uiPhase` family).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmitePhase {
    Phase1,
    Phase2,
    Phase3,
    EquipNull,
    EquipStart,
    EquipProcess,
    EquipEnd,
}

/// `boss_mr_smiteAI` state.
pub(crate) struct SmiteState {
    pub phase: SmitePhase,
    pub equip_timer_ms: u32,
    pub slam_timer_ms: u32,
    pub thrash_timer_ms: u32,
    pub equiping: bool,
    pub in_spline: bool,
}

impl SmiteState {
    pub(crate) fn new() -> Self {
        Self {
            phase: SmitePhase::Phase1,
            equip_timer_ms: 0,
            slam_timer_ms: 9000,
            thrash_timer_ms: 4000,
            equiping: false,
            in_spline: false,
        }
    }
}

/// `instance_deadmines` state.
pub(crate) struct DeadminesState {
    encounter_enddoor: u32,
    gunpowder_done: u32,
    rhahk_dead: bool,
    rhahk_patrol_ms: u32,
    rhahk_death_pos: Position,
    gilnid_dead: bool,
    gilnid_patrol_ms: u32,
    gilnid_death_pos: Position,
    iron_door_timer_ms: u32,
    iron_door_step: u32,
}

impl DeadminesState {
    pub(crate) fn new() -> Self {
        Self {
            encounter_enddoor: 0,
            gunpowder_done: 0,
            rhahk_dead: false,
            rhahk_patrol_ms: 30_000,
            rhahk_death_pos: Position::default(),
            gilnid_dead: false,
            gilnid_patrol_ms: 30_000,
            gilnid_death_pos: Position::default(),
            iron_door_timer_ms: 0,
            iron_door_step: 0,
        }
    }
}

impl Map {
    // ------------------------------------------------ shared helpers

    /// `LoadEquipment(equip_id, true)` over the stores.
    pub(crate) fn creature_load_equipment(&mut self, guid: ObjectGuid, equip_id: u32) {
        let items = |id: u32| {
            self.stores.items.get(id).map(|p| VirtualItemPrototype {
                display_id: p.display_id,
                class: p.class,
                subclass: p.subclass,
                material: p.material,
                inventory_type: p.inventory_type,
                sheath: p.sheath,
            })
        };
        // Split borrow: rng is a separate field.
        let equipment = &self.stores.equipment;
        if let Some(c) = self.creatures.get_mut(&guid) {
            c.load_equipment(equip_id, equipment, &items, &mut self.rng);
        }
    }

    fn item_proto(&self, item_id: u32) -> Option<VirtualItemPrototype> {
        self.stores
            .items
            .get(item_id)
            .map(|p| VirtualItemPrototype {
                display_id: p.display_id,
                class: p.class,
                subclass: p.subclass,
                material: p.material,
                inventory_type: p.inventory_type,
                sheath: p.sheath,
            })
    }

    /// `SetVirtualItem(slot, itemId)` (0 clears).
    pub(crate) fn set_virtual_item_by_id(&mut self, guid: ObjectGuid, slot: u16, item_id: u32) {
        let proto = if item_id == 0 {
            None
        } else {
            self.item_proto(item_id)
        };
        if let Some(c) = self.creatures.get_mut(&guid) {
            c.apply_virtual_item(slot, proto);
        }
    }

    /// Find a live map gameobject by entry (optionally
    /// with an x-position window, as the instance
    /// script's door disambiguation does).
    fn find_gameobject(&self, entry: u32, x_window: Option<(f32, f32)>) -> Option<ObjectGuid> {
        self.gameobjects
            .iter()
            .find(|(_, go)| {
                go.world
                    .object
                    .values()
                    .get_u32(offsets::OBJECT_FIELD_ENTRY)
                    == entry
                    && x_window.is_none_or(|(lo, hi)| {
                        let x = go.world.position().x;
                        x > lo && x < hi
                    })
            })
            .map(|(g, _)| *g)
    }

    fn find_creature_by_entry(&self, entry: u32) -> Option<ObjectGuid> {
        self.creatures
            .values()
            .find(|c| c.info.entry == entry)
            .map(|c| c.unit.guid())
    }

    /// `DoUseDoorOrButton` (open when closed).
    fn use_door(&mut self, go: ObjectGuid) {
        if self.object_get_field(go, offsets::GAMEOBJECT_STATE) != GO_STATE_ACTIVE {
            let _ =
                self.object_set_field(go, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_ACTIVE);
        }
    }

    fn go_state(&self, go: ObjectGuid) -> u32 {
        self.object_get_field(go, offsets::GAMEOBJECT_STATE)
    }

    // ------------------------------------------------ instance: deadmines

    /// `instance_deadmines::OnCreatureCreate`.
    pub(crate) fn instance_on_creature_create(&mut self, guid: ObjectGuid) {
        if self.deadmines.is_none() {
            return;
        }
        let delay = self
            .creatures
            .get(&guid)
            .and_then(|c| c.data.as_ref())
            .map_or(0, |d| d.spawntimesecsmin);
        if delay == 43199 || delay == 43201 {
            self.hidden_objects.insert(guid);
            let _ = self.object_set_field(guid, u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE), 35);
        }
    }

    /// `instance_deadmines::OnObjectCreate`.
    pub(crate) fn instance_on_object_create(&mut self, guid: ObjectGuid) {
        if self.deadmines.is_none() {
            return;
        }
        if self.object_entry(guid) == Some(GO_DMF_CHEST) {
            self.hidden_objects.insert(guid);
        }
    }

    /// `instance_deadmines::OnCreatureDeath`.
    pub(crate) fn instance_on_creature_death(&mut self, victim: ObjectGuid) {
        if self.deadmines.is_none() {
            return;
        }
        let (entry, pos) = match self.creatures.get(&victim) {
            Some(c) => (c.info.entry, c.unit.world.position()),
            None => return,
        };
        let door = match entry {
            NPC_RHAHKZOR => {
                if let Some(dm) = &mut self.deadmines {
                    dm.rhahk_dead = true;
                    dm.rhahk_patrol_ms = 60_000;
                    dm.rhahk_death_pos = pos;
                }
                self.find_gameobject(GO_DOOR1, None)
            }
            NPC_SNEED => self.find_gameobject(GO_DOOR2, Some((-291.0, -290.0))),
            NPC_GILDNID => {
                if let Some(dm) = &mut self.deadmines {
                    dm.gilnid_dead = true;
                    dm.gilnid_patrol_ms = 30_000;
                    dm.gilnid_death_pos = pos;
                }
                self.find_gameobject(GO_DOOR3, Some((-169.0, -168.0)))
            }
            _ => None,
        };
        if let Some(door) = door
            && self.go_state(door) != GO_STATE_ACTIVE
        {
            self.use_door(door);
        }
    }

    /// `instance_deadmines::SetData` reactions. Returns
    /// true when the instance script owns the field.
    pub(crate) fn instance_set_data(&mut self, field: u32, data: u32) -> bool {
        if self.deadmines.is_none() {
            return false;
        }
        match field {
            TYPE_DEFIAS_ENDDOOR => {
                if data == IN_PROGRESS
                    && let Some(iron) = self.find_gameobject(GO_IRON_CLAD, None)
                {
                    if self.go_state(iron) != GO_STATE_ACTIVE {
                        self.use_door(iron);
                    }
                    if let Some(dm) = &mut self.deadmines {
                        dm.iron_door_timer_ms = 3000;
                    }
                }
                if let Some(dm) = &mut self.deadmines {
                    dm.encounter_enddoor = data;
                }
                true
            }
            GUN_POWDER_EVENT => {
                if let Some(dm) = &mut self.deadmines {
                    dm.gunpowder_done = data;
                }
                true
            }
            _ => false,
        }
    }

    /// `instance_deadmines::GetData`.
    pub(crate) fn instance_get_data(&self, field: u32) -> Option<u32> {
        let dm = self.deadmines.as_ref()?;
        match field {
            TYPE_DEFIAS_ENDDOOR => Some(dm.encounter_enddoor),
            GUN_POWDER_EVENT => Some(dm.gunpowder_done),
            _ => None,
        }
    }

    /// `instance_deadmines::GetData64`.
    pub(crate) fn instance_get_data64(&self, field: u32) -> Option<ObjectGuid> {
        if self.deadmines.is_none() || field != DATA_DEFIAS_DOOR {
            return None;
        }
        self.find_gameobject(GO_IRON_CLAD, None)
    }

    /// `instance_deadmines::Update`.
    pub(crate) fn instance_update(&mut self, diff_ms: u32) {
        let Some(dm) = &mut self.deadmines else {
            return;
        };
        let rhahk_fire = dm.rhahk_dead && dm.rhahk_patrol_ms > 0 && {
            if dm.rhahk_patrol_ms <= diff_ms {
                dm.rhahk_patrol_ms = 0;
                true
            } else {
                dm.rhahk_patrol_ms -= diff_ms;
                false
            }
        };
        let gilnid_fire = dm.gilnid_dead && dm.gilnid_patrol_ms > 0 && {
            if dm.gilnid_patrol_ms <= diff_ms {
                dm.gilnid_patrol_ms = 0;
                true
            } else {
                dm.gilnid_patrol_ms -= diff_ms;
                false
            }
        };
        let iron_fire = dm.iron_door_timer_ms > 0 && {
            if dm.iron_door_timer_ms <= diff_ms {
                true
            } else {
                dm.iron_door_timer_ms -= diff_ms;
                false
            }
        };
        let rhahk_pos = dm.rhahk_death_pos;
        let gilnid_pos = dm.gilnid_death_pos;
        if rhahk_fire {
            self.activate_patrol(rhahk_pos, &[634, 1729], 43199);
        }
        if gilnid_fire {
            self.activate_patrol(gilnid_pos, &[4417, 4418], 43201);
        }
        if iron_fire {
            self.iron_door_step();
        }
    }

    fn activate_patrol(&mut self, center: Position, entries: &[u32], delay_marker: u32) {
        let guids: Vec<ObjectGuid> = self
            .creatures
            .values()
            .filter(|c| {
                entries.contains(&c.info.entry)
                    && c.data.as_ref().map_or(0, |d| d.spawntimesecsmin) == delay_marker
                    && crate::visibility::dist2d(center, c.unit.world.position()) <= 400.0
            })
            .map(|c| c.unit.guid())
            .collect();
        for guid in guids {
            self.hidden_objects.remove(&guid);
            let _ = self.object_set_field(guid, u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE), 17);
        }
    }

    fn iron_door_step(&mut self) {
        let step = self.deadmines.as_ref().map_or(0, |dm| dm.iron_door_step);
        let Some(smite) = self.find_creature_by_entry(NPC_MR_SMITE) else {
            if let Some(dm) = &mut self.deadmines {
                dm.iron_door_timer_ms = 0;
            }
            return;
        };
        if step == 0 {
            self.do_script_text(INST_SAY_ALARM1, smite, None, -1);
            let pirates: Vec<ObjectGuid> = self
                .creatures
                .values()
                .filter(|c| {
                    c.info.entry == 657
                        && c.data.as_ref().map_or(0, |d| d.spawntimesecsmin) == 43202
                })
                .map(|c| c.unit.guid())
                .collect();
            let dest = Position::new(-99.6611, -671.071_66, 7.42241, 0.0);
            for pirate in pirates {
                self.creature_move_point(pirate, 0, dest, 0, 0.0, -10.0);
            }
            if let Some(dm) = &mut self.deadmines {
                dm.iron_door_step = 1;
                dm.iron_door_timer_ms = 15_000;
            }
        } else {
            self.do_script_text(INST_SAY_ALARM2, smite, None, -1);
            if let Some(dm) = &mut self.deadmines {
                dm.iron_door_step = 0;
                dm.iron_door_timer_ms = 0;
            }
        }
    }

    // ------------------------------------------------ gameobject use

    /// `GameObject::Use` subset + the three Deadmines
    /// `GOHello` scripts. Called from
    /// `CMSG_GAMEOBJ_USE`.
    pub fn gameobject_use(&mut self, player: ObjectGuid, go_guid: ObjectGuid) {
        let Some(go) = self.gameobjects.get(&go_guid) else {
            return;
        };
        let script_name = go.info.script_name.clone();
        let entry = go
            .world
            .object
            .values()
            .get_u32(offsets::OBJECT_FIELD_ENTRY);
        let go_pos = go.world.position();
        let Some(player_pos) = self.unit_position(player) else {
            return;
        };
        if crate::visibility::dist2d(player_pos, go_pos) > 10.0 {
            return;
        }
        match script_name.as_str() {
            "go_defias_gunpowder" => {
                if self.instance_get_data(GUN_POWDER_EVENT).unwrap_or(0) == 0 {
                    let spawn = Position::new(-131.290_83, -591.2431, 18.077_19, 4.792_192);
                    if let Some(pirate) = self.summon_creature(
                        634,
                        spawn,
                        go_guid,
                        TEMPSUMMON_TIMED_DESPAWN_OUT_OF_COMBAT,
                        310_000,
                    ) {
                        let dest = Position::new(-128.925_98, -616.494_6, 13.532_34, 6.269_623);
                        self.creature_move_point(pirate, 0, dest, 0, 0.0, 6.269_623);
                    }
                    self.instance_set_data(GUN_POWDER_EVENT, 1);
                }
            }
            "go_defias_cannon" => {
                let state = self.instance_get_data(TYPE_DEFIAS_ENDDOOR).unwrap_or(0);
                if state != DONE && state != IN_PROGRESS {
                    self.instance_set_data(TYPE_DEFIAS_ENDDOOR, IN_PROGRESS);
                }
            }
            "go_door_lever_dm" => {
                // GOHello blocks the default use while
                // the door is open; otherwise the button
                // use opens the iron door (subset).
                if let Some(iron) = self.instance_get_data64(DATA_DEFIAS_DOOR)
                    && self.go_state(iron) == GO_STATE_READY
                {
                    self.use_door(iron);
                }
            }
            _ => {
                // Default door/button toggle subset.
                let go_type = self
                    .gameobjects
                    .get(&go_guid)
                    .map_or(u32::MAX, |g| g.info.go_type);
                if go_type == 0 || go_type == 1 {
                    let state = self.go_state(go_guid);
                    let next = if state == GO_STATE_READY {
                        GO_STATE_ACTIVE
                    } else {
                        GO_STATE_READY
                    };
                    let _ =
                        self.object_set_field(go_guid, u32::from(offsets::GAMEOBJECT_STATE), next);
                }
                let _ = entry;
            }
        }
    }

    /// `go_defias_gunpowderAI::SummonedMovementInform`:
    /// the summoned overseer's two-leg walk.
    pub(crate) fn gunpowder_summon_arrived(&mut self, summon: ObjectGuid, point_id: u32) {
        match point_id {
            0 => {
                let dest = Position::new(-115.263_67, -617.396_1, 13.579_387, 6.182_347);
                self.creature_move_point(summon, 1, dest, 0, 0.0, 6.182_347);
            }
            1 => {
                if let Some(c) = self.creatures.get_mut(&summon) {
                    c.home_position = Position::new(-115.263_67, -617.396_1, 13.579_387, 6.182_347);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------ Mr. Smite

    /// `boss_mr_smiteAI::Reset`.
    pub(crate) fn smite_reset(&mut self, guid: ObjectGuid) {
        let equip_id = self.creatures.get(&guid).map_or(0, |c| c.info.equipment_id);
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.smite = Some(SmiteState::new());
        }
        self.creature_load_equipment(guid, equip_id);
    }

    fn smite_cast(&mut self, guid: ObjectGuid, spell_id: u32, target: CastTarget) -> bool {
        self.cast_spell(
            guid,
            spell_id,
            target,
            CastOptions {
                triggered: false,
                skip_known: true,
                cast_item: None,
            },
        ) == CastOutcome::Started
    }

    fn smite_health_percent(&self, guid: ObjectGuid) -> f32 {
        let cur = f64::from(self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH));
        let max = f64::from(self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH));
        if max == 0.0 {
            return 100.0;
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            (cur * 100.0 / max) as f32
        }
    }

    /// The equip-phase branch of `UpdateAI` (also runs
    /// with no victim, as in C++).
    fn smite_equip_update(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let due = {
            let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) else {
                return;
            };
            if smite.equip_timer_ms > 0 {
                if smite.equip_timer_ms > diff_ms {
                    smite.equip_timer_ms -= diff_ms;
                    return;
                }
                smite.equip_timer_ms = 0;
            }
            smite.phase
        };
        match due {
            SmitePhase::EquipStart => self.smite_phase_equip_start(guid),
            SmitePhase::EquipProcess => self.smite_phase_equip_process(guid),
            SmitePhase::EquipEnd => self.smite_phase_equip_end(guid),
            _ => {}
        }
    }

    fn smite_phase_equip_start(&mut self, guid: ObjectGuid) {
        let Some(chest) = self
            .unit_position(guid)
            .and_then(|_| self.find_gameobject(GO_SMITE_CHEST, None))
        else {
            if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
                smite.phase = SmitePhase::EquipProcess;
            }
            return;
        };
        let chest_pos = self.object_position(chest).unwrap_or_default();
        let my_pos = self.unit_position(guid).unwrap_or_default();
        // `GetContactPoint(.., CONTACT_DISTANCE)`.
        let dx = my_pos.x - chest_pos.x;
        let dy = my_pos.y - chest_pos.y;
        let len = (dx * dx + dy * dy).sqrt().max(0.01);
        let contact = Position::new(
            chest_pos.x + dx / len * 1.5,
            chest_pos.y + dy / len * 1.5,
            chest_pos.z,
            0.0,
        );
        self.stop_moving(guid);
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.phase = SmitePhase::EquipNull;
            smite.in_spline = true;
        }
        self.creature_move_point(guid, 0, contact, 0, 0.0, -10.0);
    }

    /// `MovementInform`/`SplineFinished` while running
    /// to the chest.
    pub(crate) fn smite_spline_finished(&mut self, guid: ObjectGuid) {
        let equiping = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.smite.as_ref())
            .is_some_and(|s| s.equiping);
        if !equiping {
            return;
        }
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.in_spline = false;
        }
        self.creature_load_equipment(guid, 0);
        self.unit_set_byte(guid, offsets::UNIT_FIELD_BYTES_2, 0, SHEATH_UNARMED);
        self.unit_set_byte(guid, offsets::UNIT_FIELD_BYTES_1, 0, STAND_STATE_KNEEL);
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.equip_timer_ms = 3000;
            smite.phase = SmitePhase::EquipProcess;
        }
    }

    fn smite_phase_equip_process(&mut self, guid: ObjectGuid) {
        if self.smite_health_percent(guid) < 33.0 {
            self.set_virtual_item_by_id(guid, 0, EQUIP_ID_HAMMER);
            self.set_virtual_item_by_id(guid, 1, 0);
            self.set_virtual_item_by_id(guid, 2, 0);
            self.smite_cast(guid, SPELL_SMITE_HAMMER, CastTarget::SelfCast);
        } else {
            self.set_virtual_item_by_id(guid, 0, EQUIP_ID_AXE);
            self.set_virtual_item_by_id(guid, 1, EQUIP_ID_AXE);
            self.set_virtual_item_by_id(guid, 2, 0);
        }
        self.unit_set_byte(guid, offsets::UNIT_FIELD_BYTES_1, 0, STAND_STATE_STAND);
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.phase = SmitePhase::EquipEnd;
            smite.equip_timer_ms = 1000;
        }
    }

    fn smite_phase_equip_end(&mut self, guid: ObjectGuid) {
        let Some(victim) = self.threat_targets_sorted(guid).first().copied() else {
            self.enter_evade_mode(guid);
            return;
        };
        self.unit_set_byte(guid, offsets::UNIT_FIELD_BYTES_2, 0, SHEATH_MELEE);
        let phase = if self.smite_health_percent(guid) < 33.0 {
            SmitePhase::Phase3
        } else {
            SmitePhase::Phase2
        };
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.phase = phase;
            smite.equiping = false;
        }
        self.attack_start(guid, victim);
        self.creature_move_chase(guid, victim, 0.0, 0.0);
    }

    fn smite_start_equip(&mut self, guid: ObjectGuid, say: i32) {
        self.do_script_text(say, guid, None, -1);
        self.stop_moving(guid);
        self.attack_stop(guid);
        if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
            smite.phase = SmitePhase::EquipStart;
            smite.equip_timer_ms = 2500;
            smite.equiping = true;
        }
    }

    /// `boss_mr_smiteAI::UpdateAI` (called instead of
    /// the generic creature update for script-bound
    /// Smite).
    pub(crate) fn update_smite(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let phase = {
            let Some(smite) = self.ai.get(&guid).and_then(|ai| ai.smite.as_ref()) else {
                return;
            };
            smite.phase
        };
        // Equip phases run with or without a victim
        // (AttackStop cleared it), mirroring C++.
        if matches!(
            phase,
            SmitePhase::EquipNull
                | SmitePhase::EquipStart
                | SmitePhase::EquipProcess
                | SmitePhase::EquipEnd
        ) {
            self.smite_equip_update(guid, diff_ms);
            return;
        }
        let Some(victim) = self.get_hostile_target(guid) else {
            if self.ai_in_combat(guid) {
                self.enter_evade_mode(guid);
            } else {
                // `MoveInLineOfSight` proximity aggro
                // (the ScriptedAI base behavior).
                self.try_proximity_aggro(guid);
            }
            return;
        };
        if self.should_leash_evade(guid) {
            self.enter_evade_mode(guid);
            return;
        }
        let current_victim = self.unit_victim(guid);
        if current_victim != Some(victim) {
            self.attack_start(guid, victim);
        }
        if self.motion_type_of(guid) != crate::motion::CHASE_MOTION_TYPE
            || self.chase_target_of(guid) != Some(victim)
        {
            self.creature_move_chase(guid, victim, 0.0, 0.0);
        }
        let health_pct = self.smite_health_percent(guid);
        match phase {
            SmitePhase::Phase1 => {
                if health_pct < 66.0
                    && self.smite_cast(guid, SPELL_SMITE_STOMP, CastTarget::SelfCast)
                {
                    let say = if health_pct < 33.0 {
                        SAY_PHASE_3
                    } else {
                        SAY_PHASE_2
                    };
                    self.smite_start_equip(guid, say);
                    self.cancel_aura(guid, SPELL_NIBLE_REFLEXES);
                }
            }
            SmitePhase::Phase2 => {
                let due = {
                    let smite = self
                        .ai
                        .get_mut(&guid)
                        .and_then(|ai| ai.smite.as_mut())
                        .expect("smite");
                    if smite.thrash_timer_ms < diff_ms {
                        true
                    } else {
                        smite.thrash_timer_ms -= diff_ms;
                        false
                    }
                };
                if due && self.smite_cast(guid, SPELL_THRASH, CastTarget::Unit(victim)) {
                    let next = self.rng.urand(1500, 4000);
                    if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
                        smite.thrash_timer_ms = next;
                    }
                }
                if health_pct < 33.0
                    && self.smite_cast(guid, SPELL_SMITE_STOMP, CastTarget::SelfCast)
                {
                    self.smite_start_equip(guid, SAY_PHASE_3);
                }
            }
            SmitePhase::Phase3 => {
                let due = {
                    let smite = self
                        .ai
                        .get_mut(&guid)
                        .and_then(|ai| ai.smite.as_mut())
                        .expect("smite");
                    if smite.slam_timer_ms < diff_ms {
                        true
                    } else {
                        smite.slam_timer_ms -= diff_ms;
                        false
                    }
                };
                if due && self.smite_cast(guid, SPELL_SMITE_SLAM, CastTarget::Unit(victim)) {
                    if let Some(smite) = self.ai.get_mut(&guid).and_then(|ai| ai.smite.as_mut()) {
                        smite.slam_timer_ms = 11_000;
                    }
                }
            }
            _ => {}
        }
    }
}
