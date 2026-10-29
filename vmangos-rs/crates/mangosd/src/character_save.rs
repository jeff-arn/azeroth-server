//! Character-save snapshot assembly (Phase 6 Slice 1):
//! builds the `mangos_db::character_save` bundle from
//! the live `Player` aggregate and the map-side aura /
//! cooldown snapshots, mirroring fork `SaveToDB`'s
//! value choices (`Player.cpp:16229`). Scheduling and
//! the `IsSavingDisabled` bot gate live in the actor.

use mangos_db::character_load::{AuraRow, SpellCooldownRow};
use mangos_db::character_save::{
    CharacterSaveBundle, CharacterSaveRow, CharacterStatsRow, InventorySaveItem, ItemSaveState,
    LootSave, diff_actions, diff_quests, diff_reputation, diff_skills, diff_spells,
    forgotten_to_save,
};
use mangos_entities::aura::{CooldownSnapshot, SavableAura};
use mangos_entities::player::Player;
use mangos_protocol::update_field_table::offsets;

/// Actor-supplied per-save facts.
#[derive(Clone, Copy, Debug)]
pub struct SaveContext {
    pub map_id: u32,
    pub instance_id: u32,
    /// `GetCachedZoneId`.
    pub zone: u32,
    /// Fork `online` bind (false on logout saves).
    pub online: bool,
    /// `time(nullptr)` at save start.
    pub logout_time: u64,
    /// `_SaveStats` caller gate (logout or config).
    pub save_stats: bool,
    /// `Player::CanSpeak` (account mute not active).
    pub can_speak: bool,
}

fn byte_of(word: u32, index: u32) -> u8 {
    #[allow(clippy::cast_possible_truncation)]
    {
        (word >> (index * 8)) as u8
    }
}

/// `finiteAlways` + orientation normalization
/// (`Geometry::NormalizeOrientation`: fmod into
/// `[0, 2*PI)`).
fn finite_normalized(o: f32) -> f32 {
    let o = if o.is_finite() { o } else { 0.0 };
    let tau = std::f32::consts::TAU;
    let r = o % tau;
    if r < 0.0 { r + tau } else { r }
}

/// Space-separated words with trailing space
/// (`ostringstream` `<<` loop form).
fn words(words: &[u32]) -> String {
    let mut out = String::new();
    for w in words {
        out.push_str(&w.to_string());
        out.push(' ');
    }
    out
}

fn updated_character_flags(player: &Player, can_speak: bool) -> u32 {
    let v = player.unit.world.object.values();
    let player_flags = v.get_u32(offsets::PLAYER_FLAGS);
    let unit_flags = v.get_u32(offsets::UNIT_FIELD_FLAGS);
    let mut flags = player.character_flags;
    let mut set = |bit: u32, present: bool| {
        if present {
            flags |= bit;
        } else {
            flags &= !bit;
        }
    };
    set(0x0000_0002, player_flags & 0x0000_0020 != 0);
    set(0x0000_0008, !can_speak);
    set(0x0000_0020, unit_flags & 0x0000_0080 != 0);
    set(
        0x0000_0040,
        byte_of(v.get_u32(offsets::UNIT_FIELD_BYTES_2), 1) & 0x01 != 0,
    );
    set(
        0x0000_0200,
        byte_of(v.get_u32(offsets::PLAYER_FIELD_BYTES), 3) != 0,
    );
    set(0x0000_0400, player_flags & 0x0000_0400 != 0);
    set(0x0000_0800, player_flags & 0x0000_0800 != 0);
    set(0x0000_1000, unit_flags & 0x0400_0000 != 0);
    set(0x0000_2000, player_flags & 0x0000_0010 != 0);
    set(0x0001_0000, player_flags & 0x0000_0200 != 0);
    set(0x0002_0000, player_flags & 0x0000_0008 != 0);
    flags
}

