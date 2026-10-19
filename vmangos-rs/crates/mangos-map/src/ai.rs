//! Creature AI (Slice 6a): proximity aggro, the
//! `BasicAI` combat loop over the `mangos-threat`
//! manager, white melee with the C++ outcome table,
//! creature spell-list casting, and evade/home.
//! Slice 6b adds the `CreatureEventAI` runtime (event
//! holders, 500 ms cadence, phases) driving the DB
//! script engine in `scripts.rs`, plus scripted flee.
//!
//! Recorded subset deviations: no assistance calls
//! (`CallAssistance`), no pet/charm handling, hostility
//! is player-vs-creature (no faction reactions), static
//! creature flags beyond critter/totem types are not
//! consulted, spell-list target types outside
//! self/victim/random-threat fall back to the victim,
//! `CF_MAIN_RANGED_SPELL` kiting and spell-list scripts
//! are not modeled (Slice 6b engine), in-combat mana
//! regen is identity (out-of-combat max/3 regen is
//! implemented), melee facing/leeway is reach-only,
//! dual wield does not exist (`BASE_ATTACK` only), the
//! player damage fields are the unarmed bases until
//! item-stat projection lands, and `IsOutOfThreatArea`
//! is existence+alive only.

use mangos_entities::creature::Creature;
use mangos_guid::ObjectGuid;
use mangos_movement::movement_info::Position;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id as op;
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::world::WorldPacket;
use mangos_scripts_engine::{
    EFLAG_CHECK_RESULT, EFLAG_NOT_CASTING, EFLAG_RANDOM_ACTION, EFLAG_REPEATABLE,
    EVENT_UPDATE_TIME, EventAiEvent, event_type,
};
use mangos_threat::{AddDecision, ThreatHost, ThreatManager};
use mangos_world_data::creature_spells::CreatureSpellEntry;

use mangos_entities::creature::SpawnRng;

use crate::map::Map;
use crate::motion::CHASE_MOTION_TYPE;
use crate::spells::{CastOptions, CastOutcome, CastTarget};

/// `CREATURE_FLAG_EXTRA_NO_AGGRO`.
const FLAG_EXTRA_NO_AGGRO: u32 = 0x0000_0002;
/// `CREATURE_FLAG_EXTRA_NO_LEASH_EVADE`.
const FLAG_EXTRA_NO_LEASH_EVADE: u32 = 0x0000_0001;
/// `CREATURE_TYPE_CRITTER`.
const CREATURE_TYPE_CRITTER: u32 = 8;
/// `CREATURE_TYPE_TOTEM`.
const CREATURE_TYPE_TOTEM: u32 = 11;
/// `UNIT_FLAG_IN_COMBAT`.
const UNIT_FLAG_IN_COMBAT: u32 = 0x0008_0000;
/// `CREATURE_Z_ATTACK_RANGE`.
const CREATURE_Z_ATTACK_RANGE: f32 = 3.0;
/// `MAX_LEVEL_DIFF_FOR_AGGRO_RANGE`.
const MAX_LEVEL_DIFF_FOR_AGGRO_RANGE: i32 = 25;
/// `REGEN_TIME_CREATURE_FULL` fallback (regen tick).
const REGEN_TIME_MS: u32 = 1000;
/// Player combat-drop delay after the last damage
/// (`m_combatTimer` approximation).
const PLAYER_COMBAT_DROP_MS: u32 = 5000;

// HitInfo (5875 branch, build > 1_9_4).
const HITINFO_AFFECTS_VICTIM: u32 = 0x0000_0002;
const HITINFO_MISS: u32 = 0x0000_0010;
const HITINFO_CRITICALHIT: u32 = 0x0000_0080;
const HITINFO_GLANCING: u32 = 0x0000_4000;
const HITINFO_CRUSHING: u32 = 0x0000_8000;
const HITINFO_SWINGNOHITSOUND: u32 = 0x0008_0000;
// VictimState.
const VICTIMSTATE_UNAFFECTED: u32 = 0;
const VICTIMSTATE_NORMAL: u32 = 1;
const VICTIMSTATE_DODGE: u32 = 2;
const VICTIMSTATE_PARRY: u32 = 3;
const VICTIMSTATE_BLOCKS: u32 = 5;
const VICTIMSTATE_EVADES: u32 = 6;
/// `AI_REACTION_HOSTILE`.
const AI_REACTION_HOSTILE: u32 = 2;
/// `CF_*` creature-spell cast flags used by the subset.
const CF_INTERRUPT_PREVIOUS: u16 = 0x001;
const CF_TRIGGERED: u16 = 0x002;
const CF_AURA_NOT_PRESENT: u16 = 0x020;
const CF_ONLY_IN_MELEE: u16 = 0x040;
const CF_NOT_IN_MELEE: u16 = 0x080;
const CF_TARGET_CASTING: u16 = 0x100;
/// `CREATURE_CASTING_DELAY` (spell-list refresh cadence).
const CREATURE_CASTING_DELAY: u32 = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiKind {
    /// `NullAI` / passive templates (critters, totems).
    Null,
    /// `BasicAI` core.
    Basic,
    /// `CreatureEventAI`: the Basic core plus DB event
    /// processing (`ai_name == "EventAI"`).
    EventAi,
}

/// One event's live state (`CreatureEventAIHolder`
/// minus the event copy, which sits beside it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EventHolder {
    pub time_ms: u32,
    pub enabled: bool,
}

/// Per-creature `EventAI` runtime
/// (`CreatureEventAI` state).
pub(crate) struct EventAiRuntime {
    pub events: Vec<EventAiEvent>,
    pub holders: Vec<EventHolder>,
    pub event_update_ms: u32,
    pub event_diff_ms: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct SpellRuntime {
    entry: CreatureSpellEntry,
    cooldown_ms: u32,
}

/// Per-creature AI runtime (`CreatureAI` subset).
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct CreatureAi {
    pub kind: AiKind,
    pub threat: ThreatManager,
    pub in_combat: bool,
    pub combat_start: Position,
    pub combat_start_ms: u32,
    pub spells: Vec<SpellRuntime>,
    pub casting_delay_ms: u32,
    pub evading: bool,
    pub regen_timer_ms: u32,
    /// `EventAI` `m_Phase` (0..31).
    pub phase: u8,
    pub eventai: Option<EventAiRuntime>,
    /// `SetMeleeAttack` toggle.
    pub melee_enabled: bool,
    /// `SetCombatMovement` toggle (chase on/off).
    pub combat_movement: bool,
    /// Scripted flee expiry (map ms; 0 = not fleeing).
    pub flee_until_ms: u32,
    pub flee_from: ObjectGuid,
    pub flee_next_ms: u32,
    /// `SetInvincibilityHpThreshold`.
    pub invincibility_hp: u32,
    /// `SCRIPT_COMMAND_SET_REACT_STATE` override.
    pub react_override: Option<u8>,
    /// Hand-ported `boss_mr_smite` state (Slice 7).
    pub smite: Option<crate::content::SmiteState>,
}

impl CreatureAi {
    fn new(kind: AiKind, owner: ObjectGuid, spells: Vec<SpellRuntime>) -> Self {
        Self {
            kind,
            threat: ThreatManager::new(owner),
            in_combat: false,
            combat_start: Position::default(),
            combat_start_ms: 0,
            spells,
            casting_delay_ms: 0,
            evading: false,
            regen_timer_ms: REGEN_TIME_MS,
            phase: 0,
            eventai: None,
            melee_enabled: true,
            combat_movement: true,
            flee_until_ms: 0,
            flee_from: ObjectGuid::EMPTY,
            flee_next_ms: 0,
            invincibility_hp: 0,
            react_override: None,
            smite: None,
        }
    }
}

/// Read-only threat queries over the map.
pub(crate) struct MapThreatHost<'a> {
    pub(crate) map: &'a Map,
}

impl ThreatHost for MapThreatHost<'_> {
    fn spellmod_threat(&self, _hated: ObjectGuid, _spell_id: u32, threat: f32) -> f32 {
        threat
    }

    fn crit_threat_multiplier(&self, _hated: ObjectGuid, _school_mask: u32) -> f32 {
        1.0
    }

    fn apply_total_threat_modifier(
        &self,
        _hated: ObjectGuid,
        threat: f32,
        _school_mask: u32,
    ) -> f32 {
        threat
    }

    fn add_gate(&self, _owner: ObjectGuid, victim: ObjectGuid, _is_assist: bool) -> AddDecision {
        if self.map.unit_is_alive(victim) {
            AddDecision::Allow
        } else {
            AddDecision::Reject
        }
    }

    fn direct_gate(&self, _owner: ObjectGuid, victim: ObjectGuid) -> bool {
        self.map.unit_position(victim).is_some()
    }

    fn is_gamemaster_player(&self, _victim: ObjectGuid) -> bool {
        false
    }

    fn refresh_online(&self, _owner: ObjectGuid, victim: ObjectGuid) -> bool {
        self.map.unit_position(victim).is_some()
    }

    fn pet_owner_to_register(&self, _owner: ObjectGuid, _victim: ObjectGuid) -> Option<ObjectGuid> {
        None
    }

    fn out_of_threat_area(&self, _attacker: ObjectGuid, target: ObjectGuid) -> bool {
        !self.map.unit_is_alive(target)
    }

    fn valid_attack_target(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool {
        self.map.unit_is_alive(target)
            && self.map.unit_is_alive(attacker)
            && attacker.is_player() != target.is_player()
    }

    fn low_priority_target(&self, _attacker: ObjectGuid, _target: ObjectGuid) -> bool {
        false
    }

    fn can_reach_with_melee(&self, attacker: ObjectGuid, target: ObjectGuid) -> bool {
        self.map.can_reach_melee(attacker, target)
    }

    fn faction_id(&self, unit: ObjectGuid) -> u32 {
        self.map
            .unit_value_u32(unit, offsets::UNIT_FIELD_FACTIONTEMPLATE)
    }
}

