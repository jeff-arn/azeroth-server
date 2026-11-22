//! Game-event runtime on a map (Tier 4 B0-8): the
//! map-side half of `GameEventMgr`
//! (`GameEventSpawn`/`GameEventUnspawn`,
//! `UpdateCreatureData`,
//! `Creature::ApplyGameEventSpells`).
//!
//! The world actor owns the schedule
//! ([`mangos_world_data::game_events::next_check_delay`])
//! and pushes the active-event set into every map via
//! [`Map::set_active_game_events`]; the map keeps its
//! live spawn population consistent with it:
//!
//! - `spawn_creature`/`spawn_gameobject` refuse a
//!   spawn whose signed `game_event` is positive
//!   while the event is inactive, or negative while
//!   the event is active (fork `GameEventSpawn` /
//!   `GameEventUnspawn` per-cell materialization;
//!   only loaded cells exist here, so transitions
//!   re-evaluate loaded cells). This also covers
//!   respawns, which re-run `spawn_creature`.
//! - A transition despawns/materializes the signed
//!   spawn lists (only guids whose cell is loaded
//!   or that are currently live).
//! - `game_event_creature_data` re-templating
//!   (entry/display/equipment/spells) applies at
//!   spawn and on toggle, fork
//!   `Creature::UpdateEntry(entry, eventData)` /
//!   `ApplyGameEventSpells` semantics within the
//!   existing [`Map::creature_update_entry`] subset
//!   (stats re-init of a full `UpdateEntry` is
//!   unmodeled there).
//!
//! UNMODELED: pools (`GameEventMgr` pool spawn/
//! unspawn) — the pool manager does not exist in
//! this port at all (pre-existing gap; pool spawns
//! are excluded from the grid at load). Hardcoded
//! `WorldEvent` content is actor-side.

use std::collections::HashSet;

use mangos_world_data::game_events::GameEventCreatureData;

use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::spawn_support::RespawnTimes;

use crate::map::Map;
use crate::spells::{CastOptions, CastTarget};

/// `SPELL_EFFECT_APPLY_AURA`.
const SPELL_EFFECT_APPLY_AURA: u32 = 6;

impl Map {
    /// Whether a spawn row carrying this signed
    /// event id may exist right now.
    pub(crate) fn event_spawn_allowed(&self, signed_event: i16) -> bool {
        if signed_event == 0 {
            return true;
        }
        let active = self
            .active_game_events
            .contains(&signed_event.unsigned_abs());
        active == (signed_event > 0)
    }

    /// Push the actor's active-event set; apply the
    /// delta to live spawns and creature data
    /// (fork `ApplyNewEvent`/`UnApplyEvent` map
    /// halves).
    pub fn set_active_game_events(&mut self, active: HashSet<u16>, respawns: &RespawnTimes) {
        let started: Vec<u16> = active
            .difference(&self.active_game_events)
            .copied()
            .collect();
        let ended: Vec<u16> = self
            .active_game_events
            .difference(&active)
            .copied()
            .collect();
        self.active_game_events = active;
        for event in started {
            self.game_event_spawn_set(event, true, respawns);
            self.game_event_creature_data_toggle(event, true);
        }
        for event in ended {
            self.game_event_spawn_set(event, false, respawns);
            self.game_event_creature_data_toggle(event, false);
        }
    }