fn equipment_cache(player: &Player) -> String {
    const MAX_VISIBLE_ITEM_OFFSET: u16 = 12;
    const EQUIPMENT_SLOT_END: u16 = 19;
    let v = player.unit.world.object.values();
    let mut cache_words: Vec<u32> = Vec::with_capacity(40);
    for slot in 0..EQUIPMENT_SLOT_END {
        let base = offsets::PLAYER_VISIBLE_ITEM_1_0 + slot * MAX_VISIBLE_ITEM_OFFSET;
        let ench_perm = v.get_u32(base + 1);
        let ench_temp = v.get_u32(base + 2);
        cache_words.push(v.get_u32(base));
        cache_words.push(ench_perm | (ench_temp << 16));
    }
    // INVENTORY_SLOT_BAG_START (19): equipped bag item.
    let bag_item = player
        .saved
        .inventory
        .iter()
        .find(|inv| inv.bag == 0 && inv.slot == 19)
        .map_or(0, |inv| inv.item.item_id);
    cache_words.push(bag_item);
    cache_words.push(0);

    words(&cache_words)
}

fn character_row(player: &Player, ctx: &SaveContext) -> CharacterSaveRow {
    let unit = &player.unit;
    let v = unit.world.object.values();
    let pos = unit.world.position();
    let player_bytes = v.get_u32(offsets::PLAYER_BYTES);
    let player_bytes_2 = v.get_u32(offsets::PLAYER_BYTES_2);

    // `known_taxi_mask`: TaxiMaskSize (8) words.
    let mut taxi = [0u32; 8];
    for (slot, word) in taxi.iter_mut().zip(player.taxi_mask.iter()) {
        *slot = *word;
    }
    // `explored_zones`: PLAYER_EXPLORED_ZONES_SIZE words.
    let mut explored = [0u32; 64];
    for (i, slot) in explored.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        {
            *slot = v.get_u32(offsets::PLAYER_EXPLORED_ZONES_1 + i as u16);
        }
    }
    CharacterSaveRow {
        guid: unit.guid().counter(),
        account: player.account_id,
        name: unit.world.name.clone(),
        race: unit.race(),
        class: unit.class(),
        gender: unit.gender(),
        level: unit.level(),
        xp: v.get_u32(offsets::PLAYER_XP),
        money: v.get_u32(offsets::PLAYER_FIELD_COINAGE),
        skin: byte_of(player_bytes, 0),
        face: byte_of(player_bytes, 1),
        hair_style: byte_of(player_bytes, 2),
        hair_color: byte_of(player_bytes, 3),
        facial_hair: u32::from(byte_of(player_bytes_2, 0)),
        bank_bag_slots: u32::from(byte_of(player_bytes_2, 2)),
        character_flags: updated_character_flags(player, ctx.can_speak),
        map: ctx.map_id,
        instance: ctx.instance_id,
        position_x: pos.x,
        position_y: pos.y,
        position_z: pos.z,
        orientation: finite_normalized(pos.o),
        // Transport objects are unmodeled (guid 0), but
        // fork always writes the live movement-info
        // transport offsets.
        transport_guid: 0,
        transport_x: unit.world.movement_info.transport_pos.x,
        transport_y: unit.world.movement_info.transport_pos.y,
        transport_z: unit.world.movement_info.transport_pos.z,
        transport_o: finite_normalized(unit.world.movement_info.transport_pos.o),
        known_taxi_mask: words(&taxi),
        current_taxi_path: if player.taxi_destinations.len() >= 2 {
            words(&player.taxi_destinations[..2])
        } else {
            String::new()
        },
        online: u32::from(ctx.online),
        played_time_total: player.played_time_total,
        played_time_level: player.played_time_level,
        rest_bonus: player.rest_bonus,
        logout_time: ctx.logout_time,
        reset_talents_multiplier: player.reset_talents_multiplier,
        reset_talents_time: player.reset_talents_time,
        extra_flags: player.extra_flags,
        stable_slots: u32::from(player.stable_slots),
        zone: ctx.zone,
        death_expire_time: player.death_expire_time,
        honor_rank_points: player.honor_rank_points,
        #[allow(clippy::cast_sign_loss)]
        honor_highest_rank: player.honor_highest_rank as u32,
        #[allow(clippy::cast_sign_loss)]
        honor_standing: player.honor_standing as u32,
        #[allow(clippy::cast_sign_loss)]
        honor_last_week_hk: player.honor_last_week_hk as u32,
        honor_last_week_cp: player.honor_last_week_cp,
        #[allow(clippy::cast_sign_loss)]
        honor_stored_hk: player.honor_stored_hk as u32,
        #[allow(clippy::cast_sign_loss)]
        honor_stored_dk: player.honor_stored_dk as u32,
        #[allow(clippy::cast_possible_wrap)]
        watched_faction: v.get_u32(offsets::PLAYER_FIELD_WATCHED_FACTION_INDEX) as i32,
        #[allow(clippy::cast_possible_truncation)]
        drunk: (v.get_u32(offsets::PLAYER_BYTES_3) & 0xFFFE) as u16,
        health: unit.health(),
        power: [
            unit.power(0),
            unit.power(1),
            unit.power(2),
            unit.power(3),
            unit.power(4),
        ],
        explored_zones: words(&explored),
        equipment_cache: equipment_cache(player),
        ammo_id: v.get_u32(offsets::PLAYER_AMMO_ID),
        action_bars: u32::from(byte_of(v.get_u32(offsets::PLAYER_FIELD_BYTES), 2)),
        world_phase_mask: unit.world.world_mask,
        create_time: player.create_time,
    }
}

