//! Spell target collection and `DoAllEffectOnTarget` effect arms.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Area victim modes (fork fill-time `CheckTarget` scope):
/// the per-effect main-target exclusion applies to these,
/// never to explicit-unit modes (`PushTarget` bypasses the
/// fill-time script check for the explicit unit target).
fn is_area_target_mode(mode: u32) -> bool {
    matches!(
        mode,
        tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC
            | tgt::ENUM_UNITS_ENEMY_IN_CONE
            | tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::LOCATION_CASTER_SRC
            | tgt::ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC
            | tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC
    )
}

impl Map {
    /// `ComputeEffectRadius` data half: radius entry
    /// for one effect (spellmod applied by callers
    /// that need it).
    pub(crate) fn effect_radius(&self, spell: &SpellEntry, i: usize) -> f32 {
        self.stores
            .spells
            .tables
            .radii
            .get(&spell.effects[i].radius_index)
            .copied()
            .unwrap_or(0.0)
    }

    /// Units within `radius` (2D) of `center`
    /// hostile (`enemy`) or friendly to `caster`;
    /// the ground-enumeration victim search.
    pub(crate) fn units_around(
        &self,
        caster: ObjectGuid,
        center: Position,
        radius: f32,
        enemy: bool,
    ) -> Vec<ObjectGuid> {
        let mut out = Vec::new();
        let within = |pos: Position| {
            (pos.x - center.x).powi(2) + (pos.y - center.y).powi(2) <= radius * radius
        };
        for (guid, creature) in &self.creatures {
            if !creature.unit.is_alive() {
                continue;
            }
            if Self::is_hostile_pair(caster, *guid) == enemy
                && within(creature.unit.world.position())
            {
                out.push(*guid);
            }
        }
        for guid in self.players.keys() {
            if !self.unit_is_alive(*guid) {
                continue;
            }
            if Self::is_hostile_pair(caster, *guid) == enemy
                && self.unit_position(*guid).is_some_and(within)
            {
                out.push(*guid);
            }
        }
        out
    }

