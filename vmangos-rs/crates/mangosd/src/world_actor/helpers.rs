//! Free-standing quest, packet, inventory, and name helpers
//! used by the world actor.
//!
//! These helpers do not own or mutate `World` state. They
//! build packets and answer predicate/planning questions
//! from player and template inputs; the actor remains the
//! single owner of sessions, maps, and side effects.

#[allow(clippy::wildcard_imports)]
use super::*;

/// The five `CreateDefaultGuildRanks` names
/// (`LANG_GUILD_MASTER` 811..815) in one locale, with
/// the English text as fallback for missing rows.
pub(super) fn default_guild_rank_names(strings: &MangosStrings, locale: u8) -> [String; 5] {
    const FALLBACK: [&str; 5] = ["Guild Master", "Officer", "Veteran", "Member", "Initiate"];
    std::array::from_fn(|index| {
        let entry = 811 + i32::try_from(index).unwrap_or(0);
        strings
            .get(entry)
            .map(|text| text.text(locale))
            .filter(|text| !text.is_empty())
            .map_or_else(|| FALLBACK[index].to_owned(), str::to_owned)
    })
}

/// Appends an `ObjectGuid` in plain wire form (the
/// C++ `ByteBuffer << ObjectGuid` writes the raw
/// u64; only `PackedGuid` streams packed, which the
/// petition/mail handlers do not use).
pub(super) fn write_guid(writer: &mut PacketWriter, guid: ObjectGuid) {
    writer.write_u64_le(guid.raw());
}

// ---- Tier 1 Slice 1 quest checks + packets ----
// Fork anchors: Objects/Player.cpp Satisfy*/Can*
// family, GossipDef.cpp builders. Status ids:
// QUEST_STATUS NONE 0, COMPLETE 1, UNAVAILABLE 2,
// INCOMPLETE 3, AVAILABLE 4, FAILED 5.

use mangos_world_data::quest_template::{QuestTemplate, QuestTemplates};

pub(super) const DIALOG_STATUS_NONE: u8 = 0;
pub(super) const DIALOG_STATUS_UNAVAILABLE: u8 = 1;
pub(super) const DIALOG_STATUS_CHAT: u8 = 2;
pub(super) const DIALOG_STATUS_INCOMPLETE: u8 = 3;
pub(super) const DIALOG_STATUS_REWARD_REP: u8 = 4;
pub(super) const DIALOG_STATUS_AVAILABLE: u8 = 5;
pub(super) const DIALOG_STATUS_REWARD2: u8 = 7;

pub(super) const QUEST_STATUS_COMPLETE: i32 = 1;
pub(super) const QUEST_STATUS_INCOMPLETE: i32 = 3;

pub(super) fn quest_rewarded(player: &Player, quest_id: u32) -> bool {
    player.quest_row(quest_id).is_some_and(|r| r.rewarded != 0)
}

pub(super) fn quest_standing(player: &Player, faction: u32) -> i32 {
    player
        .deferred
        .reputation
        .iter()
        .find(|r| r.faction == faction)
        .map_or(0, |r| r.standing)
}

/// `Player::SendEquipError` (`Player.cpp:11464`):
/// result byte alone for OK, else the required
/// level (level errors only), two item GUIDs, and
/// a zero byte.
pub(super) fn equip_error_packet(result: u8, item: ObjectGuid, required_level: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(result);
    if result != 0 {
        if result == 1 {
            writer.write_u32_le(required_level);
        }
        write_guid(&mut writer, item);
        write_guid(&mut writer, ObjectGuid::EMPTY);
        writer.write_u8(0);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_INVENTORY_CHANGE_FAILURE);
    packet.take_writer(writer);
    packet
}

/// `ReputationMgr::PointsInRank`.
pub(super) const POINTS_IN_RANK: [i32; 8] = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];

/// `ReputationMgr::GetRank` over the baseline:
/// base reputation for the player's race/class fit
/// plus standing, through `ReputationToRank`.
pub(super) fn reputation_rank(
    player: &Player,
    factions: &mangos_world_data::reputations::FactionStore,
    faction: u32,
) -> u32 {
    let Some(entry) = factions.get(faction) else {
        return 3; // unknown faction: reputation 0 -> Neutral
    };
    let race_mask = 1u32 << u32::from(player.unit.race().saturating_sub(1));
    let class_mask = 1u32 << u32::from(player.unit.class().saturating_sub(1));
    let base = entry
        .index_fit_to(race_mask, class_mask)
        .map_or(0, |idx| entry.base_values[idx]);
    let reputation = base + quest_standing(player, faction);
    let mut limit = 43_000i32; // Reputation_Cap + 1
    for rank in (0..8u32).rev() {
        limit -= POINTS_IN_RANK[rank as usize];
        if reputation >= limit {
            return rank;
        }
    }
    0
}

/// `Player::GetQuestLevelForPlayer`.
pub(super) fn quest_level_for_player(q: &QuestTemplate, player_level: u32) -> u32 {
    if q.quest_level > 0 {
        q.quest_level
    } else {
        player_level
    }
}