    /// Fork `GameEventSpawn(id)`/`GameEventUnspawn(id)`
    /// folded with the negative mirror: when an event
    /// starts, spawn its +id rows and unspawn its -id
    /// rows; when it ends, vice versa.
    fn game_event_spawn_set(&mut self, event: u16, started: bool, respawns: &RespawnTimes) {
        let signed = i16::try_from(event).unwrap_or(0);
        let (spawn_on, despawn_on) = if started {
            (signed, -signed)
        } else {
            (-signed, signed)
        };
        let creature_spawn: Vec<u32> = self
            .stores
            .game_events
            .creature_guids(spawn_on)
            .iter()
            .copied()
            .filter(|guid| {
                // Only loaded cells materialize; the
                // spawn gate covers cells loaded
                // later.
                self.spawn_cell_loaded_creature(*guid)
            })
            .collect();
        let creature_despawn: Vec<u32> =
            self.stores.game_events.creature_guids(despawn_on).to_vec();
        let go_spawn: Vec<u32> = self
            .stores
            .game_events
            .gameobject_guids(spawn_on)
            .iter()
            .copied()
            .filter(|guid| self.spawn_cell_loaded_gameobject(*guid))
            .collect();
        let go_despawn: Vec<u32> = self
            .stores
            .game_events
            .gameobject_guids(despawn_on)
            .to_vec();
        for guid_low in creature_despawn {
            if let Some(guid) = self.live_db_creature(guid_low) {
                if self.creatures.contains_key(&guid) {
                    self.despawn_creature_now(guid, 0);
                }
            }
        }
        for guid_low in go_despawn {
            if let Some(guid) = self.live_db_gameobject(guid_low) {
                if self.gameobjects.contains_key(&guid) {
                    self.despawn_gameobject_now(guid);
                }
            }
        }
        for guid_low in creature_spawn {
            if self.live_db_creature(guid_low).is_none() {
                self.spawn_creature(guid_low, respawns);
            }
        }
        for guid_low in go_spawn {
            if self.live_db_gameobject(guid_low).is_none() {
                self.spawn_gameobject(guid_low, respawns);
            }
        }
    }

    /// Live creature whose DB spawn is `guid_low`
    /// (guid counter == spawn guid).
    fn live_db_creature(&self, guid_low: u32) -> Option<ObjectGuid> {
        self.creatures
            .keys()
            .copied()
            .find(|g| g.counter() == guid_low)
    }

    fn live_db_gameobject(&self, guid_low: u32) -> Option<ObjectGuid> {
        self.gameobjects
            .keys()
            .copied()
            .find(|g| g.counter() == guid_low)
    }

    fn spawn_cell_loaded_creature(&self, guid_low: u32) -> bool {
        self.stores.spawns.creature(guid_low).is_some_and(|data| {
            u32::from(data.position.map_id) == self.map_id
                && self
                    .spawn_cells_loaded
                    .contains(&mangos_world_data::spawns::cell_id(
                        data.position.x,
                        data.position.y,
                    ))
        })
    }

    fn spawn_cell_loaded_gameobject(&self, guid_low: u32) -> bool {
        self.stores.spawns.gameobject(guid_low).is_some_and(|data| {
            u32::from(data.position.map_id) == self.map_id
                && self
                    .spawn_cells_loaded
                    .contains(&mangos_world_data::spawns::cell_id(
                        data.position.x,
                        data.position.y,
                    ))
        })
    }

    /// Fork `UpdateCreatureData(event, activate)`.
    fn game_event_creature_data_toggle(&mut self, event: u16, activate: bool) {
        let rows: Vec<(u32, GameEventCreatureData)> = self
            .stores
            .game_events
            .creature_data_for(event)
            .iter()
            .map(|(g, d)| (*g, *d))
            .collect();
        for (guid_low, data) in rows {
            let Some(guid) = self.live_db_creature(guid_low) else {
                continue;
            };
            if !self.creatures.contains_key(&guid) {
                continue;
            }
            if activate {
                self.game_event_apply_creature_data(guid, &data);
            } else {
                self.game_event_revert_creature_data(guid, &data);
            }
        }
    }

