//! Per-spell hardcoded effect customs:
//! `Spell::EffectDummy` and
//! `Spell::EffectScriptEffect`
//! (`SpellEffects.cpp:304-1451` / `3578-4467`),
//! ported by reachable-corpus tranches (see
//! `docs/rust-migration-evidence/TIER2-PROGRESS.md`
//! Slice 5). Unhandled spell ids keep the fork's
//! fallthrough behavior: `EffectDummy` would
//! delegate to content scripts
//! (`sScriptMgr.OnEffectDummy`, Tier 3) and
//! `EffectScriptEffect` to the DB `spell_scripts`
//! system (not executed), so the observable
//! behavior here is a no-op, as in the fork when
//! no script is registered. Fork `CastSpell`
//! calls made with `triggered=false` from these
//! arms ride the triggered cast path here; the
//! distinction is recorded per tranche.
//!
//! Fork caster nuance: many arms cast through
//! `m_originalCaster` rather than `m_caster`.
//! For the unit casts that reach this code the
//! two coincide; GO-caster identity is recorded
//! as unmodeled.

use mangos_entities::unit::{DeathState, MOVE_RUN};
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_spells::SpellEntry;
use mangos_spells::defines::aura as aura_type;
use mangos_world_data::spawns::WorldLocation;

use crate::Map;
use crate::spells::{CastItemCtx, CastTarget};

/// `IMMUNE_TO_ROOT_AND_SNARE_MASK`
/// (`SpellDefines.h:662`).
const ROOT_AND_SNARE_MASK: i32 = (1 << 6) | (1 << 10);
/// `MECHANIC_NOT_REMOVED_BY_SHAPESHIFT`
/// (`SpellDefines.h:669`).
const NOT_REMOVED_BY_SHAPESHIFT: i32 = (1 << 0)
    | (1 << 1)
    | (1 << 4)
    | (1 << 7)
    | (1 << 11)
    | (1 << 12)
    | (1 << 17)
    | (1 << 19)
    | (1 << 22)
    | (1 << 23)
    | (1 << 26)
    | (1 << 29);

impl Map {
    fn urand(&mut self, lo: u32, hi: u32) -> u32 {
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        {
            self.spell_rng().irand(lo as i32, hi as i32) as u32
        }
    }

    fn pick_spell(&mut self, spells: &[u32]) -> u32 {
        #[allow(clippy::cast_possible_truncation)]
        {
            spells[self.urand(0, spells.len() as u32 - 1) as usize]
        }
    }

    fn unit_is_male(&self, guid: ObjectGuid) -> bool {
        self.unit_base(guid).is_none_or(|u| u.gender() == 0)
    }

    pub(crate) fn unit_has_aura(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.aura_holders(guid)
            .iter()
            .any(|h| h.spell_id == spell_id)
    }

    /// `GetShapeshiftForm() != FORM_NONE` via
    /// shapeshift aura presence (no separate
    /// form-byte model).
    fn unit_shapeshifted(&self, guid: ObjectGuid) -> bool {
        self.aura_holders(guid)
            .iter()
            .flat_map(|h| h.effects.iter().flatten())
            .any(|e| e.aura_type == aura_type::MOD_SHAPESHIFT)
    }

    /// `IsInSameRaidWith`: self or one shared group.
    fn in_same_raid(&self, a: ObjectGuid, b: ObjectGuid) -> bool {
        if a == b {
            return true;
        }
        let group_of = |g: ObjectGuid| self.players.get(&g).and_then(|e| e.player.group_id);
        group_of(a).is_some() && group_of(a) == group_of(b)
    }