/// The non-status, non-level `Satisfy*` gates shared
/// by `CanSeeStartQuest` (class, race, skill,
/// exclusive group, reputation, previous-quest and
/// breadcrumb chains; `SatisfyQuestCondition` has
/// no condition store and reads true).
pub(super) fn quest_satisfy_gates(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
) -> bool {
    if q.exclusive_group != 0
        && templates.iter().any(|other| {
            other.exclusive_group == q.exclusive_group
                && other.entry != q.entry
                && (player.quest_status(other.entry) == QUEST_STATUS_INCOMPLETE
                    || player.quest_status(other.entry) == QUEST_STATUS_COMPLETE
                    || quest_rewarded(player, other.entry))
        })
    {
        return false;
    }
    if !quest_satisfy_prev(player, q, templates) {
        return false;
    }
    if q.breadcrumb_for_quest_id != 0
        && (player.quest_status(q.breadcrumb_for_quest_id) != 0
            || quest_rewarded(player, q.breadcrumb_for_quest_id))
    {
        return false;
    }
    if q.required_classes != 0
        && q.required_classes & (1u32 << (player.unit.class().saturating_sub(1))) == 0
    {
        return false;
    }
    if q.required_races != 0
        && q.required_races & (1u32 << (player.unit.race().saturating_sub(1))) == 0
    {
        return false;
    }
    if q.required_skill != 0 {
        let value = player
            .deferred
            .skills
            .iter()
            .find(|s| s.skill == q.required_skill)
            .map_or(0, |s| s.value);
        if value < q.required_skill_value {
            return false;
        }
    }
    if q.required_min_rep_faction != 0
        && quest_standing(player, q.required_min_rep_faction) < q.required_min_rep_value
    {
        return false;
    }
    if q.required_max_rep_faction != 0
        && quest_standing(player, q.required_max_rep_faction) >= q.required_max_rep_value
    {
        return false;
    }
    true
}

pub(super) fn quest_satisfy_prev(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
) -> bool {
    // Single-Value prev-chain subset: the fork
    // derives prevQuests from PrevQuestId plus
    // NextQuestInChain back-references; the
    // back-reference set is rebuilt here by scan.
    let mut prev_ids: Vec<i32> = Vec::new();
    if q.prev_quest_id != 0 {
        prev_ids.push(q.prev_quest_id);
    }
    for other in templates.iter() {
        if other.next_quest_in_chain == q.entry && other.entry != q.entry {
            #[allow(clippy::cast_possible_wrap)]
            prev_ids.push(other.entry as i32);
        }
    }
    if prev_ids.is_empty() {
        return true;
    }
    // Multiple signed ids require one from each
    // sign group; single-value rows reduce to the
    // fork's PrevQuestId rule.
    let mut have_positive = false;
    let mut need_positive = false;
    let mut have_negative = false;
    let mut need_negative = false;
    for prev in prev_ids {
        if prev > 0 {
            need_positive = true;
            #[allow(clippy::cast_sign_loss)]
            let id = prev as u32;
            if player.quest_status(id) == QUEST_STATUS_COMPLETE || quest_rewarded(player, id) {
                have_positive = true;
            }
        } else {
            need_negative = true;
            #[allow(clippy::cast_sign_loss)]
            let id = prev.unsigned_abs();
            if !quest_rewarded(player, id) {
                have_negative = true;
            }
        }
    }
    (!need_positive || have_positive) && (!need_negative || have_negative)
}

