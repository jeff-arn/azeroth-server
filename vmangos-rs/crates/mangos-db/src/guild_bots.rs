//! `guild_bot_roster` + `guild_bot_spec_weights`
//! access (`GuildBotMgr::LoadRoster`,
//! `LoadSpecWeights`, `DetectPlayerGuild`) and
//! the roster/level writes its update issues.

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// One joined roster row.
#[derive(Clone, Debug, PartialEq)]
pub struct GuildBotRosterRow {
    pub guid: u32,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub spec: String,
    pub role: String,
    pub raid_assigned: bool,
    pub active: bool,
    pub gear_score: f32,
    pub gear_tier: String,
    pub personality: String,
}

/// One `guild_bot_spec_weights` row (15 weights).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuildBotWeightsRow {
    pub class: u8,
    pub weights: [f32; 15],
}

/// Boot load output.
#[derive(Clone, Debug, Default)]
pub struct LoadedGuildBots {
    pub roster: Vec<GuildBotRosterRow>,
    pub weights: Vec<(GuildBotWeightsRow, String)>,
    /// Guild of the first roster bot, if membered.
    pub bot_guild_id: Option<u32>,
    /// First guild with a human member (account < 100).
    pub player_guild_id: Option<u32>,
}

/// Roster + weights + guild detection queries.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_guild_bots(conn: &mut mysql::Conn) -> Result<LoadedGuildBots, DbError> {
    let rows: Vec<mysql::Row> = conn.query(
        "SELECT r.guid, c.name, c.race, c.class, r.spec, r.role, \
         r.raid_assigned, r.active, r.gear_score, r.gear_tier, r.personality \
         FROM guild_bot_roster r JOIN characters c ON r.guid = c.guid \
         ORDER BY r.guid",
    )?;
    let roster: Vec<GuildBotRosterRow> = rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            GuildBotRosterRow {
                guid: f.u32(0),
                name: f.string(1),
                race: f.u8(2),
                class: f.u8(3),
                spec: f.string(4),
                role: f.string(5),
                raid_assigned: f.u8(6) != 0,
                active: f.u8(7) != 0,
                gear_score: f.f32(8),
                gear_tier: f.string(9),
                personality: f.string(10),
            }
        })
        .collect();
    let weight_rows: Vec<mysql::Row> = conn.query(
        "SELECT class, spec, stamina, strength, agility, intellect, spirit, \
         attack_power, spell_power, healing_power, hit_rating, crit_rating, \
         defense, dodge, parry, block, mp5 FROM guild_bot_spec_weights",
    )?;
    let weights = weight_rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            let mut w = [0f32; 15];
            for (i, slot) in w.iter_mut().enumerate() {
                *slot = f.f32(i + 2);
            }
            (
                GuildBotWeightsRow {
                    class: f.u8(0),
                    weights: w,
                },
                f.string(1),
            )
        })
        .collect();
    let bot_guild_id = match roster.first() {
        Some(first) => {
            let rows: Vec<mysql::Row> = conn.exec(
                "SELECT guild_id FROM guild_member WHERE guid = ?",
                (first.guid,),
            )?;
            rows.into_iter()
                .next()
                .map(|row| Fields::from_row(row).u32(0))
        }
        None => None,
    };
    let player_guild_id = {
        let rows: Vec<mysql::Row> = conn.query(
            "SELECT g.guild_id FROM guild g \
             JOIN guild_member gm ON g.guild_id = gm.guild_id \
             JOIN characters c ON gm.guid = c.guid \
             WHERE c.account < 100 ORDER BY g.guild_id ASC LIMIT 1",
        )?;
        rows.into_iter()
            .next()
            .map(|row| Fields::from_row(row).u32(0))
    };
    Ok(LoadedGuildBots {
        roster,
        weights,
        bot_guild_id,
        player_guild_id,
    })
}

/// `SELECT level FROM characters WHERE guid = ?`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn character_level(conn: &mut mysql::Conn, guid: u32) -> Result<Option<u32>, DbError> {
    let rows: Vec<mysql::Row> =
        conn.exec("SELECT level FROM characters WHERE guid = ?", (guid,))?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| Fields::from_row(row).u32(0)))
}

/// `UPDATE characters SET level = ? WHERE guid = ?`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_character_level(conn: &mut mysql::Conn, guid: u32, level: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE characters SET level = ? WHERE guid = ?",
        (level, guid),
    )?;
    Ok(())
}

/// `UPDATE guild_bot_roster SET gear_score = ? WHERE guid = ?`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn set_gear_score(conn: &mut mysql::Conn, guid: u32, score: f32) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE guild_bot_roster SET gear_score = ? WHERE guid = ?",
        (score, guid),
    )?;
    Ok(())
}
