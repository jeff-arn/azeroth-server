//! `Spell::EffectDummy` customs (`SpellEffects.cpp:304-1451`).

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `Spell::EffectDummy` per-victim unit branch.
    /// `damage` is the fork's per-effect
    /// `CalculateDamage` value.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    pub(crate) fn effect_dummy(
        &mut self,
        caster: ObjectGuid,
        affective_caster: ObjectGuid,
        spell: &SpellEntry,
        eff_index: usize,
        victim: ObjectGuid,
        damage: f32,
        cast_item: Option<CastItemCtx>,
        cast_counter: u8,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
    ) {
        match spell.id {
            // Spirit Heal (SpellEffects.cpp:805-819): the
            // affective (original) player confirms at the
            // spell's unit target, the spirit healer.
            17251 => {
                if victim.raw() != 0 && self.players.contains_key(&affective_caster) {
                    let mut payload = PacketWriter::new();
                    payload.write_u64_le(victim.raw());
                    let packet =
                        WorldPacket::with_body(op::SMSG_SPIRIT_HEALER_CONFIRM, payload.into_vec());
                    self.events.push(crate::map::MapEvent::Packet {
                        to: affective_caster,
                        packet,
                    });
                }
            }
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
                if cast_item.is_none() && !caster.is_player() {
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
                if cast_item.is_none() {
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
                if cast_item.is_none() {
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
}