/// Current skill rows read back from the packed
/// `PLAYER_SKILL_INFO_1_1` triplets (the same fields
/// fork `_SaveSkills` reads).
fn current_skills(player: &Player) -> Vec<mangos_db::character_load::SkillRow> {
    let v = player.unit.world.object.values();
    let mut rows = Vec::new();
    for slot in 0..127u16 {
        let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
        let skill_word = v.get_u32(base);
        if skill_word == 0 {
            continue;
        }
        let packed = v.get_u32(base + 1);
        rows.push(mangos_db::character_load::SkillRow {
            skill: skill_word & 0xFFFF,
            value: packed & 0xFFFF,
            max: packed >> 16,
        });
    }
    rows
}

fn stats_row(player: &Player) -> CharacterStatsRow {
    let unit = &player.unit;
    let v = unit.world.object.values();
    let get_f = |index: u16| f32::from_bits(v.get_u32(index));
    CharacterStatsRow {
        max_health: unit.max_health(),
        max_power: [
            unit.max_power(0),
            unit.max_power(1),
            unit.max_power(2),
            unit.max_power(3),
            unit.max_power(4),
        ],
        strength: get_f(offsets::UNIT_FIELD_STAT0),
        agility: get_f(offsets::UNIT_FIELD_STAT0 + 1),
        stamina: get_f(offsets::UNIT_FIELD_STAT0 + 2),
        intellect: get_f(offsets::UNIT_FIELD_STAT0 + 3),
        spirit: get_f(offsets::UNIT_FIELD_STAT0 + 4),
        resistance: core::array::from_fn(|i| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
            {
                v.get_u32(offsets::UNIT_FIELD_RESISTANCES + i as u16) as i32
            }
        }),
        block_chance: get_f(offsets::PLAYER_BLOCK_PERCENTAGE),
        dodge_chance: get_f(offsets::PLAYER_DODGE_PERCENTAGE),
        parry_chance: get_f(offsets::PLAYER_PARRY_PERCENTAGE),
        crit_chance: get_f(offsets::PLAYER_CRIT_PERCENTAGE),
        ranged_crit_chance: get_f(offsets::PLAYER_RANGED_CRIT_PERCENTAGE),
        // Fork: `GetSpellCritPercent(SPELL_SCHOOL_HOLY)`
        // computed from intellect + auras; the computed
        // crit fields are not modeled yet.
        spell_crit_chance: 0.0,
        attack_power: v.get_u32(offsets::UNIT_FIELD_ATTACK_POWER),
        ranged_attack_power: v.get_u32(offsets::UNIT_FIELD_RANGED_ATTACK_POWER),
        // `PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 1` (fire).
        spell_damage: v.get_u32(offsets::PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 1),
        // `GetTotalAuraModifier(SPELL_AURA_MOD_HEALING_DONE)`:
        // aura modifier sums are unmodeled.
        spell_healing: 0,
    }
}

