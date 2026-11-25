//! Weekly honor maintenance (`HonorMaintenancer`,
//! `HonorMgr.cpp:17-601`, Tier 4 B0-7): startup
//! initialize + marker-gated weekly recompute
//! (standings, RP distribution/decay, city
//! protector flags, flush, CP cleanup, HCR
//! report text). Runtime per-kill honor lives in
//! `mangos-map::honor`; rank math mirrors
//! `mangos-entities::honor` (duplicated as plain
//! functions here — this crate cannot depend on
//! entities; the two `CalculateRank` copies must
//! stay in lockstep).
//!
//! Fork format quirk: CP/RP writes in the fork go
//! through `%.1f` SQL formatting; values here are
//! rounded to one decimal before binding (half
//! away from zero — C `printf` rounds the exact
//! binary value, differing only on exact .05
//! binary ties).

use std::collections::HashMap;
use std::fmt::Write as _;

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// Maintenance configuration (fork config keys).
#[derive(Clone, Copy, Debug)]
pub struct HonorMaintenanceConfig {
    pub wow_patch: u32,
    pub maintenance_day: u32,
    pub min_honor_kills: u32,
    pub rp_decay: f32,
    pub pool_size_per_faction: u32,
    pub city_protector: bool,
    pub time_zone_offset_secs: i64,
}

/// Result of the startup initialize/maintenance.
#[derive(Clone, Debug, Default)]
pub struct HonorMaintenanceOutcome {
    /// `GetWeekBeginDay()` after any run.
    pub week_begin_day: u32,
    pub next_maintenance_day: u32,
    pub marker_to_start: bool,
    /// True when the weekly recompute ran.
    pub ran: bool,
    pub alliance_count: usize,
    pub horde_count: usize,
    pub inactive_count: usize,
    /// HCR report body (file write is the
    /// caller's, fork `HonorDir` + timestamp name).
    pub report: Option<String>,
}

/// `GetGameDay` (`(gameTime + tz) / DAY`).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub fn game_day(now_unix: u64, time_zone_offset_secs: i64) -> u32 {
    ((now_unix as i64 + time_zone_offset_secs) / 86_400) as u32
}

/// `World::GetLastMaintenanceDay`
/// (`World.h:744`): most recent maintenance
/// weekday. Fork takes the weekday from OS
/// localtime while the day count uses the config
/// offset; both use the offset here (equal when
/// the host runs UTC with offset 0, the
/// production shape).
#[must_use]
pub fn last_maintenance_day(now_unix: u64, cfg: &HonorMaintenanceConfig) -> u32 {
    let day = game_day(now_unix, cfg.time_zone_offset_secs);
    // 1970-01-01 was a Thursday; tm_wday Sunday=0.
    let wday = (day + 4) % 7;
    day - ((wday + 7 - cfg.maintenance_day.min(6)) % 7)
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Fork `%.1f` SQL formatting of honor floats.
fn round1(v: f32) -> f32 {
    (finite(v) * 10.0).round() / 10.0
}

/// Stored maintenance days from `saved_variables`
/// (characters DB), all zero when the row/table
/// value is absent.
fn load_saved_days(conn: &mut mysql::Conn) -> Result<(u32, u32, bool), DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `honor_last_maintenance_day`, `honor_next_maintenance_day`, \
         `honor_maintenance_marker` FROM `saved_variables`",
        (),
    )?;
    let Some(row) = rows.into_iter().next() else {
        return Ok((0, 0, false));
    };
    let f = Fields::from_row(row);
    Ok((f.u32(0), f.u32(1), f.u32(2) != 0))
}

fn store_maintenance_days(conn: &mut mysql::Conn, last: u32, next: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `saved_variables` (`key`, `honor_last_maintenance_day`, \
         `honor_next_maintenance_day`) VALUES (0, ?, ?) ON DUPLICATE KEY UPDATE \
         `honor_last_maintenance_day` = ?, `honor_next_maintenance_day` = ?",
        (last, next, last, next),
    )?;
    Ok(())
}

/// `ToggleMaintenanceMarker` write (runtime
/// check sets it when the next day is reached).
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn store_maintenance_marker(conn: &mut mysql::Conn, marker: bool) -> Result<(), DbError> {
    let value = u32::from(marker);
    conn.exec_drop(
        "INSERT INTO `saved_variables` (`key`, `honor_maintenance_marker`) \
         VALUES (0, ?) ON DUPLICATE KEY UPDATE `honor_maintenance_marker` = ?",
        (value, value),
    )?;
    Ok(())
}