impl Map {
    pub(crate) fn unit_is_alive(&self, guid: ObjectGuid) -> bool {
        if let Some(entry) = self.players.get(&guid) {
            return entry.player.unit.is_alive();
        }
        self.creatures.get(&guid).is_some_and(|c| c.unit.is_alive())
    }

    /// Has a `BasicAI` combat core (Basic or `EventAI`).
    pub(crate) fn ai_kind_is_basic(creature: &Creature) -> bool {
        Self::ai_kind_for(creature) != AiKind::Null
    }

    fn ai_kind_for(creature: &Creature) -> AiKind {
        let info = &creature.info;
        if info.ai_name == "NullAI"
            || info.ai_name == "TotemAI"
            || info.creature_type == CREATURE_TYPE_CRITTER
            || info.creature_type == CREATURE_TYPE_TOTEM
        {
            return AiKind::Null;
        }
        if info.ai_name == "EventAI" {
            return AiKind::EventAi;
        }
        AiKind::Basic
    }

    /// `ReactStates` subset: 0 passive, 1 defensive,
    /// 2 aggressive (from the ctor/`UpdateEntry` arms
    /// reachable with loaded template data).
    fn react_state_for(creature: &Creature) -> u8 {
        let info = &creature.info;
        if info.creature_type == CREATURE_TYPE_TOTEM {
            return 0;
        }
        if info.flags_extra & FLAG_EXTRA_NO_AGGRO != 0 || info.civilian {
            return 1;
        }
        2
    }

    pub(crate) fn ensure_ai(&mut self, guid: ObjectGuid) {
        if self.ai.contains_key(&guid) {
            return;
        }
        let Some(creature) = self.creatures.get(&guid) else {
            return;
        };
        let kind = Self::ai_kind_for(creature);
        let list_id = creature.info.spell_list_id;
        let entry = creature.info.entry;
        let is_smite = creature.info.script_name == "boss_mr_smite";
        let spells = self.build_spell_runtime(list_id);
        let mut ai = CreatureAi::new(kind, guid, spells);
        if kind == AiKind::EventAi {
            let events = self
                .stores
                .scripts
                .eventai_events
                .get(&entry)
                .cloned()
                .unwrap_or_default();
            let holders = events
                .iter()
                .map(|_| EventHolder {
                    time_ms: 0,
                    enabled: true,
                })
                .collect();
            ai.eventai = Some(EventAiRuntime {
                events,
                holders,
                event_update_ms: EVENT_UPDATE_TIME,
                event_diff_ms: 0,
            });
        }
        self.ai.insert(guid, ai);
        if kind == AiKind::EventAi {
            self.eventai_reset(guid);
        }
        if is_smite {
            self.smite_reset(guid);
        }
    }

    /// `SelectHostileTarget` over the threat manager
    /// (shared by the generic loop and hand-ported
    /// script AIs).
    pub(crate) fn get_hostile_target(&mut self, guid: ObjectGuid) -> Option<ObjectGuid> {
        let mut ai = self.ai.remove(&guid)?;
        let selected = {
            let host = MapThreatHost { map: self };
            ai.threat.get_hostile_target(&host)
        };
        self.ai.insert(guid, ai);
        selected
    }

    /// Build the runtime rows of one creature spell
    /// list (initial delays rolled).
    pub(crate) fn build_spell_runtime(&mut self, list_id: u32) -> Vec<SpellRuntime> {
        let mut spells: Vec<SpellRuntime> = Vec::new();
        if let Some(list) = self.stores.creature_spells.get(&list_id).cloned() {
            for entry in list {
                let cooldown_ms = if entry.delay_initial_max >= entry.delay_initial_min
                    && entry.delay_initial_max > 0
                {
                    self.rng
                        .urand(entry.delay_initial_min, entry.delay_initial_max)
                } else {
                    entry.delay_initial_min
                };
                spells.push(SpellRuntime { entry, cooldown_ms });
            }
        }
        spells
    }