    /// `UpdateEntry(original, data)` subset: entry
    /// swap when the row names one, then display /
    /// equipment overrides, then event spells.
    pub(crate) fn game_event_apply_creature_data(
        &mut self,
        guid: ObjectGuid,
        data: &GameEventCreatureData,
    ) {
        if data.entry != 0 && self.creatures.get(&guid).map(|c| c.info.entry) != Some(data.entry) {
            self.creature_update_entry(guid, data.entry);
        }
        if data.display != 0 {
            self.creature_set_display(guid, data.display);
        }
        if data.equipment >= 0 {
            self.creature_load_equipment(guid, data.equipment as u32);
        }
        self.game_event_apply_spells(guid, data, true);
    }

    /// `UpdateEntry(original, nullptr)` + the
    /// worker's explicit `ApplyGameEventSpells(
    /// data, false)`.
    fn game_event_revert_creature_data(&mut self, guid: ObjectGuid, data: &GameEventCreatureData) {
        if data.entry != 0 {
            if let Some(original) = self.creatures.get(&guid).map(|c| c.original_entry) {
                if self.creatures.get(&guid).map(|c| c.info.entry) != Some(original) {
                    self.creature_update_entry(guid, original);
                }
            }
        }
        if data.display != 0 {
            // Restore the (original) template display
            // (fork re-chooses; the port's template
            // choice is display_id[0], as in
            // creature_update_entry).
            if let Some(original) = self.creatures.get(&guid).map(|c| c.original_entry) {
                if let Some(template) = self.stores.creature_templates.get(original) {
                    self.creature_set_display(guid, template.display_id[0]);
                }
            }
        }
        if data.equipment >= 0 {
            if let Some(entry) = self.creatures.get(&guid).map(|c| c.original_entry) {
                if let Some(template) = self.stores.creature_templates.get(entry) {
                    self.creature_load_equipment(guid, template.equipment_id);
                }
            }
        }
        self.game_event_apply_spells(guid, data, false);
    }

    /// Fork `Creature::ApplyGameEventSpells(data,
    /// apply)`: remove the opposite spell's aura (if
    /// it applies an aura) and triggered-cast the
    /// direction's spell on self.
    fn game_event_apply_spells(
        &mut self,
        guid: ObjectGuid,
        data: &GameEventCreatureData,
        apply: bool,
    ) {
        let (remove, cast) = if apply {
            (data.spell_end, data.spell_start)
        } else {
            (data.spell_start, data.spell_end)
        };
        if remove != 0 && self.spell_applies_aura(remove) {
            let at: Vec<usize> = self
                .unit_base(guid)
                .map(|u| {
                    u.auras
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.spell_id == remove)
                        .map(|(i, _)| i)
                        .collect()
                })
                .unwrap_or_default();
            for index in at.into_iter().rev() {
                self.remove_aura_at(guid, index);
            }
        }
        if cast != 0 {
            let options = CastOptions {
                triggered: true,
                ..CastOptions::default()
            };
            self.cast_spell(guid, cast, CastTarget::Unit(guid), options);
        }
    }

    fn spell_applies_aura(&self, spell_id: u32) -> bool {
        self.stores.spells.get(spell_id).is_some_and(|spell| {
            spell
                .effects
                .iter()
                .any(|e| e.effect == SPELL_EFFECT_APPLY_AURA)
        })
    }

    /// Event creature-data row applying to a live
    /// spawn right now (first active event that has
    /// one; fork `GetGameEventDataForCreature`).
    pub(crate) fn active_creature_event_data(
        &self,
        guid_low: u32,
    ) -> Option<GameEventCreatureData> {
        for event in &self.active_game_events {
            if let Some(data) = self
                .stores
                .game_events
                .creature_data_entry(*event, guid_low)
            {
                return Some(*data);
            }
        }
        None
    }

    /// Fork `SetDisplayId` (values write, watcher
    /// diff picked up by the visibility pass).
    fn creature_set_display(&mut self, guid: ObjectGuid, display_id: u32) {
        if let Some(u) = self.unit_base_mut(guid) {
            u.values()
                .set_u32(offsets::UNIT_FIELD_DISPLAYID, display_id);
            u.values()
                .set_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID, display_id);
        }
    }
}