/// One `WeeklyScore` (fork struct + race for the
/// `ObjectMgr` team resolution).
#[derive(Clone, Copy, Debug, Default)]
struct WeeklyScore {
    level: u8,
    account: u32,
    race: u8,
    hk: u32,
    dk: u32,
    cp: f32,
    old_rp: f32,
    new_rp: f32,
    earning: f32,
    standing: u32,
    highest_rank: u8,
}

/// `HonorStanding`.
#[derive(Clone, Copy, Debug, Default)]
struct HonorStanding {
    guid: u32,
    cp: f32,
}

/// `HonorScores`.
#[derive(Clone, Copy, Debug, Default)]
struct HonorScores {
    fx: [f32; 15],
    fy: [f32; 15],
    brk: [f32; 14],
}

/// `TeamForRace` for vanilla races: `ChrRaces`
/// team 7 (Alliance) races 1/3/4/7, team 1
/// (Horde) races 2/5/6/8. Returns true =
/// Alliance; the fork falls back to the alliance
/// list for anything else (`LoadStandingLists`
/// else-arm is Horde for non-Alliance — mirror:
/// unknown races go Horde as in the fork's
/// `team == ALLIANCE ? A : H`).
fn race_is_alliance(race: u8) -> bool {
    matches!(race, 1 | 3 | 4 | 7)
}

/// `GetStandingCPByPosition` (1-based).
fn standing_cp_by_position(list: &[HonorStanding], position: u32) -> f32 {
    if position == 0 {
        return 0.0;
    }
    list.get((position - 1) as usize).map_or(0.0, |s| s.cp)
}

/// `GenerateScores` (`HonorMgr.cpp:404`).
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn generate_scores(list: &[HonorStanding], wow_patch: u32, pool_size_cfg: u32) -> HonorScores {
    let mut sc = HonorScores::default();
    if wow_patch < 10 {
        sc.brk = [
            1.000, 0.858, 0.715, 0.587, 0.477, 0.377, 0.287, 0.207, 0.137, 0.077, 0.037, 0.017,
            0.007, 0.002,
        ];
    } else {
        sc.brk = [
            1.000, 0.845, 0.697, 0.566, 0.436, 0.327, 0.228, 0.159, 0.100, 0.060, 0.035, 0.020,
            0.008, 0.003,
        ];
    }
    #[allow(clippy::cast_precision_loss)]
    let pool_size = if pool_size_cfg == 0 {
        list.len() as f32
    } else {
        pool_size_cfg as f32
    };
    for group in &mut sc.brk {
        *group = (*group * pool_size + 0.5).floor();
    }
    sc.fy[0] = 0.0;
    sc.fy[1] = 400.0;
    for i in 2..=13 {
        sc.fy[i] = (i as f32 - 1.0) * 1000.0;
    }
    sc.fy[14] = 13000.0;
    sc.fx[0] = 0.0;
    let mut top = false;
    for i in 1..=13usize {
        let mut honor = 0.0f32;
        let pos = sc.brk[i] as u32;
        let mut temp = standing_cp_by_position(list, pos);
        if temp != 0.0 {
            honor += temp;
            temp = standing_cp_by_position(list, pos + 1);
            if temp != 0.0 {
                honor += temp;
            }
        }
        sc.fx[i] = if honor == 0.0 { 0.0 } else { honor / 2.0 };
        if !top && honor == 0.0 {
            sc.fx[i] = if sc.fx[i - 1] == 0.0 {
                0.0
            } else {
                list.first().map_or(0.0, |s| s.cp)
            };
            top = true;
        }
    }
    sc.fx[14] = if top {
        0.0
    } else {
        list.first().map_or(0.0, |s| s.cp)
    };
    sc
}

/// `CalculateRpEarning` (`HonorMgr.cpp:503`).
fn calculate_rp_earning(cp: f32, sc: &HonorScores) -> f32 {
    let mut i = 0usize;
    while i < 14 && sc.brk[i] > 0.0 && sc.fx[i] <= cp {
        i += 1;
    }
    if i > 0 && sc.fx[i] > cp && cp >= sc.fx[i - 1] {
        return (sc.fy[i] - sc.fy[i - 1]) * (cp - sc.fx[i - 1]) / (sc.fx[i] - sc.fx[i - 1])
            + sc.fy[i - 1];
    }
    sc.fy[i]
}