    #[allow(
        clippy::too_many_lines,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )] // one SetTargetMap port
    pub(super) fn collect_targets(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        target: CastTarget,
    ) -> Vec<ObjectGuid> {
        let eff_index = spell
            .effects
            .iter()
            .position(|e| e.effect != effect::NONE)
            .unwrap_or(0);
        let mut mode = spell.effects[eff_index].implicit_target_a;
        if spell.effects[eff_index].effect == effect::NONE {
            mode = tgt::UNIT_CASTER;
        }
        // `SPELL_EFFECT_SKINNING` always lands on the
        // explicit unit target (the corpse), independent
        // of the DBC implicit target (fork
        // `Spell::SetTargetMap` unit-target capture;
        // the cast check already requires
        // `CastTarget::Unit`). Pickpocket shares the
        // shape: the live victim is the explicit unit
        // target. `SKIN_PLAYER_CORPSE` rides the same
        // capture (fork `Spell.cpp:3305` pushes the unit
        // target): a corpse guid takes the corpse arm, a
        // dead-player guid the `RemovedInsignia` arm.
        if spell.effects.iter().any(|e| {
            e.effect == effect::SKINNING
                || e.effect == effect::PICKPOCKET
                || e.effect == effect::SKIN_PLAYER_CORPSE
        }) {
            mode = tgt::UNIT;
        }
        // `SpellScript::OnSetTargetMap` (fork
        // `Spell::SetTargetMap` head): script may
        // override mode/radius/max-targets/closest
        // before the target switch. Radius starts at
        // the effect radius the arms below used
        // before this seam (the fork's max-range
        // fallback for radius_index 0 is a
        // pre-existing subset gap). The cone arm
        // keeps its own radius computation.
        let mut radius = self.effect_radius(spell, 0);
        let mut max_targets = spell.max_affected_targets;
        let mut select_closest = false;
        self.spell_script_on_set_target_map(
            caster,
            spell,
            eff_index,
            &mut mode,
            &mut radius,
            &mut max_targets,
            &mut select_closest,
        );
        let mut targets = match mode {
            // Chain heal rides the unit-target fill: the
            // single-target component is the explicit unit
            // target (fork `Spell.cpp:2949` pushes
            // `pUnitTarget`; jumps are execution-time and
            // stay unverified).
            tgt::UNIT_ENEMY
            | tgt::UNIT_FRIEND
            | tgt::UNIT
            | tgt::UNIT_FRIEND_CHAIN_HEAL
            | tgt::UNIT_RAID => match target {
                CastTarget::Unit(g) => vec![g],
                CastTarget::Corpse(g) => self
                    .corpses
                    .get(&g)
                    .map(|corpse| vec![corpse.owner])
                    .unwrap_or_default(),
                CastTarget::SelfCast | CastTarget::Dest(_) => vec![caster],
            },
            tgt::ENUM_UNITS_ENEMY_AOE_AT_DEST_LOC | tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC => {
                let center = if mode == tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC {
                    self.world_object_position(caster).unwrap_or_default()
                } else {
                    self.cast_dest(caster, spell, target)
                };
                self.units_around(caster, center, radius, true)
            }
            tgt::ENUM_UNITS_ENEMY_IN_CONE => {
                // `FillAreaTargets(PUSH_IN_CONE)`:
                // enemies within radius (effect
                // radius, else max range) inside the
                // spell's cone arc (negative arc =
                // back cone).
                let center = self.world_object_position(caster).unwrap_or_default();
                let radius = if spell.effects[0].radius_index != 0 {
                    self.effect_radius(spell, 0)
                } else {
                    spell.range_max(&self.stores.spells.tables)
                };
                let angle = self.stores.spells.cone_angle(spell.id);
                self.units_around(caster, center, radius, true)
                    .into_iter()
                    .filter(|g| {
                        if angle > 0.0 {
                            self.has_in_arc(caster, *g, angle)
                        } else if angle < 0.0 {
                            !self.has_in_arc(caster, *g, std::f32::consts::TAU + angle)
                        } else {
                            false
                        }
                    })
                    .collect()
            }
            tgt::ENUM_UNITS_FRIEND_AOE_AT_DEST_LOC | tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC => {
                let center = if mode == tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC {
                    self.world_object_position(caster).unwrap_or_default()
                } else {
                    self.cast_dest(caster, spell, target)
                };
                self.units_around(caster, center, radius, false)
            }
            tgt::ENUM_UNITS_PARTY_WITHIN_CASTER_RANGE
            | tgt::UNIT_PARTY
            | tgt::UNIT_FRIEND_AND_PARTY => {
                // Party system not built: caster only
                // (Battle Shout self-buff evidence).
                vec![caster]
            }
            tgt::UNIT_RAID_AND_CLASS => {
                // Greater Blessings (fork `Spell.cpp:3026`):
                // the explicit unit target, expanded to
                // every member of the *target's* group
                // sharing the target's class within the
                // effect radius of the target and not
                // hostile to the caster. No group (or a
                // non-player target) keeps the single
                // target.
                let unit_target = match target {
                    CastTarget::Unit(g) => g,
                    CastTarget::Corpse(g) => {
                        self.corpses.get(&g).map_or(caster, |corpse| corpse.owner)
                    }
                    CastTarget::SelfCast | CastTarget::Dest(_) => caster,
                };
                let group_id = self.player(unit_target).and_then(|p| p.group_id);
                match group_id {
                    None => vec![unit_target],
                    Some(group_id) => {
                        let target_class = self.unit_class_of(unit_target);
                        let Some(tpos) = self.unit_position(unit_target) else {
                            return Vec::new();
                        };
                        self.players
                            .keys()
                            .copied()
                            .filter(|g| {
                                self.unit_is_alive(*g)
                                    && self.player(*g).and_then(|p| p.group_id) == Some(group_id)
                                    && self.unit_class_of(*g) == target_class
                                    && !Self::is_hostile_pair(caster, *g)
                                    && self.unit_position(*g).is_some_and(|pos| {
                                        (pos.x - tpos.x).powi(2) + (pos.y - tpos.y).powi(2)
                                            <= radius * radius
                                    })
                            })
                            .collect()
                    }
                }
            }
            tgt::LOCATION_CASTER_SRC | tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT => {
                // A names the caster as the area source; B
                // selects the units there (fork `Spell.cpp`
                // target map). Script AoE (`7`) fills
                // `SPELL_TARGETS_ALL` around the source and
                // drops the caster; `spell_script_target`
                // bounds filtering is not modeled yet.
                let center = self.world_object_position(caster).unwrap_or_default();
                let radius = radius;
                let target_b = spell
                    .effects
                    .iter()
                    .find(|e| e.effect != effect::NONE)
                    .map_or(0, |e| e.implicit_target_b);
                if mode == tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT
                    || target_b == tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT
                {
                    let mut all = self.units_around(caster, center, radius, true);
                    all.extend(self.units_around(caster, center, radius, false));
                    all.retain(|g| *g != caster);
                    all
                } else if target_b == tgt::ENUM_UNITS_ENEMY_AOE_AT_SRC_LOC {
                    self.units_around(caster, center, radius, true)
                } else if target_b == tgt::ENUM_UNITS_FRIEND_AOE_AT_SRC_LOC {
                    self.units_around(caster, center, radius, false)
                } else {
                    vec![caster]
                }
            }
            // `TARGET_NONE` (A=0,B=0 routes here too)
            // and `TARGET_LOCATION_CASTER_DEST`: the
            // fork's per-effect custom fill
            // (`Spell.cpp:3126-3316`). Modeled groups:
            // the learn/create/resurrect family takes
            // the explicit unit target, trigger/skill
            // effects take the caster, modifier auras
            // take the caster, dummy defaults to the
            // unit target. UNMODELED: the remaining
            // fork fills (summons, enchants, cannibalize
            // and kin) keep the caster fallback.
            tgt::NONE | tgt::LOCATION_CASTER_DEST => {
                let eff = &spell.effects[eff_index];
                let unit_target = match target {
                    CastTarget::Unit(g) => g,
                    CastTarget::Corpse(g) => {
                        self.corpses.get(&g).map_or(caster, |corpse| corpse.owner)
                    }
                    CastTarget::SelfCast | CastTarget::Dest(_) => caster,
                };
                match eff.effect {
                    // Fork unit-target group: BIND,
                    // PARRY, BLOCK, CREATE_ITEM,
                    // TRIGGER_MISSILE, LEARN_SPELL,
                    // SKILL_STEP, ADD_HONOR,
                    // PROFICIENCY, SUMMON_POSSESSED,
                    // SUMMON_OBJECT_WILD,
                    // SELF_RESURRECT, REPUTATION,
                    // SEND_TAXI, plus SUMMON_PLAYER,
                    // RESURRECT and DUMMY's default.
                    11
                    | 22
                    | 23
                    | 24
                    | 32
                    | 36
                    | 44
                    | 45
                    | 60
                    | 73
                    | 76
                    | 85
                    | 94
                    | 103
                    | 123
                    | effect::DUMMY
                    | effect::RESURRECT
                    | effect::RESURRECT_NEW => {
                        vec![unit_target]
                    }
                    // Fork caster group: TRIGGER_SPELL
                    // (in the unit-target group but
                    // self-cast), SUMMON_CHANGE_ITEM,
                    // SUMMON_GUARDIAN, ADD_FARSIGHT,
                    // STUCK, DESTROY_ALL_TOTEMS,
                    // CREATE_HOUSE, SKILL.
                    34 | 42 | 72 | 81 | 84 | 110 | effect::TRIGGER_SPELL | effect::SKILL => {
                        vec![caster]
                    }
                    effect::APPLY_AURA => {
                        // Flat/pct spell-modifier auras
                        // land on the caster; other
                        // auras take the unit target.
                        if eff.apply_aura_name == aura_type::ADD_FLAT_MODIFIER
                            || eff.apply_aura_name == aura_type::ADD_PCT_MODIFIER
                        {
                            vec![caster]
                        } else {
                            vec![unit_target]
                        }
                    }
                    // Direct damage on a dest target has no
                    // fork fill (`SetTargetMap` dest-switch
                    // default breaks with no targets): the
                    // tick appliers (Hellfire Effect) only
                    // carry victims on trigger casts. The
                    // caster fallback would wrongly
                    // self-hit.
                    effect::SCHOOL_DAMAGE if matches!(target, CastTarget::Dest(_)) => Vec::new(),
                    _ => vec![caster],
                }
            }
            // The caster's pet or nothing (fork
            // `FillTargetMap` `TARGET_UNIT_CASTER_PET`:
            // pet, else charm, else no target). Charm
            // fallback is unmodeled; an empty victim
            // list skips the effect fork-exactly.
            tgt::UNIT_CASTER_PET => self.pet_of(caster).map_or_else(Vec::new, |pet| vec![pet]),
            // The caster's owner or nothing (fork
            // `FillTargetMap` `TARGET_UNIT_CASTER_MASTER`:
            // the pet's master; a player caster has none,
            // so the effect skips fork-exactly — e.g.
            // Voidwalker Sacrifice shields the owner).
            tgt::UNIT_CASTER_MASTER => self
                .pet_owner(caster)
                .map_or_else(Vec::new, |owner| vec![owner]),
            // Unmodeled location modes and
            // `TARGET_LOCATION_CASTER_FISHING_SPOT`
            // (fork `Spell.cpp:2945-2948` pushes the
            // caster): the caster fallback.
            _ => vec![caster],
        };
        // `SpellScript::OnCheckTarget` per-candidate
        // filter (fork `Spell::CheckTarget` during
        // target-map fill; eff-index granularity is
        // the first-effect subset of this merged
        // victim list). The explicit unit target always
        // survives: the fork carries it on the separate
        // main-target path (`PushTarget`), which bypasses
        // the fill-time script check, so unit-targeted
        // effects (Intimidating Shout's 20511 trigger) still
        // land on it; area effects exclude it per effect
        // below instead.
        let main_target = Self::script_unit_target(target);
        // Creature-type victim filter (fork
        // `CheckTarget` inside the fill): non-caster
        // victims outside the spell's creature-type
        // mask take no effects. The explicit main
        // target already passed the CheckCast gate.
        targets.retain(|v| *v == caster || self.check_target_creature_type(spell, *v));
        targets.retain(|v| {
            Some(*v) == main_target
                || self.spell_script_on_check_target(caster, spell, main_target, *v, eff_index)
        });
        // Fork max-target trim (`Spell.cpp` tail of
        // SetTargetMap): closest-first when the
        // script asked, else random erasure; the
        // explicit unit target is kept (fork handles
        // it on the separate main-target path).
        if max_targets > 0 && targets.len() > max_targets as usize {
            if select_closest {
                let cpos = self.unit_position(caster).unwrap_or_default();
                targets.sort_by(|a, b| {
                    let da = self.unit_position(*a).map_or(f32::MAX, |p| {
                        (p.x - cpos.x).powi(2) + (p.y - cpos.y).powi(2)
                    });
                    let db = self.unit_position(*b).map_or(f32::MAX, |p| {
                        (p.x - cpos.x).powi(2) + (p.y - cpos.y).powi(2)
                    });
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });
                targets.truncate(max_targets as usize);
            } else {
                while targets.len() > max_targets as usize {
                    let candidates: Vec<usize> = targets
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| Some(**g) != main_target)
                        .map(|(i, _)| i)
                        .collect();
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let pick = if candidates.is_empty() {
                        self.spell_rng().irand(0, targets.len() as i32 - 1) as usize
                    } else {
                        candidates[self.spell_rng().irand(0, candidates.len() as i32 - 1) as usize]
                    };
                    targets.remove(pick);
                }
            }
        }
        targets
    }

    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one DoAllEffectOnTarget port
    pub(super) fn apply_effects(
        &mut self,
        caster: ObjectGuid,
        affective_caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        victim: ObjectGuid,
        client_dest: Option<Position>,
        main_target: Option<ObjectGuid>,
        mut applied_mods: Option<&mut Vec<(u32, u8)>>,
        proc_ctx: Option<&ProcRaiseCtx>,
        bp: [Option<i32>; 3],
        cast_item: Option<CastItemCtx>,
        cast_counter: u8,
        triggered_by_aura_spell: u32,
        triggered_by_spell: u32,
        script_effect_pick: Option<u32>,
    ) -> bool {
        // EventAI SpellHit hooks (Slice 6b), once per
        // hit target as in `DoAllEffectOnTarget`.
        if victim.is_creature() && victim != caster {
            self.eventai_on_spell_hit(victim, caster, spell.id, spell.school);
        }
        if victim.is_creature() {
            self.creature_script_spell_hit(victim, caster, spell.id);
        }
        // `ZoneScript::OnCreatureSpellHit` (`Spell.cpp:1672`),
        // beside the EventAI hooks above (the fork fires
        // both at the end of `DoAllEffectOnTarget`).
        if victim.is_creature() {
            self.instance_on_creature_spell_hit(caster, victim, spell.id);
        }
        // `InstanceData::CustomSpellCasted`: the fork's
        // only call site is the spell 27517 dummy arm
        // (`SpellEffects.cpp:567`, BRD Theldren).
        if spell.id == 27_517 {
            self.instance_custom_spell_casted(spell.id, caster);
        }
        if caster.is_creature() && victim != caster {
            self.eventai_on_spell_hit_target(caster, victim, spell.id, spell.school);
        }
        if caster.is_creature() {
            self.creature_script_spell_hit_target(caster, victim, spell.id);
        }
        if !self.unit_is_alive(victim) && victim.is_creature() {
            // The hit hook may have despawned the
            // target (scripts); C++ would still apply
            // to a live object only.
            if !self.creatures.contains_key(&victim) {
                return true;
            }
        }
        // `SpellScript::OnHit` (fork `Spell.cpp:1489`
        // family; hit path — the miss path fires from
        // `finish_cast`).
        if proc_ctx.is_some_and(|c| c.can_trigger) {
            self.spell_script_on_hit(caster, spell, victim, true);
        }
        // In-combat flags (DealDamage side effects land
        // with effects too via combat in ApplyAura).
        let mut holder_applied = false;
        // `Spell::damage` script channel for this
        // victim's effect pass (`OnEffectExecute` writes).
        let mut script_damage = 0.0f32;
        // Fork `finish()` inside an effect aborts the
        // cast (the fishing water gate); the flag rides
        // out through `execute_cast`, which then skips
        // the channel start and the successful-finish
        // tail (the arm already sent the fail packet).
        let mut effects_ok = true;
        for i in 0..3 {
            let eff = spell.effects[i];
            if eff.effect == effect::NONE {
                continue;
            }
            // Per-effect target map (fork `SetTargetMap`
            // per effect): a `TARGET_UNIT_CASTER` effect
            // lands on the caster even inside a mixed
            // spell whose victim list came from another
            // effect's mode (Sacrifice: shield on the
            // master via effect 1, suicide on the pet
            // itself via effect 2). The merged victim
            // list otherwise stays the subset shape.
            let victim = if eff.implicit_target_a == tgt::UNIT_CASTER {
                caster
            } else {
                victim
            };
            // Per-effect main-target exclusion (fork
            // per-effect `CheckTarget` during fill):
            // area effects skip the explicit unit target
            // when the script rejects it (Intimidating
            // Shout 5246: the main target is stunned by
            // the trigger, never feared). Unit-mode
            // effects always land on it — the merged list
            // keeps it unconditionally above (fork
            // `PushTarget`). Script-less spells return
            // true here, so nothing else changes.
            if Some(victim) == main_target
                && is_area_target_mode(eff.implicit_target_a)
                && !self.spell_script_on_check_target(caster, spell, main_target, victim, i)
            {
                continue;
            }
            // Per-effect immunity (fork strips the
            // effect from the target's effectMask).
            if self.is_immune_to_spell_effect(victim, spell, i, victim == caster) {
                continue;
            }
            if !self.spell_script_on_effect_execute(
                caster,
                spell,
                victim,
                i,
                &mut script_damage,
                triggered_by_aura_spell,
                triggered_by_spell,
                script_effect_pick,
                cast_item,
                caster_level,
                bp,
            ) {
                continue;
            }
            match eff.effect {
                // `EffectInstaKill` (fork
                // SpellEffects.cpp:265): direct damage
                // equal to current health, no armor.
                // `EffectDispel` (fork
                // SpellEffects.cpp:2402): candidates
                // are holders whose spell dispel type
                // is in the effect's mask; for MAGIC/
                // POISON, positive auras stay on
                // friendly targets and negative on
                // hostile. Removes up to the effect
                // value (fork picks randomly with a
                // level-based resist roll; stored
                // order + no resist here, recorded).
                effect::DISPEL => {
                    let dispel_mask: u32 = if eff.misc_value < 0 || eff.misc_value == 7 {
                        0x3FE
                    } else if eff.misc_value < 31 {
                        1u32 << eff.misc_value
                    } else {
                        0
                    };
                    let friendly = !Self::is_hostile_pair(caster, victim);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count = self.effect_value_bp(spell, i, caster_level, bp[i]) as usize;
                    let mut remove: Vec<usize> = Vec::new();
                    let holders = self.aura_holders(victim);
                    {
                        for (idx, holder) in holders.iter().enumerate() {
                            if remove.len() >= count {
                                break;
                            }
                            let Some(entry) = self.stores.spells.get(holder.spell_id) else {
                                continue;
                            };
                            if entry.dispel >= 32 || (1u32 << entry.dispel) & dispel_mask == 0 {
                                continue;
                            }
                            if matches!(entry.dispel, 1 | 4) && holder.positive == friendly {
                                continue;
                            }
                            remove.push(idx);
                        }
                    }
                    let dispelled_any = !remove.is_empty();
                    for idx in remove.into_iter().rev() {
                        self.remove_aura_at_mode(victim, idx, AuraRemoveMode::Dispel);
                    }
                    // `SpellScript::OnSuccessfulDispel`
                    // (`SpellEffects.cpp:2521`).
                    if dispelled_any {
                        self.spell_script_on_successful_dispel(caster, spell, i);
                    }
                }
                // `EffectLearnSpell` (fork
                // SpellEffects.cpp): teach the
                // trigger spell to a player victim.
                // Runtime set = `deferred.spells`,
                // which the save diff persists as
                // `character_spell` inserts; pet
                // branch unmodeled (no pet runtime).
                effect::LEARN_SPELL => {
                    if victim.is_player() && eff.trigger_spell != 0 {
                        let learned = eff.trigger_spell;
                        let fresh = self
                            .players
                            .get_mut(&victim)
                            .is_some_and(|entry| entry.player.learn_spell(learned));
                        if fresh {
                            let mut payload = PacketWriter::new();
                            payload.write_u32_le(learned);
                            let packet =
                                WorldPacket::with_body(op::SMSG_LEARNED_SPELL, payload.into_vec());
                            self.send_to_player(victim, packet);
                        }
                    }
                }
                // `EffectSendTaxi`
                // (`SpellEffects.cpp:5380`): start the
                // misc-value taxi path for a player
                // victim (nocheck activation).
                effect::SEND_TAXI => {
                    if victim.is_player()
                        && let Ok(path_id) = u32::try_from(eff.misc_value)
                        && let Some(path) = self.stores.taxi.path_by_id(path_id).copied()
                    {
                        self.activate_taxi_path_to(
                            victim,
                            &[path.from, path.to],
                            None,
                            spell.id,
                            true,
                        );
                    }
                }
                effect::INSTAKILL => {
                    let hp = self.unit_value_u32(victim, offsets::UNIT_FIELD_HEALTH);
                    if hp > 0 {
                        self.deal_damage(caster, victim, hp, false);
                    }
                }
                // `EffectThreat` (fork
                // SpellEffects.cpp:3432): flat threat
                // from the effect value on the victim's
                // threat list.
                effect::THREAT => {
                    if victim.is_creature() {
                        let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
                        self.add_threat(victim, caster, value);
                    }
                }
                effect::SCHOOL_DAMAGE => {
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let base =
                        mangos_spells::ditheru(value + script_damage, self.spell_rng()) as i32;
                    self.school_damage(
                        caster,
                        spell,
                        victim,
                        base,
                        i as u32,
                        None,
                        applied_mods.as_deref_mut(),
                        proc_ctx,
                    );
                }
                // `EffectWeaponDmg` (fork SpellEffects.cpp:3321):
                // bonus = sum of WEAPON_DAMAGE / NOSCHOOL /
                // NORMALIZED effect values; WEAPON_PERCENT
                // multiplies the whole swing; only the first
                // weapon effect executes (one swing per cast).
                // Deviations: main-hand only; AP
                // normalization unmodeled (unit fields bake
                // AP/speed already); avoidance rides the
                // spell hit roll, not the melee table.
                effect::WEAPON_DAMAGE
                | effect::WEAPON_DAMAGE_NOSCHOOL
                | effect::WEAPON_PERCENT_DAMAGE
                | effect::NORMALIZED_WEAPON_DMG => {
                    let is_weapon = |e: u32| {
                        matches!(
                            e,
                            effect::WEAPON_DAMAGE
                                | effect::WEAPON_DAMAGE_NOSCHOOL
                                | effect::WEAPON_PERCENT_DAMAGE
                                | effect::NORMALIZED_WEAPON_DMG
                        )
                    };
                    if !spell.effects[..i].iter().any(|e| is_weapon(e.effect)) {
                        let mut bonus = script_damage;
                        let mut pct = 1.0f32;
                        for (j, e) in spell.effects.iter().enumerate() {
                            let v = self.effect_value_bp(spell, j, caster_level, bp[j]);
                            match e.effect {
                                effect::WEAPON_DAMAGE
                                | effect::WEAPON_DAMAGE_NOSCHOOL
                                | effect::NORMALIZED_WEAPON_DMG => bonus += v,
                                effect::WEAPON_PERCENT_DAMAGE => pct *= v / 100.0,
                                _ => {}
                            }
                        }
                        let min_dmg = f32::from_bits(
                            self.unit_value_u32(caster, offsets::UNIT_FIELD_MINDAMAGE),
                        );
                        let max_dmg = f32::from_bits(
                            self.unit_value_u32(caster, offsets::UNIT_FIELD_MAXDAMAGE),
                        );
                        let (lo, hi) = if min_dmg > max_dmg {
                            (max_dmg, min_dmg)
                        } else {
                            (min_dmg, max_dmg)
                        };
                        let swing = self
                            .rng
                            .frand(lo.max(0.0), if hi == 0.0 { 5.0 } else { hi });
                        #[allow(clippy::cast_possible_wrap)]
                        let damage = mangos_spells::ditheru(
                            ((swing + bonus) * pct).max(0.0),
                            self.spell_rng(),
                        ) as i32;
                        if damage > 0 {
                            self.school_damage(
                                caster,
                                spell,
                                victim,
                                damage,
                                i as u32,
                                None,
                                applied_mods.as_deref_mut(),
                                proc_ctx,
                            );
                        }
                    }
                }
                effect::HEAL => {
                    // Script `spell->damage` adjustment
                    // (e.g. Swiftmend) rides the same
                    // additive channel as SCHOOL_DAMAGE.
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]) + script_damage;
                    // `EffectHeal`: HealingBonusDone
                    // (coefficient on the advertised
                    // benefit, spellmod tail) then
                    // HealingBonusTaken.
                    let value = self.spell_healing_bonus_done(
                        caster,
                        spell,
                        i,
                        value,
                        false,
                        1,
                        applied_mods.as_deref_mut(),
                    );
                    let value = self.spell_healing_bonus_taken(
                        victim,
                        spell,
                        i,
                        value,
                        false,
                        1,
                        applied_mods.as_deref_mut(),
                    );
                    if value > 0.0 {
                        let amount = mangos_spells::ditheru(value, self.spell_rng());
                        self.heal_unit(victim, amount);
                        // Fork `DealHeal` sends the heal log
                        // when the caster or the victim is a
                        // player (`SpellCaster.cpp:767`); the
                        // log carries the applied amount even
                        // when the health gain clamps at max.
                        if amount > 0 && (caster.is_player() || victim.is_player()) {
                            self.send_heal_log(caster, victim, spell, amount);
                        }
                        if let Some(ctx) = proc_ctx {
                            self.raise_damage_procs(
                                caster,
                                victim,
                                Some(spell.id),
                                ctx,
                                proc_ex::NORMAL_HIT,
                                amount,
                                amount,
                            );
                        }
                    }
                }
                effect::HEAL_MAX_HEALTH => {
                    // `Spell::EffectHealMaxHealth` (fork
                    // `SpellEffects.cpp:3414`): heal equals the
                    // caster's max health (the victim's when the
                    // caster unit is unavailable), through the
                    // done/taken percent modifiers only — no
                    // spellpower benefit, unlike HEAL.
                    // `heal_unit` clamps to max.
                    if self.unit_is_alive(victim) {
                        let value = self.heal_max_health_value(caster, victim);
                        if value > 0.0 {
                            let amount = mangos_spells::ditheru(value, self.spell_rng());
                            self.heal_unit(victim, amount);
                            if let Some(ctx) = proc_ctx {
                                self.raise_damage_procs(
                                    caster,
                                    victim,
                                    Some(spell.id),
                                    ctx,
                                    proc_ex::NORMAL_HIT,
                                    amount,
                                    amount,
                                );
                            }
                        }
                    }
                }
                // `Spell::EffectEnergize` +
                // `Unit::EnergizeBySpell`: direct power
                // gain (the SMSG_SPELL_ENERGIZE_LOG rides
                // the same deferral as heal's log here).
                effect::ENERGIZE => {
                    let value = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    if value > 0.0
                        && let Ok(power_type) = u32::try_from(eff.misc_value)
                        && power_type < 5
                    {
                        let amount = mangos_spells::ditheru(value, self.spell_rng());
                        let max_power = self.unit_max_power(victim, power_type);
                        if max_power > 0 {
                            let current = self.unit_power(victim, power_type);
                            self.unit_set_power(
                                victim,
                                power_type,
                                (current + amount).min(max_power),
                            );
                        }
                    }
                }
                // `Spell::EffectTriggerSpell`: cast the
                // triggered spell at the effect's victim
                // through the triggered pipeline (no cost,
                // GCD, cooldown, or known checks; the
                // parent cast already left `casts`, so the
                // nested cast can start and complete here).
                effect::TRIGGER_SPELL => {
                    if eff.trigger_spell != 0 {
                        self.cast_spell(
                            caster,
                            eff.trigger_spell,
                            CastTarget::Unit(victim),
                            CastOptions {
                                triggered: true,
                                triggered_by_aura: false,
                                skip_known: true,
                                cast_item: None,
                                custom_base_points: [None; 3],
                                triggered_by_aura_spell: 0,
                                triggered_by_spell: spell.id,
                            },
                        );
                    }
                }
                effect::APPLY_AURA | effect::APPLY_AREA_AURA_PARTY => {
                    // Dynobj-location aura effects (28/29)
                    // are delivered by the dynamic object
                    // updater, never directly at cast time.
                    // All aura effects ride one holder
                    // (built once); later non-aura effects
                    // still execute (fork `HandleEffects`
                    // walks the full effect mask).
                    if !holder_applied
                        && eff.implicit_target_a != tgt::ENUM_UNITS_ENEMY_AOE_AT_DYNOBJ_LOCATION
                        && eff.implicit_target_a != tgt::ENUM_UNITS_FRIEND_AOE_AT_DYNOBJ_LOCATION
                    {
                        self.apply_spell_aura(
                            caster,
                            spell,
                            victim,
                            applied_mods.as_deref_mut(),
                            bp,
                        );
                        holder_applied = true;
                    }
                }
                // SPELL_EFFECT_SUMMON / SUMMON_WILD /
                // SUMMON_GUARDIAN (Slice 7).
                28 | 41 | 42 if eff.misc_value > 0 => {
                    self.summon_effect(caster, caster_level, spell, victim, i, bp[i]);
                }
                // SPELL_EFFECT_SUMMON_TOTEM(_SLOT1..4):
                // fork ignores the effect victim and
                // summons for the caster; run once on
                // the caster's own hit.
                effect::SUMMON_TOTEM
                | effect::SUMMON_TOTEM_SLOT1
                | effect::SUMMON_TOTEM_SLOT2
                | effect::SUMMON_TOTEM_SLOT3
                | effect::SUMMON_TOTEM_SLOT4
                    if victim == caster && eff.misc_value > 0 =>
                {
                    self.summon_totem_effect(caster, caster_level, spell, i, bp[i]);
                }
                // SPELL_EFFECT_SUMMON_PET (Tier 1 Slice
                // 2): the fork ignores the effect victim
                // and summons for the caster; run once
                // on the caster's own hit.
                effect::SUMMON_PET if victim == caster => {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_sign_loss,
                        clippy::cast_possible_truncation
                    )]
                    let pet_level = if caster.is_player() {
                        caster_level
                    } else {
                        (caster_level as i32 + eff.multiple_value as i32).max(1) as u32
                    };
                    #[allow(clippy::cast_sign_loss)]
                    self.summon_pet_effect(
                        caster,
                        spell.id,
                        eff.misc_value.max(0) as u32,
                        pet_level,
                    );
                }
                // `EffectSummonObjectWild` (Tier 4
                // B0-5, `SpellEffects.cpp:3490`).
                effect::SUMMON_OBJECT_WILD => {
                    self.summon_object_wild_effect(caster, spell, i, client_dest);
                }
                // `EffectSummonObject` slots 1-4
                // (`SpellEffects.cpp:5045`).
                effect::SUMMON_OBJECT_SLOT1
                | effect::SUMMON_OBJECT_SLOT2
                | effect::SUMMON_OBJECT_SLOT3
                | effect::SUMMON_OBJECT_SLOT4 => {
                    self.summon_object_effect(caster, spell, i, client_dest);
                }
                // SPELL_EFFECT_LEARN_PET_SPELL.
                effect::LEARN_PET_SPELL
                    if victim == caster && caster.is_player() && eff.trigger_spell != 0 =>
                {
                    self.learn_pet_spell_effect(caster, eff.trigger_spell);
                }
                // SPELL_EFFECT_CREATE_ITEM (Tier 1
                // Slice 3a).
                effect::CREATE_ITEM if victim.is_player() => {
                    self.create_item_effect(victim, spell, i, caster_level, bp[i]);
                }
                // SPELL_EFFECT_TELEPORT_UNITS /
                // SPELL_EFFECT_LEAP (Tier 1 Slice 5).
                effect::TELEPORT_UNITS => {
                    self.teleport_units_effect(caster, spell, i, victim, client_dest);
                }
                effect::TELEPORT_UNITS_FACE_CASTER => {
                    self.tele_units_face_caster_effect(caster, spell, i, victim, client_dest);
                }
                effect::LEAP => {
                    self.leap_effect(caster, spell, i, victim, client_dest);
                }
                // SPELL_EFFECT_KNOCK_BACK /
                // SPELL_EFFECT_PLAYER_PULL (Tier 1
                // Slice 6). Values: horizontal =
                // misc/10, vertical = value/10;
                // PLAYER_PULL pulls toward the caster
                // with distance-clamped negative
                // horizontal speed and misc vertical.
                effect::KNOCK_BACK => {
                    #[allow(clippy::cast_precision_loss)]
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    #[allow(clippy::cast_precision_loss)]
                    let horizontal = eff.misc_value as f32 / 10.0;
                    self.remove_auras_due_to_spell(victim, 24778);
                    self.knock_back_from(victim, caster, horizontal, damage / 10.0);
                }
                effect::PLAYER_PULL => {
                    #[allow(clippy::cast_precision_loss)]
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    let dist = match (self.unit_position(victim), self.unit_position(caster)) {
                        (Some(v), Some(c)) => {
                            let d = ((v.x - c.x).powi(2) + (v.y - c.y).powi(2)).sqrt();
                            if damage != 0.0 && d > damage {
                                damage
                            } else {
                                d
                            }
                        }
                        _ => 0.0,
                    };
                    #[allow(clippy::cast_precision_loss)]
                    let vertical = eff.misc_value as f32 / 10.0;
                    self.knock_back_from(victim, caster, -dist, vertical);
                }
                // `EffectDummy` / `EffectScriptEffect`:
                // per-spell customs (customs.rs); the
                // value is the fork's per-effect
                // `CalculateDamage` result.
                effect::DUMMY => {
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    self.effect_dummy(
                        caster,
                        affective_caster,
                        spell,
                        i,
                        victim,
                        damage,
                        cast_item,
                        cast_counter,
                        triggered_by_aura_spell,
                        triggered_by_spell,
                    );
                }
                effect::SCRIPT_EFFECT => {
                    let damage = self.effect_value_bp(spell, i, caster_level, bp[i]);
                    self.effect_script_effect(caster, spell, victim, damage, cast_item);
                }
                // SPELL_EFFECT_SKINNING (fork
                // `SpellEffects.cpp:5265-5284`): corpse
                // window + flag clear + gather gain.
                effect::SKINNING => {
                    self.effect_skinning(caster, victim);
                }
                // SPELL_EFFECT_PICKPOCKET (fork
                // `SpellEffects.cpp:2580-2590`): live
                // victim window; guards inside.
                effect::PICKPOCKET => {
                    self.effect_pickpocket(caster, victim);
                }
                // SPELL_EFFECT_SKIN_PLAYER_CORPSE
                // (fork `SpellEffects.cpp:5735-5760`):
                // corpse-guid convert + insignia window,
                // or the `RemovedInsignia` dead-player
                // arm; guards inside.
                effect::SKIN_PLAYER_CORPSE => {
                    self.effect_skin_player_corpse(caster, victim);
                }
                // SPELL_EFFECT_TRANS_DOOR (fork
                // `SpellEffects.cpp:5538-5678`): the
                // FISHINGNODE bobber arm plus the
                // generic transmitted-object summon
                // (Lightwell); a failed fishing water
                // gate aborts the cast fork-exactly
                // (`&=` keeps the arm running for
                // every effect slot, like the fork's
                // effect loop).
                effect::TRANS_DOOR => {
                    effects_ok &= self.effect_transmitted(caster, spell, i, client_dest);
                }
                // SPELL_EFFECT_PERSISTENT_AREA_AURA
                // spawns once per cast in
                // `finish_cast` (it is not a per-victim
                // effect; mixed spells like Flamestrike
                // enumerate direct-hit victims too).
                // Includes PERSISTENT_AREA_AURA (see
                // above): no per-victim work.
                // `EffectAddComboPoints` (fork
                // SpellEffects.cpp:4513-4526): the count
                // rides the effect value; non-positive
                // counts and non-player casters do
                // nothing. `SetComboPoints` runs inside
                // `add_combo_points`.
                effect::ADD_COMBO_POINTS => {
                    #[allow(clippy::cast_possible_truncation)]
                    let count = self.effect_value_bp(spell, i, caster_level, bp[i]) as i32;
                    if count > 0 && caster.is_player() {
                        self.add_combo_points(caster, victim, count);
                    }
                }
                // `EffectResurrect` / `EffectResurrectNew`
                // (fork SpellEffects.cpp:5123 / :202): dead
                // players get a resurrect offer (percent
                // health/mana for RESURRECT, flat health +
                // misc mana for RESURRECT_NEW); dead pets
                // revive at the caster. Living targets and
                // already-offered players do nothing.
                effect::RESURRECT | effect::RESURRECT_NEW => {
                    self.effect_resurrect(caster, spell, i, victim, caster_level, bp[i]);
                }
                _ => {}
            }
        }
        // `SpellScript::OnAfterHit` (fork `Spell.cpp`
        // DoAllEffectOnTarget unit case, after the
        // effect pass and procs for this target).
        self.spell_script_on_after_hit(caster, spell, victim, cast_item);
        effects_ok
    }

    /// `Unit::NearTeleportTo` dispatch: players take
    /// the controller ACK train, creatures the
    /// observer teleport packets.
    fn near_teleport_unit(&mut self, victim: ObjectGuid, position: Position) {
        if victim.is_player() {
            self.teleport_player_in_map(victim, position);
        } else {
            self.teleport_creature_in_map(victim, position);
        }
    }

    /// Same-map near teleport or cross-map player
    /// transfer (`EffectTeleportUnits` location arms;
    /// creatures are silently skipped cross-map,
    /// exactly as the fork).
    pub(crate) fn teleport_to_location(
        &mut self,
        victim: ObjectGuid,
        loc: mangos_world_data::spawns::WorldLocation,
    ) {
        if u32::from(loc.map_id) == self.map_id {
            self.near_teleport_unit(victim, loc.position());
        } else if victim.is_player() {
            self.events.push(MapEvent::TeleportPlayer {
                player: victim,
                map_id: u32::from(loc.map_id),
                position: loc.position(),
            });
        }
    }

    /// `GetObjectBoundingRadius` (the unit field).
    fn object_bounding_radius(&self, guid: ObjectGuid) -> f32 {
        self.unit_base(guid).map_or(0.389, |u| {
            u.world
                .object
                .values()
                .get_f32(offsets::UNIT_FIELD_BOUNDINGRADIUS)
        })
    }

    /// `Spell::EffectTeleUnitsFaceCaster`
    /// (`SpellEffects.cpp` effect 43): teleport the
    /// victim to the cast destination when the
    /// target map carries one, else to the caster's
    /// close point at the effect radius
    /// (`GetClosePoint` primary position; the
    /// `ObjectPosSelector` crowding walk and its
    /// vmap `IsWithinLOS` retries ride the recorded
    /// vmap-manager remainder), facing
    /// `-caster orientation` (fork quirk, not
    /// +π). The `TELE_TO_NOT_LEAVE_COMBAT |
    /// TELE_TO_NOT_UNSUMMON_PET | TELE_TO_SPELL`
    /// (self) flags match `near_teleport_unit`'s
    /// existing behavior. The taxi-flight gate has
    /// no system behind it yet (same as
    /// `EffectTeleportUnits`).
    fn tele_units_face_caster_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        let Some(caster_pos) = self.unit_position(caster) else {
            return;
        };
        if self.unit_position(victim).is_none() {
            return;
        }
        let mut dest = if let Some(dest) = client_dest {
            dest
        } else {
            let dis = self.effect_radius(spell, i);
            let reach =
                self.object_bounding_radius(caster) + self.object_bounding_radius(victim) + dis;
            // GetNearPoint2D + NormalizeMapCoord.
            let x =
                (caster_pos.x + reach * caster_pos.o.cos()).clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
            let y =
                (caster_pos.y + reach * caster_pos.o.sin()).clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
            // searcher == null: UpdateGroundPositionZ
            let z = self.ground_z(x, y, caster_pos.z);
            Position::new(x, y, z, caster_pos.o)
        };
        dest.o = -caster_pos.o;
        self.near_teleport_unit(victim, dest);
    }

    /// `Spell::EffectTeleportUnits`
    /// (`SpellEffects.cpp:1536`), switched on the
    /// effect's implicit target B. The taxi-flight
    /// gate has no system behind it yet.
    fn teleport_units_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        if self.unit_position(victim).is_none() {
            return;
        }
        match spell.effects[i].implicit_target_b {
            tgt::LOCATION_CASTER_HOME_BIND => {
                if !victim.is_player() {
                    return;
                }
                let Some(loc) = self
                    .players
                    .get(&victim)
                    .map(|entry| entry.player.homebind.location)
                else {
                    return;
                };
                self.teleport_to_location(victim, loc);
            }
            tgt::LOCATION_DATABASE | tgt::ENEMY_AOE_AT_SRC_LOC_SCRIPT => {
                let Some(loc) = self.stores.spell_target_positions.get(spell.id) else {
                    return;
                };
                self.teleport_to_location(victim, loc);
            }
            tgt::LOCATION_CASTER_DEST => {
                let (Some(mut dest), Some(caster_pos)) =
                    (self.unit_position(victim), self.unit_position(caster))
                else {
                    return;
                };
                dest.o = caster_pos.o;
                self.near_teleport_unit(caster, dest);
            }
            _ => {
                let Some(mut dest) = client_dest else {
                    return;
                };
                if let Some(victim_pos) = self.unit_position(victim) {
                    dest.o = victim_pos.o;
                }
                self.near_teleport_unit(victim, dest);
            }
        }
    }

    /// Directional dest-fill angle offset
    /// (`Spell.cpp` target-map build).
    fn directional_offset(mode: u32) -> Option<f32> {
        use std::f32::consts::PI;
        match mode {
            tgt::LOCATION_CASTER_FRONT_LEFT | tgt::LOCATION_UNIT_MINION_POSITION => Some(PI * 0.25),
            tgt::LOCATION_CASTER_BACK_LEFT => Some(PI * 0.75),
            tgt::LOCATION_CASTER_BACK => Some(PI),
            tgt::LOCATION_CASTER_BACK_RIGHT => Some(PI * 1.25),
            tgt::LOCATION_CASTER_FRONT_RIGHT => Some(PI * 1.75),
            tgt::LOCATION_CASTER_LEFT => Some(PI / 2.0),
            tgt::LOCATION_CASTER_RIGHT => Some(-PI / 2.0),
            tgt::LOCATION_CASTER_FRONT | tgt::LOCATION_CASTER_FRONT_LEAP => Some(0.0),
            _ => None,
        }
    }

    /// `WorldObject::MovePositionToFirstCollision`
    /// (`Object.cpp:1999`): navmesh path toward
    /// `from + dist·angle` (`PathFinder::calculate
    /// (src, dest, false, true)`, back point wins),
    /// `UpdateAllowedPositionZ`, the two vmap
    /// `GetLosHitPosition` trims at collision-height
    /// offsets (−1.0f horizontal, −0.5f vertical;
    /// static tree only — the dynamic gameobject
    /// half is not modeled), then the 10-step
    /// Z-safety walkback and `NormalizeMapCoord`.
    /// The water-surface arm of
    /// `UpdateAllowedPositionZ` rides the same
    /// `ground_z` approximation as the rest of the
    /// motion code.
    fn first_collision_dest(
        &mut self,
        from: Position,
        angle: f32,
        mut dist: f32,
        half_height: f32,
    ) -> Position {
        /// `ATTACK_DISTANCE` (`ObjectDefines.h:25`).
        const ATTACK_DISTANCE: f32 = 5.0;
        if dist <= 0.0 {
            return from;
        }
        let mut dest_x = from.x + dist * angle.cos();
        let mut dest_y = from.y + dist * angle.sin();
        let mut dest_z = from.z;
        let end = Position::new(dest_x, dest_y, dest_z + half_height, from.o);
        let path = self.find_path_points(from, end, false, true);
        if let Some(back) = path.points.last() {
            dest_x = back.x;
            dest_y = back.y;
            dest_z = back.z;
        }
        dest_z = self.ground_z(dest_x, dest_y, dest_z);
        // Horizontal trim at eye level, then the
        // vertical probe onto the destination.
        if let Some((hx, hy, hz)) = self.get_los_hit_position(
            from.x,
            from.y,
            from.z + half_height,
            dest_x,
            dest_y,
            dest_z + half_height,
            -1.0,
        ) {
            dest_x = hx;
            dest_y = hy;
            dest_z = hz - half_height;
            dist = ((from.x - dest_x).powi(2) + (from.y - dest_y).powi(2)).sqrt();
        }
        if let Some((_, _, hz)) = self.get_los_hit_position(
            dest_x,
            dest_y,
            dest_z + half_height,
            dest_x,
            dest_y,
            dest_z,
            -0.5,
        ) {
            dest_z = hz;
        }
        // Fork Z-safety probe: step back toward the
        // source in tenths until the terrain Z is
        // within ATTACK_DISTANCE of the source Z. A
        // step-to-step cliff (> ATTACK_DISTANCE/2)
        // marks the full destination unsafe; a fully
        // safe probe restores the full destination
        // (fork overwrites the accepted point with
        // `tempPos` too).
        let step = dist / 10.0;
        let full = (dest_x, dest_y, dest_z);
        let mut result = from;
        let mut z_safe = true;
        for _ in 0..10 {
            if (from.z - dest_z).abs() <= ATTACK_DISTANCE {
                result = Position::new(dest_x, dest_y, dest_z, from.o);
                break;
            }
            let previous_z = dest_z;
            dest_x -= step * angle.cos();
            dest_y -= step * angle.sin();
            dest_z = self.ground_z(dest_x, dest_y, dest_z);
            if (previous_z - dest_z).abs() > ATTACK_DISTANCE * 0.5 {
                z_safe = false;
            }
        }
        if z_safe {
            result = Position::new(full.0, full.1, full.2, from.o);
        }
        result.x = result.x.clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
        result.y = result.y.clamp(-MAX_MAP_COORD, MAX_MAP_COORD);
        result.z = self.ground_z(result.x, result.y, result.z);
        result
    }

    /// `Spell::EffectLeapForward`
    /// (`SpellEffects.cpp:5187`) with the directional
    /// destination fill from target-map build
    /// (client dest wins when present, as in the
    /// fork).
    fn leap_effect(
        &mut self,
        caster: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        victim: ObjectGuid,
        client_dest: Option<Position>,
    ) {
        let dest = if let Some(dest) = client_dest {
            dest
        } else {
            let Some(from) = self.unit_position(caster) else {
                return;
            };
            let eff = spell.effects[i];
            let offset = Self::directional_offset(eff.implicit_target_b)
                .or_else(|| Self::directional_offset(eff.implicit_target_a))
                .unwrap_or(0.0);
            let radius = if eff.radius_index == 0 {
                0.0
            } else {
                self.effect_radius(spell, i)
            };
            // GetCollisionHeight (model height
            // already scale-ratioed at display set).
            let half_height = self
                .unit_base(caster)
                .map_or(2.0, |u| u.model_collision_height);
            self.first_collision_dest(from, from.o + offset, radius, half_height)
        };
        let mut dest = dest;
        if let Some(victim_pos) = self.unit_position(victim) {
            dest.o = victim_pos.o;
        }
        self.near_teleport_unit(victim, dest);
    }

    /// `Unit::KnockBackFrom` + `Unit::KnockBack`
    /// (`Unit.cpp:9991`): stunned/rooted or
    /// mid-spline units ignore it; only
    /// player-moved units launch in this fork
    /// (creatures and pets are a deliberate no-op —
    /// see the Slice 6 inventory). The controller
    /// packet carries negated vertical speed, and
    /// the pending change stores the wire values.
    fn knock_back_from(
        &mut self,
        victim: ObjectGuid,
        source: ObjectGuid,
        horizontal_speed: f32,
        vertical_speed: f32,
    ) {
        use mangos_entities::unit::{
            KnockbackInfo, MOVEMENT_CHANGE_KNOCK_BACK, PendingMovementChange, UNIT_STATE_ROOT,
            UNIT_STATE_STUNNED,
        };
        let Some(victim_pos) = self.unit_position(victim) else {
            return;
        };
        let blocked = self.unit_base_mut(victim).is_some_and(|unit| {
            unit.has_unit_state(UNIT_STATE_STUNNED | UNIT_STATE_ROOT) || !unit.spline.finalized()
        });
        if blocked {
            return;
        }
        // `InterruptNonMeleeSpells(false)` runs for
        // every unit kind before the player branch.
        self.cancel_cast(victim, true);
        if !victim.is_player() {
            return;
        }
        let angle = if victim == source {
            victim_pos.o + std::f32::consts::PI
        } else {
            let Some(source_pos) = self.unit_position(source) else {
                return;
            };
            (victim_pos.y - source_pos.y).atan2(victim_pos.x - source_pos.x)
        };
        let info = KnockbackInfo {
            vcos: angle.cos(),
            vsin: angle.sin(),
            speed_xy: horizontal_speed,
            speed_z: -vertical_speed,
        };
        let Some(counter) = self.unit_base_mut(victim).map(|unit| {
            let counter = unit.movement_counter_and_inc();
            unit.pending_movement_changes
                .push_back(PendingMovementChange {
                    movement_counter: counter,
                    change_type: MOVEMENT_CHANGE_KNOCK_BACK,
                    new_value: 0.0,
                    apply: false,
                    knockback: Some(info),
                });
            counter
        }) else {
            return;
        };
        let mut payload = PacketWriter::new();
        payload.write_bytes(&Self::packed_guid_bytes(victim));
        payload.write_u32_le(counter);
        payload.write_f32_le(info.vcos);
        payload.write_f32_le(info.vsin);
        payload.write_f32_le(info.speed_xy);
        payload.write_f32_le(info.speed_z);
        let packet = WorldPacket::with_body(op::SMSG_MOVE_KNOCK_BACK, payload.into_vec());
        self.events.push(MapEvent::Packet { to: victim, packet });
        self.events.push(MapEvent::KnockBack {
            player: victim,
            vertical_speed,
        });
    }

    /// `Player::SendEquipError` (`SMSG_INVENTORY_CHANGE_FAILURE`:
    /// result byte, two empty item GUIDs, zero byte).
    fn send_equip_error(&mut self, player: ObjectGuid, result: u8) {
        let mut payload = PacketWriter::new();
        payload.write_u8(result);
        payload.write_u64_le(0);
        payload.write_u64_le(0);
        payload.write_u8(0);
        let packet = WorldPacket::with_body(op::SMSG_INVENTORY_CHANGE_FAILURE, payload.into_vec());
        self.send_to_player(player, packet);
    }

    /// `Spell::DoCreateItem` (`SpellEffects.cpp:1820`)
    /// map half: count from the effect value clamped
    /// to `[1, Stackable]`; unknown prototypes fail
    /// with `EQUIP_ERR_ITEM_NOT_FOUND` (23). The
    /// store itself crosses to the world actor as
    /// [`MapEvent::CreateItem`] (inventory baseline
    /// and DB writes live there); battleground-mark
    /// mail overflow and `UpdateCraftSkill` are
    /// recorded as unmodeled.
    fn create_item_effect(
        &mut self,
        victim: ObjectGuid,
        spell: &SpellEntry,
        i: usize,
        caster_level: u32,
        bp: Option<i32>,
    ) {
        #[allow(clippy::cast_possible_truncation)]
        let item_id = spell.effects[i].item_type as u32;
        self.do_create_item(victim, item_id, spell, i, caster_level, bp);
    }

    /// `Spell::DoCreateItem(effIdx, itemId)` with an
    /// explicit item id (script-computed creates like
    /// healthstones, whose stone rank depends on the
    /// caster's talent aura).
    pub(crate) fn do_create_item(
        &mut self,
        victim: ObjectGuid,
        item_id: u32,
        spell: &SpellEntry,
        i: usize,
        caster_level: u32,
        bp: Option<i32>,
    ) {
        let Some(stackable) = self
            .stores
            .item_prototypes
            .get(item_id)
            .map(|proto| proto.stackable.max(1))
        else {
            self.send_equip_error(victim, 23);
            return;
        };
        let value = self.effect_value_bp(spell, i, caster_level, bp);
        let rolled = mangos_spells::ditheru(value, self.spell_rng());
        let count = rolled.clamp(1, stackable);
        self.events.push(MapEvent::CreateItem {
            player: victim,
            item_id,
            count,
        });
    }

    /// First packed GUID of `SMSG_SPELL_START`/`GO`:
    /// the cast item for item casts, else the caster
    /// (`Spell.cpp:4611/4645`).
    pub(super) fn cast_first_guid(caster: ObjectGuid, cast: &ActiveCast) -> ObjectGuid {
        match cast.cast_item {
            Some(item) => ObjectGuid::with_entry(mangos_guid::HighGuid::ITEM, 0, item.item_guid),
            None => caster,
        }
    }

    /// `Spell::TakeCastItem` (`Spell.cpp:5166`):
    /// walk every prototype spell with charges —
    /// negative proto charges mark the item
    /// expendable, each use steps charges one
    /// toward 0 (written back only when
    /// `Stackable < 2`; `withoutCharges` is
    /// overwritten per charged spell, fork-faithful)
    /// — and an expendable item out of charges
    /// loses one count. Baseline edits land here;
    /// DB writes cross as events.
    pub(super) fn take_cast_item(&mut self, caster: ObjectGuid, item: CastItemCtx) {
        let Some(proto) = self.stores.item_prototypes.get(item.item_entry).cloned() else {
            return;
        };
        let mut events = Vec::new();
        {
            let Some(entry) = self.players.get_mut(&caster) else {
                return;
            };
            let inv = &mut entry.player.deferred.inventory;
            let Some(idx) = inv
                .iter()
                .position(|row| row.item.item_guid == item.item_guid)
            else {
                return;
            };
            let mut charges: Vec<i32> = inv[idx]
                .item
                .charges
                .split_whitespace()
                .map(|token| token.parse().unwrap_or(0))
                .collect();
            charges.resize(5, 0);
            let mut expendable = false;
            let mut without_charges = false;
            let mut changed = false;
            for (i, spell_row) in proto.spells.iter().enumerate() {
                if spell_row.spell_id == 0 || spell_row.charges == 0 {
                    continue;
                }
                if spell_row.charges < 0 {
                    expendable = true;
                }
                let mut charge = charges[i];
                if charge != 0 {
                    charge += if charge > 0 { -1 } else { 1 };
                    if proto.stackable < 2 {
                        charges[i] = charge;
                        changed = true;
                    }
                }
                without_charges = charge == 0;
            }
            if changed {
                let text = format!(
                    "{} {} {} {} {} ",
                    charges[0], charges[1], charges[2], charges[3], charges[4]
                );
                inv[idx].item.charges.clone_from(&text);
                events.push(MapEvent::SetItemCharges {
                    item_guid: item.item_guid,
                    charges: text,
                });
            }
            if expendable && without_charges {
                let remaining = inv[idx].item.count.saturating_sub(1);
                if remaining == 0 {
                    inv.remove(idx);
                } else {
                    inv[idx].item.count = remaining;
                }
                events.push(MapEvent::SetItemCount {
                    item_guid: item.item_guid,
                    count: remaining,
                });
            }
        }
        self.events.extend(events);
    }

    /// `Spell::EffectSummonType` for `SUMMON` /
    /// `SUMMON_WILD` / `SUMMON_GUARDIAN`: temp-summon kind
    /// from effect + duration, scattered around the
    /// victim (caster fallback).
    fn summon_effect(
        &mut self,
        caster: ObjectGuid,
        caster_level: u32,
        spell: &SpellEntry,
        victim: ObjectGuid,
        i: usize,
        bp: Option<i32>,
    ) {
        let eff = spell.effects[i];
        let value = self.effect_value_bp(spell, i, caster_level, bp);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let amount = if value > 0.0 {
            (value as u32).clamp(1, 16)
        } else {
            1
        };
        let duration = spell.calculate_duration_ms(&self.stores.spells.tables);
        #[allow(clippy::cast_sign_loss)]
        let duration_ms = if duration > 0 { duration as u32 } else { 0 };
        let kind = match eff.effect {
            41 => {
                if duration_ms == 0 {
                    7 // TEMPSUMMON_DEAD_DESPAWN
                } else {
                    11 // TEMPSUMMON_TIMED_DEATH_AND_DEAD_DESPAWN
                }
            }
            42 => {
                if duration_ms == 0 {
                    7
                } else {
                    1 // TEMPSUMMON_TIMED_OR_DEAD_DESPAWN
                }
            }
            _ => {
                if duration_ms == 0 {
                    7
                } else {
                    3 // TEMPSUMMON_TIMED_DESPAWN
                }
            }
        };
        let center = self
            .unit_position(victim)
            .or_else(|| self.unit_position(caster))
            .unwrap_or_default();
        for n in 0..amount {
            let pos = if n == 0 {
                center
            } else {
                self.random_point(center, 3.0).unwrap_or(center)
            };
            if let Some(summon) =
                self.summon_creature(eff.misc_value as u32, pos, caster, kind, duration_ms)
            {
                self.spell_script_on_summon(caster, summon, spell);
            }
        }
    }
}