/// Assemble the full save bundle. `auras`/`cooldowns`
/// are the map-side snapshots. The returned stats
/// row is `Some` when `ctx.save_stats` (written by
/// the caller post-bundle, as fork writes it after
/// commit).
/// Fork quest save projection: `_SaveQuestStatus`
/// writes `timer = now + m_timer/1000` for NEW/CHANGED
/// quests only, where `m_timer` is the live remainder
/// converted at load (0 for non-timed quests). Returns
/// the row set to diff against the baseline — forced
/// quests (timed-active, deliver-recounted at load)
/// carry the projected timer so the diff emits the
/// fork's unconditional rewrite; the rest carry the
/// baseline timer so untouched quests stay untouched —
/// plus the timer every written row must carry. The
/// returned rows are also the post-save baseline
/// (written rows already carry their written timer).
fn quest_projection(
    player: &Player,
    now: u64,
) -> (
    Vec<mangos_db::character_load::QuestStatusRow>,
    std::collections::HashMap<u32, u64>,
) {
    #[allow(clippy::cast_possible_truncation)]
    let elapsed_ms = (now.saturating_sub(player.load_now_unix) * 1000) as u32;
    let baseline: std::collections::HashMap<u32, u64> = player
        .saved
        .quest_status
        .iter()
        .map(|row| (row.quest, row.timer))
        .collect();
    let mut written_timers = std::collections::HashMap::new();
    let current = player
        .deferred
        .quest_status
        .iter()
        .map(|row| {
            let state = player.quest_timers.get(&row.quest);
            let remaining_ms =
                state.map_or(0, |t| t.remaining_ms_at_load.saturating_sub(elapsed_ms));
            let projected = now + u64::from(remaining_ms / 1000);
            written_timers.insert(row.quest, projected);
            let forced = state.is_some_and(|t| t.timed_active)
                || player.quest_load_changed.contains(&row.quest);
            let mut out = row.clone();
            out.timer = if forced {
                projected
            } else if let Some(stored) = baseline.get(&row.quest) {
                *stored
            } else {
                projected
            };
            out
        })
        .collect();
    (current, written_timers)
}

#[must_use]
pub fn build_save_bundle(
    player: &Player,
    auras: Vec<SavableAura>,
    cooldowns: Vec<CooldownSnapshot>,
    ctx: &SaveContext,
) -> (
    CharacterSaveBundle,
    Option<CharacterStatsRow>,
    Vec<mangos_db::character_load::QuestStatusRow>,
) {
    // Fork `_SaveSpells` never inserts dependent spells
    // (implicit lower ranks, skill abilities). No runtime
    // learn path mutates `deferred.spells` yet, so the
    // dependent set is empty by construction; a future
    // learn path must keep dependent rows out of
    // `deferred.spells` or pass them here.
    let (spell_inserts, spell_deletes) =
        diff_spells(&player.saved.spells, &player.deferred.spells, &[]);
    let skills_now = current_skills(player);
    let (skill_inserts, skill_updates, skill_deletes) =
        diff_skills(&player.saved.skills, &skills_now);
    let (mut quest_current, written_timers) = quest_projection(player, ctx.logout_time);
    let (mut quest_inserts, mut quest_updates, quest_deletes) =
        diff_quests(&player.saved.quest_status, &quest_current);
    // Every written row carries the projected timer
    // (fork binds `m_timer/1000 + now` unconditionally),
    // including value-changed quests that were not in
    // the forced set.
    for row in quest_inserts.iter_mut().chain(quest_updates.iter_mut()) {
        if let Some(timer) = written_timers.get(&row.quest) {
            row.timer = *timer;
        }
    }
    for row in &mut quest_current {
        let written = quest_inserts
            .iter()
            .chain(quest_updates.iter())
            .any(|w| w.quest == row.quest);
        if written {
            if let Some(timer) = written_timers.get(&row.quest) {
                row.timer = *timer;
            }
        }
    }
    let reputation_dirty = diff_reputation(&player.saved.reputation, &player.deferred.reputation);
    // Honor CP: fork inserts STATE_NEW entries only;
    // the aggregate keeps no state flags, so new rows
    // are those past the as-loaded multiset.
    let honor_cp_inserts = {
        let mut rest = player.deferred.honor_cp.clone();
        for old in &player.saved.honor_cp {
            if let Some(pos) = rest.iter().position(|r| r == old) {
                rest.remove(pos);
            }
        }
        rest
    };
    // Inventory is DB-backed by the service layer:
    // the fork's item update queue is empty here, so
    // every loaded row saves Unchanged (no statements).
    let inventory = player
        .saved
        .inventory
        .iter()
        .cloned()
        .map(|row| InventorySaveItem {
            row,
            state: ItemSaveState::Unchanged,
            wrapped: false,
            has_saved_loot: false,
            loot: LootSave::Unchanged,
        })
        .collect();
    let bundle = CharacterSaveBundle {
        row: character_row(player, ctx),
        // `_SaveBGData`: outer None skips the delete
        // entirely (`m_needSave` false — the fork
        // early-out, also correct for a player who
        // never queued).
        bg: player.battleground.save_row(),
        buyback_purge: Vec::new(),
        inventory,
        quest_inserts,
        quest_updates,
        quest_deletes,
        spell_inserts,
        spell_deletes,
        cooldowns: cooldowns
            .into_iter()
            .map(|c| SpellCooldownRow {
                spell: c.spell,
                spell_expire_time: c.spell_expire_time,
                category: c.category,
                category_expire_time: c.category_expire_time,
                item_id: c.item_id,
            })
            .collect(),
        auras: auras
            .into_iter()
            .map(|a| AuraRow {
                caster_guid: a.caster_guid,
                item_guid: a.item_guid,
                spell: a.spell,
                stacks: a.stacks,
                charges: a.charges,
                base_points: a.base_points,
                periodic_time: a.periodic_time,
                max_duration: a.max_duration,
                duration: a.duration,
                effect_index_mask: a.effect_index_mask,
            })
            .collect(),
        skill_inserts,
        skill_updates,
        skill_deletes,
        forgotten_skills: forgotten_to_save(&player.deferred.forgotten_skills),
        reputation_dirty,
        honor_cp_inserts,
    };
    let stats = ctx.save_stats.then(|| stats_row(player));
    (bundle, stats, quest_current)
}