/// `CalculateRpDecay` (`HonorMgr.cpp:518`).
fn calculate_rp_decay(rp_earning: f32, rp: f32, decay_multiplier: f32) -> f32 {
    let decay = (decay_multiplier * rp + 0.5).floor();
    let mut delta = rp_earning - decay;
    if delta < 0.0 {
        delta /= 2.0;
    }
    if delta < -2500.0 {
        delta = -2500.0;
    }
    rp + delta
}

/// `MaximumRpAtLevel` (`HonorMgr.cpp:534`).
#[allow(clippy::cast_precision_loss)]
fn maximum_rp_at_level(level: u8) -> f32 {
    let level = f32::from(level);
    if level <= 29.0 {
        6500.0
    } else if level <= 35.0 {
        7150.0 + 975.0 * (level - 30.0)
    } else if level <= 39.0 {
        12025.0 + 1300.0 * (level - 35.0)
    } else if level <= 43.0 {
        17225.0 + 1625.0 * (level - 39.0)
    } else if level <= 52.0 {
        23725.0 + 2275.0 * (level - 43.0)
    } else if level <= 60.0 {
        44200.0 + 2600.0 * (level - 52.0)
    } else {
        65000.0
    }
}

/// (rank, visual rank) — `CalculateRank`
/// duplicate (see module docs).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn calculate_rank(rank_points: f32) -> (u8, i8) {
    if rank_points == 0.0 {
        return (0, 0);
    }
    let positive = rank_points > 0.0;
    let rp = if positive { rank_points } else { -rank_points };
    let rank_count: u8 = if positive { 15 - 2 } else { 4 };
    let first_rank: u8 = if positive { 4 + 1 } else { 1 };
    let rank = if rp < 2000.0 {
        if positive { first_rank } else { 4 }
    } else if rp > f32::from(rank_count - 1) * 5000.0 {
        if positive { 19 - 1 } else { first_rank }
    } else {
        let r = (rp / 5000.0) as u32 + u32::from(first_rank);
        if positive {
            (r + 1) as u8
        } else {
            (4u32 - r) as u8
        }
    };
    let visual = if rank > 4 {
        (rank - 4) as i8
    } else {
        -(rank as i8)
    };
    (rank, visual)
}

/// Visual rank of a stored rank byte
/// (`CalculateRankInfo` on a fresh positive
/// info, as in `FlushRankPoints`).
#[allow(clippy::cast_possible_wrap)]
fn visual_of_rank(rank: u8) -> i8 {
    if rank == 0 {
        0
    } else if rank > 4 {
        (rank - 4) as i8
    } else {
        -(rank as i8)
    }
}