    fn health_pct_of(&self, guid: ObjectGuid) -> f32 {
        let max = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
        if max == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH) as f32 / max as f32 * 100.0
        }
    }

    /// `GetVictim()`: the unit's current melee
    /// victim.
    pub(crate) fn current_victim(&self, guid: ObjectGuid) -> Option<ObjectGuid> {
        self.unit_base(guid).and_then(|u| u.melee_victim)
    }

    /// Snapshot of one creature's threat-list
    /// targets (`getThreatList`).
    fn threat_targets(&self, guid: ObjectGuid) -> Vec<ObjectGuid> {
        self.ai.get(&guid).map_or_else(Vec::new, |ai| {
            ai.threat
                .threat_list()
                .iter()
                .map(mangos_threat::HostileRef::target)
                .collect()
        })
    }

    /// `Unit::CombatStop` subset: stop attacking,
    /// optionally interrupt a non-melee cast, drop
    /// combat state. The reverse attacker cleanup
    /// (`RemoveAllAttackers`) has no central
    /// registry here.
    pub(crate) fn combat_stop(&mut self, guid: ObjectGuid, including_cast: bool) {
        if including_cast {
            self.cancel_cast(guid, true);
        }
        self.attack_stop(guid);
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.in_combat = false;
        }
        self.set_in_combat_flag(guid, false);
    }

    /// `Creature::DespawnOrUnsummon`: all three
    /// fork arms (temp summon, pet, forced
    /// despawn) end in the same immediate despawn
    /// here.
    fn despawn_or_unsummon(&mut self, guid: ObjectGuid) {
        self.despawn_creature_now(guid, 0);
    }

    /// `GetCombatDistance`: 3D center distance
    /// minus both combat reaches, floored at 0.
    fn combat_distance(&self, a: ObjectGuid, b: ObjectGuid) -> f32 {
        let (Some(pa), Some(pb)) = (self.unit_position(a), self.unit_position(b)) else {
            return 0.0;
        };
        let dist = ((pa.x - pb.x).powi(2) + (pa.y - pb.y).powi(2) + (pa.z - pb.z).powi(2)).sqrt();
        (dist - self.combat_reach_of(a) - self.combat_reach_of(b)).max(0.0)
    }

    /// `Creature::UpdateEntry` field subset,
    /// mirroring the generic-script `UPDATE_ENTRY`
    /// arm: swap template info and the
    /// entry/display/faction fields, rebuild AI.
    pub(crate) fn creature_update_entry(&mut self, guid: ObjectGuid, new_entry: u32) {
        let Some(new_info) = self.stores.creature_templates.get(new_entry).cloned() else {
            return;
        };
        let display = new_info.display_id[0];
        let faction = new_info.faction;
        if let Some(c) = self.creatures.get_mut(&guid) {
            if c.info.entry == new_entry {
                return;
            }
            c.info = new_info;
        } else {
            return;
        }
        if let Some(u) = self.unit_base_mut(guid) {
            u.values().set_u32(offsets::OBJECT_FIELD_ENTRY, new_entry);
            u.values().set_u32(offsets::UNIT_FIELD_DISPLAYID, display);
            u.values()
                .set_u32(offsets::UNIT_FIELD_NATIVEDISPLAYID, display);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FACTIONTEMPLATE, faction);
        }
        self.ai.remove(&guid);
    }
}

mod effect_dummy;
mod effect_script_effect;

impl Map {
    /// `Player::RemoveSomeCooldown` (`Player.h:1411`).
    pub(crate) fn remove_some_cooldowns(
        &mut self,
        caster: ObjectGuid,
        check: impl Fn(&SpellEntry) -> bool,
    ) {
        let mut erased_keys: Vec<u32> = Vec::new();
        let mut erased_spells: Vec<u32> = Vec::new();
        for ((guid, key), entry) in &self.cooldowns {
            if *guid != caster || entry.spell_id == 0 {
                continue;
            }
            if erased_spells.contains(&entry.spell_id) {
                erased_keys.push(*key);
                continue;
            }
            let Some(spell) = self.stores.spells.get(entry.spell_id) else {
                continue;
            };
            if check(spell) {
                erased_keys.push(*key);
                erased_spells.push(entry.spell_id);
            }
        }
        for key in erased_keys {
            self.cooldowns.remove(&(caster, key));
        }
        for spell_id in erased_spells {
            let mut payload = PacketWriter::new();
            payload.write_u32_le(spell_id);
            payload.write_u64_le(caster.raw());
            let packet = WorldPacket::with_body(op::SMSG_CLEAR_COOLDOWN, payload.into_vec());
            self.send_to_player(caster, packet);
        }
    }
}
