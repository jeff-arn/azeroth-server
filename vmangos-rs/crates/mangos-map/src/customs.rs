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
    fn current_victim(&self, guid: ObjectGuid) -> Option<ObjectGuid> {
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
    fn combat_stop(&mut self, guid: ObjectGuid, including_cast: bool) {
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
    fn creature_update_entry(&mut self, guid: ObjectGuid, new_entry: u32) {
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

    /// `Spell::EffectDummy` per-victim unit branch.
    /// `damage` is the fork's per-effect
    /// `CalculateDamage` value.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    pub(crate) fn effect_dummy(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        victim: ObjectGuid,
        damage: f32,
        has_cast_item: bool,
        cast_counter: u8,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
    ) {
        match spell.id {
            // Deep Wounds (SpellEffects.cpp:696-730).
            12162 | 12850 | 12868 => {
                let Some(unit) = self.unit_base(caster) else {
                    return;
                };
                let field = |idx: u16| f32::from_bits(unit.world.object.values().get_u32(idx));
                let have_offhand = field(offsets::UNIT_FIELD_MINOFFHANDDAMAGE) > 0.0;
                #[allow(clippy::manual_midpoint)]
                let mut avg = if have_offhand && unit.attack_timers[0] > unit.attack_timers[1] {
                    (field(offsets::UNIT_FIELD_MINOFFHANDDAMAGE)
                        + field(offsets::UNIT_FIELD_MAXOFFHANDDAMAGE))
                        / 2.0
                } else {
                    (field(offsets::UNIT_FIELD_MINDAMAGE) + field(offsets::UNIT_FIELD_MAXDAMAGE))
                        / 2.0
                };
                avg *= match spell.id {
                    12162 => 0.2,
                    12850 => 0.4,
                    _ => 0.6,
                };
                #[allow(clippy::cast_possible_truncation)]
                let bp0 = (avg / 4.0) as i32;
                self.cast_custom_spell(caster, victim, 12721, [Some(bp0), None, None]);
            }
            // Last Stand (SpellEffects.cpp:731-739).
            12975 => {
                #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
                let bp0 = (self.unit_value_u32(caster, offsets::UNIT_FIELD_MAXHEALTH) as f32 * 0.3)
                    as i32;
                self.cast_custom_spell(caster, caster, 12976, [Some(bp0), None, None]);
            }
            // Tame Beast (SpellEffects.cpp:758-770).
            13535 => {
                if !caster.is_player() {
                    return;
                }
                let Some(cast) = self.casts.get(&caster) else {
                    return;
                };
                let CastTarget::Unit(beast) = cast.target else {
                    return;
                };
                if !cast.channeled || !beast.is_creature() {
                    return;
                }
                self.cast_custom_spell(caster, beast, 13481, [None; 3]);
            }
            // Preparation (SpellEffects.cpp:788-797).
            14185 => {
                if !caster.is_player() {
                    return;
                }
                self.remove_some_cooldowns(caster, |entry| {
                    entry.spell_family_name == 8
                        && entry.recovery_time.max(entry.category_recovery_time) > 0
                });
            }
            // Blood Fury (SpellEffects.cpp:920-934).
            20572 => {
                if !caster.is_player() {
                    return;
                }
                self.cast_custom_spell(caster, caster, 23230, [None; 3]);
                #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
                let ap = self.unit_value_u32(caster, offsets::UNIT_FIELD_ATTACK_POWER) as i32;
                #[allow(clippy::cast_precision_loss)]
                let dmg = mangos_spells::dither(damage * ap as f32 / 100.0, self.spell_rng());
                if dmg > 0 {
                    self.cast_custom_spell(caster, caster, 23234, [Some(dmg), None, None]);
                }
            }
            // Universal Remote
            // (SpellEffects.cpp:475-498).
            8344 => match self.urand(0, 2) {
                0 => self.cast_custom_spell(caster, victim, 8345, [None; 3]),
                1 => self.cast_custom_spell(caster, victim, 8346, [None; 3]),
                _ => self.cast_custom_spell(victim, victim, 8599, [None; 3]),
            },
            // Symbol of Life (SpellEffects.cpp:669-676):
            // JUST_ALIVED promotion subset.
            8593 => {
                let Some(creature) = self.creatures.get_mut(&victim) else {
                    return;
                };
                if creature.unit.is_alive() {
                    return;
                }
                creature.unit.death_state = DeathState::Alive;
                if creature.unit.health() == 0 {
                    let max = creature.unit.max_health();
                    creature.unit.set_health(max);
                }
                creature
                    .unit
                    .values()
                    .set_u32(offsets::UNIT_DYNAMIC_FLAGS, 0);
                let npc_flags = creature.info.npc_flags;
                creature
                    .unit
                    .values()
                    .set_u32(offsets::UNIT_NPC_FLAGS, npc_flags);
            }
            // Capture spells + Summon Screecher
            // Spirit (SpellEffects.cpp:419-429):
            // despawn the dead creature (fork delays
            // 1000ms; immediate here).
            11885 | 11886 | 11887 | 11888 | 11889 | 12699 => {
                if victim.is_creature() && !self.unit_is_alive(victim) {
                    self.despawn_creature_now(victim, 0);
                }
            }
            // Shrink Ray (SpellEffects.cpp:499-536).
            13006 => {
                let r = self.urand(0, 99);
                if r > 15 {
                    self.cast_custom_spell(caster, victim, 13003, [None; 3]);
                } else if r > 13 {
                    self.add_aura_and_go(caster, caster, 13004, cast_counter);
                } else if r > 11 {
                    self.add_aura_and_go(caster, caster, 13010, cast_counter);
                } else if r > 9 {
                    self.add_aura_and_go(caster, victim, 13004, cast_counter);
                } else if r > 4 {
                    self.cast_custom_spell(caster, victim, 13010, [None; 3]);
                } else {
                    self.cast_custom_spell(caster, victim, 13004, [None; 3]);
                }
            }
            // Net-o-Matic (SpellEffects.cpp:740-757).
            13120 => {
                let roll = self.urand(0, 99);
                let spell_id = if roll < 10 {
                    16566
                } else if roll < 20 {
                    13119
                } else {
                    13099
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Gnomish Mind Control Cap
            // (SpellEffects.cpp:538-559).
            13180 => match self.urand(0, 5) {
                0 => {}
                1 => {
                    if !self.unit_shapeshifted(caster) {
                        self.cast_custom_spell(victim, caster, 13181, [None; 3]);
                    }
                }
                _ => self.cast_custom_spell(caster, victim, 13181, [None; 3]),
            },
            // Six Demon Bag (SpellEffects.cpp:570-602).
            14537 => {
                let roll = self.urand(0, 99);
                let (spell_id, target) = if roll < 25 {
                    (11921, victim)
                } else if roll < 50 {
                    (13322, victim)
                } else if roll < 70 {
                    (21179, victim)
                } else if roll < 77 {
                    (13323, victim)
                } else if roll < 80 {
                    (13323, caster)
                } else if roll < 95 {
                    (25189, victim)
                } else {
                    (14642, caster)
                };
                self.cast_custom_spell(caster, target, spell_id, [None; 3]);
            }
            // Test Fetid Skull
            // (SpellEffects.cpp:821-835). The fork
            // gate reads the cast's item *target*,
            // which is not plumbed; the cast-item
            // flag stands in (identical for the
            // item-use path that reaches this).
            17271 => {
                if !has_cast_item && !caster.is_player() {
                    return;
                }
                let spell_id = if self.roll_chance_i(50) { 17269 } else { 17270 };
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Snowball (SpellEffects.cpp:1286-1297).
            21343 => {
                if victim.is_player()
                    && caster.is_player()
                    && !self.unit_has_aura(victim, 21354)
                    && self.in_same_raid(victim, caster)
                {
                    self.cast_custom_spell(victim, victim, 21167, [None; 3]);
                }
            }
            // Engineering dragonlings / battle
            // chicken (SpellEffects.cpp:988-1019).
            23074 | 23075 | 23076 | 23133 => {
                if !has_cast_item {
                    return;
                }
                let spell_id = match spell.id {
                    23074 => 19804,
                    23075 => 12749,
                    23076 => 4073,
                    _ => 13166,
                };
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Goblin Bomb Dispenser
            // (SpellEffects.cpp:1020-1033).
            23134 => {
                if !has_cast_item {
                    return;
                }
                if self.urand(0, 99) < 10 {
                    self.cast_custom_spell(victim, caster, 13261, [None; 3]);
                } else {
                    self.cast_custom_spell(caster, caster, 13258, [None; 3]);
                }
            }
            // Transporter Arrival
            // (SpellEffects.cpp:1058-1070).
            23448 => {
                let r = self.spell_rng().irand(0, 119);
                if r < 20 {
                    self.cast_custom_spell(caster, caster, 23444, [None; 3]);
                } else if r < 100 {
                    self.cast_custom_spell(caster, caster, 23445, [None; 3]);
                }
            }
            // Ultrasafe Transporter: Gadgetzan
            // (SpellEffects.cpp:1071-1091).
            23453 => {
                #[allow(clippy::same_functions_in_if_condition)]
                // fork draws the second roll only on failure
                if self.roll_chance_i(50) {
                    self.cast_custom_spell(caster, caster, 23441, [None; 3]);
                } else if self.roll_chance_i(50) {
                    self.cast_custom_spell(caster, caster, 23446, [None; 3]);
                } else if caster.is_player() {
                    #[allow(clippy::excessive_precision)]
                    self.teleport_to_location(
                        caster,
                        WorldLocation {
                            map_id: 1,
                            x: -7341.38,
                            y: -3908.11,
                            z: 150.7,
                            o: 0.51,
                        },
                    );
                }
            }
            // Hourglass Sand
            // (SpellEffects.cpp:1092-1095).
            23645 => {
                self.remove_auras_due_to_spell(caster, 23170);
            }
            // Gift of Life
            // (SpellEffects.cpp:1096-1102).
            23725 => {
                #[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
                let bp0 = (self.unit_value_u32(caster, offsets::UNIT_FIELD_MAXHEALTH) as f32 * 0.15)
                    as i32;
                self.cast_custom_spell(caster, caster, 23782, [Some(bp0), None, None]);
            }
            // Hallow's End Treat
            // (SpellEffects.cpp:1123-1131).
            24930 => {
                let spell_id = self.pick_spell(&[24924, 24925, 24926, 24927]);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Reindeer Transformation
            // (SpellEffects.cpp:1132-1152).
            25860 => {
                let mounted = self
                    .aura_holders(caster)
                    .iter()
                    .flat_map(|h| h.effects.iter().flatten())
                    .any(|e| e.aura_type == aura_type::MOUNTED);
                if !mounted {
                    return;
                }
                let speed = self
                    .unit_base(caster)
                    .map_or(0.0, |u| u.speed_rate[MOVE_RUN]);
                self.remove_spells_causing_aura(caster, aura_type::MOUNTED);
                let spell_id = if speed >= 2.0 { 25859 } else { 25858 };
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Friendship Bracelet
            // (SpellEffects.cpp:1298-1307).
            26899 => {
                if self.unit_has_aura(victim, 26898) {
                    self.remove_auras_due_to_spell(victim, 26898);
                    self.cast_custom_spell(victim, victim, 26921, [None; 3]);
                    self.cast_custom_spell(caster, caster, 26664, [None; 3]);
                }
            }
            // Silver Shafted Arrow
            // (SpellEffects.cpp:1308-1316). The
            // minipet presence check has no model;
            // treated as absent.
            27662 => {
                if victim.is_player() {
                    self.cast_custom_spell(victim, victim, 27570, [None; 3]);
                }
            }
            // Call of the Ashbringer
            // (SpellEffects.cpp:1266-1273).
            28414 => {
                if !caster.is_player() {
                    return;
                }
                let sound = self.pick_spell(&[
                    8906, 8907, 8908, 8920, 8921, 8922, 8923, 8924, 8925, 8926, 8927, 8928,
                ]);
                self.play_direct_sound(caster, sound, Some(caster));
            }
            // Polly Eats the E.C.A.C.
            // (SpellEffects.cpp:685-695; fork
            // despawn delay 100ms, immediate here).
            9976 => {
                if victim.is_creature() {
                    self.cast_custom_spell(victim, victim, 9998, [None; 3]);
                    self.despawn_creature_now(victim, 0);
                }
            }
            // Curse of the Eye
            // (SpellEffects.cpp:612-621).
            3360 => {
                let spell_id = if self.unit_is_male(victim) {
                    10651
                } else {
                    10653
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Explosive Sheep
            // (SpellEffects.cpp:1034-1045): the
            // player owner takes the creating
            // spell's cooldown.
            4050 => {
                let owner = self
                    .summons
                    .get(&caster)
                    .map(|s| s.summoner)
                    .or_else(|| self.pet_owner(caster));
                let Some(owner) = owner.filter(|g| g.is_player()) else {
                    return;
                };
                let created_by = self.unit_value_u32(caster, offsets::UNIT_CREATED_BY_SPELL);
                if let Some(entry) = self.stores.spells.get(created_by).cloned() {
                    let mut mods = Vec::new();
                    self.add_cooldowns(owner, &entry, &mut mods, None);
                    self.remove_exhausted_spell_mods(owner, &mods);
                }
            }
            // Force Target - Bow / Force Self - Bow
            // (SpellEffects.cpp:447-458).
            6655 => self.handle_emote_command(victim, 2),
            25716 => self.handle_emote_command(caster, 2),
            // Pyrewood Transformation
            // (SpellEffects.cpp:622-668).
            7671 => {
                let Some(entry) = self.creatures.get(&victim).map(|c| c.info.entry) else {
                    return;
                };
                let new_entry = match entry {
                    1891 => 1892,
                    1892 => 1891,
                    1893 => 1894,
                    1894 => 1893,
                    1895 => 1896,
                    1896 => 1895,
                    3528 => 3529,
                    3529 => 3528,
                    3530 => 3531,
                    3531 => 3530,
                    3532 => 3533,
                    3533 => 3532,
                    _ => return,
                };
                self.creature_update_entry(victim, new_entry);
            }
            // Destroy Rocket Boots
            // (SpellEffects.cpp:1241-1245).
            8897 => {
                self.cast_custom_spell(caster, victim, 8893, [None; 3]);
            }
            // Coarse Dynamite (SpellEffects.cpp:347-356;
            // fork defers one tick via a lambda event).
            9009 => {
                let spell_id = self.pick_spell(&[9002, 9003, 9004]);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Hate to Zero (SpellEffects.cpp:677-684).
            9204 => {
                self.modify_threat_percent_of(caster, victim, -100);
            }
            // Dummy Trigger
            // (SpellEffects.cpp:771-787).
            13567 => {
                if triggered_by_aura_spell == 26467 {
                    #[allow(clippy::cast_possible_truncation)]
                    let bp0 = damage as i32;
                    self.cast_custom_spell(caster, victim, 26470, [Some(bp0), None, None]);
                }
            }
            // Capture Worg Pup
            // (SpellEffects.cpp:798-805).
            15998 | 18954 => {
                if victim.is_creature() {
                    self.despawn_creature_now(victim, 0);
                }
            }
            // Merging Oozes
            // (SpellEffects.cpp:1366-1376).
            16032 => {
                if !victim.is_creature() || !caster.is_creature() {
                    return;
                }
                if let Some(pos) = self.unit_position(caster) {
                    self.summon_creature(9621, pos, caster, 1, 420_000);
                }
                self.despawn_or_unsummon(caster);
                self.despawn_or_unsummon(victim);
            }
            // Voodoo (SpellEffects.cpp:603-611).
            17009 => {
                if !victim.is_player() {
                    return;
                }
                let spell_id = self.pick_spell(&[16707, 16708, 16709, 16711, 16712, 16713, 16716]);
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Ras Frostwhisper Visual Dummy
            // (SpellEffects.cpp:1357-1365).
            17190 => {
                self.cast_custom_spell(victim, victim, 17186, [None; 3]);
                let max = self.unit_value_u32(victim, offsets::UNIT_FIELD_MAXHEALTH);
                if let Some(u) = self.unit_base_mut(victim) {
                    u.set_health(max);
                }
            }
            // Shadow Portal
            // (SpellEffects.cpp:836-850).
            17950 => {
                if self.threat_on(caster, victim) != 0.0 {
                    self.modify_threat_percent_of(caster, victim, -100);
                }
                let spell_id = self.pick_spell(&[17863, 17939, 17943, 17944, 17946, 17948]);
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Dummy Trigger (shaman T3)
            // (SpellEffects.cpp:851-877).
            18350 => {
                if !victim.is_player() || triggered_by_spell != 28820 {
                    return;
                }
                let has_shield = self.aura_holders(victim).iter().any(|h| {
                    self.stores.spells.get(h.spell_id).is_some_and(|s| {
                        s.spell_family_name == 11 && s.spell_family_flags & 0x400 != 0
                    })
                });
                if !has_shield {
                    self.remove_auras_due_to_spell(victim, 28820);
                }
            }
            // Ranshalla's Torch/Altar Trap
            // (SpellEffects.cpp:316-322).
            18955 | 18993 => {
                self.remove_auras_due_to_spell(victim, 18953);
            }
            // Lava Bomb (SpellEffects.cpp:878-893).
            19411 => {
                self.cast_custom_spell(victim, victim, 20494, [None; 3]);
            }
            20474 => {
                self.cast_custom_spell(victim, victim, 20495, [None; 3]);
            }
            // Egg Explosion, Buru
            // (SpellEffects.cpp:895-918).
            19593 => {
                let dmg = if victim.is_player() {
                    let dist = self.combat_distance(caster, victim).min(25.0);
                    mangos_spells::dither((25.0 - dist) / 25.0 * 400.0 + 100.0, self.spell_rng())
                } else if self.creatures.get(&victim).map(|c| c.info.entry) == Some(15370) {
                    #[allow(clippy::cast_precision_loss)]
                    let health = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH) as f32;
                    let dmg = mangos_spells::dither(health * 15.0 / 100.0, self.spell_rng());
                    if let Some(mv) = self.current_victim(victim) {
                        self.modify_threat_percent_of(victim, mv, -100);
                    }
                    dmg
                } else {
                    return;
                };
                self.cast_custom_spell(caster, victim, 5255, [Some(dmg), None, None]);
            }
            // Despawn Caster
            // (SpellEffects.cpp:440-446).
            19614 => {
                if victim.is_creature() {
                    self.despawn_or_unsummon(victim);
                }
            }
            // Arcane Vacuum
            // (SpellEffects.cpp:935-949).
            21147 => {
                if self.threat_on(caster, victim) != 0.0 {
                    self.modify_threat_percent_of(caster, victim, -100);
                }
                self.cast_custom_spell(caster, victim, 21150, [None; 3]);
            }
            // Gate of Shazzrah
            // (SpellEffects.cpp:1046-1057).
            23138 => {
                self.cast_custom_spell(caster, victim, 23139, [None; 3]);
            }
            // Aura of Frost / Aura of Nature
            // (SpellEffects.cpp:1246-1265).
            23185 | 25044 => {
                if self.unit_has_aura(victim, 25040) {
                    self.cast_custom_spell(victim, victim, 25043, [None; 3]);
                }
                if self.unit_has_aura(victim, 23182) {
                    self.cast_custom_spell(victim, victim, 23186, [None; 3]);
                }
                self.remove_auras_at_mechanic_immunity(victim, 1 << 15, 0, false);
                self.remove_spells_causing_aura(victim, aura_type::MOD_STEALTH);
                if self.unit_base(victim).is_some_and(|u| !u.is_standing_up()) {
                    self.unit_set_byte(victim, offsets::UNIT_FIELD_BYTES_1, 0, 0);
                }
            }
            // Attract Jubjub
            // (SpellEffects.cpp:1334-1343). The
            // contact point is approximated by the
            // gameobject's position.
            23845 => {
                let Some(go) = self.nearest_gameobject_with_entry(caster, 165_578, 3.0) else {
                    return;
                };
                let (Some(dest), Some(vp)) = (self.object_position(go), self.unit_position(victim))
                else {
                    return;
                };
                let speed = self.unit_base(victim).map_or(0.0, |u| u.speeds()[0]);
                let dest = Position {
                    x: dest.x,
                    y: dest.y,
                    z: vp.z,
                    o: vp.o,
                };
                self.creature_move_point(victim, 1, dest, 0x002, speed, vp.o);
            }
            // Stinger Charge Primer
            // (SpellEffects.cpp:1103-1114).
            24150 => {
                let spell_id = if self.unit_has_aura(victim, 25187) {
                    25191
                } else {
                    25190
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Dream Fog (SpellEffects.cpp:1115-1122).
            24781 => {
                if caster.is_creature() {
                    self.enter_combat(caster, victim);
                }
            }
            // Viscidus explode
            // (SpellEffects.cpp:1153-1177).
            25938 => {
                if self.health_pct_of(caster) < 5.0 {
                    self.cast_custom_spell(caster, caster, 26003, [None; 3]);
                    return;
                }
                self.combat_stop(caster, false);
                if let Some(u) = self.unit_base_mut(caster) {
                    let flags = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
                    u.values()
                        .set_u32(offsets::UNIT_FIELD_FLAGS, flags | 0x0000_0002);
                    let dyn_flags = u.values().get_u32(offsets::UNIT_DYNAMIC_FLAGS);
                    u.values()
                        .set_u32(offsets::UNIT_DYNAMIC_FLAGS, dyn_flags | 0x0020);
                    u.add_unit_state(0x0000_0004);
                }
                self.cast_custom_spell(caster, caster, 25885, [None; 3]);
            }
            // Viscidus suicide
            // (SpellEffects.cpp:1178-1188).
            26003 => {
                if self.unit_is_alive(caster) {
                    self.cast_custom_spell(caster, caster, 26002, [None; 3]);
                }
            }
            // Despawn Tentacles
            // (SpellEffects.cpp:1192-1198). Fork
            // damage carries no school mask; the
            // direct-damage funnel's NORMAL school
            // stands in.
            26399 => {
                let health = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
                self.deal_damage(caster, victim, health, false);
            }
            // Mana Burn Area
            // (SpellEffects.cpp:1199-1206).
            26626 => {
                if self
                    .unit_base(victim)
                    .map_or(0, mangos_entities::unit::UnitBase::power_type)
                    == 0
                {
                    self.cast_custom_spell(caster, victim, 25779, [None; 3]);
                }
            }
            // Remove Amorous
            // (SpellEffects.cpp:1352-1356).
            26879 => {
                self.remove_auras_due_to_spell(victim, 26869);
            }
            // Nature's Bounty
            // (SpellEffects.cpp:459-474).
            27798 => {
                let spell_id = match self
                    .unit_base(victim)
                    .map_or(0, mangos_entities::unit::UnitBase::power_type)
                {
                    1 => 27783,
                    3 => 27784,
                    0 => 27782,
                    _ => return,
                };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Arcane Cloaking
            // (SpellEffects.cpp:1207-1214).
            28006 => {
                if victim.is_player() {
                    self.cast_custom_spell(caster, victim, 29296, [None; 3]);
                }
            }
            // Scourge Invasion despawner
            // (SpellEffects.cpp:357-362).
            28091 => {
                if self.unit_base(caster).is_some_and(|u| !u.is_in_combat()) {
                    self.cast_custom_spell(caster, caster, 17680, [None; 3]);
                }
            }
            // Stalagg/Feugen Tesla Effect
            // (SpellEffects.cpp:1215-1240).
            28098 | 28110 => {
                if !victim.is_creature() {
                    return;
                }
                let stalagg = spell.id == 28098;
                if self.current_victim(caster).is_some() && !self.within_dist(caster, victim, 60.0)
                {
                    let targets = self.threat_targets(caster);
                    if !targets.is_empty() {
                        #[allow(clippy::cast_possible_truncation)]
                        let idx = self.urand(0, targets.len() as u32 - 1) as usize;
                        self.cast_custom_spell(victim, targets[idx], 28099, [None; 3]);
                    }
                } else {
                    self.remove_auras_due_to_spell(victim, if stalagg { 28097 } else { 28109 });
                    if let Some(ai) = self.ai.get_mut(&victim) {
                        ai.threat.clear_references();
                    }
                    self.combat_stop(victim, true);
                    self.cast_custom_spell(
                        victim,
                        caster,
                        if stalagg { 28096 } else { 28111 },
                        [None; 3],
                    );
                }
            }
            // Scourge Invasion communique
            // (SpellEffects.cpp:363-367).
            28345 => {
                self.cast_custom_spell(victim, victim, 28281, [None; 3]);
            }
            // Forgiveness (SpellEffects.cpp:1278-1285).
            28697 => {
                self.kill_unit(victim, Some(caster));
            }
            // Silithus Flag Click
            // (SpellEffects.cpp:1344-1351); the PvP
            // contested flag has no model here.
            29518 => {
                self.cast_custom_spell(victim, victim, 29519, [None; 3]);
            }
            // EPL tower kill credit
            // (SpellEffects.cpp:368-375).
            32061 => {
                if !caster.is_player() {
                    return;
                }
                if let Some(entry) = self.creatures.get(&victim).map(|c| c.info.entry) {
                    self.quest_kill_credit_one(caster, ObjectGuid::EMPTY, entry);
                }
            }
            _ => {
                // Berserking (icon switch,
                // SpellEffects.cpp:1382-1399). The
                // aura-state write is unmodeled (no
                // aura-state store).
                if spell.spell_family_name == 0 && spell.spell_icon_id == 1661 {
                    #[allow(clippy::cast_possible_truncation)]
                    let health_pct = self.health_pct_of(caster) as i32;
                    let melee_mod: i32 = if health_pct <= 40 {
                        30
                    } else if health_pct < 100 {
                        10 + (100 - health_pct) / 3
                    } else {
                        10
                    };
                    self.cast_custom_spell(
                        caster,
                        caster,
                        26635,
                        [Some(melee_mod), Some(melee_mod), Some(melee_mod)],
                    );
                }
            }
        }
        // `Spell::EffectDummy` tail (`SpellEffects.cpp:1447`):
        // creature-script dummy for creature victims (the
        // gameobject-target arm has no Rust call site —
        // GO spell targets are unmodeled).
        if victim.is_creature() {
            self.effect_dummy_creature(caster, spell.id, eff_index, victim);
        }
    }

    /// Shrink Ray's `AddAura` + `SendSpellGo` pair.
    fn add_aura_and_go(
        &mut self,
        caster: ObjectGuid,
        target: ObjectGuid,
        spell_id: u32,
        cast_counter: u8,
    ) {
        if let Some(entry) = self.stores.spells.get(spell_id).cloned() {
            self.apply_spell_aura(caster, &entry, target, None, [None; 3]);
        }
        self.send_spell_go_simple(caster, spell_id, target, cast_counter);
    }

    /// `Spell::EffectScriptEffect`.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn effect_script_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        damage: f32,
        has_cast_item: bool,
    ) {
        // Family arms (fork switches on family
        // first; generic ids never overlap).
        match spell.spell_family_name {
            // SPELLFAMILY_PALADIN
            // (SpellEffects.cpp:4378-4437).
            10 => {
                self.script_paladin(caster, spell, victim, damage);
                return;
            }
            // SPELLFAMILY_DRUID shapeshift form
            // effect (SpellEffects.cpp:4338-4377).
            7 if spell.id == 9033 => {
                self.script_shapeshift_form(victim);
                return;
            }
            _ => {}
        }
        match spell.id {
            // Rockbiter Weapon Proc threat
            // (SHAMAN arm, SpellEffects.cpp:4441-4456).
            20865 | 20866 | 20867 | 20868 | 20870 | 20871 => {
                self.threat_from_weapon_proc(caster, victim, damage);
            }
            // Bending Shinbone
            // (SpellEffects.cpp:3586-3594). Item
            // *target* gate approximated by the
            // cast-item flag (see 17271).
            8856 => {
                if !has_cast_item && !caster.is_player() {
                    return;
                }
                let spell_id = if self.urand(1, 5) == 1 { 8854 } else { 8855 };
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Jubling Cooldown
            // (SpellEffects.cpp:3638-3646).
            23853 => {
                if let Some(entry) = self.stores.spells.get(23851).cloned() {
                    let ctx = CastItemCtx {
                        item_guid: 0,
                        item_entry: 19462,
                    };
                    let mut mods = Vec::new();
                    self.add_cooldowns_item(victim, &entry, ctx, &mut mods);
                }
            }
            // Uther's / Grom's Tribute
            // (SpellEffects.cpp:3647-3698).
            24194 | 24195 => {
                if !caster.is_player() {
                    return;
                }
                let race = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::race);
                let spell_id = match (spell.id, race) {
                    (24194, 1) => 24105,
                    (24194, 3) => 24107,
                    (24194, 4) => 24108,
                    (24194, 7) => 24106,
                    (24195, 2) => 24104,
                    (24195, 5) => 24103,
                    (24195, 6) => 24102,
                    (24195, 8) => 24101,
                    _ => 0,
                };
                if spell_id != 0 {
                    self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
                }
            }
            // Stack burns
            // (SpellEffects.cpp:3722-3727, 3952-3954).
            24590 => {
                self.remove_aura_holder_from_stack(victim, 24575);
            }
            24660 => {
                self.remove_aura_holder_from_stack(victim, 24659);
            }
            26465 => {
                self.remove_aura_holder_from_stack(victim, 26464);
            }
            // Trick (SpellEffects.cpp:3739-3761).
            24714 => {
                if !caster.is_player() {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spells: [u32; 8] = [
                    if male { 24708 } else { 24709 },
                    if male { 24711 } else { 24710 },
                    if male { 24712 } else { 24713 },
                    if male { 24735 } else { 24736 },
                    24723,
                    24732,
                    24740,
                    24753,
                ];
                let spell_id = self.pick_spell(&spells);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Hallowed wand costumes
            // (SpellEffects.cpp:3762-3860).
            24717 | 24718 | 24719 | 24737 => {
                if !victim.is_player()
                    || self
                        .unit_base(victim)
                        .is_some_and(mangos_entities::unit::UnitBase::is_in_combat)
                {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spell_id = match (spell.id, male) {
                    (24717, true) => 24708,
                    (24717, false) => 24709,
                    (24718, true) => 24711,
                    (24718, false) => 24710,
                    (24719, true) => 24712,
                    (24719, false) => 24713,
                    (24737, true) => 24735,
                    _ => 24736,
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Random Costume
            // (SpellEffects.cpp:3801-3839).
            24720 => {
                if !victim.is_player()
                    || self
                        .unit_base(victim)
                        .is_some_and(mangos_entities::unit::UnitBase::is_in_combat)
                {
                    return;
                }
                let male = self.unit_is_male(victim);
                let spell_id = match self.urand(0, 6) {
                    0 => {
                        if male {
                            24708
                        } else {
                            24709
                        }
                    }
                    1 => {
                        if male {
                            24711
                        } else {
                            24710
                        }
                    }
                    2 => {
                        if male {
                            24712
                        } else {
                            24713
                        }
                    }
                    3 => 24723,
                    4 => 24732,
                    5 => {
                        if male {
                            24735
                        } else {
                            24736
                        }
                    }
                    _ => 24740,
                };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Cannon Fire / Magic Wings
            // (SpellEffects.cpp:3840-3868).
            24731 => {
                if caster.is_player() {
                    self.cast_custom_spell(victim, caster, 24742, [None; 3]);
                }
            }
            24742 => {
                self.remove_auras_due_to_spell(victim, 24754);
            }
            // Trick or Treat
            // (SpellEffects.cpp:3869-3880).
            24751 => {
                if !victim.is_player() {
                    return;
                }
                self.cast_custom_spell(victim, victim, 24755, [None; 3]);
                let spell_id = if self.roll_chance_i(50) { 24714 } else { 24715 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Mistletoe (SpellEffects.cpp:3913-3936).
            26004 => {
                self.cast_custom_spell(victim, victim, 26005, [None; 3]);
            }
            26218 => {
                if !victim.is_player() {
                    return;
                }
                let spell_id = if self.urand(0, 1) == 1 { 26206 } else { 26207 };
                self.cast_custom_spell(caster, victim, spell_id, [None; 3]);
            }
            // Rotate Trigger
            // (SpellEffects.cpp:3921-3928).
            26137 => {
                let spell_id = if self.urand(0, 1) == 1 { 26009 } else { 26136 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Winter Wondervolt TRAP
            // (SpellEffects.cpp:3937-3951).
            26275 => {
                if !self.within_dist(caster, victim, 1.0) {
                    return;
                }
                for old in [26272u32, 26157, 26273, 26274] {
                    if self.unit_has_aura(victim, old) {
                        self.remove_auras_due_to_spell(victim, old);
                    }
                }
                let spell_id = self.pick_spell(&[26272, 26157, 26273, 26274]);
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Summon Black Qiraji Battle Tank
            // (SpellEffects.cpp:3955-3974). No
            // Map.dbc mount flag is loaded, so the
            // outdoor spell is used (recorded).
            26656 => {
                self.remove_spells_causing_aura(victim, aura_type::MOUNTED);
                self.cast_custom_spell(victim, victim, 26655, [None; 3]);
            }
            // Bag of Candies
            // (SpellEffects.cpp:4260-4268).
            26678 => {
                let spell_id =
                    self.pick_spell(&[26668, 26670, 26671, 26672, 26673, 26674, 26675, 26676]);
                self.cast_custom_spell(caster, caster, spell_id, [None; 3]);
            }
            // Knock Away (SpellEffects.cpp:3595-3603).
            10101 => {
                self.modify_threat_percent_of(caster, victim, -100);
            }
            // Piccolo of the Flaming Fire
            // (SpellEffects.cpp:3604-3612).
            17512 => {
                if victim.is_player() {
                    self.handle_emote_command(victim, 10);
                }
            }
            // Shadow Flame (SpellEffects.cpp:3613-3637).
            22539 | 22972 | 22975 | 22976 | 22977 | 22978 | 22979 | 22980 | 22981 | 22982
            | 22983 | 22984 | 22985 => {
                if self.unit_is_alive(victim) && !self.unit_has_aura(victim, 22683) {
                    self.cast_custom_spell(caster, victim, 22682, [None; 3]);
                }
            }
            // Poisonous Blood
            // (SpellEffects.cpp:3699-3703).
            24320 => {
                self.cast_custom_spell(victim, victim, 24321, [None; 3]);
            }
            // Blood Siphon (SpellEffects.cpp:3704-3721).
            24324 => {
                if !self.unit_is_alive(victim) {
                    return;
                }
                let spell_id = if self.unit_has_aura(victim, 24321) {
                    24323
                } else {
                    24322
                };
                self.cast_custom_spell(victim, caster, spell_id, [None; 3]);
            }
            // Drain Mana (SpellEffects.cpp:3894-3912).
            25671 | 25755 => {
                self.cast_custom_spell(victim, caster, 26639, [None; 3]);
            }
            25676 | 26559 => {
                self.cast_custom_spell(caster, victim, 25671, [None; 3]);
            }
            25754 | 26457 => {
                self.cast_custom_spell(caster, victim, 25755, [None; 3]);
            }
            // Valentine, guards
            // (SpellEffects.cpp:3975-4028).
            26923 => {
                if !victim.is_player() || self.unit_has_aura(victim, 26898) {
                    return;
                }
                let tokens = self
                    .players
                    .get(&victim)
                    .map_or(0, |e| e.player.quest_item_count(21815));
                if tokens == 0 {
                    return;
                }
                let faction = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::faction_template);
                let pair = match faction {
                    11 => (27242, 27510),
                    85 => (27247, 27507),
                    57 => (27244, 27506),
                    68 | 71 => (27246, 27515),
                    79 => (27245, 27504),
                    105 => (27248, 27513),
                    _ => return,
                };
                self.valentine(victim, pair);
            }
            // Valentine, citizens
            // (SpellEffects.cpp:4029-4090).
            26663 | 27541 | 27547 | 27548 | 27549 | 27550 => {
                if !victim.is_player() {
                    return;
                }
                let faction = self
                    .unit_base(caster)
                    .map_or(0, mangos_entities::unit::UnitBase::faction_template);
                let pair = match faction {
                    12 | 123 => (27525, 27509),
                    29 | 83 | 85 | 126 | 876 | 1215 => (27523, 27505),
                    55 | 57 | 875 => (27520, 27503),
                    68 => (27529, 27512),
                    80 => (27519, 26901),
                    104 | 105 | 412 => (27524, 27511),
                    _ => return,
                };
                self.valentine(victim, pair);
            }
            // Amorous timers
            // (SpellEffects.cpp:4091-4102).
            27654 | 26870 => {
                if victim.is_player() {
                    return;
                }
                if let Some(u) = self.unit_base_mut(victim) {
                    let mut flags = u.values().get_u32(offsets::UNIT_NPC_FLAGS);
                    flags |= 0x0000_0001;
                    if flags & 0x0000_0080 != 0 {
                        flags |= 0x0000_0004;
                    }
                    u.values().set_u32(offsets::UNIT_NPC_FLAGS, flags);
                }
                let spell_id = if spell.id == 26870 { 26869 } else { 27741 };
                self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
            }
            // Valentine End Check
            // (SpellEffects.cpp:4269-4280). Game
            // events are unmodeled; event 8 reads
            // inactive, as in the fork off-season.
            27657 => {
                for id in [26870u32, 27742, 26869, 27654, 27741] {
                    self.remove_auras_due_to_spell(caster, id);
                }
            }
            // Teleport to Razelikh, group
            // (SpellEffects.cpp:4281-4296).
            27686 => {
                let Some(group) = self.players.get(&victim).and_then(|e| e.player.group_id) else {
                    return;
                };
                let members: Vec<ObjectGuid> = self
                    .players
                    .iter()
                    .filter(|(_, e)| e.player.group_id == Some(group))
                    .map(|(g, _)| *g)
                    .collect();
                for member in members {
                    if self.within_dist(caster, member, 20.0) {
                        self.cast_custom_spell(member, member, 13142, [None; 3]);
                    }
                }
            }
            // Summon Bone Minions / Mages
            // (SpellEffects.cpp:4103-4124).
            27687 => {
                for id in [27690u32, 27691, 27692, 27693] {
                    self.cast_custom_spell(victim, victim, id, [None; 3]);
                }
            }
            27695 => {
                for id in [27696u32, 27697, 27698, 27699] {
                    self.cast_custom_spell(victim, victim, id, [None; 3]);
                }
            }
            // Breath of Sargeras
            // (SpellEffects.cpp:4220-4225).
            28352 => {
                self.cast_custom_spell(caster, victim, 28342, [None; 3]);
            }
            // Chains of Kel'Thuzad
            // (SpellEffects.cpp:4141-4181).
            28408 => {
                let mut viable: Vec<ObjectGuid> = self
                    .threat_targets(caster)
                    .into_iter()
                    .filter(|g| g.is_player() && self.unit_is_alive(*g))
                    .collect();
                #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
                let mut num_targets = viable.len().min(5) as i32 - 1;
                if let Some(maintank) = self.current_victim(caster) {
                    viable.retain(|g| *g != maintank);
                    num_targets -= 1;
                    self.cast_custom_spell(maintank, maintank, 28409, [None; 3]);
                    self.cast_custom_spell(caster, maintank, 28410, [None; 3]);
                }
                while num_targets > 0 && !viable.is_empty() {
                    #[allow(clippy::cast_possible_truncation)]
                    let idx = self.urand(0, viable.len() as u32 - 1) as usize;
                    let target = viable.remove(idx);
                    self.cast_custom_spell(target, target, 28409, [None; 3]);
                    self.cast_custom_spell(caster, target, 28410, [None; 3]);
                    num_targets -= 1;
                }
            }
            // Summon Blizzard
            // (SpellEffects.cpp:4182-4189).
            28560 => {
                self.cast_custom_spell(victim, victim, 28561, [None; 3]);
            }
            // Widow's Embrace
            // (SpellEffects.cpp:4297-4302).
            28732 => {
                if self.unit_is_alive(caster) {
                    self.kill_unit(caster, None);
                }
            }
            // Teleports, player self-cast
            // (SpellEffects.cpp:4199-4210).
            29182 => {
                if victim.is_player() {
                    self.cast_custom_spell(victim, victim, 29181, [None; 3]);
                }
            }
            29188 => {
                if victim.is_player() {
                    self.cast_custom_spell(victim, victim, 29190, [None; 3]);
                }
            }
            // Ribbon Pole Channel Trigger
            // (SpellEffects.cpp:4303-4308).
            29710 => {
                if victim.is_player() {
                    let spell_id = self.pick_spell(&[29705, 29726, 29727]);
                    self.cast_custom_spell(victim, victim, spell_id, [None; 3]);
                }
            }
            // Despawn Ice Block
            // (SpellEffects.cpp:4211-4219).
            30132 => {
                if victim.is_player() {
                    self.remove_auras_due_to_spell(victim, 31800);
                    self.remove_auras_due_to_spell(victim, 28522);
                }
            }
            // Improved Sprint
            // (SpellEffects.cpp:4190-4198).
            30918 => {
                self.remove_auras_at_mechanic_immunity(victim, ROOT_AND_SNARE_MASK, 30918, true);
            }
            // EPL tower capture
            // (SpellEffects.cpp:4325-4330).
            31929 if victim.is_player() => {
                self.cast_custom_spell(victim, victim, 32061, [None; 3]);
            }
            _ => {}
        }
    }

    /// Shared Valentine body (guards/citizens):
    /// 1-in-6 heartbroken, else adored + pledge.
    fn valentine(&mut self, victim: ObjectGuid, pair: (u32, u32)) {
        let (friendship, adoration) = pair;
        let broken = self.urand(0, 5) == 0;
        let pledge = if !broken && !self.unit_has_aura(victim, 26680) {
            adoration
        } else {
            friendship
        };
        self.cast_custom_spell(
            victim,
            victim,
            if broken { 26898 } else { 26680 },
            [None; 3],
        );
        if broken {
            return;
        }
        self.cast_custom_spell(victim, victim, pledge, [None; 3]);
        self.cast_custom_spell(victim, victim, 26879, [None; 3]);
    }

    /// Rockbiter / Seal of Fury threat term.
    fn threat_from_weapon_proc(&mut self, caster: ObjectGuid, victim: ObjectGuid, damage: f32) {
        if !victim.is_creature() {
            return;
        }
        if self.threat_on(victim, caster) != 0.0 {
            let attack_time =
                f32::from_bits(self.unit_value_u32(caster, offsets::UNIT_FIELD_BASEATTACKTIME));
            self.add_threat(victim, caster, damage * attack_time / 1000.0);
        }
    }

    /// Paladin family arms
    /// (SpellEffects.cpp:4378-4437).
    fn script_paladin(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        victim: ObjectGuid,
        damage: f32,
    ) {
        // Flash of Light carrier chain.
        if spell.spell_icon_id == 242 {
            if !self.unit_is_alive(victim) {
                return;
            }
            let mut heal = mangos_spells::dither(damage, self.spell_rng());
            if self.unit_has_aura(caster, 28853) {
                heal += 53;
            }
            if self.unit_has_aura(caster, 28851) {
                heal += 83;
            }
            #[allow(clippy::cast_possible_wrap)]
            let bp1 = spell.id as i32;
            self.cast_custom_spell(caster, victim, 19993, [Some(heal), Some(bp1), None]);
            return;
        }
        // Judgement: consume the seal, cast its
        // judgement spell (CF_PALADIN_JUDGEMENT,
        // CM0 bit 23).
        if spell.spell_family_flags & 0x0080_0000 != 0 {
            if !self.unit_is_alive(victim) {
                return;
            }
            let seal = self.aura_holders(caster).iter().find_map(|h| {
                let dummy_at_2 = h.effects[2]
                    .as_ref()
                    .is_some_and(|e| e.aura_type == aura_type::DUMMY);
                if !dummy_at_2 {
                    return None;
                }
                let proto = self.stores.spells.get(h.spell_id)?;
                let is_seal = proto.spell_family_name == 10
                    && proto.spell_family_flags & (0x200 | 0x0200_0000 | 0x0800_0000) != 0;
                let judgement = proto.simple_value(2);
                (is_seal && judgement > 1).then_some((h.spell_id, judgement))
            });
            if let Some((seal_id, judgement)) = seal {
                self.remove_auras_due_to_spell(caster, seal_id);
                #[allow(clippy::cast_sign_loss)]
                self.cast_custom_spell(caster, victim, judgement as u32, [None; 3]);
            }
            return;
        }
        // Seal of Fury proc.
        if spell.spell_family_flags == 4096 {
            self.threat_from_weapon_proc(caster, victim, damage);
        }
    }

    /// Druid shapeshift form effect 9033
    /// (SpellEffects.cpp:4340-4375): strip roots
    /// and removable snares.
    fn script_shapeshift_form(&mut self, target: ObjectGuid) {
        let mut remove: Vec<u32> = Vec::new();
        for h in self.aura_holders(target) {
            let Some(proto) = self.stores.spells.get(h.spell_id) else {
                continue;
            };
            let mask = Self::mechanic_mask(proto);
            if mask == 0 {
                continue;
            }
            let has_root = h
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_ROOT);
            if has_root {
                remove.push(h.spell_id);
                continue;
            }
            let has_slow = h
                .effects
                .iter()
                .flatten()
                .any(|e| e.aura_type == aura_type::MOD_DECREASE_SPEED);
            if !has_slow {
                continue;
            }
            if mask & NOT_REMOVED_BY_SHAPESHIFT != 0 {
                continue;
            }
            // Daze-by-icon exception.
            if proto.spell_icon_id == 15 && proto.dispel == 0 && mask & (1 << 10) == 0 {
                continue;
            }
            remove.push(h.spell_id);
        }
        for id in remove {
            self.remove_auras_due_to_spell(target, id);
        }
    }

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