/// The weekly recompute (`DoMaintenance` body).
fn do_maintenance(
    conn: &mut mysql::Conn,
    cfg: &HonorMaintenanceConfig,
    week_begin: u32,
) -> Result<HonorMaintenanceOutcome, DbError> {
    let week_end = week_begin + 6;
    // LoadWeeklyScores (fork SQL + race column
    // for the ObjectMgr team resolution).
    let sql = format!(
        "SELECT `scores`.`guid`, `c`.`level`, `c`.`account`, `c`.`honor_rank_points`, \
         `c`.`honor_highest_rank`, SUM(`hk`), SUM(`dk`), SUM(`cp`), `c`.`race` FROM (\
         SELECT `guid` AS `guid`, COUNT(*) AS `hk`, 0 AS `dk`, SUM(`cp`) AS `cp` \
         FROM `character_honor_cp` WHERE `type` = 1 AND (`date` BETWEEN {week_begin} \
         AND {week_end}) GROUP BY `guid` UNION \
         SELECT `guid` AS `guid`, 0 AS `hk`, COUNT(*) AS `dk`, 0 AS `cp` \
         FROM `character_honor_cp` WHERE `type` = 2 AND (`date` BETWEEN {week_begin} \
         AND {week_end}) GROUP BY `guid` UNION \
         SELECT `guid` AS `guid`, 0 AS `hk`, 0 AS `dk`, SUM(`cp`) AS `cp` \
         FROM `character_honor_cp` WHERE `type` NOT IN (1, 2) AND (`date` BETWEEN \
         {week_begin} AND {week_end}) GROUP BY `guid` UNION \
         SELECT `guid` AS `guid`, 0 AS `hk`, 0 AS `dk`, 0 AS `cp` FROM `characters` \
         WHERE `honor_rank_points` > 0) AS `scores` INNER JOIN `characters` AS `c` \
         ON `scores`.`guid` = `c`.`guid` GROUP BY `guid` ORDER BY `guid`"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, ())?;
    let mut scores: HashMap<u32, WeeklyScore> = HashMap::new();
    for row in rows {
        let f = Fields::from_row(row);
        scores.insert(
            f.u32(0),
            WeeklyScore {
                level: f.u8(1),
                account: f.u32(2),
                old_rp: f.f32(3),
                highest_rank: f.u8(4),
                hk: f.u32(5),
                dk: f.u32(6),
                cp: f.f32(7),
                race: f.u8(8),
                ..WeeklyScore::default()
            },
        );
    }

    // LoadStandingLists.
    let min_hk = if cfg.min_honor_kills == 0 {
        if cfg.wow_patch >= 8 { 15 } else { 25 }
    } else {
        cfg.min_honor_kills
    };
    let mut alliance: Vec<HonorStanding> = Vec::new();
    let mut horde: Vec<HonorStanding> = Vec::new();
    let mut inactive: Vec<HonorStanding> = Vec::new();
    for (guid, score) in &scores {
        let standing = HonorStanding {
            guid: *guid,
            cp: score.cp,
        };
        if score.hk < min_hk || score.account == 0 {
            inactive.push(standing);
        } else if race_is_alliance(score.race) {
            alliance.push(standing);
        } else {
            horde.push(standing);
        }
    }
    let by_cp_desc = |a: &HonorStanding, b: &HonorStanding| {
        b.cp.partial_cmp(&a.cp).unwrap_or(std::cmp::Ordering::Equal)
    };
    alliance.sort_by(by_cp_desc);
    horde.sort_by(by_cp_desc);

    // DistributeRankPoints per team.
    for list in [&alliance, &horde] {
        if list.is_empty() {
            continue;
        }
        let sc = generate_scores(list, cfg.wow_patch, cfg.pool_size_per_faction);
        let mut position = 1u32;
        for standing in list {
            let Some(score) = scores.get_mut(&standing.guid) else {
                continue;
            };
            score.earning = calculate_rp_earning(score.cp, &sc);
            score.new_rp = calculate_rp_decay(score.earning, score.old_rp, cfg.rp_decay);
            score.new_rp = score.new_rp.min(maximum_rp_at_level(score.level));
            score.standing = position;
            position += 1;
        }
    }

    // InactiveDecayRankPoints.
    for standing in &inactive {
        if let Some(score) = scores.get_mut(&standing.guid) {
            score.new_rp = finite(calculate_rp_decay(0.0, score.old_rp, cfg.rp_decay));
        }
    }

    set_city_ranks_and_flush(conn, cfg, week_end, &scores)?;

    // CreateCalculationReport.
    let report = build_report(&alliance, &horde, &inactive, &scores, cfg);

    Ok(HonorMaintenanceOutcome {
        alliance_count: alliance.len(),
        horde_count: horde.len(),
        inactive_count: inactive.len(),
        ran: true,
        report: Some(report),
        ..HonorMaintenanceOutcome::default()
    })
}

