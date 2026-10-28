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
    /// new object (the fork's single-target
    /// replacement key `SPELL_CUSTOM_SINGLE_TARGET_AURA`
    /// is never set on any spell — fork-dead, see
    /// TIER2-PROGRESS Slice 6.4). The player mod
    /// owner's `SPELLMOD_RADIUS` applies to the
    /// radius, as in the fork handler.
    pub(crate) fn spawn_dynobject(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        effect_index: usize,
        dest: Position,
        applied: Option<&mut Vec<(u32, u8)>>,
    ) {
        let duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        if duration == 0 {
            return;
        }
        let radius = self.apply_spell_mod(
            caster,
            spell,
            mangos_spells::defines::spellmod::RADIUS,
            self.effect_radius(spell, effect_index),
            true,
            applied,
        );
        self.summon_counter += 1;
        let guid_low = self.summon_counter;
        let mut dynobj = DynObject::create(
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
        dynobj.channeled = spell.is_channeled();
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

    /// `Unit::GetChannelObjectGuid` (0 → `None`).
    pub(crate) fn channel_object(&self, caster: ObjectGuid) -> Option<ObjectGuid> {
        use mangos_protocol::update_field_table::offsets;
        let raw = self
            .unit_base(caster)?
            .world
            .object
            .values()
            .get_u64(offsets::UNIT_FIELD_CHANNEL_OBJECT);
        (raw != 0).then(|| ObjectGuid::from_raw(raw))
    }

    /// `SpellCaster::RemoveDynObject(spellId)`: the
    /// channel-end cleanup (`Spell::cancel` and the
    /// channel-finish path both call it).
    pub(crate) fn remove_dynobjects_by_spell(&mut self, caster: ObjectGuid, spell_id: u32) {
        let doomed: Vec<ObjectGuid> = self
            .dynobjects
            .iter()
            .filter(|(_, d)| d.caster == caster && d.spell_id == spell_id)
            .map(|(g, _)| *g)
            .collect();
        for guid in doomed {
            self.delete_dynobject(guid);
        }
    }

    /// Per-tick lifetime countdown + affected pass.
    /// Caster death deletes the object; channeled
    /// spells end theirs at channel end via
    /// `remove_dynobjects_by_spell`.
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
            let (spent, caster, channeled) = {
                let Some(dynobj) = self.dynobjects.get_mut(guid) else {
                    continue;
                };
                #[allow(clippy::cast_possible_wrap)]
                {
                    dynobj.alive_ms -= diff_ms as i32;
                }
                (dynobj.alive_ms <= 0, dynobj.caster, dynobj.channeled)
            };
            // DynamicObject::Update: expiry at
            // alive == 0 unless this object is the
            // caster's current channel object (the
            // Farsight focus case; channel end
            // removes spell dynobjects separately).
            let dead = spent && (!channeled || self.channel_object(caster) != Some(*guid));
            if dead || !self.unit_is_alive(caster) {
                expired.push(*guid);
            }
        }
        // Fork order inside Update: the affected
        // pass runs before the deletion applies.
        if do_affected {
            for guid in &guids {
                self.dynobject_affect(*guid);
            }
        }
        for guid in expired {
            self.delete_dynobject(guid);
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
            self.apply_spell_aura(caster, &spell, victim, None, [None; 3]);
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
