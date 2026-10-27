//! Dynamic objects runtime (`DynamicObject::Update`,
//! `DynamicObjectUpdater`): lifetime countdown and
//! the throttled affected-units pass that applies
//! persistent area auras to units standing in the
//! area. Removal of out-of-range holders lives in
//! the aura update (`PersistentAreaAura::Update`).

use mangos_entities::dynobject::DynObject;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::world::WorldPacket;
use mangos_spells::SpellEntry;

use crate::map::Map;

/// `DYNAMIC_OBJECTS_UPDATE_INTERVAL`-style cadence:
/// the fork refreshes affected areas on a 2s map
/// timer (`Map::UpdateInRangeObjects` dynobj pass).
const AFFECTED_INTERVAL_MS: u32 = 2000;

impl Map {
    /// `Spell::EffectPersistentAA` + `DynamicObject
    /// ::Create` + map add: spawn a ground area object
    /// at the cast destination. Every cast creates a
    /// new object; single-target replacement in
    /// `AddDynObject` is not modeled (recorded).
    pub(crate) fn spawn_dynobject(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        effect_index: usize,
        dest: Position,
    ) {
        let duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        if duration == 0 {
            return;
        }
        let radius = self.effect_radius(spell, effect_index);
        self.summon_counter += 1;
        let guid_low = self.summon_counter;
        let dynobj = DynObject::create(
            guid_low,
            self.map_id,
            caster,
            spell.id,
            effect_index,
            dest,
            duration,
            radius,
            spell.is_positive_spell(),
        );
        let guid = dynobj.guid();
        let cell = Self::cell_of(dest);
        self.cells.entry(cell).or_default().dynobjects.insert(guid);
        self.dynobjects.insert(guid, dynobj);
        for entry in self.players.values_mut() {
            entry.scan_due = true;
        }
        // The fork applies the area aura immediately
        // on creation (first updater visit happens in
        // the same world tick); run one affected pass
        // now so ground effects land without the 2s
        // throttle delay.
        self.dynobject_affect(guid);
    }

    /// `DynamicObject::Delete`: despawn animation to
    /// viewers of the object, then removal from the
    /// map stores and visibility sets.
    fn delete_dynobject(&mut self, guid: ObjectGuid) {
        let Some(dynobj) = self.dynobjects.remove(&guid) else {
            return;
        };
        let cell = Self::cell_of(dynobj.world.position());
        if let Some(objects) = self.cells.get_mut(&cell) {
            objects.dynobjects.remove(&guid);
        }
        let viewers: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, e)| e.visible.contains(&guid))
            .map(|(g, _)| *g)
            .collect();
        if !viewers.is_empty() {
            // `SendObjectDeSpawnAnim`: guid + 1.
            let mut payload = PacketWriter::new();
            payload.write_u64_le(guid.raw());
            payload.write_u32_le(1);
            let packet =
                WorldPacket::with_body(op::SMSG_GAMEOBJECT_DESPAWN_ANIM, payload.into_vec());
            for viewer in viewers {
                self.send_to_player(viewer, packet.clone());
                if let Some(entry) = self.players.get_mut(&viewer) {
                    entry.visible.remove(&guid);
                }
            }
        }
    }

    /// Per-tick lifetime countdown + affected pass.
    /// Caster death deletes the object (channeled
    /// spells end it through the channel path, which
    /// is also covered by caster-alive here).
    pub(crate) fn update_dynobjects(&mut self, diff_ms: u32) {
        if self.dynobjects.is_empty() {
            return;
        }
        self.dynobj_tick_ms = self.dynobj_tick_ms.saturating_add(diff_ms);
        let do_affected = self.dynobj_tick_ms >= AFFECTED_INTERVAL_MS;
        if do_affected {
            self.dynobj_tick_ms %= AFFECTED_INTERVAL_MS;
        }
        let guids: Vec<ObjectGuid> = self.dynobjects.keys().copied().collect();
        let mut expired: Vec<ObjectGuid> = Vec::new();
        for guid in &guids {
            let Some(dynobj) = self.dynobjects.get_mut(guid) else {
                continue;
            };
            #[allow(clippy::cast_possible_wrap)]
            {
                dynobj.alive_ms -= diff_ms as i32;
            }
            let dead = dynobj.alive_ms <= 0;
            let caster = dynobj.caster;
            if dead || !self.unit_is_alive(caster) {
                expired.push(*guid);
            }
        }
        for guid in expired {
            self.delete_dynobject(guid);
        }
        if do_affected {
            let live: Vec<ObjectGuid> = self.dynobjects.keys().copied().collect();
            for guid in live {
                self.dynobject_affect(guid);
            }
        }
    }

    /// One `DynamicObjectUpdater` visit: every living
    /// unit of the matching hostility within radius
    /// gains (or refreshes) the spell's aura holder,
    /// stamped with this dynobject and a duration set
    /// to the object's remaining lifetime.
    fn dynobject_affect(&mut self, guid: ObjectGuid) {
        let Some(snapshot) = self.dynobjects.get(&guid).map(|d| {
            (
                d.caster,
                d.spell_id,
                d.radius,
                d.alive_ms,
                d.positive,
                d.world.position(),
            )
        }) else {
            return;
        };
        let (caster, spell_id, radius, alive_ms, positive, center) = snapshot;
        let Some(spell) = self.stores.spells.get(spell_id).cloned() else {
            return;
        };
        let victims = self.units_around(caster, center, radius, !positive);
        for victim in victims {
            self.apply_spell_aura(caster, &spell, victim, None);
            let adjusted = self.unit_base_mut(victim).and_then(|unit| {
                unit.auras
                    .iter_mut()
                    .find(|h| h.spell_id == spell_id && h.caster == caster)
                    .map(|h| {
                        h.dynobj = guid;
                        h.duration_ms = alive_ms;
                        h.max_duration_ms = alive_ms;
                        h.slot
                    })
            });
            if let Some(slot) = adjusted {
                self.send_aura_duration(victim, slot, alive_ms);
            }
        }
    }
}