/// `SetCityRanks` + `FlushRankPoints` split out
/// of `do_maintenance` (fork order preserved:
/// city flags read last week's standings before
/// the flush overwrites them).
fn set_city_ranks_and_flush(
    conn: &mut mysql::Conn,
    cfg: &HonorMaintenanceConfig,
    week_end: u32,
    scores: &HashMap<u32, WeeklyScore>,
) -> Result<(), DbError> {
    // SetCityRanks (reads last week's standing,
    // before the flush below overwrites it).
    if cfg.city_protector {
        conn.exec_drop(
            "UPDATE `characters` SET `extra_flags` = `extra_flags` & ~0x0400",
            (),
        )?;
        for race in 1..9u32 {
            let rows: Vec<mysql::Row> = conn.exec(
                "SELECT `guid`, `honor_standing` FROM `characters` \
             WHERE `honor_standing` > 0 AND `race` = ? \
             ORDER BY `honor_standing` ASC LIMIT 1",
                (race,),
            )?;
            for row in rows {
                let f = Fields::from_row(row);
                let guid = f.u32(0);
                if guid > 0 {
                    conn.exec_drop(
                        "UPDATE `characters` SET `extra_flags` = `extra_flags` | 0x0400 \
                     WHERE `guid` = ?",
                        (guid,),
                    )?;
                }
            }
        }
    }

    // FlushRankPoints.
    conn.exec_drop(
        "UPDATE `characters` SET `honor_standing` = 0 WHERE `honor_standing` > 0",
        (),
    )?;
    for (guid, score) in scores {
        let (cur_rank, cur_visual) = calculate_rank(score.new_rp);
        let highest = if cur_visual > 0 && cur_visual > visual_of_rank(score.highest_rank) {
            cur_rank
        } else {
            score.highest_rank
        };
        conn.exec_drop(
            "UPDATE `characters` SET `honor_highest_rank` = ?, `honor_rank_points` = ?, \
         `honor_standing` = ?, `honor_last_week_hk` = ?, \
         `honor_stored_hk` = (`honor_stored_hk` + ?), \
         `honor_stored_dk` = (`honor_stored_dk` + ?), `honor_last_week_cp` = ? \
         WHERE `guid` = ?",
            (
                highest,
                round1(score.new_rp),
                score.standing,
                score.hk,
                score.hk,
                score.dk,
                round1(score.cp),
                *guid,
            ),
        )?;
    }
    conn.exec_drop(
        "DELETE FROM `character_honor_cp` WHERE `date` < ?",
        (week_end,),
    )?;
    Ok(())
}

fn report_team(
    out: &mut String,
    name: &str,
    list: &[HonorStanding],
    scores: &HashMap<u32, WeeklyScore>,
    cfg: &HonorMaintenanceConfig,
) {
    if list.is_empty() {
        return;
    }
    let sc = generate_scores(list, cfg.wow_patch, cfg.pool_size_per_faction);
    let _ = write!(
        out,
        "{name} Honor Scores\n\nStanding size: {}\n\n",
        list.len()
    );
    for (i, v) in sc.brk.iter().enumerate() {
        let _ = writeln!(out, "BRK[{i}] = {v}");
    }
    out.push('\n');
    for (i, v) in sc.fx.iter().enumerate() {
        let _ = writeln!(out, "FX[{i}] = {v}");
    }
    out.push('\n');
    for (i, v) in sc.fy.iter().enumerate() {
        let _ = writeln!(out, "FY[{i}] = {v}");
    }
    out.push('\n');
    for st in list {
        let Some(ws) = scores.get(&st.guid) else {
            continue;
        };
        let _ = writeln!(
            out,
            "Guid: {}, HK: {}, DK: {}, CP: {}, oldRp: {}, earning: {}, newRp: {}, \
             capRp: {}, standing: {}",
            st.guid,
            ws.hk,
            ws.dk,
            ws.cp,
            ws.old_rp,
            ws.earning,
            ws.new_rp,
            maximum_rp_at_level(ws.level),
            ws.standing
        );
    }
}

fn build_report(
    alliance: &[HonorStanding],
    horde: &[HonorStanding],
    inactive: &[HonorStanding],
    scores: &HashMap<u32, WeeklyScore>,
    cfg: &HonorMaintenanceConfig,
) -> String {
    let mut out = String::new();
    report_team(&mut out, "Alliance", alliance, scores, cfg);
    out.push_str("--------------------------------------------------\n\n");
    report_team(&mut out, "Horde", horde, scores, cfg);
    out.push_str("--------------------------------------------------\n\n");
    if !inactive.is_empty() {
        let _ = write!(
            out,
            "Inactive players decay\n\nCount: {}\n\n",
            inactive.len()
        );
        for st in inactive {
            let Some(ws) = scores.get(&st.guid) else {
                continue;
            };
            let _ = writeln!(
                out,
                "Guid: {}, HK: {}, DK: {}, CP: {}, oldRp: {}, newRp: {}, \
                 capRp: {}, standing: {}",
                st.guid,
                ws.hk,
                ws.dk,
                ws.cp,
                ws.old_rp,
                ws.new_rp,
                maximum_rp_at_level(ws.level),
                ws.standing
            );
        }
    }
    out
}

