//! The `LoginQueryHolder` transcription
//! (`CharacterHandler.cpp:65-103`): all 21 queries actually
//! issued (22 slots; the battleground-queue slot is never
//! populated in C++), mapped positionally into typed rows.
//!
//! Phase 3 keeps this a read-only snapshot: the C++
//! sub-loaders' *pruning writes* (expired cooldown/aura
//! DELETEs inside `_LoadSpellCooldowns`/`_LoadAuras`, the
//! name-check rename-flag UPDATE in `LoadFromDB`) are NOT
//! reproduced — they are recorded as deferred write paths in
//! PHASE3-PROGRESS.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::characters::AccountDataEntry;
use crate::row::Fields;

/// The 58-column `characters` row (query 0, field map at
/// `Player.cpp:14472-14483`).
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterRow {
    pub guid: u32,
    pub account: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u8,
    pub xp: u32,
    pub money: u32,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub bank_bag_slots: u8,
    pub character_flags: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub map: u32,
    pub orientation: f32,
    pub known_taxi_mask: String,
    pub played_time_total: u32,
    pub played_time_level: u32,
    pub rest_bonus: f32,
    pub logout_time: u64,
    pub reset_talents_multiplier: u32,
    pub reset_talents_time: u64,
    pub transport_guid: u64,
    pub transport_x: f32,
    pub transport_y: f32,
    pub transport_z: f32,
    pub transport_o: f32,
    pub extra_flags: u32,
    pub stable_slots: u8,
    pub death_expire_time: u64,
    pub current_taxi_path: String,
    pub honor_rank_points: f32,
    pub honor_highest_rank: i32,
    pub honor_standing: i32,
    pub honor_last_week_hk: i32,
    pub honor_last_week_cp: f32,
    pub honor_stored_hk: i32,
    pub honor_stored_dk: i32,
    pub watched_faction: i32,
    pub drunk: u16,
    pub health: u32,
    pub power1: u32,
    pub power2: u32,
    pub power3: u32,
    pub power4: u32,
    pub power5: u32,
    pub explored_zones: String,
    pub ammo_id: u32,
    pub action_bars: u8,
    pub world_phase_mask: u32,
    pub create_time: u64,
    pub instance: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundInstanceRow {
    pub id: Option<u32>,
    pub permanent: u8,
    pub map: Option<u32>,
    pub reset_time: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuraRow {
    pub caster_guid: u64,
    pub item_guid: u32,
    pub spell: u32,
    pub stacks: i32,
    pub charges: i32,
    pub base_points: [f32; 3],
    pub periodic_time: [i32; 3],
    pub max_duration: i32,
    pub duration: i32,
    pub effect_index_mask: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpellRow {
    pub spell: u32,
    pub active: u8,
    pub disabled: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuestStatusRow {
    pub quest: u32,
    pub status: i32,
    pub rewarded: u8,
    pub explored: u8,
    pub timer: u64,
    pub mob_count: [u32; 4],
    pub item_count: [u32; 4],
    pub reward_choice: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HonorCpRow {
    pub victim_type: u8,
    pub victim_id: u32,
    pub cp: f32,
    pub date: u32,
    pub kind: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReputationRow {
    pub faction: u32,
    pub standing: i32,
    pub flags: i32,
}

/// Shared shape of the inventory and mailed-items joins; the
/// placement columns differ (`bag`/`slot` vs `mail_id`), kept
/// as the two trailing fields in SELECT order.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemInstanceRow {
    pub creator_guid: u32,
    pub gift_creator_guid: u32,
    pub count: u32,
    pub duration: i32,
    pub charges: String,
    pub flags: u32,
    pub enchantments: String,
    /// Signed on read, per architecture section 5
    /// (`random_property_id` read i16 / written u16).
    pub random_property_id: i16,
    pub durability: u16,
    pub text: u32,
    pub item_guid: u32,
    pub item_id: u32,
    pub generated_loot: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryRow {
    pub item: ItemInstanceRow,
    pub bag: u32,
    pub slot: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MailedItemRow {
    pub item: ItemInstanceRow,
    pub mail_id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemLootRow {
    pub guid: u32,
    pub item_id: u32,
    pub amount: i32,
    pub property: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionRow {
    pub button: u8,
    pub action: u32,
    pub kind: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SocialRow {
    pub friend: u32,
    pub flags: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomebindRow {
    pub map: u32,
    pub zone: u32,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpellCooldownRow {
    pub spell: u32,
    pub spell_expire_time: i64,
    pub category: u32,
    pub category_expire_time: i64,
    pub item_id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuildRow {
    pub guild_id: u32,
    pub rank: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BgDataRow {
    pub instance_id: u32,
    pub team: u32,
    pub join_x: f32,
    pub join_y: f32,
    pub join_z: f32,
    pub join_o: f32,
    pub join_map: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillRow {
    pub skill: u32,
    pub value: u32,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForgottenSkillRow {
    pub skill: u32,
    pub value: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MailRow {
    pub id: u32,
    pub message_type: u8,
    pub sender_guid: u32,
    pub receiver_guid: u32,
    pub subject: String,
    pub item_text_id: u32,
    pub expire_time: i64,
    pub deliver_time: i64,
    pub money: i32,
    pub cod: i32,
    pub checked: u8,
    pub stationery: u8,
    pub mail_template_id: u32,
    pub has_items: u8,
}

/// The full read-only load: one field per issued query, in
/// holder order.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterLoad {
    pub row: CharacterRow,
    pub group_id: Option<u32>,
    pub bound_instances: Vec<BoundInstanceRow>,
    pub auras: Vec<AuraRow>,
    pub spells: Vec<SpellRow>,
    pub quest_status: Vec<QuestStatusRow>,
    pub honor_cp: Vec<HonorCpRow>,
    pub reputation: Vec<ReputationRow>,
    pub inventory: Vec<InventoryRow>,
    pub item_loot: Vec<ItemLootRow>,
    pub actions: Vec<ActionRow>,
    pub social: Vec<SocialRow>,
    pub homebind: Option<HomebindRow>,
    pub spell_cooldowns: Vec<SpellCooldownRow>,
    pub guild: Option<GuildRow>,
    pub bg_data: Option<BgDataRow>,
    pub account_data: Vec<AccountDataEntry>,
    pub skills: Vec<SkillRow>,
    pub mails: Vec<MailRow>,
    pub mailed_items: Vec<MailedItemRow>,
    pub forgotten_skills: Vec<ForgottenSkillRow>,
}

fn query_rows(conn: &mut mysql::Conn, sql: &str, guid_low: u32) -> Result<Vec<Fields>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(sql, (guid_low,))?;
    Ok(rows.into_iter().map(Fields::from_row).collect())
}

fn map_character_row(f: &Fields) -> CharacterRow {
    CharacterRow {
        guid: f.u32(0),
        account: f.u32(1),
        name: f.string(2),
        race: f.u8(3),
        class: f.u8(4),
        gender: f.u8(5),
        level: f.u8(6),
        xp: f.u32(7),
        money: f.u32(8),
        skin: f.u8(9),
        face: f.u8(10),
        hair_style: f.u8(11),
        hair_color: f.u8(12),
        facial_hair: f.u8(13),
        bank_bag_slots: f.u8(14),
        character_flags: f.u32(15),
        position_x: f.f32(16),
        position_y: f.f32(17),
        position_z: f.f32(18),
        map: f.u32(19),
        orientation: f.f32(20),
        known_taxi_mask: f.string(21),
        played_time_total: f.u32(22),
        played_time_level: f.u32(23),
        rest_bonus: f.f32(24),
        logout_time: f.u64(25),
        reset_talents_multiplier: f.u32(26),
        reset_talents_time: f.u64(27),
        transport_guid: f.u64(28),
        transport_x: f.f32(29),
        transport_y: f.f32(30),
        transport_z: f.f32(31),
        transport_o: f.f32(32),
        extra_flags: f.u32(33),
        stable_slots: f.u8(34),
        death_expire_time: f.u64(35),
        current_taxi_path: f.string(36),
        honor_rank_points: f.f32(37),
        honor_highest_rank: f.i32(38),
        honor_standing: f.i32(39),
        honor_last_week_hk: f.i32(40),
        honor_last_week_cp: f.f32(41),
        honor_stored_hk: f.i32(42),
        honor_stored_dk: f.i32(43),
        watched_faction: f.i32(44),
        drunk: f.u16(45),
        health: f.u32(46),
        power1: f.u32(47),
        power2: f.u32(48),
        power3: f.u32(49),
        power4: f.u32(50),
        power5: f.u32(51),
        explored_zones: f.string(52),
        ammo_id: f.u32(53),
        action_bars: f.u8(54),
        world_phase_mask: f.u32(55),
        create_time: f.u64(56),
        instance: f.u32(57),
    }
}

fn map_item_instance(f: &Fields, base: usize) -> ItemInstanceRow {
    ItemInstanceRow {
        creator_guid: f.u32(base),
        gift_creator_guid: f.u32(base + 1),
        count: f.u32(base + 2),
        duration: f.i32(base + 3),
        charges: f.string(base + 4),
        flags: f.u32(base + 5),
        enchantments: f.string(base + 6),
        random_property_id: f.i16(base + 7),
        durability: f.u16(base + 8),
        text: f.u32(base + 9),
        item_guid: 0, // filled by caller (position differs per query)
        item_id: 0,
        generated_loot: 0,
    }
}

const LOADFROM_SQL: &str = "SELECT `guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `xp`, `money`, `skin`, `face`, `hair_style`, `hair_color`, `facial_hair`, `bank_bag_slots`, `character_flags`, `position_x`, `position_y`, `position_z`, `map`, `orientation`, `known_taxi_mask`, `played_time_total`, `played_time_level`, `rest_bonus`, `logout_time`, `reset_talents_multiplier`, `reset_talents_time`, `transport_guid`, `transport_x`, `transport_y`, `transport_z`, `transport_o`, `extra_flags`, `stable_slots`, `death_expire_time`, `current_taxi_path`, `honor_rank_points`, `honor_highest_rank`, `honor_standing`, `honor_last_week_hk`, `honor_last_week_cp`, `honor_stored_hk`, `honor_stored_dk`, `watched_faction`, `drunk`, `health`, `power1`, `power2`, `power3`, `power4`, `power5`, `explored_zones`, `ammo_id`, `action_bars`, `world_phase_mask`, `create_time`, `instance` FROM `characters` WHERE `guid` = ?";

/// Run the holder's 21 queries on one connection, in holder
/// order. `Ok(None)` when the `characters` row does not exist
/// (the C++ `LoadFromDB` "not found" failure).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_character(
    conn: &mut mysql::Conn,
    guid_low: u32,
) -> Result<Option<CharacterLoad>, DbError> {
    let rows = query_rows(conn, LOADFROM_SQL, guid_low)?;
    let Some(row) = rows.first().map(map_character_row) else {
        return Ok(None);
    };
    let progression = load_progression(conn, guid_low)?;
    let items = load_items_and_actions(conn, guid_low)?;
    let social = load_social_and_mail(conn, guid_low)?;
    Ok(Some(CharacterLoad {
        row,
        group_id: progression.0,
        bound_instances: progression.1,
        auras: progression.2,
        spells: progression.3,
        quest_status: progression.4,
        honor_cp: progression.5,
        reputation: progression.6,
        inventory: items.0,
        item_loot: items.1,
        actions: items.2,
        social: items.3,
        homebind: items.4,
        spell_cooldowns: items.5,
        guild: social.0,
        bg_data: social.1,
        account_data: social.2,
        skills: social.3,
        mails: social.4,
        mailed_items: social.5,
        forgotten_skills: social.6,
    }))
}

type Progression = (
    Option<u32>,
    Vec<BoundInstanceRow>,
    Vec<AuraRow>,
    Vec<SpellRow>,
    Vec<QuestStatusRow>,
    Vec<HonorCpRow>,
    Vec<ReputationRow>,
);

fn load_progression(conn: &mut mysql::Conn, guid_low: u32) -> Result<Progression, DbError> {
    let group_id = query_rows(
        conn,
        "SELECT `group_id` FROM `group_member` WHERE `member_guid` = ?",
        guid_low,
    )?
    .first()
    .map(|f| f.u32(0));

    let bound_instances = query_rows(conn, "SELECT `id`, `permanent`, `map`, `reset_time` FROM `character_instance` LEFT JOIN `instance` ON `instance` = `id` WHERE `guid` = ?", guid_low)?
        .iter()
        .map(|f| BoundInstanceRow {
            id: (!f.is_null(0)).then(|| f.u32(0)),
            permanent: f.u8(1),
            map: (!f.is_null(2)).then(|| f.u32(2)),
            reset_time: (!f.is_null(3)).then(|| f.u64(3)),
        })
        .collect();

    let auras = query_rows(conn, "SELECT `caster_guid`, `item_guid`, `spell`, `stacks`, `charges`, `base_points0`, `base_points1`, `base_points2`, `periodic_time0`, `periodic_time1`, `periodic_time2`, `max_duration`, `duration`, `effect_index_mask` FROM `character_aura` WHERE `guid` = ?", guid_low)?
        .iter()
        .map(|f| AuraRow {
            caster_guid: f.u64(0),
            item_guid: f.u32(1),
            spell: f.u32(2),
            stacks: f.i32(3),
            charges: f.i32(4),
            base_points: [f.f32(5), f.f32(6), f.f32(7)],
            periodic_time: [f.i32(8), f.i32(9), f.i32(10)],
            max_duration: f.i32(11),
            duration: f.i32(12),
            effect_index_mask: f.u8(13),
        })
        .collect();

    let spells = query_rows(
        conn,
        "SELECT `spell`, `active`, `disabled` FROM `character_spell` WHERE `guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| SpellRow {
        spell: f.u32(0),
        active: f.u8(1),
        disabled: f.u8(2),
    })
    .collect();

    let quest_status = query_rows(conn, "SELECT `quest`, `status`, `rewarded`, `explored`, `timer`, `mob_count1`, `mob_count2`, `mob_count3`, `mob_count4`, `item_count1`, `item_count2`, `item_count3`, `item_count4`, `reward_choice` FROM `character_queststatus` WHERE `guid` = ?", guid_low)?
        .iter()
        .map(|f| QuestStatusRow {
            quest: f.u32(0),
            status: f.i32(1),
            rewarded: f.u8(2),
            explored: f.u8(3),
            timer: f.u64(4),
            mob_count: [f.u32(5), f.u32(6), f.u32(7), f.u32(8)],
            item_count: [f.u32(9), f.u32(10), f.u32(11), f.u32(12)],
            reward_choice: f.i32(13),
        })
        .collect();

    let honor_cp = query_rows(conn, "SELECT `victim_type`, `victim_id`, `cp`, `date`, `type` FROM `character_honor_cp` WHERE `guid` = ?", guid_low)?
        .iter()
        .map(|f| HonorCpRow {
            victim_type: f.u8(0),
            victim_id: f.u32(1),
            cp: f.f32(2),
            date: f.u32(3),
            kind: f.u8(4),
        })
        .collect();

    let reputation = query_rows(
        conn,
        "SELECT `faction`, `standing`, `flags` FROM `character_reputation` WHERE `guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| ReputationRow {
        faction: f.u32(0),
        standing: f.i32(1),
        flags: f.i32(2),
    })
    .collect();

    Ok((
        group_id,
        bound_instances,
        auras,
        spells,
        quest_status,
        honor_cp,
        reputation,
    ))
}

type ItemsAndActions = (
    Vec<InventoryRow>,
    Vec<ItemLootRow>,
    Vec<ActionRow>,
    Vec<SocialRow>,
    Option<HomebindRow>,
    Vec<SpellCooldownRow>,
);

fn load_items_and_actions(
    conn: &mut mysql::Conn,
    guid_low: u32,
) -> Result<ItemsAndActions, DbError> {
    let inventory = query_rows(conn, "SELECT * FROM (SELECT `creator_guid`, `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, `random_property_id`, `durability`, `text`, `bag`, `slot`, `item_guid`, `item_instance`.`item_id`, `generated_loot` FROM `character_inventory` JOIN `item_instance` ON `character_inventory`.`item_guid` = `item_instance`.`guid` WHERE `character_inventory`.`guid` = ?) as t ORDER BY `bag`, `slot`", guid_low)?
        .iter()
        .map(|f| {
            let mut item = map_item_instance(f, 0);
            item.item_guid = f.u32(12);
            item.item_id = f.u32(13);
            item.generated_loot = f.u8(14);
            InventoryRow {
                item,
                bag: f.u32(10),
                slot: f.u8(11),
            }
        })
        .collect();

    let item_loot = query_rows(
        conn,
        "SELECT `guid`, `item_id`, `amount`, `property` FROM `item_loot` WHERE `owner_guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| ItemLootRow {
        guid: f.u32(0),
        item_id: f.u32(1),
        amount: f.i32(2),
        property: f.i32(3),
    })
    .collect();

    let actions = query_rows(conn, "SELECT `button`, `action`, `type` FROM `character_action` WHERE `guid` = ? ORDER BY `button`", guid_low)?
        .iter()
        .map(|f| ActionRow {
            button: f.u8(0),
            action: f.u32(1),
            kind: f.u8(2),
        })
        .collect();

    let social = query_rows(
        conn,
        "SELECT `friend`, `flags` FROM `character_social` WHERE `guid` = ? LIMIT 255",
        guid_low,
    )?
    .iter()
    .map(|f| SocialRow {
        friend: f.u32(0),
        flags: f.u8(1),
    })
    .collect();

    let homebind = query_rows(conn, "SELECT `map`, `zone`, `position_x`, `position_y`, `position_z` FROM `character_homebind` WHERE `guid` = ?", guid_low)?
        .first()
        .map(|f| HomebindRow {
            map: f.u32(0),
            zone: f.u32(1),
            position_x: f.f32(2),
            position_y: f.f32(3),
            position_z: f.f32(4),
        });

    let spell_cooldowns = query_rows(conn, "SELECT `spell`, `spell_expire_time`, `category`, `category_expire_time`, `item_id` FROM `character_spell_cooldown` WHERE `guid` = ?", guid_low)?
        .iter()
        .map(|f| SpellCooldownRow {
            spell: f.u32(0),
            spell_expire_time: f.i64(1),
            category: f.u32(2),
            category_expire_time: f.i64(3),
            item_id: f.u32(4),
        })
        .collect();

    Ok((
        inventory,
        item_loot,
        actions,
        social,
        homebind,
        spell_cooldowns,
    ))
}

type SocialAndMail = (
    Option<GuildRow>,
    Option<BgDataRow>,
    Vec<AccountDataEntry>,
    Vec<SkillRow>,
    Vec<MailRow>,
    Vec<MailedItemRow>,
    Vec<ForgottenSkillRow>,
);

fn load_social_and_mail(conn: &mut mysql::Conn, guid_low: u32) -> Result<SocialAndMail, DbError> {
    let guild = query_rows(
        conn,
        "SELECT `guild_id`, `rank` FROM `guild_member` WHERE `guid` = ?",
        guid_low,
    )?
    .first()
    .map(|f| GuildRow {
        guild_id: f.u32(0),
        rank: f.u8(1),
    });

    let bg_data = query_rows(conn, "SELECT `instance_id`, `team`, `join_x`, `join_y`, `join_z`, `join_o`, `join_map` FROM `character_battleground_data` WHERE `guid` = ?", guid_low)?
        .first()
        .map(|f| BgDataRow {
            instance_id: f.u32(0),
            team: f.u32(1),
            join_x: f.f32(2),
            join_y: f.f32(3),
            join_z: f.f32(4),
            join_o: f.f32(5),
            join_map: f.u32(6),
        });

    let account_data = query_rows(
        conn,
        "SELECT `type`, `time`, `data` FROM `character_account_data` WHERE `guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| AccountDataEntry {
        data_type: f.u32(0),
        timestamp: f.u64(1),
        data: f.string(2),
    })
    .collect();

    let skills = query_rows(
        conn,
        "SELECT `skill`, `value`, `max` FROM `character_skills` WHERE `guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| SkillRow {
        skill: f.u32(0),
        value: f.u32(1),
        max: f.u32(2),
    })
    .collect();

    let mails = query_rows(conn, "SELECT `id`, `message_type`, `sender_guid`, `receiver_guid`, `subject`, `item_text_id`, `expire_time`, `deliver_time`, `money`, `cod`, `checked`, `stationery`, `mail_template_id`, `has_items` FROM `mail` WHERE `receiver_guid` = ? ORDER BY `id` DESC", guid_low)?
        .iter()
        .map(|f| MailRow {
            id: f.u32(0),
            message_type: f.u8(1),
            sender_guid: f.u32(2),
            receiver_guid: f.u32(3),
            subject: f.string(4),
            item_text_id: f.u32(5),
            expire_time: f.i64(6),
            deliver_time: f.i64(7),
            money: f.i32(8),
            cod: f.i32(9),
            checked: f.u8(10),
            stationery: f.u8(11),
            mail_template_id: f.u32(12),
            has_items: f.u8(13),
        })
        .collect();

    let mailed_items = query_rows(conn, "SELECT `creator_guid`, `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, `random_property_id`, `durability`, `text`, `mail_id`, `item_guid`, `item_instance`.`item_id`, `generated_loot` FROM `mail_items` JOIN `item_instance` ON `item_guid` = `guid` WHERE `receiver_guid` = ?", guid_low)?
        .iter()
        .map(|f| {
            let mut item = map_item_instance(f, 0);
            item.item_guid = f.u32(11);
            item.item_id = f.u32(12);
            item.generated_loot = f.u8(13);
            MailedItemRow {
                item,
                mail_id: f.u32(10),
            }
        })
        .collect();

    let forgotten_skills = query_rows(
        conn,
        "SELECT `skill`, `value` FROM `character_forgotten_skills` WHERE `guid` = ?",
        guid_low,
    )?
    .iter()
    .map(|f| ForgottenSkillRow {
        skill: f.u32(0),
        value: f.u32(1),
    })
    .collect();

    Ok((
        guild,
        bg_data,
        account_data,
        skills,
        mails,
        mailed_items,
        forgotten_skills,
    ))
}