    /// `CreatureAI::SetSpellsList`.
    pub(crate) fn set_creature_spells_list(&mut self, guid: ObjectGuid, list_id: u32) {
        self.ensure_ai(guid);
        let spells = self.build_spell_runtime(list_id);
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.spells = spells;
        }
    }

    fn set_in_combat_flag(&mut self, guid: ObjectGuid, on: bool) {
        if let Some(u) = self.unit_base_mut(guid) {
            let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            let next = if on {
                flags | UNIT_FLAG_IN_COMBAT
            } else {
                flags & !UNIT_FLAG_IN_COMBAT
            };
            u.values().set_u32(offsets::UNIT_FIELD_FLAGS, next);
        }
    }

    /// `Creature::GetAttackDistance` (detection range
    /// replaces the formula when set; stealth not
    /// modeled).
    fn attack_distance(&self, creature: ObjectGuid, target: ObjectGuid) -> f32 {
        let Some(c) = self.creatures.get(&creature) else {
            return 0.0;
        };
        #[allow(clippy::cast_possible_wrap)]
        let target_level = self.unit_level(target) as i32;
        #[allow(clippy::cast_possible_wrap)]
        let my_level = self.unit_level(creature) as i32;
        let level_diff = (target_level - my_level).clamp(
            -MAX_LEVEL_DIFF_FOR_AGGRO_RANGE,
            MAX_LEVEL_DIFF_FOR_AGGRO_RANGE,
        );
        #[allow(clippy::cast_precision_loss)]
        let distance = if c.info.detection_range > 0.0 {
            c.info.detection_range
        } else {
            25.0 - level_diff as f32
        };
        (distance + 2.0 * self.combat_reach_of(creature)).max(self.combat_reach_of(creature))
    }

    fn can_initiate_attack(&self, creature: ObjectGuid, target: ObjectGuid) -> bool {
        let Some(c) = self.creatures.get(&creature) else {
            return false;
        };
        if !c.unit.is_alive() || self.react_state_of(creature) != 2 {
            return false;
        }
        if !self.unit_is_alive(target) || !target.is_player() {
            return false;
        }
        // `MoveInLineOfSight` rides visibility notifiers:
        // no aggro before the creature is in the player's
        // view.
        if !self
            .visible_to(target)
            .is_some_and(|v| v.contains(&creature))
        {
            return false;
        }
        let (Some(cp), Some(tp)) = (self.unit_position(creature), self.unit_position(target))
        else {
            return false;
        };
        if (cp.z - tp.z).abs() > CREATURE_Z_ATTACK_RANGE {
            return false;
        }
        self.within_dist(creature, target, self.attack_distance(creature, target))
    }

    /// Proximity aggro scan (`MoveInLineOfSight`):
    /// engage the first attackable visible player.
    pub(crate) fn try_proximity_aggro(&mut self, guid: ObjectGuid) {
        let players: Vec<ObjectGuid> = self.players.keys().copied().collect();
        for player in players {
            if self.can_initiate_attack(guid, player) {
                self.enter_combat(guid, player);
                break;
            }
        }
    }

    fn react_state_of(&self, guid: ObjectGuid) -> u8 {
        if let Some(state) = self.ai.get(&guid).and_then(|ai| ai.react_override) {
            return state;
        }
        self.creatures.get(&guid).map_or(0, Self::react_state_for)
    }

    /// `CreatureAI::AttackStart` + `SetInCombatWith`
    /// (proximity/damage entry into combat).
    pub(crate) fn enter_combat(&mut self, creature: ObjectGuid, target: ObjectGuid) {
        self.ensure_ai(creature);
        let position = self.unit_position(creature).unwrap_or_default();
        let now = self.now_ms;
        let fresh = self.ai.get(&creature).is_some_and(|ai| !ai.in_combat);
        if let Some(ai) = self.ai.get_mut(&creature) {
            if fresh {
                ai.in_combat = true;
                ai.evading = false;
                ai.combat_start = position;
                ai.combat_start_ms = now;
            }
        }
        self.add_threat(creature, target, 0.0);
        self.set_in_combat_flag(creature, true);
        self.set_in_combat_flag(target, true);
        if let Some(u) = self.unit_base_mut(target) {
            u.last_combat_ms = now;
        }
        if fresh {
            self.eventai_enter_combat(creature, target);
            // SMSG_AI_REACTION (raw guid + reaction).
            let mut payload = PacketWriter::new();
            payload.write_u64_le(creature.raw());
            payload.write_u32_le(AI_REACTION_HOSTILE);
            let packet = WorldPacket::with_body(op::SMSG_AI_REACTION, payload.into_vec());
            self.broadcast_movement(creature, &packet);
        }
        self.attack_start(creature, target);
    }

    pub(crate) fn add_threat(&mut self, creature: ObjectGuid, victim: ObjectGuid, amount: f32) {
        self.ensure_ai(creature);
        let Some(mut ai) = self.ai.remove(&creature) else {
            return;
        };
        {
            let host = MapThreatHost { map: self };
            ai.threat.add_threat_simple(&host, victim, amount);
        }
        self.ai.insert(creature, ai);
    }

    /// Threat for `victim` on `creature` (tests).
    #[must_use]
    pub fn threat_on(&self, creature: ObjectGuid, victim: ObjectGuid) -> f32 {
        self.ai
            .get(&creature)
            .map_or(0.0, |ai| ai.threat.get_threat(victim, true))
    }

    /// Bot query: units melee-attacking `victim`
    /// (`GetAttackers` reduced to the melee-victim
    /// relation the Rust AI maintains).
    #[must_use]
    pub fn bot_attackers_of(&self, victim: ObjectGuid) -> Vec<ObjectGuid> {
        let mut out = Vec::new();
        for (guid, creature) in &self.creatures {
            if creature.unit.melee_victim == Some(victim) && creature.unit.is_alive() {
                out.push(*guid);
            }
        }
        for (guid, entry) in &self.players {
            if entry.player.unit.melee_victim == Some(victim) && entry.player.unit.is_alive() {
                out.push(*guid);
            }
        }
        out
    }

    /// Bot query: `GetIncomingdamage` — reachable
    /// attackers' average weapon damage.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn bot_incoming_damage(&self, victim: ObjectGuid) -> i32 {
        let Some(victim_pos) = self.unit_position(victim) else {
            return 0;
        };
        let mut damage = 0i32;
        for attacker in self.bot_attackers_of(victim) {
            let Some(pos) = self.unit_position(attacker) else {
                continue;
            };
            let dist = ((pos.x - victim_pos.x).powi(2)
                + (pos.y - victim_pos.y).powi(2)
                + (pos.z - victim_pos.z).powi(2))
            .sqrt();
            if dist > 5.0 {
                continue;
            }
            let unit = self
                .creature(attacker)
                .map(|c| &c.unit)
                .or_else(|| self.player(attacker).map(|p| &p.unit));
            if let Some(unit) = unit {
                let values = unit.world.object.values();
                let min = values.get_f32(offsets::UNIT_FIELD_MINDAMAGE);
                let max = values.get_f32(offsets::UNIT_FIELD_MAXDAMAGE);
                damage += f32::midpoint(min, max) as i32;
            }
        }
        damage
    }

    /// Bot query: alive hostile creatures within
    /// `radius` of `center` (player<->creature
    /// hostility, matching `is_hostile_pair`).
    #[must_use]
    pub fn bot_hostile_count_around(&self, center: ObjectGuid, radius: f32) -> u8 {
        let Some(center_pos) = self.unit_position(center) else {
            return 0;
        };
        let mut count = 0u8;
        for (guid, creature) in &self.creatures {
            if !creature.unit.is_alive() || *guid == center {
                continue;
            }
            let pos = creature.unit.world.position();
            let dist = ((pos.x - center_pos.x).powi(2)
                + (pos.y - center_pos.y).powi(2)
                + (pos.z - center_pos.z).powi(2))
            .sqrt();
            if dist <= radius {
                count = count.saturating_add(1);
            }
        }
        count
    }

    /// Bot query: any alive hostile creature
    /// within `radius` of a point (formation
    /// slot/flee safety scans).
    #[must_use]
    pub fn bot_hostiles_near_point(&self, x: f32, y: f32, z: f32, radius: f32) -> bool {
        self.creatures.values().any(|creature| {
            if !creature.unit.is_alive() {
                return false;
            }
            let pos = creature.unit.world.position();
            ((pos.x - x).powi(2) + (pos.y - y).powi(2) + (pos.z - z).powi(2)).sqrt() <= radius
        })
    }

    /// AI combat state probe (tests).
    #[must_use]
    pub fn ai_in_combat(&self, creature: ObjectGuid) -> bool {
        self.ai.get(&creature).is_some_and(|ai| ai.in_combat)
    }

    /// `Unit::Attack`: set the melee victim, target
    /// field and swing timer; `SMSG_ATTACKSTART`.
    pub fn attack_start(&mut self, attacker: ObjectGuid, victim: ObjectGuid) {
        if !self.unit_is_alive(attacker) || !self.unit_is_alive(victim) {
            return;
        }
        let already = self
            .unit_base_mut(attacker)
            .is_some_and(|u| u.melee_victim == Some(victim));
        if already {
            return;
        }
        if let Some(u) = self.unit_base_mut(attacker) {
            if u.melee_victim.is_none() {
                u.attack_timers[0] = 0;
            }
            u.melee_victim = Some(victim);
            u.values().set_guid(offsets::UNIT_FIELD_TARGET, victim);
        }
        self.set_in_combat_flag(attacker, true);
        let mut payload = PacketWriter::new();
        payload.write_u64_le(attacker.raw());
        payload.write_u64_le(victim.raw());
        let packet = WorldPacket::with_body(op::SMSG_ATTACKSTART, payload.into_vec());
        self.broadcast_movement(attacker, &packet);
    }

    /// `Unit::AttackStop` + `SMSG_ATTACKSTOP` (packed
    /// guids + 0 on 5875).
    pub fn attack_stop(&mut self, attacker: ObjectGuid) {
        let victim = self
            .unit_base_mut(attacker)
            .and_then(|u| u.melee_victim.take());
        let Some(victim) = victim else {
            return;
        };
        if let Some(u) = self.unit_base_mut(attacker) {
            u.values()
                .set_guid(offsets::UNIT_FIELD_TARGET, ObjectGuid::EMPTY);
        }
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(attacker));
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(0);
        let packet = WorldPacket::with_body(op::SMSG_ATTACKSTOP, payload.into_vec());
        self.broadcast_movement(attacker, &packet);
    }

    // ------------------------------------------------ AI update

    /// Home arrivals from this tick's motion step
    /// (`JustReachedHome`): clear the evade flag so the
    /// creature can fight again.
    fn process_home_arrivals(&mut self) {
        let arrived: Vec<ObjectGuid> = self
            .events
            .iter()
            .filter_map(|e| match e {
                crate::map::MapEvent::JustReachedHome { guid } => Some(*guid),
                _ => None,
            })
            .collect();
        for guid in arrived {
            if let Some(ai) = self.ai.get_mut(&guid) {
                ai.evading = false;
            }
            self.eventai_process_type(guid, event_type::REACHED_HOME, None);
            self.eventai_reset(guid);
        }
    }

    /// `Creature::Update` AI half + `BasicAI::UpdateAI`
    /// + melee-timer decrement for every unit.
    pub(crate) fn update_ai(&mut self, diff_ms: u32) {
        self.process_home_arrivals();
        self.process_movement_informs();
        // Attack timers tick for players and creatures.
        let units: Vec<ObjectGuid> = self
            .players
            .keys()
            .chain(self.creatures.keys())
            .copied()
            .collect();
        for guid in &units {
            if let Some(u) = self.unit_base_mut(*guid) {
                for timer in &mut u.attack_timers {
                    *timer = timer.saturating_sub(diff_ms);
                }
            }
        }
        let creature_guids: Vec<ObjectGuid> = self.creatures.keys().copied().collect();
        for guid in creature_guids {
            self.update_creature_ai(guid, diff_ms);
        }
        // Melee swings for everyone with a victim.
        for guid in &units {
            self.update_melee_attacking(*guid);
        }
        self.update_player_combat_drop();
    }

    fn update_creature_ai(&mut self, guid: ObjectGuid, diff_ms: u32) {
        if !self.unit_is_alive(guid) {
            return;
        }
        // Hidden patrol spawns (Deadmines) stay inert.
        if self.hidden_objects.contains(&guid) {
            return;
        }
        self.ensure_ai(guid);
        let kind = self.ai.get(&guid).map_or(AiKind::Null, |ai| ai.kind);
        if kind == AiKind::Null {
            return;
        }
        self.regenerate_creature(guid, diff_ms);
        if self.ai.get(&guid).is_some_and(|ai| ai.smite.is_some()) {
            self.update_smite(guid, diff_ms);
            return;
        }

        // Scripted flee drives away-points and suppresses
        // the combat loop (events still tick below via
        // the in-combat path state).
        if self.update_fleeing(guid) {
            self.eventai_update(guid, diff_ms, true);
            return;
        }

        let in_combat = self.ai.get(&guid).is_some_and(|ai| ai.in_combat);
        if !in_combat {
            // EventAI OOC events + OOC line-of-sight
            // scan, then proximity aggro
            // (`MoveInLineOfSight` order).
            self.eventai_update(guid, diff_ms, false);
            self.eventai_ooc_los_scan(guid);
            self.try_proximity_aggro(guid);
            return;
        }

        // Casting creatures skip the rest while a
        // non-melee cast runs (BasicAI gate).
        if self.casts.contains_key(&guid) {
            return;
        }

        // Leash (`Creature::Update` distance arm).
        if self.should_leash_evade(guid) {
            self.enter_evade_mode(guid);
            return;
        }

        let victim = {
            let Some(mut ai) = self.ai.remove(&guid) else {
                return;
            };
            let selected = {
                let host = MapThreatHost { map: self };
                ai.threat.get_hostile_target(&host)
            };
            self.ai.insert(guid, ai);
            selected
        };
        let Some(victim) = victim else {
            // `OnCombatStop` before the evade unwind.
            self.eventai_process_type(guid, event_type::LEAVE_COMBAT, None);
            self.enter_evade_mode(guid);
            return;
        };

        // EventAI combat events run after victim
        // selection, before spell lists/melee
        // (`CreatureEventAI::UpdateAI` order).
        self.eventai_update(guid, diff_ms, true);
        if !self.unit_is_alive(guid) {
            return;
        }

        // Victim management + chase.
        let current_victim = self.unit_base_mut(guid).and_then(|u| u.melee_victim);
        if current_victim != Some(victim) {
            self.attack_start(guid, victim);
        }
        let combat_movement = self.ai.get(&guid).is_some_and(|ai| ai.combat_movement);
        if combat_movement
            && (self.motion_type_of(guid) != CHASE_MOTION_TYPE
                || self.chase_target_of(guid) != Some(victim))
        {
            self.creature_move_chase(guid, victim, 0.0, 0.0);
        }
        self.update_spells_list(guid, victim, diff_ms);
    }

    pub(crate) fn should_leash_evade(&self, guid: ObjectGuid) -> bool {
        let Some(ai) = self.ai.get(&guid) else {
            return false;
        };
        let Some(c) = self.creatures.get(&guid) else {
            return false;
        };
        if c.info.flags_extra & FLAG_EXTRA_NO_LEASH_EVADE != 0 || ai.evading {
            return false;
        }
        let leash = c.info.leash_range;
        let Some(pos) = self.unit_position(guid) else {
            return false;
        };
        if leash > 0.0 {
            let dx = pos.x - ai.combat_start.x;
            let dy = pos.y - ai.combat_start.y;
            let dz = pos.z - ai.combat_start.z;
            return (dx * dx + dy * dy + dz * dz).sqrt() > leash;
        }
        // Distance-from-home rule: 2x detection or 50 yd.
        let check_dist = (2.0 * c.info.detection_range).max(50.0);
        let dx = pos.x - ai.combat_start.x;
        let dy = pos.y - ai.combat_start.y;
        (dx * dx + dy * dy).sqrt() > check_dist
    }

    /// `CreatureAI::EnterEvadeMode`.
    pub(crate) fn enter_evade_mode(&mut self, guid: ObjectGuid) {
        self.attack_stop(guid);
        self.cancel_cast(guid, false);
        self.remove_all_auras(guid);
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.threat = ThreatManager::new(guid);
            ai.in_combat = false;
            ai.evading = true;
            for spell in &mut ai.spells {
                spell.cooldown_ms = self.rng.urand(
                    spell.entry.delay_initial_min,
                    spell
                        .entry
                        .delay_initial_max
                        .max(spell.entry.delay_initial_min),
                );
            }
        }
        self.set_in_combat_flag(guid, false);
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.flee_until_ms = 0;
        }
        if self.ai.get(&guid).is_some_and(|ai| ai.smite.is_some()) {
            self.smite_reset(guid);
        }
        self.eventai_process_type(guid, event_type::EVADE, None);
        if let Some(mut master) = self.motion.remove(&guid) {
            master.clear(self, guid);
            self.motion.entry(guid).or_insert(master);
        }
        self.creature_move_home(guid);
    }

    /// Out-of-combat creature regen (`RegenerateAll`:
    /// max/3 per regen tick for health and mana).
    fn regenerate_creature(&mut self, guid: ObjectGuid, diff_ms: u32) {
        let due = {
            let Some(ai) = self.ai.get_mut(&guid) else {
                return;
            };
            if ai.in_combat {
                return;
            }
            ai.regen_timer_ms = ai.regen_timer_ms.saturating_sub(diff_ms);
            if ai.regen_timer_ms > 0 {
                return;
            }
            ai.regen_timer_ms = REGEN_TIME_MS;
            true
        };
        if !due {
            return;
        }
        let max_health = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
        let health = self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH);
        if health < max_health
            && let Some(u) = self.unit_base_mut(guid)
        {
            u.set_health((health + max_health / 3).min(max_health));
        }
        let max_mana = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXPOWER1);
        let mana = self.unit_value_u32(guid, offsets::UNIT_FIELD_POWER1);
        if mana < max_mana
            && let Some(u) = self.unit_base_mut(guid)
        {
            u.set_power(0, (mana + max_mana / 3).min(max_mana));
        }
    }

    /// Player combat drop: clear `UNIT_FLAG_IN_COMBAT`
    /// and stop swings once nothing references the
    /// player and the drop delay elapsed.
    fn update_player_combat_drop(&mut self) {
        let players: Vec<ObjectGuid> = self.players.keys().copied().collect();
        let now = self.now_ms;
        for player in players {
            let referenced = self
                .ai
                .values()
                .any(|ai| ai.in_combat && ai.threat.get_threat(player, true) > 0.0)
                || self
                    .creatures
                    .values()
                    .any(|c| c.unit.melee_victim == Some(player));
            let last = self.unit_base_mut(player).map_or(0, |u| u.last_combat_ms);
            let victim_alive = self
                .unit_base_mut(player)
                .and_then(|u| u.melee_victim)
                .is_some_and(|v| self.unit_is_alive(v));
            if !referenced && !victim_alive && now.saturating_sub(last) > PLAYER_COMBAT_DROP_MS {
                if self
                    .unit_base_mut(player)
                    .and_then(|u| u.melee_victim)
                    .is_some()
                {
                    self.attack_stop(player);
                }
                self.set_in_combat_flag(player, false);
            }
        }
    }

    // ------------------------------------------------ spell list

    fn resolve_spell_target(
        &mut self,
        creature: ObjectGuid,
        victim: ObjectGuid,
        entry: &CreatureSpellEntry,
    ) -> CastTarget {
        match entry.cast_target {
            // PROVIDED (= self in the spells-list call),
            // OWNER_OR_SELF, and the FRIENDLY_* family
            // (subset resolves friendlies to self).
            0 | 8 | 16..=20 => CastTarget::SelfCast,
            // HOSTILE_RANDOM / RANDOM_NOT_TOP / second
            // aggro: uniform over the threat list subset.
            2 | 4 | 5 => {
                let refs: Vec<ObjectGuid> = self
                    .ai
                    .get(&creature)
                    .map(|ai| {
                        ai.threat
                            .threat_list()
                            .iter()
                            .map(mangos_threat::HostileRef::target)
                            .filter(|g| entry.cast_target != 5 || *g != victim)
                            .collect()
                    })
                    .unwrap_or_default();
                if refs.is_empty() {
                    CastTarget::Unit(victim)
                } else if entry.cast_target == 2 && refs.len() > 1 {
                    CastTarget::Unit(refs[1])
                } else {
                    #[allow(clippy::cast_possible_truncation)]
                    let idx = self.rng.urand(0, refs.len() as u32 - 1) as usize;
                    CastTarget::Unit(refs[idx])
                }
            }
            // HOSTILE (top aggro); every unhandled type
            // also falls back to the victim.
            _ => CastTarget::Unit(victim),
        }
    }

    /// `CreatureAI::UpdateSpellsList` +
    /// `DoSpellsListCasts` subset.
    fn update_spells_list(&mut self, guid: ObjectGuid, victim: ObjectGuid, diff_ms: u32) {
        let due = {
            let Some(ai) = self.ai.get_mut(&guid) else {
                return;
            };
            if ai.casting_delay_ms <= diff_ms {
                let desync = diff_ms - ai.casting_delay_ms;
                ai.casting_delay_ms = CREATURE_CASTING_DELAY.saturating_sub(desync);
                CREATURE_CASTING_DELAY + desync
            } else {
                ai.casting_delay_ms -= diff_ms;
                return;
            }
        };

        let mut dont_cast = false;
        let spell_count = self.ai.get(&guid).map_or(0, |ai| ai.spells.len());
        for i in 0..spell_count {
            let (entry, cooldown) = {
                let ai = self.ai.get(&guid).expect("ai");
                (ai.spells[i].entry.clone(), ai.spells[i].cooldown_ms)
            };
            if cooldown > due {
                if let Some(ai) = self.ai.get_mut(&guid) {
                    ai.spells[i].cooldown_ms -= due;
                }
                continue;
            }
            if let Some(ai) = self.ai.get_mut(&guid) {
                ai.spells[i].cooldown_ms = 0;
            }
            let triggered = entry.cast_flags & CF_TRIGGERED != 0;
            if !triggered
                && entry.cast_flags & CF_INTERRUPT_PREVIOUS == 0
                && (dont_cast || self.casts.contains_key(&guid))
            {
                continue;
            }
            if entry.cast_flags & CF_ONLY_IN_MELEE != 0 && !self.can_reach_melee(guid, victim) {
                continue;
            }
            if entry.cast_flags & CF_NOT_IN_MELEE != 0 && self.can_reach_melee(guid, victim) {
                continue;
            }
            if entry.cast_flags & CF_AURA_NOT_PRESENT != 0 {
                let target_guid = match entry.cast_target {
                    0 | 8 | 16..=20 => guid,
                    _ => victim,
                };
                if self
                    .aura_holders(target_guid)
                    .iter()
                    .any(|h| h.spell_id == entry.spell_id)
                {
                    continue;
                }
            }
            if entry.cast_flags & CF_TARGET_CASTING != 0 && !self.casts.contains_key(&victim) {
                continue;
            }
            // Probability roll (`SPELL_FAILED_TRY_AGAIN`).
            let roll = self.rng.frand(0.0, 100.0);
            if roll >= f32::from(entry.probability) {
                self.reset_spell_cooldown(guid, i, &entry);
                continue;
            }
            let target = self.resolve_spell_target(guid, victim, &entry);
            let outcome = self.cast_spell(
                guid,
                entry.spell_id,
                target,
                CastOptions {
                    triggered,
                    skip_known: true,
                },
            );
            if outcome == CastOutcome::Started {
                dont_cast = !triggered;
                self.reset_spell_cooldown(guid, i, &entry);
                // `creature_spells_scripts` companion
                // script (`ScriptsStart`), Slice 6b.
                if entry.script_id != 0
                    && let Some(steps) = self
                        .stores
                        .scripts
                        .creature_spells_scripts
                        .get(&entry.script_id)
                        .cloned()
                {
                    let script_target = match target {
                        CastTarget::Unit(g) => Some(g),
                        CastTarget::SelfCast => Some(guid),
                    };
                    self.schedule_steps(&steps, Some(guid), script_target);
                }
            }
        }
    }

    fn reset_spell_cooldown(&mut self, guid: ObjectGuid, index: usize, entry: &CreatureSpellEntry) {
        let cooldown = if entry.delay_repeat_max > 0 {
            self.rng.urand(
                entry.delay_repeat_min,
                entry.delay_repeat_max.max(entry.delay_repeat_min),
            )
        } else {
            entry.delay_repeat_min
        };
        if let Some(ai) = self.ai.get_mut(&guid) {
            ai.spells[index].cooldown_ms = cooldown;
        }
    }

    // ------------------------------------------------ melee

    fn skill_max_for_level(&self, guid: ObjectGuid) -> i32 {
        #[allow(clippy::cast_possible_wrap)]
        {
            (self.unit_level(guid) * 5) as i32
        }
    }

    /// `MeleeMissChanceCalc(BASE_ATTACK)` at maxed
    /// weapon/defense skills (no skill system yet).
    #[allow(clippy::cast_precision_loss)]
    fn melee_miss_chance(&self, attacker: ObjectGuid, victim: ObjectGuid) -> f32 {
        let skill_diff = self.skill_max_for_level(attacker) - self.skill_max_for_level(victim);
        let mut miss = if victim.is_player() {
            5.0 - skill_diff as f32 * 0.04
        } else if skill_diff < -10 {
            5.0 - skill_diff as f32 * 0.2
        } else {
            5.0 - skill_diff as f32 * 0.1
        };
        let victim_level = self.unit_level(victim);
        if !victim.is_player() && victim_level < 10 {
            miss *= victim_level as f32 / 10.0;
        }
        miss.clamp(0.0, 60.0)
    }

    /// `GetUnitDodgeChance`: players read the percentage
    /// field (set at materialization), creatures 5%.
    fn unit_dodge_chance(&self, guid: ObjectGuid) -> f32 {
        if self.casts.contains_key(&guid) {
            return 0.0;
        }
        if guid.is_player() {
            f32::from_bits(self.unit_value_u32(guid, offsets::PLAYER_DODGE_PERCENTAGE))
        } else if self
            .creatures
            .get(&guid)
            .is_some_and(|c| c.info.creature_type == CREATURE_TYPE_TOTEM)
        {
            0.0
        } else {
            5.0
        }
    }

    /// `GetUnitParryChance`.
    fn unit_parry_chance(&self, guid: ObjectGuid) -> f32 {
        if self.casts.contains_key(&guid) {
            return 0.0;
        }
        if guid.is_player() {
            f32::from_bits(self.unit_value_u32(guid, offsets::PLAYER_PARRY_PERCENTAGE))
        } else if self
            .creatures
            .get(&guid)
            .is_some_and(|c| c.info.creature_type == CREATURE_TYPE_TOTEM)
        {
            0.0
        } else {
            5.0
        }
    }

    /// `GetUnitBlockChance` (players 0 until shield
    /// detection lands with item stats).
    fn unit_block_chance(&self, guid: ObjectGuid) -> f32 {
        if self.casts.contains_key(&guid) || guid.is_player() {
            return 0.0;
        }
        if self
            .creatures
            .get(&guid)
            .is_some_and(|c| c.info.creature_type == CREATURE_TYPE_TOTEM)
        {
            0.0
        } else {
            5.0
        }
    }

    /// `GetShieldBlockValue`: creature arm
    /// (`level/2 + strength/20`); players 0 (no shield).
    fn unit_block_value(&self, guid: ObjectGuid) -> u32 {
        if guid.is_player() {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            self.unit_level(guid) / 2 + (self.unit_stat_f32(guid, 0) / 20.0) as u32
        }
    }

    fn unit_class_of(&self, guid: ObjectGuid) -> u32 {
        self.unit_value_u32(guid, offsets::UNIT_FIELD_BYTES_0) >> 8 & 0xFF
    }

    fn unit_stat_f32(&self, guid: ObjectGuid, stat: u16) -> f32 {
        f32::from_bits(self.unit_value_u32(guid, offsets::UNIT_FIELD_STAT0 + stat))
    }

    /// `GetUnitCriticalChance(BASE_ATTACK, victim)`:
    /// players read the percentage field, creatures 5%,
    /// plus the defense-skill adjustment.
    #[allow(clippy::cast_precision_loss)]
    fn unit_crit_chance(&self, attacker: ObjectGuid, victim: ObjectGuid) -> f32 {
        let mut crit = if attacker.is_player() {
            f32::from_bits(self.unit_value_u32(attacker, offsets::PLAYER_CRIT_PERCENTAGE))
        } else {
            5.0
        };
        let skill_diff = self.skill_max_for_level(attacker) - self.skill_max_for_level(victim);
        #[allow(clippy::cast_precision_loss)]
        {
            crit += if victim.is_player() || skill_diff > 0 {
                skill_diff as f32 * 0.04
            } else {
                skill_diff as f32 * 0.2
            };
        }
        crit.max(0.0)
    }

    /// `UpdateMeleeAttackingState` (`BASE_ATTACK` only;
    /// the swing itself is in the reach check of
    /// `melee_swing`).
    fn update_melee_attacking(&mut self, attacker: ObjectGuid) {
        let Some(victim) = self.unit_base_mut(attacker).and_then(|u| u.melee_victim) else {
            return;
        };
        if !self.unit_is_alive(attacker) || !self.unit_is_alive(victim) {
            self.attack_stop(attacker);
            return;
        }
        // Casting units do not swing (`IsNonMeleeSpellCasted`).
        if self.casts.contains_key(&attacker) {
            return;
        }
        // Script melee toggle + fleeing creatures do
        // not swing (Slice 6b).
        if attacker.is_creature()
            && self
                .ai
                .get(&attacker)
                .is_some_and(|ai| !ai.melee_enabled || ai.flee_until_ms > self.now_ms)
        {
            return;
        }
        let timer_ready = self
            .unit_base_mut(attacker)
            .is_some_and(|u| u.attack_timers[0] == 0);
        if !timer_ready || !self.can_reach_melee(attacker, victim) {
            return;
        }
        let swing_ms = self.swing_time_ms(attacker);
        if let Some(u) = self.unit_base_mut(attacker) {
            u.attack_timers[0] = swing_ms;
        }
        self.melee_swing(attacker, victim);
    }

    fn swing_time_ms(&self, attacker: ObjectGuid) -> u32 {
        let ms = f32::from_bits(self.unit_value_u32(attacker, offsets::UNIT_FIELD_BASEATTACKTIME));
        if ms > 0.0 {
            #[allow(clippy::cast_possible_truncation)]
            {
                ms as u32
            }
        } else {
            2000
        }
    }

    /// `CalculateMeleeDamage` + `RollMeleeOutcomeAgainst`
    /// + `DealMeleeDamage` subset.
    ///
    /// Chances are integer hundredths over a 0..9999 roll,
    /// like C++.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::too_many_lines
    )]
    fn melee_swing(&mut self, attacker: ObjectGuid, victim: ObjectGuid) {
        // Victim creature in evade mode: MELEE_HIT_EVADE.
        if victim.is_creature() && self.ai.get(&victim).is_some_and(|ai| ai.evading) {
            self.send_attacker_state(
                attacker,
                victim,
                HITINFO_MISS | HITINFO_AFFECTS_VICTIM | HITINFO_SWINGNOHITSOUND,
                0,
                VICTIMSTATE_EVADES,
                0,
            );
            return;
        }

        let victim_level = self.unit_level(victim);
        let skill_diff = self.skill_max_for_level(attacker) - self.skill_max_for_level(victim);
        // Hundredths.
        let miss_c = (self.melee_miss_chance(attacker, victim) * 100.0) as i32;
        let mut dodge_c = (self.unit_dodge_chance(victim) * 100.0) as i32;
        let mut parry_c = (self.unit_parry_chance(victim) * 100.0) as i32;
        let mut block_c = (self.unit_block_chance(victim) * 100.0) as i32;
        let crit_c = (self.unit_crit_chance(attacker, victim) * 100.0) as i32;

        let low_level_scale = |chance: i32| -> i32 {
            if !victim.is_player() && victim_level < 10 {
                (chance as f32 * victim_level as f32 / 10.0) as i32
            } else {
                chance
            }
        };
        dodge_c -= if victim.is_player() {
            4 * skill_diff
        } else {
            10 * skill_diff
        };
        dodge_c = low_level_scale(dodge_c);
        parry_c -= if victim.is_player() {
            4 * skill_diff
        } else if skill_diff < -10 {
            60 * skill_diff
        } else {
            20 * skill_diff
        };
        parry_c = low_level_scale(parry_c);
        block_c -= if victim.is_player() {
            4 * skill_diff
        } else {
            10 * skill_diff
        };
        if !victim.is_player() && block_c > 500 {
            block_c = 500;
        }
        block_c = low_level_scale(block_c);

        let glancing_c = if attacker.is_player() && !victim.is_player() {
            // (10 + (defense - weapon) * 2) percent.
            ((10 + (-skill_diff) * 2) * 100).clamp(0, 4000)
        } else {
            0
        };
        let crushing_c = if !attacker.is_player() && skill_diff >= 15 {
            skill_diff * 200 - 1500
        } else {
            0
        };

        let no_parry = self
            .creatures
            .get(&victim)
            .is_some_and(|c| c.info.flags_extra & 0x0000_0004 != 0);
        let no_block = self
            .creatures
            .get(&victim)
            .is_some_and(|c| c.info.flags_extra & 0x0000_0010 != 0);

        #[allow(clippy::cast_possible_wrap)]
        let roll = self.rng.urand(0, 9999) as i32;
        // Cumulative bands in C++ outcome order.
        let bands: [(bool, i32, MeleeOutcome); 7] = [
            (miss_c > 0, miss_c, MeleeOutcome::Miss),
            (dodge_c > 0, dodge_c, MeleeOutcome::Dodge),
            (!no_parry && parry_c > 0, parry_c, MeleeOutcome::Parry),
            (glancing_c > 0, glancing_c, MeleeOutcome::Glancing),
            (!no_block && block_c > 0, block_c, MeleeOutcome::Block),
            (crit_c > 0, crit_c, MeleeOutcome::Crit),
            (crushing_c > 0, crushing_c, MeleeOutcome::Crushing),
        ];
        let mut sum = 0i32;
        let mut outcome = MeleeOutcome::Normal;
        for (enabled, width, kind) in bands {
            if !enabled {
                continue;
            }
            sum += width;
            if roll < sum {
                outcome = kind;
                break;
            }
        }

        match outcome {
            MeleeOutcome::Miss => {
                self.send_attacker_state(
                    attacker,
                    victim,
                    HITINFO_MISS | HITINFO_AFFECTS_VICTIM,
                    0,
                    VICTIMSTATE_UNAFFECTED,
                    0,
                );
                return;
            }
            MeleeOutcome::Dodge => {
                self.send_attacker_state(
                    attacker,
                    victim,
                    HITINFO_AFFECTS_VICTIM,
                    0,
                    VICTIMSTATE_DODGE,
                    0,
                );
                return;
            }
            MeleeOutcome::Parry => {
                self.send_attacker_state(
                    attacker,
                    victim,
                    HITINFO_AFFECTS_VICTIM,
                    0,
                    VICTIMSTATE_PARRY,
                    0,
                );
                return;
            }
            _ => {}
        }

        // Base damage: field range roll, dithered, then
        // armor (CalculateMeleeDamage order).
        let min_dmg = f32::from_bits(self.unit_value_u32(attacker, offsets::UNIT_FIELD_MINDAMAGE));
        let max_dmg = f32::from_bits(self.unit_value_u32(attacker, offsets::UNIT_FIELD_MAXDAMAGE));
        let (lo, hi) = if min_dmg > max_dmg {
            (max_dmg, min_dmg)
        } else {
            (min_dmg, max_dmg)
        };
        let rolled = mangos_spells::ditheru(
            self.rng
                .frand(lo.max(0.0), if hi == 0.0 { 5.0 } else { hi }),
            self.spell_rng(),
        );
        let victim_armor = self.unit_resistance(victim, 0);
        let mut total = mangos_spells::armor_reduced_damage(
            rolled as f32,
            victim_armor,
            self.unit_level(attacker),
        ) as u32;

        let mut hit_info = HITINFO_AFFECTS_VICTIM;
        let mut victim_state = VICTIMSTATE_NORMAL;
        let mut blocked_amount = 0u32;
        match outcome {
            MeleeOutcome::Crit => {
                hit_info |= HITINFO_CRITICALHIT;
                total = (total as f32 * 2.0) as u32;
            }
            MeleeOutcome::Glancing => {
                hit_info |= HITINFO_GLANCING;
                // Baeza reduction band.
                let diff = -skill_diff as f32;
                let mut low = 1.3 - 0.05 * diff;
                let mut high = 1.2 - 0.03 * diff;
                let (mut low_cap, high_cap) = (0.91f32, 0.99f32);
                if matches!(self.unit_class_of(attacker), 5 | 8 | 9) {
                    low -= 0.7;
                    high -= 0.3;
                    low_cap = 0.6;
                }
                low = low.clamp(0.01, low_cap);
                high = high.clamp(0.2, high_cap);
                let reduce = self.rng.frand(low, high.max(low));
                total = (reduce * total as f32) as u32;
            }
            MeleeOutcome::Crushing => {
                hit_info |= HITINFO_CRUSHING;
                total += total / 2;
            }
            MeleeOutcome::Block => {
                blocked_amount = self.unit_block_value(victim);
                if blocked_amount >= total {
                    blocked_amount = total;
                    victim_state = VICTIMSTATE_BLOCKS;
                }
                total -= blocked_amount;
            }
            _ => {}
        }

        if total > 0 {
            self.deal_damage(attacker, victim, total);
        }
        self.send_attacker_state(
            attacker,
            victim,
            hit_info,
            total,
            victim_state,
            blocked_amount,
        );
    }

    /// `SMSG_ATTACKERSTATEUPDATE` (5875 layout, one
    /// NORMAL-school sub-damage, no absorb/resist).
    fn send_attacker_state(
        &mut self,
        attacker: ObjectGuid,
        victim: ObjectGuid,
        hit_info: u32,
        damage: u32,
        victim_state: u32,
        blocked_amount: u32,
    ) {
        let mut payload = PacketWriter::new();
        payload.write_u32_le(hit_info);
        payload.write_bytes(&Self::packed_guid_bytes(attacker));
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(damage);
        payload.write_u8(1); // sub-damage count
        payload.write_u32_le(0); // school index (NORMAL)
        #[allow(clippy::cast_precision_loss)]
        payload.write_f32_le(damage as f32);
        payload.write_u32_le(damage);
        payload.write_u32_le(0); // absorb
        payload.write_u32_le(0); // resist (signed on the wire)
        payload.write_u32_le(victim_state);
        payload.write_u32_le(0);
        payload.write_u32_le(0); // spell id
        payload.write_u32_le(blocked_amount);
        let packet = WorldPacket::with_body(op::SMSG_ATTACKERSTATEUPDATE, payload.into_vec());
        self.broadcast_movement(attacker, &packet);
    }

    // ------------------------------------------------ EventAI (Slice 6b)

    fn eventai_active(&self, guid: ObjectGuid) -> bool {
        self.ai
            .get(&guid)
            .is_some_and(|ai| ai.kind == AiKind::EventAi && ai.eventai.is_some())
    }

    /// `CreatureEventAIHolder::UpdateRepeatTimer`:
    /// returns false (and disables) on inverted ranges.
    fn holder_repeat_timer(&mut self, guid: ObjectGuid, idx: usize, min: u32, max: u32) -> bool {
        match min.cmp(&max) {
            std::cmp::Ordering::Equal => {
                if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                    rt.holders[idx].time_ms = min;
                }
                true
            }
            std::cmp::Ordering::Less => {
                let time = self.rng.urand(min, max);
                if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                    rt.holders[idx].time_ms = time;
                }
                true
            }
            std::cmp::Ordering::Greater => {
                if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                    rt.holders[idx].enabled = false;
                }
                false
            }
        }
    }

    /// `CreatureEventAI::Reset` (OOC timers re-armed).
    pub(crate) fn eventai_reset(&mut self, guid: ObjectGuid) {
        if !self.eventai_active(guid) {
            return;
        }
        let count = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
            rt.event_update_ms = EVENT_UPDATE_TIME;
            rt.event_diff_ms = 0;
        }
        for idx in 0..count {
            let (etype, p) = {
                let rt = self
                    .ai
                    .get(&guid)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                (rt.events[idx].event_type, rt.events[idx].params)
            };
            if etype == event_type::TIMER_OOC
                && self.holder_repeat_timer(guid, idx, p[0], p[1])
                && let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut())
            {
                rt.holders[idx].enabled = true;
            }
        }
    }

    /// `CreatureEventAI::JustRespawned` (spawn path).
    pub(crate) fn eventai_on_spawn(&mut self, guid: ObjectGuid) {
        let is_eventai = self
            .creatures
            .get(&guid)
            .is_some_and(|c| c.info.ai_name == "EventAI");
        if !is_eventai {
            return;
        }
        self.ensure_ai(guid);
        self.eventai_process_type(guid, event_type::SPAWNED, None);
    }

    /// `CreatureEventAI::EnterCombat`.
    fn eventai_enter_combat(&mut self, guid: ObjectGuid, enemy: ObjectGuid) {
        if !self.eventai_active(guid) {
            return;
        }
        let count = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let (etype, p) = {
                let rt = self
                    .ai
                    .get(&guid)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                (rt.events[idx].event_type, rt.events[idx].params)
            };
            match etype {
                event_type::AGGRO => {
                    if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                        rt.holders[idx].enabled = true;
                    }
                    self.eventai_process_event(guid, idx, Some(enemy));
                }
                event_type::TIMER_IN_COMBAT => {
                    if self.holder_repeat_timer(guid, idx, p[0], p[1])
                        && let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut())
                    {
                        rt.holders[idx].enabled = true;
                    }
                }
                _ => {
                    if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                        rt.holders[idx].enabled = true;
                        rt.holders[idx].time_ms = 0;
                    }
                }
            }
            if !self.unit_is_alive(guid) {
                return;
            }
        }
        if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
            rt.event_update_ms = EVENT_UPDATE_TIME;
            rt.event_diff_ms = 0;
        }
    }

    /// Process every holder of one event type
    /// (hook-driven types).
    pub(crate) fn eventai_process_type(
        &mut self,
        guid: ObjectGuid,
        etype: u16,
        invoker: Option<ObjectGuid>,
    ) {
        if !self.eventai_active(guid) {
            return;
        }
        let count = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let matches = self
                .ai
                .get(&guid)
                .and_then(|ai| ai.eventai.as_ref())
                .is_some_and(|rt| rt.events[idx].event_type == etype);
            if matches {
                self.eventai_process_event(guid, idx, invoker);
            }
            if !self.creatures.contains_key(&guid) {
                return;
            }
        }
    }

    /// `CreatureEventAI::UpdateEventsOn_UpdateAI`
    /// cadence + per-type dispatch.
    fn eventai_update(&mut self, guid: ObjectGuid, diff_ms: u32, combat: bool) {
        if !self.eventai_active(guid) {
            return;
        }
        let due = {
            let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) else {
                return;
            };
            if rt.event_update_ms < diff_ms {
                rt.event_diff_ms += diff_ms;
                let due = rt.event_diff_ms;
                rt.event_diff_ms = 0;
                rt.event_update_ms = EVENT_UPDATE_TIME;
                due
            } else {
                rt.event_diff_ms += diff_ms;
                rt.event_update_ms -= diff_ms;
                return;
            }
        };
        let phase = self.ai.get(&guid).map_or(0, |ai| ai.phase);
        let count = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            // Decrement timers (phase-masked holders hold).
            let (etype, params) = {
                let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) else {
                    return;
                };
                let ev = rt.events[idx].clone();
                let holder = &mut rt.holders[idx];
                if holder.time_ms > 0 {
                    if holder.time_ms > due {
                        if ev.inverse_phase_mask & (1u32 << phase) == 0 {
                            holder.time_ms -= due;
                        }
                        continue;
                    }
                    holder.time_ms = 0;
                }
                (ev.event_type, ev.params)
            };
            let process = match etype {
                event_type::TIMER_OOC | event_type::FRIENDLY_MISSING_BUFF => true,
                event_type::TIMER_IN_COMBAT
                | event_type::MANA
                | event_type::HP
                | event_type::TARGET_HP
                | event_type::TARGET_CASTING
                | event_type::FRIENDLY_HP
                | event_type::FRIENDLY_IS_CC
                | event_type::AURA
                | event_type::TARGET_AURA
                | event_type::MISSING_AURA
                | event_type::TARGET_MISSING_AURA
                | event_type::VICTIM_ROOTED => combat,
                event_type::RANGE => {
                    combat
                        && self.unit_victim(guid).is_some_and(|v| {
                            let raw = self
                                .unit_position(guid)
                                .zip(self.unit_position(v))
                                .map_or(f32::MAX, |(a, b)| crate::visibility::dist2d(a, b));
                            #[allow(clippy::cast_precision_loss)]
                            let dist =
                                (raw - self.bounding_radius_of(guid) - self.bounding_radius_of(v))
                                    .max(0.0);
                            #[allow(clippy::cast_precision_loss)]
                            {
                                dist >= params[0] as f32 && dist <= params[1] as f32
                            }
                        })
                }
                _ => false,
            };
            if process {
                self.eventai_process_event(guid, idx, None);
            }
            if !self.creatures.contains_key(&guid) || !self.unit_is_alive(guid) {
                return;
            }
        }
    }

    /// `UpdateEventsOn_MoveInLineOfSight` (polled at AI
    /// tick rate while out of combat; holder timers
    /// gate repeats).
    fn eventai_ooc_los_scan(&mut self, guid: ObjectGuid) {
        if !self.eventai_active(guid) || self.unit_victim(guid).is_some() {
            return;
        }
        let has_los = self
            .ai
            .get(&guid)
            .and_then(|ai| ai.eventai.as_ref())
            .is_some_and(|rt| {
                rt.events
                    .iter()
                    .any(|e| e.event_type == event_type::OOC_LOS)
            });
        if !has_los {
            return;
        }
        let players: Vec<ObjectGuid> = self.players.keys().copied().collect();
        for player in players {
            if !self.visible_to(player).is_some_and(|v| v.contains(&guid)) {
                continue;
            }
            let count = self
                .ai
                .get(&guid)
                .and_then(|ai| ai.eventai.as_ref())
                .map_or(0, |rt| rt.events.len());
            for idx in 0..count {
                let ev = {
                    let rt = self
                        .ai
                        .get(&guid)
                        .and_then(|ai| ai.eventai.as_ref())
                        .expect("rt");
                    rt.events[idx].clone()
                };
                if ev.event_type != event_type::OOC_LOS {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                let max_range = ev.params[1] as f32;
                let hostile = guid.is_player() != player.is_player();
                let reaction_ok = match ev.params[0] {
                    1 => hostile,
                    2 => !hostile,
                    _ => true,
                };
                if reaction_ok && self.within_dist(guid, player, max_range) {
                    self.eventai_process_event(guid, idx, Some(player));
                }
            }
            if !self.creatures.contains_key(&guid) {
                return;
            }
        }
    }

    /// `CreatureEventAI::ProcessEvent`.
    #[allow(clippy::too_many_lines)]
    fn eventai_process_event(
        &mut self,
        guid: ObjectGuid,
        idx: usize,
        invoker: Option<ObjectGuid>,
    ) -> bool {
        let (ev, holder, phase) = {
            let Some(ai) = self.ai.get(&guid) else {
                return false;
            };
            let Some(rt) = ai.eventai.as_ref() else {
                return false;
            };
            (rt.events[idx].clone(), rt.holders[idx], ai.phase)
        };
        if !holder.enabled || holder.time_ms > 0 {
            return false;
        }
        if ev.inverse_phase_mask & (1u32 << phase) != 0 {
            return false;
        }
        if ev.flags & EFLAG_NOT_CASTING != 0 && self.casts.contains_key(&guid) {
            return false;
        }
        let victim = self.unit_victim(guid);
        if ev.condition_id != 0
            && !self.condition_satisfied(ev.condition_id, invoker.or(victim), Some(guid))
        {
            return false;
        }
        let p = ev.params;
        let mut action_invoker = invoker;
        let in_combat = self.ai_in_combat(guid);
        let pct = |cur: u32, max: u32| -> u32 {
            if max == 0 {
                0
            } else {
                (u64::from(cur) * 100 / u64::from(max)) as u32
            }
        };
        match ev.event_type {
            event_type::TIMER_IN_COMBAT => {
                if !in_combat {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::TIMER_OOC => {
                let evading = self.ai.get(&guid).is_some_and(|ai| ai.evading);
                if in_combat || evading {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::HP => {
                let max = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXHEALTH);
                if !in_combat || max == 0 {
                    return false;
                }
                let perc = pct(self.unit_value_u32(guid, offsets::UNIT_FIELD_HEALTH), max);
                if perc > p[0] || perc < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::MANA => {
                let max = self.unit_value_u32(guid, offsets::UNIT_FIELD_MAXPOWER1);
                if !in_combat || max == 0 {
                    return false;
                }
                let perc = pct(self.unit_value_u32(guid, offsets::UNIT_FIELD_POWER1), max);
                if perc > p[0] || perc < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::KILL => {
                if p[2] != 0 && !invoker.is_some_and(ObjectGuid::is_player) {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[0], p[1]);
            }
            event_type::HIT_BY_SPELL
            | event_type::SPELL_HIT_TARGET
            | event_type::RANGE
            | event_type::OOC_LOS
            | event_type::MOVEMENT_INFORM
            | event_type::HIT_BY_AURA => {
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::TARGET_HP => {
                let Some(v) = victim else { return false };
                let max = self.unit_value_u32(v, offsets::UNIT_FIELD_MAXHEALTH);
                if !in_combat || max == 0 {
                    return false;
                }
                let perc = pct(self.unit_value_u32(v, offsets::UNIT_FIELD_HEALTH), max);
                if perc > p[0] || perc < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::TARGET_CASTING => {
                let Some(v) = victim else { return false };
                if !in_combat || !self.casts.contains_key(&v) {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[0], p[1]);
            }
            event_type::TARGET_MANA => {
                let Some(v) = victim else { return false };
                let max = self.unit_value_u32(v, offsets::UNIT_FIELD_MAXPOWER1);
                if !in_combat || max == 0 {
                    return false;
                }
                let perc = pct(self.unit_value_u32(v, offsets::UNIT_FIELD_POWER1), max);
                if perc > p[0] || perc < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::FRIENDLY_HP => {
                if !in_combat {
                    return false;
                }
                #[allow(clippy::cast_precision_loss)]
                let Some(unit) = self.lowest_hp_friendly(guid, p[1] as f32, p[0], None) else {
                    return false;
                };
                action_invoker = Some(unit);
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::FRIENDLY_IS_CC => {
                // No CC auras are modeled yet.
                return false;
            }
            event_type::FRIENDLY_MISSING_BUFF => {
                #[allow(clippy::cast_precision_loss)]
                let found = self.get_target_by_type(
                    Some(guid),
                    None,
                    mangos_scripts_engine::script_target::FRIENDLY_MISSING_BUFF,
                    p[1],
                    p[0],
                    None,
                );
                let Some(unit) = found else { return false };
                action_invoker = Some(unit);
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::SUMMONED_UNIT
            | event_type::SUMMONED_JUST_DIED
            | event_type::SUMMONED_JUST_DESPAWN => {
                let Some(inv) = invoker.filter(|g| g.is_creature()) else {
                    return false;
                };
                if self.object_entry(inv) != Some(p[0]) {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[1], p[2]);
            }
            event_type::AURA => {
                if self.aura_stack_of(guid, p[0]) < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::TARGET_AURA => {
                let Some(v) = victim else { return false };
                if !in_combat || self.aura_stack_of(v, p[0]) < p[1] {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::MISSING_AURA => {
                if self.aura_stack_of(guid, p[0]) >= p[1] && p[1] > 0 {
                    return false;
                }
                if p[1] == 0 && self.aura_stack_of(guid, p[0]) > 0 {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::TARGET_MISSING_AURA => {
                let Some(v) = victim else { return false };
                if !in_combat {
                    return false;
                }
                let stack = self.aura_stack_of(v, p[0]);
                if stack >= p[1] && (p[1] > 0 || stack > 0) {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[2], p[3]);
            }
            event_type::VICTIM_ROOTED => {
                let Some(v) = victim else { return false };
                let rooted = self.aura_holders(v).iter().any(|h| {
                    h.effects
                        .iter()
                        .flatten()
                        .any(|e| e.aura_type == mangos_spells::defines::aura::MOD_ROOT)
                });
                if !rooted {
                    return false;
                }
                self.holder_repeat_timer(guid, idx, p[0], p[1]);
            }
            event_type::STEALTH_ALERT => {
                self.holder_repeat_timer(guid, idx, p[0], p[1]);
            }
            // AGGRO, DEATH, EVADE, LEAVE_COMBAT, SPAWNED,
            // REACHED_HOME, RECEIVE_EMOTE, SCRIPT,
            // GROUP_MEMBER_DIED: no extra gate.
            _ => {}
        }

        if ev.flags & EFLAG_REPEATABLE == 0
            && let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut())
        {
            rt.holders[idx].enabled = false;
        }
        if u32::from(ev.chance) <= self.rng.urand(0, 99) {
            return false;
        }

        let target = action_invoker.or(victim);
        let mut script_failed = false;
        if ev.flags & EFLAG_RANDOM_ACTION == 0 {
            for script_id in ev.action_scripts {
                if script_id != 0 && self.run_script_direct(script_id, Some(guid), target) {
                    script_failed = true;
                }
                if !self.creatures.contains_key(&guid) {
                    return true;
                }
            }
        } else {
            let present: Vec<u32> = ev
                .action_scripts
                .iter()
                .copied()
                .filter(|id| *id != 0)
                .collect();
            if !present.is_empty() {
                #[allow(clippy::cast_possible_truncation)]
                let pick = present[self.rng.urand(0, present.len() as u32 - 1) as usize];
                script_failed = self.run_script_direct(pick, Some(guid), target);
            }
        }

        if script_failed && ev.flags & EFLAG_CHECK_RESULT != 0 {
            if let Some(rt) = self.ai.get_mut(&guid).and_then(|ai| ai.eventai.as_mut()) {
                rt.holders[idx].enabled = true;
                rt.holders[idx].time_ms = 0;
            }
            return false;
        }
        true
    }

    /// Aura presence as a stack count (holders do not
    /// stack in the slice aura model: 0 or 1).
    fn aura_stack_of(&self, guid: ObjectGuid, spell_id: u32) -> u32 {
        u32::from(
            self.aura_holders(guid)
                .iter()
                .any(|h| h.spell_id == spell_id),
        )
    }

    // -- EventAI hooks called from other systems --

    /// `CreatureEventAI::JustDied` + the killer's
    /// `KilledUnit` (player victims only, as in C++).
    pub(crate) fn eventai_on_death(&mut self, victim: ObjectGuid, killer: Option<ObjectGuid>) {
        if victim.is_creature() {
            self.ensure_ai(victim);
            self.eventai_reset(victim);
            self.eventai_process_type(victim, event_type::DEATH, killer);
            if let Some(ai) = self.ai.get_mut(&victim) {
                ai.phase = 0;
            }
        }
        if let Some(k) = killer.filter(|g| g.is_creature())
            && victim.is_player()
        {
            self.eventai_process_type(k, event_type::KILL, Some(victim));
        }
    }

    /// `CreatureEventAI::SpellHit`.
    pub(crate) fn eventai_on_spell_hit(
        &mut self,
        creature: ObjectGuid,
        caster: ObjectGuid,
        spell_id: u32,
        school: u32,
    ) {
        if !self.eventai_active(creature) {
            return;
        }
        let count = self
            .ai
            .get(&creature)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let ev = {
                let rt = self
                    .ai
                    .get(&creature)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                rt.events[idx].clone()
            };
            if ev.event_type == event_type::HIT_BY_SPELL
                && (ev.params[0] == 0 || ev.params[0] == spell_id)
                && ev.params[1] & (1u32 << school) != 0
            {
                self.eventai_process_event(creature, idx, Some(caster));
            }
            if !self.creatures.contains_key(&creature) {
                return;
            }
        }
    }

    /// `CreatureEventAI::SpellHitTarget`.
    pub(crate) fn eventai_on_spell_hit_target(
        &mut self,
        creature: ObjectGuid,
        target: ObjectGuid,
        spell_id: u32,
        school: u32,
    ) {
        if !self.eventai_active(creature) {
            return;
        }
        let count = self
            .ai
            .get(&creature)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let ev = {
                let rt = self
                    .ai
                    .get(&creature)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                rt.events[idx].clone()
            };
            if ev.event_type == event_type::SPELL_HIT_TARGET
                && (ev.params[0] == 0 || ev.params[0] == spell_id)
                && ev.params[1] & (1u32 << school) != 0
            {
                self.eventai_process_event(creature, idx, Some(target));
            }
            if !self.creatures.contains_key(&creature) {
                return;
            }
        }
    }

    /// `EVENT_T_HIT_BY_AURA` (aura applied to the
    /// creature).
    pub(crate) fn eventai_on_hit_by_aura(
        &mut self,
        creature: ObjectGuid,
        aura_type_id: u32,
        caster: ObjectGuid,
    ) {
        if !self.eventai_active(creature) {
            return;
        }
        let count = self
            .ai
            .get(&creature)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let ev = {
                let rt = self
                    .ai
                    .get(&creature)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                rt.events[idx].clone()
            };
            if ev.event_type == event_type::HIT_BY_AURA && ev.params[0] == aura_type_id {
                self.eventai_process_event(creature, idx, Some(caster));
            }
            if !self.creatures.contains_key(&creature) {
                return;
            }
        }
    }

    /// `CreatureEventAI::MovementInform` + waypoint
    /// script starts, drained from the map event queue
    /// each AI tick.
    fn process_movement_informs(&mut self) {
        let informs: Vec<(ObjectGuid, u32, u32)> = self
            .events
            .iter()
            .filter_map(|e| match e {
                crate::map::MapEvent::MovementInform {
                    guid,
                    motion_type,
                    point_id,
                } => Some((*guid, *motion_type, *point_id)),
                _ => None,
            })
            .collect();
        for (guid, motion_type, point_id) in informs {
            // Hand-ported script hooks (Slice 7).
            if motion_type == crate::motion::POINT_MOTION_TYPE {
                if self
                    .ai
                    .get(&guid)
                    .and_then(|ai| ai.smite.as_ref())
                    .is_some_and(|s| s.in_spline)
                {
                    self.smite_spline_finished(guid);
                }
                let summoner_is_gunpowder = self.summons.get(&guid).is_some_and(|st| {
                    self.gameobjects
                        .get(&st.summoner)
                        .is_some_and(|go| go.info.script_name == "go_defias_gunpowder")
                });
                if summoner_is_gunpowder {
                    self.gunpowder_summon_arrived(guid, point_id);
                }
            }
            if !self.eventai_active(guid) {
                continue;
            }
            let count = self
                .ai
                .get(&guid)
                .and_then(|ai| ai.eventai.as_ref())
                .map_or(0, |rt| rt.events.len());
            for idx in 0..count {
                let ev = {
                    let rt = self
                        .ai
                        .get(&guid)
                        .and_then(|ai| ai.eventai.as_ref())
                        .expect("rt");
                    rt.events[idx].clone()
                };
                if ev.event_type == event_type::MOVEMENT_INFORM
                    && ev.params[0] == motion_type
                    && ev.params[1] == point_id
                {
                    self.eventai_process_event(guid, idx, None);
                }
                if !self.creatures.contains_key(&guid) {
                    break;
                }
            }
        }
        let scripts: Vec<(ObjectGuid, u32)> = self
            .events
            .iter()
            .filter_map(|e| match e {
                crate::map::MapEvent::WaypointScript {
                    guid, script_id, ..
                } => Some((*guid, *script_id)),
                _ => None,
            })
            .collect();
        for (guid, script_id) in scripts {
            if let Some(steps) = self
                .stores
                .scripts
                .creature_movement_scripts
                .get(&script_id)
                .cloned()
            {
                self.schedule_steps(&steps, Some(guid), None);
            }
        }
    }

    /// `CreatureEventAI::OnScriptEventHappened`.
    pub(crate) fn eventai_on_script_event(
        &mut self,
        creature: ObjectGuid,
        event_id: u32,
        data: u32,
        invoker: Option<ObjectGuid>,
    ) {
        if !self.eventai_active(creature) {
            return;
        }
        let count = self
            .ai
            .get(&creature)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let ev = {
                let rt = self
                    .ai
                    .get(&creature)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                rt.events[idx].clone()
            };
            if ev.event_type == event_type::SCRIPT
                && ev.params[0] == event_id
                && ev.params[1] == data
            {
                self.eventai_process_event(creature, idx, invoker);
            }
            if !self.creatures.contains_key(&creature) {
                return;
            }
        }
    }

    /// `CreatureEventAI::ReceiveEmote` for the one
    /// creature the player's text emote targeted.
    pub(crate) fn eventai_receive_emote(
        &mut self,
        creature: ObjectGuid,
        player: ObjectGuid,
        text_emote: u32,
    ) {
        if !self.eventai_active(creature) {
            return;
        }
        let count = self
            .ai
            .get(&creature)
            .and_then(|ai| ai.eventai.as_ref())
            .map_or(0, |rt| rt.events.len());
        for idx in 0..count {
            let ev = {
                let rt = self
                    .ai
                    .get(&creature)
                    .and_then(|ai| ai.eventai.as_ref())
                    .expect("rt");
                rt.events[idx].clone()
            };
            if ev.event_type == event_type::RECEIVE_EMOTE && ev.params[0] == text_emote {
                self.eventai_process_event(creature, idx, Some(player));
            }
            if !self.creatures.contains_key(&creature) {
                return;
            }
        }
    }

    /// Flee driving: relaunch an away-point roughly
    /// every second until the flee timer expires (the
    /// `FleeingMovementGenerator` approximation).
    /// Returns true while fleeing.
    fn update_fleeing(&mut self, guid: ObjectGuid) -> bool {
        let (until, from, next) = {
            let Some(ai) = self.ai.get(&guid) else {
                return false;
            };
            (ai.flee_until_ms, ai.flee_from, ai.flee_next_ms)
        };
        if until == 0 || until <= self.now_ms {
            if until != 0
                && let Some(ai) = self.ai.get_mut(&guid)
            {
                ai.flee_until_ms = 0;
            }
            return false;
        }
        if self.now_ms >= next {
            let pos = self.unit_position(guid).unwrap_or_default();
            let from_pos = self.unit_position(from).unwrap_or(pos);
            let (mut dx, mut dy) = (pos.x - from_pos.x, pos.y - from_pos.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.01 {
                dx = 1.0;
                dy = 0.0;
            } else {
                dx /= len;
                dy /= len;
            }
            let flee_dist = self.rng.frand(12.0, 22.0);
            let jitter = self.rng.frand(-4.0, 4.0);
            let dest = Position {
                x: pos.x + dx * flee_dist - dy * jitter,
                y: pos.y + dy * flee_dist + dx * jitter,
                z: pos.z,
                o: 0.0,
            };
            let dest = Position {
                z: self.ground_z(dest.x, dest.y, dest.z),
                ..dest
            };
            self.creature_move_point(guid, 0, dest, 0, 0.0, -10.0);
            if let Some(ai) = self.ai.get_mut(&guid) {
                ai.flee_next_ms = self.now_ms + 1000;
            }
        }
        true
    }

    /// Recompute the `UNIT_FIELD_MINDAMAGE` family from
    /// the base ranges and the current AP factor
    /// (`Creature::UpdateDamagePhysical` AP-scale arm).
    pub(crate) fn refresh_weapon_damage_fields(&mut self, guid: ObjectGuid) {
        let Some(u) = self.unit_base_mut(guid) else {
            return;
        };
        let (base_ap, mods) = (u.base_attack_power, u.attack_power_mods);
        let ranges = if let Some(c) = self.creatures.get(&guid) {
            c.base_weapon_damage
        } else {
            [(1.0, 2.0), (0.5, 1.0), (1.0, 2.0)]
        };
        let ap_now = base_ap + mods[0] + mods[1];
        let factor = if base_ap > 0.0 {
            0.7 + 0.3 * (ap_now / base_ap)
        } else {
            1.0
        };
        if let Some(u) = self.unit_base_mut(guid) {
            let v = u.values();
            v.set_f32(offsets::UNIT_FIELD_MINDAMAGE, ranges[0].0 * factor);
            v.set_f32(offsets::UNIT_FIELD_MAXDAMAGE, ranges[0].1 * factor);
            v.set_f32(offsets::UNIT_FIELD_MINOFFHANDDAMAGE, ranges[1].0 * factor);
            v.set_f32(offsets::UNIT_FIELD_MAXOFFHANDDAMAGE, ranges[1].1 * factor);
            v.set_f32(offsets::UNIT_FIELD_MINRANGEDDAMAGE, ranges[2].0 * factor);
            v.set_f32(offsets::UNIT_FIELD_MAXRANGEDDAMAGE, ranges[2].1 * factor);
        }
    }
}

/// `MeleeHitOutcome` subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeleeOutcome {
    Normal,
    Miss,
    Dodge,
    Parry,
    Glancing,
    Block,
    Crit,
    Crushing,
}