/// `Player::CanTakeQuest` (+ `CanAddQuest` log and
/// timed arms). `reason` receives the fork
/// `INVALIDREASON_*` on failure. Skill/condition
/// systems read the loaded rows; the scripted
/// `CanTakeQuest` override and group-share timer
/// escort arm are unmodeled.
#[allow(clippy::too_many_lines)]
pub(super) fn quest_can_take(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
    skip_status: bool,
    reason: &mut u32,
) -> bool {
    let level = player.unit.level();
    // SatisfyQuestStatus (+ repeatable); skipped
    // whole under the fork's `skipStatusCheck`
    // (the auto-complete arm of `CanRewardQuest`).
    if !skip_status {
        let status = player.quest_status(q.entry);
        if !(status == 0 || status == 4 || (status == QUEST_STATUS_COMPLETE && q.is_repeatable())) {
            *reason = 13; // ALREADY_ON (in-quest/ineligible read)
            return false;
        }
        if quest_rewarded(player, q.entry) && !q.is_repeatable() {
            *reason = 2;
            return false;
        }
    }
    // SatisfyQuestExclusiveGroup.
    if q.exclusive_group != 0 {
        let taken = templates.iter().any(|other| {
            other.exclusive_group == q.exclusive_group
                && other.entry != q.entry
                && (player.quest_status(other.entry) == QUEST_STATUS_INCOMPLETE
                    || player.quest_status(other.entry) == QUEST_STATUS_COMPLETE
                    || quest_rewarded(player, other.entry))
        });
        if taken {
            *reason = 2;
            return false;
        }
    }
    if !quest_satisfy_prev(player, q, templates) {
        *reason = 2;
        return false;
    }
    if q.breadcrumb_for_quest_id != 0
        && (player.quest_status(q.breadcrumb_for_quest_id) != 0
            || quest_rewarded(player, q.breadcrumb_for_quest_id))
    {
        *reason = 2;
        return false;
    }
    // Class / race (fork masks: 1 << (id - 1)).
    if q.required_classes != 0
        && q.required_classes & (1u32 << (player.unit.class().saturating_sub(1))) == 0
    {
        *reason = 2;
        return false;
    }
    if q.required_races != 0
        && q.required_races & (1u32 << (player.unit.race().saturating_sub(1))) == 0
    {
        *reason = 6; // WRONG_RACE
        return false;
    }
    if !(q.min_level == 0 || level >= q.min_level) {
        *reason = 1; // FAILED_LOW_LEVEL
        return false;
    }
    if q.quest_level != 0 && level < q.quest_level.saturating_sub(4) {
        *reason = 1;
        return false;
    }
    if q.max_level != 0 && level > q.max_level {
        *reason = 2;
        return false;
    }
    if q.required_skill != 0 {
        let value = player
            .deferred
            .skills
            .iter()
            .find(|s| s.skill == q.required_skill)
            .map_or(0, |s| s.value);
        if value < q.required_skill_value {
            *reason = 2;
            return false;
        }
    }
    if q.required_min_rep_faction != 0
        && quest_standing(player, q.required_min_rep_faction) < q.required_min_rep_value
    {
        *reason = 2;
        return false;
    }
    if q.required_max_rep_faction != 0
        && quest_standing(player, q.required_max_rep_faction) >= q.required_max_rep_value
    {
        *reason = 2;
        return false;
    }
    // No required-money gate at take time: the fork
    // checks the purse only in `CanRewardQuest`.
    // CanAddQuest: only one timed quest at a time.
    if q.limit_time != 0 && !player.quest_timers.is_empty() {
        *reason = 12; // ONLY_ONE_TIMED
        return false;
    }
    true
}

/// `Player::CanRewardQuest` (both overloads; the
/// `msg` error packets are dropped except the
/// required-money arm, which the fork sends
/// unconditionally — it surfaces here as
/// `*reason = 22` for the caller to emit).
/// Auto-complete quests gate on
/// `CanTakeQuest(skipStatusCheck)`, normal quests
/// need status COMPLETE; already-rewarded or
/// missing deliver items fail. The reward
/// overload's item-space check is unmodeled (the
/// DB-row grant drops on a full backpack), and an
/// empty choice slot passes, as in the fork.
pub(super) fn quest_can_reward(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
    reason: &mut u32,
) -> bool {
    if q.is_auto_complete() {
        if !quest_can_take(player, q, templates, true, &mut 0) {
            return false;
        }
    } else if player.quest_status(q.entry) != QUEST_STATUS_COMPLETE {
        return false;
    }
    if quest_rewarded(player, q.entry) {
        return false;
    }
    if q.is_deliver() {
        for i in 0..4 {
            if q.req_item_counts[i] != 0
                && player.quest_item_count(q.req_item_ids[i]) < q.req_item_counts[i]
            {
                return false;
            }
        }
    }
    if q.rew_or_req_money < 0 {
        use mangos_protocol::update_field_table::offsets;
        let money = player
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::PLAYER_FIELD_COINAGE);
        if money < q.rew_or_req_money.unsigned_abs() {
            *reason = 22; // QUEST_FAILED_NOT_ENOUGH_MONEY
            return false;
        }
    }
    true
}

/// `Player::CanCompleteRepeatableQuest`.
pub(super) fn quest_can_complete_repeatable(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
    reason: &mut u32,
) -> bool {
    if !quest_can_take(player, q, templates, false, &mut 0) {
        return false;
    }
    if q.is_deliver() {
        for i in 0..4 {
            if q.req_item_ids[i] != 0
                && q.req_item_counts[i] != 0
                && player.quest_item_count(q.req_item_ids[i]) < q.req_item_counts[i]
            {
                return false;
            }
        }
    }
    quest_can_reward(player, q, templates, reason)
}

/// `Player::CanSeeStartQuest`: the `Satisfy*` gates
/// plus `IsActive`, then the high-level hide window
/// against `MinLevel` (`GetLevel() + diff >=
/// MinLevel`; the negative-config arm is unreachable
/// with the built-in default).
pub(super) fn quest_can_see_start(
    player: &Player,
    q: &QuestTemplate,
    templates: &QuestTemplates,
    high_level_hide_diff: i32,
) -> bool {
    if !q.is_active() || !quest_satisfy_gates(player, q, templates) {
        return false;
    }
    if high_level_hide_diff < 0 {
        return true;
    }
    player.unit.level() + high_level_hide_diff as u32 >= q.min_level
}