/// `MasterPlayer::SaveActions` write lists. Action
/// buttons save only from `LogoutPlayer`, not the
/// periodic `Player::SaveToDB` bundle.
#[must_use]
pub fn build_action_writes(
    player: &Player,
) -> (
    Vec<mangos_db::character_load::ActionRow>,
    Vec<mangos_db::character_load::ActionRow>,
    Vec<u8>,
) {
    diff_actions(&player.saved.actions, &player.deferred.actions)
}

/// The current `character_homebind` row.
#[must_use]
pub fn homebind_row(player: &Player) -> mangos_db::character_load::HomebindRow {
    mangos_db::character_load::HomebindRow {
        map: u32::from(player.homebind.location.map_id),
        zone: player.homebind.area_id,
        position_x: player.homebind.location.x,
        position_y: player.homebind.location.y,
        position_z: player.homebind.location.z,
    }
}

/// Re-baseline after a save (fork `uState =
/// UNCHANGED` per entry): the next diff runs from
/// the just-saved live rows. `projected_quests` is
/// the third return of [`build_save_bundle`] (the
/// rows as written); quest timer remainders re-anchor
/// to the save time and the load-time recount marks
/// clear.
pub fn rebaseline(
    player: &mut Player,
    now: u64,
    projected_quests: Vec<mangos_db::character_load::QuestStatusRow>,
) {
    // `_SaveBGData` tail: `m_needSave = false`, and the
    // retained load row tracks what was just written.
    player.battleground.need_save = false;
    player.deferred.bg_data =
        if player.battleground.bg_instance_id != 0 || player.battleground.in_battleground_queue() {
            Some(mangos_db::character_load::BgDataRow {
                instance_id: player.battleground.bg_instance_id,
                team: mangos_entities::battleground::team_db_value(player.battleground.bg_team),
                join_x: player.battleground.join_pos.1,
                join_y: player.battleground.join_pos.2,
                join_z: player.battleground.join_pos.3,
                join_o: player.battleground.join_pos.4,
                join_map: player.battleground.join_pos.0,
            })
        } else {
            None
        };
    player.saved.spells = player.deferred.spells.clone();
    player.saved.skills = current_skills(player);
    player.saved.quest_status = projected_quests;
    player.saved.reputation = player.deferred.reputation.clone();
    player.saved.honor_cp = player.deferred.honor_cp.clone();
    #[allow(clippy::cast_possible_truncation)]
    let elapsed_ms = (now.saturating_sub(player.load_now_unix) * 1000) as u32;
    for state in player.quest_timers.values_mut() {
        state.remaining_ms_at_load = state.remaining_ms_at_load.saturating_sub(elapsed_ms);
    }
    player.load_now_unix = now;
    player.quest_load_changed.clear();
}

/// Re-baseline action buttons after the logout-only
/// `MasterPlayer::SaveActions` write.
pub fn rebaseline_actions(player: &mut Player) {
    player.saved.actions = player.deferred.actions.clone();
}