/// Startup `HonorMaintenancer::Initialize` +
/// `DoMaintenance` (marker-gated). Returns the
/// effective week-begin/next days for runtime.
///
/// # Errors
///
/// [`DbError`] on driver failure.
pub fn initialize_and_run(
    conn: &mut mysql::Conn,
    cfg: &HonorMaintenanceConfig,
    now_unix: u64,
) -> Result<HonorMaintenanceOutcome, DbError> {
    let (mut last, mut next, mut marker) = load_saved_days(conn)?;
    if last == 0 {
        last = last_maintenance_day(now_unix, cfg);
        next = last + 7;
        store_maintenance_days(conn, last, next)?;
    }
    let mut outcome = HonorMaintenanceOutcome {
        week_begin_day: last,
        next_maintenance_day: next,
        marker_to_start: marker,
        ..HonorMaintenanceOutcome::default()
    };
    if marker {
        let ran = do_maintenance(conn, cfg, last)?;
        outcome.alliance_count = ran.alliance_count;
        outcome.horde_count = ran.horde_count;
        outcome.inactive_count = ran.inactive_count;
        outcome.ran = true;
        outcome.report = ran.report;
        // Toggle marker off, advance the week.
        marker = false;
        store_maintenance_marker(conn, marker)?;
        last = next;
        next = last + 7;
        store_maintenance_days(conn, last, next)?;
        outcome.week_begin_day = last;
        outcome.next_maintenance_day = next;
        outcome.marker_to_start = marker;
    }
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;

    fn standing(guid: u32, cp: f32) -> HonorStanding {
        HonorStanding { guid, cp }
    }

    #[test]
    fn rank_bands() {
        assert_eq!(calculate_rank(0.0), (0, 0));
        // First positive rank below 2000 RP.
        assert_eq!(calculate_rank(100.0), (5, 1));
        // 2000 RP: (2000/5000)+5 +1 = 6.
        assert_eq!(calculate_rank(2000.0), (6, 2));
        // 65000 RP cap rank 18 (visual 14).
        assert_eq!(calculate_rank(65_000.0), (18, 14));
        // Negative: -100 -> rank 4 (visual -4).
        assert_eq!(calculate_rank(-100.0), (4, -4));
        // -6000: (6000/5000)+1=2 -> 4-2=2 (visual -2).
        assert_eq!(calculate_rank(-6000.0), (2, -2));
    }

    #[test]
    fn decay_rules() {
        // 20% decay of 10000 = 2000; earning 0 ->
        // delta -2000 -> halved -1000 -> 9000.
        assert_eq!(calculate_rp_decay(0.0, 10_000.0, 0.2), 9000.0);
        // Loss floor -2500.
        assert_eq!(calculate_rp_decay(0.0, 60_000.0, 0.2), 57_500.0);
        // Gain passes through.
        assert_eq!(calculate_rp_decay(3000.0, 10_000.0, 0.2), 11_000.0);
    }

    #[test]
    fn max_rp_levels() {
        assert_eq!(maximum_rp_at_level(29), 6500.0);
        assert_eq!(maximum_rp_at_level(30), 7150.0);
        assert_eq!(maximum_rp_at_level(60), 65_000.0);
    }

    #[test]
    fn earning_interpolation() {
        // 100 players, CP descending 100..1 *100.
        let list: Vec<HonorStanding> = (0..100)
            .map(|i| standing(i + 1, (100 - i) as f32 * 100.0))
            .collect();
        let sc = generate_scores(&list, 10, 0);
        // Top scorer earns FY at/above FX top:
        // FX[14] = top cp when breakpoints fill.
        let top_earning = calculate_rp_earning(10_000.0, &sc);
        assert!(top_earning >= 12_000.0, "top earning {top_earning}");
        // Zero CP earns nothing.
        assert_eq!(calculate_rp_earning(0.0, &sc), 0.0);
        // Monotone in CP.
        let mid = calculate_rp_earning(5_000.0, &sc);
        assert!(mid > 0.0 && mid < top_earning, "mid {mid}");
    }

    #[test]
    fn maintenance_weekday() {
        // 2026-11-02 is a Monday (bench log date).
        let cfg = HonorMaintenanceConfig {
            wow_patch: 10,
            maintenance_day: 2, // Tuesday
            min_honor_kills: 0,
            rp_decay: 0.2,
            pool_size_per_faction: 0,
            city_protector: true,
            time_zone_offset_secs: 0,
        };
        // Unix for 2026-11-02 12:00 UTC.
        let now = 1_793_000_000u64; // approx; only day math matters
        let day = game_day(now, 0);
        let last = last_maintenance_day(now, &cfg);
        assert!(last <= day && day - last < 7);
        assert_eq!((last + 4) % 7, 2, "last maintenance is a Tuesday");
    }
}