/// `Player::PrepareQuestMenu`: (quest id, dialog
/// icon) pairs for one questgiver; ender relations
/// first, then starters gated on status NONE +
/// `CanTakeQuest` (auto-complete quests show as
/// reward). The fork's `exceptQuestId` exclusion
/// has no caller here.
/// Quest-relation starts for a creature or
/// gameobject questgiver entry.
pub(super) fn quest_rel_starts(
    rel: &mangos_world_data::quest_template::QuestRelations,
    entry: u32,
    is_go: bool,
) -> &[u32] {
    if is_go {
        rel.gameobject_starts(entry)
    } else {
        rel.creature_starts(entry)
    }
}

/// Quest-relation ends (see `quest_rel_starts`).
pub(super) fn quest_rel_ends(
    rel: &mangos_world_data::quest_template::QuestRelations,
    entry: u32,
    is_go: bool,
) -> &[u32] {
    if is_go {
        rel.gameobject_ends(entry)
    } else {
        rel.creature_ends(entry)
    }
}

pub(super) fn quest_prepare_menu(
    player: &Player,
    starts: &[u32],
    ends: &[u32],
    templates: &QuestTemplates,
) -> Vec<(u32, u8)> {
    let mut menu: Vec<(u32, u8)> = Vec::new();
    for quest_id in ends {
        let Some(q) = templates.get(*quest_id) else {
            continue;
        };
        if !q.is_active() {
            continue;
        }
        let status = player.quest_status(*quest_id);
        if status == QUEST_STATUS_COMPLETE && !quest_rewarded(player, *quest_id) {
            menu.push((*quest_id, DIALOG_STATUS_REWARD_REP));
        } else if status == QUEST_STATUS_INCOMPLETE {
            menu.push((*quest_id, DIALOG_STATUS_INCOMPLETE));
        } else if status == 4 {
            // QUEST_STATUS_AVAILABLE
            menu.push((*quest_id, DIALOG_STATUS_CHAT));
        }
    }
    for quest_id in starts {
        let Some(q) = templates.get(*quest_id) else {
            continue;
        };
        if !q.is_active() {
            continue;
        }
        if q.is_auto_complete() && quest_can_take(player, q, templates, false, &mut 0) {
            menu.push((*quest_id, DIALOG_STATUS_REWARD_REP));
        } else if player.quest_status(*quest_id) == 0
            && quest_can_take(player, q, templates, false, &mut 0)
        {
            menu.push((*quest_id, DIALOG_STATUS_AVAILABLE));
        }
    }
    menu
}

/// `WorldSession::GetDialogStatus` (creature arm,
/// `defstatus = DIALOG_STATUS_NONE` as the status
/// -query caller passes it; the script-manager
/// status call and gameobject arm are unmodeled).
pub(super) fn quest_dialog_status(
    player: &Player,
    starts: &[u32],
    ends: &[u32],
    templates: &QuestTemplates,
    low_level_hide_diff: i32,
    high_level_hide_diff: i32,
) -> u8 {
    let level = player.unit.level();
    let mut result = DIALOG_STATUS_NONE;
    for quest_id in ends {
        let Some(q) = templates.get(*quest_id) else {
            continue;
        };
        if !q.is_active() {
            continue;
        }
        let status = player.quest_status(*quest_id);
        let new = if (status == QUEST_STATUS_COMPLETE && !quest_rewarded(player, *quest_id))
            || (q.is_auto_complete() && quest_can_take(player, q, templates, false, &mut 0))
        {
            if q.is_auto_complete() && q.is_repeatable() {
                DIALOG_STATUS_REWARD_REP
            } else {
                DIALOG_STATUS_REWARD2
            }
        } else if status == QUEST_STATUS_INCOMPLETE {
            DIALOG_STATUS_INCOMPLETE
        } else {
            DIALOG_STATUS_NONE
        };
        result = result.max(new);
    }
    for quest_id in starts {
        let Some(q) = templates.get(*quest_id) else {
            continue;
        };
        if !q.is_active() {
            continue;
        }
        if player.quest_status(*quest_id) != 0 {
            continue;
        }
        if !quest_can_see_start(player, q, templates, high_level_hide_diff) {
            continue;
        }
        // `SatisfyQuestLevel` is `level >= MinLevel`.
        let new = if level >= q.min_level {
            if q.is_auto_complete()
                || (q.is_repeatable()
                    && q.quest_flags & 0x0000_0100 == 0 // QUEST_FLAGS_UNK2
                    && quest_rewarded(player, *quest_id))
            {
                DIALOG_STATUS_REWARD_REP
            } else if low_level_hide_diff < 0
                || level <= quest_level_for_player(q, level) + low_level_hide_diff as u32
            {
                DIALOG_STATUS_AVAILABLE
            } else {
                DIALOG_STATUS_CHAT
            }
        } else {
            DIALOG_STATUS_UNAVAILABLE
        };
        result = result.max(new);
    }
    result
}

pub(super) fn quest_display_id(
    items: &mangos_world_data::item_prototype::ItemPrototypes,
    item_id: u32,
) -> u32 {
    items.get(item_id).map_or(0, |p| p.display_id)
}

/// `PlayerTalkClass::SendQuestGiverQuestList`.
pub(super) fn build_quest_list(
    npc_guid: ObjectGuid,
    title: &str,
    menu: &[(u32, u8)],
    templates: &QuestTemplates,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, npc_guid);
    writer.write_cstring(title);
    writer.write_u32_le(0); // emote delay
    writer.write_u32_le(0); // emote
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u8(menu.len() as u8);
    for (quest_id, icon) in menu {
        let Some(q) = templates.get(*quest_id) else {
            continue;
        };
        writer.write_u32_le(*quest_id);
        writer.write_u32_le(u32::from(*icon));
        writer.write_u32_le(q.quest_level);
        writer.write_cstring(&q.title);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_QUESTGIVER_QUEST_LIST);
    packet.take_writer(writer);
    packet
}

/// `PlayerTalkClass::SendQuestGiverQuestDetails`.
pub(super) fn build_quest_details(
    npc_guid: ObjectGuid,
    q: &QuestTemplate,
    activate_accept: bool,
    items: &mangos_world_data::item_prototype::ItemPrototypes,
    rate_drop_money: f32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, npc_guid);
    writer.write_u32_le(q.entry);
    writer.write_cstring(&q.title);
    writer.write_cstring(&q.details);
    writer.write_cstring(&q.objectives);
    writer.write_u32_le(u32::from(activate_accept));
    if q.quest_flags & 0x0200 != 0 {
        // QUEST_FLAGS_HIDDEN_REWARDS: three zero u32s.
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
    } else {
        // The fork iterates the leading `count` slots
        // (count = nonzero tally), writing any zero
        // slot among them verbatim.
        #[allow(clippy::cast_possible_truncation)]
        writer.write_u32_le(q.rew_choice_items_count() as u32);
        for i in 0..q.rew_choice_items_count() {
            writer.write_u32_le(q.rew_choice_item_ids[i]);
            writer.write_u32_le(q.rew_choice_item_counts[i]);
            writer.write_u32_le(quest_display_id(items, q.rew_choice_item_ids[i]));
        }
        #[allow(clippy::cast_possible_truncation)]
        writer.write_u32_le(q.rew_items_count() as u32);
        for i in 0..q.rew_items_count() {
            writer.write_u32_le(q.rew_item_ids[i]);
            writer.write_u32_le(q.rew_item_counts[i]);
            writer.write_u32_le(quest_display_id(items, q.rew_item_ids[i]));
        }
        #[allow(clippy::cast_sign_loss)]
        writer.write_u32_le(q.rew_or_req_money(rate_drop_money) as u32);
    }
    writer.write_u32_le(q.rew_spell);
    writer.write_u32_le(4); // QUEST_EMOTE_COUNT
    for i in 0..4 {
        writer.write_u32_le(q.details_emotes[i]);
        writer.write_u32_le(q.details_emote_delays[i]);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_QUESTGIVER_QUEST_DETAILS);
    packet.take_writer(writer);
    packet
}

/// `PlayerTalkClass::SendQuestGiverOfferReward`.
pub(super) fn build_quest_offer_reward(
    npc_guid: ObjectGuid,
    q: &QuestTemplate,
    enable_next: bool,
    items: &mangos_world_data::item_prototype::ItemPrototypes,
    rate_drop_money: f32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, npc_guid);
    writer.write_u32_le(q.entry);
    writer.write_cstring(&q.title);
    writer.write_cstring(&q.offer_reward_text);
    writer.write_u32_le(u32::from(enable_next));
    // Emote count stops at the first nonpositive
    // emote (fork compares as signed).
    #[allow(clippy::cast_possible_wrap)]
    let emote_count = q
        .offer_reward_emotes
        .iter()
        .take_while(|e| **e as i32 > 0)
        .count();
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(emote_count as u32);
    for i in 0..emote_count {
        writer.write_u32_le(q.offer_reward_emote_delays[i]);
        writer.write_u32_le(q.offer_reward_emotes[i]);
    }
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(q.rew_choice_items_count() as u32);
    for i in 0..q.rew_choice_items_count() {
        writer.write_u32_le(q.rew_choice_item_ids[i]);
        writer.write_u32_le(q.rew_choice_item_counts[i]);
        writer.write_u32_le(quest_display_id(items, q.rew_choice_item_ids[i]));
    }
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(q.rew_items_count() as u32);
    for i in 0..q.rew_items_count() {
        writer.write_u32_le(q.rew_item_ids[i]);
        writer.write_u32_le(q.rew_item_counts[i]);
        writer.write_u32_le(quest_display_id(items, q.rew_item_ids[i]));
    }
    #[allow(clippy::cast_sign_loss)]
    writer.write_u32_le(q.rew_or_req_money(rate_drop_money) as u32);
    writer.write_u32_le(0); // unused
    writer.write_u32_le(q.rew_spell);
    let mut packet = WorldPacket::new(opcode_id::SMSG_QUESTGIVER_OFFER_REWARD);
    packet.take_writer(writer);
    packet
}

/// `PlayerTalkClass::SendQuestGiverRequestItems`.
/// `None` mirrors the fork's delegation: empty
/// request text, or no required items while
/// completable, sends the offer-reward packet
/// instead (the caller builds it with
/// `EnableNext = true`).
pub(super) fn build_quest_request_items(
    npc_guid: ObjectGuid,
    q: &QuestTemplate,
    completable: bool,
    close_on_cancel: bool,
    items: &mangos_world_data::item_prototype::ItemPrototypes,
    rate_drop_money: f32,
) -> Option<WorldPacket> {
    let req_count = q.req_item_ids.iter().filter(|id| **id != 0).count();
    if q.request_items_text.is_empty() || (req_count == 0 && completable) {
        return None;
    }
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, npc_guid);
    writer.write_u32_le(q.entry);
    writer.write_cstring(&q.title);
    writer.write_cstring(&q.request_items_text);
    writer.write_u32_le(0); // emote delay
    writer.write_u32_le(if completable {
        q.complete_emote
    } else {
        q.incomplete_emote
    });
    writer.write_u32_le(u32::from(close_on_cancel));
    let scaled = q.rew_or_req_money(rate_drop_money);
    writer.write_u32_le(if scaled < 0 { scaled.unsigned_abs() } else { 0 });
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u32_le(req_count as u32);
    for i in 0..4 {
        if q.req_item_ids[i] == 0 {
            continue;
        }
        writer.write_u32_le(q.req_item_ids[i]);
        writer.write_u32_le(q.req_item_counts[i]);
        writer.write_u32_le(quest_display_id(items, q.req_item_ids[i]));
    }
    writer.write_u32_le(0x02);
    writer.write_u32_le(if completable { 0x03 } else { 0x00 });
    writer.write_u32_le(0x04);
    writer.write_u32_le(0x08);
    let mut packet = WorldPacket::new(opcode_id::SMSG_QUESTGIVER_REQUEST_ITEMS);
    packet.take_writer(writer);
    Some(packet)
}

/// `WorldSession::HandleQuestQueryOpcode` body
/// (`SMSG_QUEST_QUERY_RESPONSE`).
pub(super) fn build_quest_query_response(q: &QuestTemplate) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(q.entry);
    writer.write_u32_le(q.method);
    writer.write_u32_le(q.quest_level);
    writer.write_u32_le(q.zone_or_sort as u32);
    writer.write_u32_le(q.quest_type);
    writer.write_u32_le(q.rep_objective_faction);
    writer.write_u32_le(q.rep_objective_value as u32);
    writer.write_u32_le(0);
    writer.write_u32_le(0);
    writer.write_u32_le(q.next_quest_in_chain);
    if q.quest_flags & 0x0200 != 0 {
        writer.write_u32_le(0);
    } else {
        writer.write_u32_le(q.rew_or_req_money(1.0).max(0) as u32);
    }
    writer.write_u32_le(q.rew_money_max_level);
    writer.write_u32_le(q.rew_spell);
    writer.write_u32_le(q.src_item_id);
    writer.write_u32_le(q.quest_flags);
    if q.quest_flags & 0x0200 != 0 {
        for _ in 0..10 {
            writer.write_u32_le(0);
            writer.write_u32_le(0);
        }
    } else {
        for i in 0..4 {
            writer.write_u32_le(q.rew_item_ids[i]);
            writer.write_u32_le(q.rew_item_counts[i]);
        }
        for i in 0..6 {
            writer.write_u32_le(q.rew_choice_item_ids[i]);
            writer.write_u32_le(q.rew_choice_item_counts[i]);
        }
    }
    writer.write_u32_le(q.point_map_id);
    writer.write_f32_le(q.point_x);
    writer.write_f32_le(q.point_y);
    writer.write_u32_le(q.point_opt);
    writer.write_cstring(&q.title);
    writer.write_cstring(&q.objectives);
    writer.write_cstring(&q.details);
    writer.write_cstring(&q.end_text);
    for i in 0..4 {
        let id = q.req_creature_or_go_ids[i];
        let wire = if id < 0 {
            (id.unsigned_abs()) | 0x8000_0000
        } else {
            #[allow(clippy::cast_sign_loss)]
            {
                id as u32
            }
        };
        writer.write_u32_le(wire);
        writer.write_u32_le(q.req_creature_or_go_counts[i]);
        writer.write_u32_le(q.req_item_ids[i]);
        writer.write_u32_le(q.req_item_counts[i]);
    }
    for text in &q.objective_texts {
        writer.write_cstring(text);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_QUEST_QUERY_RESPONSE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_PETITION_SHOW_SIGNATURES` + `BuildSignatureData`.
pub(super) fn show_signatures_packet(
    item_guid: ObjectGuid,
    owner_guid: ObjectGuid,
    petition: &Petition,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, item_guid);
    write_guid(&mut writer, owner_guid);
    writer.write_u32_le(petition.id);
    writer.write_u8(petition.signature_count() as u8);
    for signature in &petition.signatures {
        write_guid(&mut writer, player_guid(signature.player_low));
        writer.write_u32_le(0);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_PETITION_SHOW_SIGNATURES);
    packet.take_writer(writer);
    packet
}

/// `SMSG_PETITION_SIGN_RESULTS`.
pub(super) fn sign_results_packet(
    item_guid: ObjectGuid,
    signer: ObjectGuid,
    result: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, item_guid);
    write_guid(&mut writer, signer);
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_PETITION_SIGN_RESULTS);
    packet.take_writer(writer);
    packet
}

/// `SMSG_TURN_IN_PETITION_RESULTS`.
pub(super) fn turn_in_results_packet(result: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_TURN_IN_PETITION_RESULTS);
    packet.take_writer(writer);
    packet
}

/// `Player::SendBuyError` for the guild charter
/// (0 = can't find item, 2 = not enough money).
pub(super) fn buy_failed_packet(creature_guid: ObjectGuid, error: u8) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, creature_guid);
    writer.write_u32_le(mangos_db::petition::GUILD_CHARTER_ENTRY);
    writer.write_u8(error);
    let mut packet = WorldPacket::new(opcode_id::SMSG_BUY_FAILED);
    packet.take_writer(writer);
    packet
}

/// A resolved mail receiver (online or offline).
pub(super) struct MailReceiver {
    pub(super) low: u32,
    pub(super) session: Option<SessionId>,
    pub(super) race: u8,
    pub(super) account: u32,
}

/// Parsed `CMSG_SEND_MAIL` fields passed between the
/// handler halves.
pub(super) struct SendMailRequest {
    pub(super) subject: String,
    pub(super) body: String,
    pub(super) money: u32,
    pub(super) cod: u32,
    pub(super) item_raw: u64,
}

/// One outgoing mail's variable fields
/// (`MailDraft` + `MailSender` flattened).
pub(super) struct OutgoingMail {
    pub(super) subject: String,
    pub(super) item_text_id: u32,
    pub(super) mail_template_id: u16,
    pub(super) stationery: u8,
    pub(super) message_type: u8,
    pub(super) money: u32,
    pub(super) cod: u32,
    pub(super) checked: u32,
    pub(super) deliver_delay_secs: u32,
}

/// `SendEquipError(EQUIP_ERR_INVENTORY_FULL, ...)` (50).
pub(super) fn inventory_full_packet() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(50);
    write_guid(&mut writer, ObjectGuid::EMPTY);
    write_guid(&mut writer, ObjectGuid::EMPTY);
    writer.write_u8(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_INVENTORY_CHANGE_FAILURE);
    packet.take_writer(writer);
    packet
}

/// `Player::SendNewItem` (`Player.cpp:11854`) for a
/// created item: received + created + showInChat,
/// bag slot byte (255 for the backpack), then the
/// slot — or `0xFFFF_FFFF` when the grant stacked
/// onto (or split across) existing stacks.
pub(super) fn new_item_packet(
    player: ObjectGuid,
    bag_slot: u8,
    slot: u32,
    item_id: u32,
    count: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, player);
    writer.write_u32_le(1); // received
    writer.write_u32_le(1); // created
    writer.write_u32_le(1); // showInChat
    writer.write_u8(bag_slot);
    writer.write_u32_le(slot);
    writer.write_u32_le(item_id);
    writer.write_u32_le(0); // suffix factor
    writer.write_u32_le(0); // random property id
    writer.write_u32_le(count);
    let mut packet = WorldPacket::new(opcode_id::SMSG_ITEM_PUSH_RESULT);
    packet.take_writer(writer);
    packet
}

/// One `ItemPosCount` destination from the
/// `CanStoreNewItem` subset below.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StoreDest {
    Merge {
        item_guid: u32,
        add: u32,
        new_count: u32,
    },
    New {
        bag: u32,
        slot: u32,
        count: u32,
    },
}

/// `Player::_CanStoreItem(NULL_BAG, NULL_SLOT, ...)`
/// destination plan over the row baseline:
/// unique `max_count` cap first
/// (`_CanTakeMoreSimilarItems` count arm; bank
/// rows count), then stack merges, then free
/// slots — each pass walking the backpack
/// (slots 23..39) before equipped bags
/// (19..23) in order. Recorded subset: no
/// keyring priority, no `BagFamily`
/// specialization, no unique-equipped checks.
pub(super) fn plan_store_new_item(
    inv: &[mangos_db::character_load::InventoryRow],
    proto: &mangos_world_data::item_prototype::ItemPrototype,
    mut count: u32,
    prototypes: &mangos_world_data::item_prototype::ItemPrototypes,
) -> Vec<StoreDest> {
    let item_id = proto.item_id;
    let stackable = proto.stackable.max(1);
    if proto.max_count > 0 {
        let carried: u32 = inv
            .iter()
            .filter(|row| row.item.item_id == item_id)
            .map(|row| row.item.count)
            .sum();
        count = count.min(proto.max_count.saturating_sub(carried));
    }
    // (container bag key, slot range) in fork walk order.
    let mut containers: Vec<(u32, Vec<u32>)> = vec![(0, (23..39).collect())];
    for bag_slot in 19..23u8 {
        let Some(bag_row) = inv.iter().find(|row| row.bag == 0 && row.slot == bag_slot) else {
            continue;
        };
        let capacity = prototypes
            .get(bag_row.item.item_id)
            .map_or(0, |bag_proto| bag_proto.container_slots);
        if capacity > 0 {
            containers.push((bag_row.item.item_guid, (0..capacity).collect()));
        }
    }
    let mut dest = Vec::new();
    if stackable > 1 {
        for (bag_key, slots) in &containers {
            for slot in slots {
                if count == 0 {
                    break;
                }
                let Some(row) = inv.iter().find(|row| {
                    row.bag == *bag_key
                        && u32::from(row.slot) == *slot
                        && row.item.item_id == item_id
                }) else {
                    continue;
                };
                let room = stackable.saturating_sub(row.item.count);
                if room == 0 {
                    continue;
                }
                let add = count.min(room);
                dest.push(StoreDest::Merge {
                    item_guid: row.item.item_guid,
                    add,
                    new_count: row.item.count + add,
                });
                count -= add;
            }
        }
    }
    for (bag_key, slots) in &containers {
        for slot in slots {
            if count == 0 {
                break;
            }
            if inv
                .iter()
                .any(|row| row.bag == *bag_key && u32::from(row.slot) == *slot)
            {
                continue;
            }
            let take = count.min(stackable);
            dest.push(StoreDest::New {
                bag: *bag_key,
                slot: *slot,
                count: take,
            });
            count -= take;
        }
    }
    dest
}

/// `CHAR_NAME_SUCCESS` (`SharedDefines.h:1594`).
pub(super) const CHAR_NAME_SUCCESS: u8 = 82;

/// `ObjectMgr::CheckPlayerName(name, create=true)`
/// (`ObjectMgr.cpp:9195`) minus `ValidateName`'s
/// profane/reserved DBC regexes (no stores; recorded
/// unmodeled). Returns the `CHAR_NAME_*` code.
pub(super) fn check_player_name(
    name: &str,
    min_name: u32,
    strict_mask: u32,
    realm_zone: u32,
) -> u8 {
    const TOO_SHORT: u8 = 70;
    const TOO_LONG: u8 = 71;
    const MIXED_LANGUAGES: u8 = 73;
    let len = name.chars().count();
    if len > 12 {
        return TOO_LONG;
    }
    if len < min_name as usize {
        return TOO_SHORT;
    }
    if !is_valid_name_string(name, strict_mask, realm_zone) {
        return MIXED_LANGUAGES;
    }
    CHAR_NAME_SUCCESS
}

/// `isValidString(wstr, strictMask, numericOrSpace=false,
/// create=true)` (`ObjectMgr.cpp:9159`) over the fork's
/// `Util.h` character families.
pub(super) fn is_valid_name_string(name: &str, strict_mask: u32, realm_zone: u32) -> bool {
    let basic = |c: char| c.is_ascii_alphabetic();
    let extended = |c: char| {
        basic(c)
            || matches!(c as u32, 0x00C0..=0x00D6 | 0x00D8..=0x00DF | 0x00E0..=0x00F6
                | 0x00F8..=0x00FE | 0x0100..=0x012F | 0x1E9E)
    };
    let cyrillic = |c: char| matches!(c as u32, 0x0410..=0x044F | 0x0401 | 0x0451);
    let east_asian = |c: char| {
        matches!(c as u32, 0x1100..=0x11F9 | 0x3041..=0x30FF | 0x3131..=0x318E
            | 0x31F0..=0x31FF | 0x3400..=0x4DB5 | 0x4E00..=0x9FC3
            | 0xAC00..=0xD7A3 | 0xFF01..=0xFFEE)
    };
    let all = |pred: &dyn Fn(char) -> bool| name.chars().all(pred);
    if strict_mask == 0 {
        return all(&extended) || all(&cyrillic) || all(&east_asian);
    }
    if strict_mask & 0x2 != 0 {
        // `GetRealmLanguageType(create=true)` by RealmZone.
        let zone_ok = match realm_zone {
            0 | 1 | 5 | 6 => all(&extended) || all(&cyrillic) || all(&east_asian),
            2..=4 | 7..=10 => all(&extended),
            12..=14 => all(&east_asian),
            11 => all(&cyrillic),
            _ => all(&basic),
        };
        if zone_ok {
            return true;
        }
    }
    if strict_mask & 0x1 != 0 && all(&basic) {
        return true;
    }
    false
}

/// `Player::IsActionButtonDataValid` with a null player
/// (`Player.cpp:6008`; the `MasterPlayer::Create`
/// seeding filter): bounds, spell/item existence.
pub(super) fn is_action_button_data_valid(
    button: u8,
    action: u32,
    action_type: u8,
    spell_exists: impl Fn(u32) -> bool,
    item_exists: impl Fn(u32) -> bool,
) -> bool {
    if usize::from(button) >= crate::player_login::MAX_ACTION_BUTTONS {
        return false;
    }
    if action >= 0x0100_0000 {
        return false; // MAX_ACTION_BUTTON_ACTION_VALUE
    }
    match action_type {
        0 => spell_exists(action), // ACTION_BUTTON_SPELL
        64 => item_exists(action), // ACTION_BUTTON_ITEM
        _ => true,
    }
}
