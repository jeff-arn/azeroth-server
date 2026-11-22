//! Per-player honor manager state and rank math
//! (`HonorMgr` in `HonorMgr.{h,cpp}`, Tier 4 B0-7):
//! rank info calculation, dishonorable-kill
//! points, CP row recording (`Add` row half; the
//! PVP credit packet and kill flows live map-side
//! in `mangos-map`'s honor module), and the
//! client-field recompute (`Update`).

use mangos_protocol::update_field_table::offsets;

use crate::player::Player;

/// `HONORABLE` (`HonorMgr.h` `HonorType`).
pub const HONOR_TYPE_HONORABLE: u8 = 1;
/// `DISHONORABLE`.
pub const HONOR_TYPE_DISHONORABLE: u8 = 2;
/// `BONUS`.
pub const HONOR_TYPE_BONUS: u8 = 3;
/// `QUEST`.
pub const HONOR_TYPE_QUEST: u8 = 4;
/// `OTHER`.
pub const HONOR_TYPE_OTHER: u8 = 5;

/// `TYPEID_PLAYER` as stored in `HonorCP::victimType`
/// (`ObjectDefines.h` `TypeID`: PLAYER=4, UNIT=3; the
/// fork stores `source->GetTypeId()`, 0 for self).
pub const HONOR_VICTIM_PLAYER: u8 = 4;
/// `TYPEID_UNIT`.
pub const HONOR_VICTIM_UNIT: u8 = 3;

/// `NEGATIVE_HONOR_RANK_COUNT`.
const NEGATIVE_HONOR_RANK_COUNT: u8 = 4;
/// `POSITIVE_HONOR_RANK_COUNT`.
const POSITIVE_HONOR_RANK_COUNT: u8 = 15;
/// `HONOR_RANK_COUNT`.
const HONOR_RANK_COUNT: u8 = 19;

/// `HonorRankInfo` (`HonorMgr.h`): internal rank
/// [0..18], visualized rank [-4..14], RP band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HonorRankInfo {
    pub rank: u8,
    pub visual_rank: i8,
    pub max_rp: f32,
    pub min_rp: f32,
    pub positive: bool,
}

impl Default for HonorRankInfo {
    fn default() -> Self {
        Self::init()
    }
}

impl HonorRankInfo {
    /// `InitRankInfo` (`HonorMgr.cpp:867`).
    #[must_use]
    pub fn init() -> Self {
        Self {
            rank: 0,
            visual_rank: 0,
            max_rp: 2000.0,
            min_rp: 0.0,
            positive: true,
        }
    }

    /// `CalculateRankInfo` (`HonorMgr.cpp:905`):
    /// fill band + visual rank from `rank`
    /// (rank 0 re-inits).
    pub fn calculate_info(&mut self) {
        if self.rank == 0 {
            *self = Self::init();
            return;
        }
        #[allow(clippy::cast_possible_wrap)]
        let rank = if self.positive {
            i32::from(self.rank) - i32::from(NEGATIVE_HONOR_RANK_COUNT) - 1
        } else {
            i32::from(self.rank) - i32::from(NEGATIVE_HONOR_RANK_COUNT)
        };
        #[allow(clippy::cast_precision_loss)]
        let mut max_rp = rank as f32 * 5000.0;
        if max_rp < 0.0 {
            max_rp *= -1.0;
        }
        self.max_rp = max_rp;
        self.min_rp = if max_rp > 5000.0 {
            max_rp - 5000.0
        } else {
            2000.0
        };
        if self.rank == 5 {
            self.max_rp = 2000.0;
            self.min_rp = 0.0;
        }
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        {
            self.visual_rank = if self.rank > NEGATIVE_HONOR_RANK_COUNT {
                (self.rank - NEGATIVE_HONOR_RANK_COUNT) as i8
            } else {
                -(self.rank as i8)
            };
        }
    }

    /// `CalculateRank` (`HonorMgr.cpp:926`; the
    /// fork body ignores `totalHK`).
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn calculate(rank_points: f32) -> Self {
        let mut prk = Self::init();
        if rank_points == 0.0 {
            return prk;
        }
        prk.positive = rank_points > 0.0;
        let rp = if prk.positive {
            rank_points
        } else {
            -rank_points
        };
        let rank_count: u8 = if prk.positive {
            POSITIVE_HONOR_RANK_COUNT - 2
        } else {
            NEGATIVE_HONOR_RANK_COUNT
        };
        let first_rank: u8 = if prk.positive {
            NEGATIVE_HONOR_RANK_COUNT + 1
        } else {
            1
        };
        if rp < 2000.0 {
            prk.rank = if prk.positive {
                first_rank
            } else {
                NEGATIVE_HONOR_RANK_COUNT
            };
        } else if rp > (f32::from(rank_count - 1) * 5000.0) {
            prk.rank = if prk.positive {
                HONOR_RANK_COUNT - 1
            } else {
                first_rank
            };
        } else {
            let rank = (rp / 5000.0) as u32 + u32::from(first_rank);
            prk.rank = if prk.positive {
                (rank + 1) as u8
            } else {
                (u32::from(NEGATIVE_HONOR_RANK_COUNT) - rank) as u8
            };
        }
        prk.calculate_info();
        prk
    }
}

/// `DishonorableKillPoints` (`HonorMgr.cpp:960`).
#[must_use]
pub fn dishonorable_kill_points(level: u8) -> f32 {
    let level = f32::from(level);
    let mut result = 10.0f32;
    if (30.0..=35.0).contains(&level) {
        result += 1.5 * (level - 29.0);
    }
    if (36.0..=41.0).contains(&level) {
        result += 9.0 + 2.0 * (level - 35.0);
    }
    if (42.0..=50.0).contains(&level) {
        result += 21.0 + 3.2 * (level - 41.0);
    }
    if level >= 51.0 {
        result += 50.0 + 4.0 * (level - 50.0);
    }
    if result > 100.0 { 100.0 } else { result }
}

/// Aggregates computed by `honor_update` from
/// the CP rows + stored scalars.
struct HonorTotals {
    today_hk: u32,
    today_dk: u32,
    yesterday_kills: u32,
    this_week_kills: u32,
    yesterday_cp: f32,
    this_week_cp: f32,
    total_hk: u32,
    total_dk: u32,
}

impl Player {
    /// The cached current rank info (`GetRank`).
    #[must_use]
    pub fn honor_rank(&self) -> HonorRankInfo {
        self.honor_rank_info
    }

    /// Highest rank info from the stored scalar
    /// (`SetHighestRank(uint8)` + `CalculateRankInfo`
    /// on a fresh positive info, as at load).
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn honor_highest_rank_info(&self) -> HonorRankInfo {
        let mut info = HonorRankInfo::init();
        info.rank = self.honor_highest_rank.clamp(0, 255) as u8;
        info.calculate_info();
        info
    }

    /// `CalculateTotalKills` (`HonorMgr.cpp:876`):
    /// today's CP rows (any type) for one victim.
    #[must_use]
    pub fn honor_total_kills_today(&self, victim_type: u8, victim_id: u32, game_day: u32) -> u32 {
        self.deferred
            .honor_cp
            .iter()
            .filter(|r| {
                r.victim_type == victim_type && r.victim_id == victim_id && r.date == game_day
            })
            .count() as u32
    }

    /// Row half of `HonorMgr::Add` (`HonorMgr.cpp:
    /// 719`): zero CP rejected; dishonorable CP
    /// subtracts rank points immediately (floored
    /// at 0). The caller sends `SMSG_PVP_CREDIT`
    /// and runs `honor_update`, as the fork tail
    /// does.
    pub fn honor_add(
        &mut self,
        cp: f32,
        kind: u8,
        victim_type: u8,
        victim_id: u32,
        game_day: u32,
    ) -> bool {
        if cp == 0.0 {
            return false;
        }
        if kind == HONOR_TYPE_DISHONORABLE {
            self.honor_rank_points = if self.honor_rank_points > cp {
                self.honor_rank_points - cp
            } else {
                0.0
            };
        }
        self.deferred
            .honor_cp
            .push(mangos_db::character_load::HonorCpRow {
                victim_type,
                victim_id,
                cp,
                date: game_day,
                kind,
            });
        true
    }

    /// CP-row totals for `honor_update` (today /
    /// yesterday / this-week / lifetime splits).
    #[allow(clippy::similar_names)]
    fn honor_totals(&self, game_day: u32, week_begin_day: u32) -> HonorTotals {
        let mut today_hk: u32 = 0;
        let mut today_dk: u32 = 0;
        let mut yesterday_kills: u32 = 0;
        let mut this_week_kills: u32 = 0;
        let mut yesterday_cp: f32 = 0.0;
        let mut this_week_cp: f32 = 0.0;

        let today = game_day;
        let yesterday = today.wrapping_sub(1);

        let mut total_dk = self.honor_stored_dk.max(0) as u32;
        let mut total_hk = self.honor_stored_hk.max(0) as u32;

        for row in &self.deferred.honor_cp {
            if row.kind == HONOR_TYPE_HONORABLE {
                if row.date == today {
                    today_hk += 1;
                }
                if row.date == yesterday {
                    yesterday_kills += 1;
                }
                if row.date >= week_begin_day {
                    this_week_kills += 1;
                    total_hk += 1;
                }
            }
            if row.kind != HONOR_TYPE_DISHONORABLE {
                if row.date == yesterday {
                    yesterday_cp += row.cp;
                }
                if row.date >= week_begin_day {
                    this_week_cp += row.cp;
                }
            }
            if row.kind == HONOR_TYPE_DISHONORABLE {
                if row.date == today {
                    today_dk += 1;
                }
                total_dk += 1;
            }
        }

        HonorTotals {
            today_hk,
            today_dk,
            yesterday_kills,
            this_week_kills,
            yesterday_cp,
            this_week_cp,
            total_hk,
            total_dk,
        }
    }

    /// `HonorMgr::Update` (`HonorMgr.cpp:768`):
    /// recompute lifetime totals from the stored
    /// scalars + CP rows, refresh rank/highest,
    /// and write the honor client fields.
    /// `game_day` is `sWorld.GetGameDay()`,
    /// `week_begin_day` the maintenancer's
    /// `GetWeekBeginDay()`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::similar_names
    )]
    pub fn honor_update(&mut self, game_day: u32, week_begin_day: u32) {
        let totals = self.honor_totals(game_day, week_begin_day);
        let (
            today_hk,
            today_dk,
            yesterday_kills,
            this_week_kills,
            yesterday_cp,
            this_week_cp,
            total_hk,
            total_dk,
        ) = (
            totals.today_hk,
            totals.today_dk,
            totals.yesterday_kills,
            totals.this_week_kills,
            totals.yesterday_cp,
            totals.this_week_cp,
            totals.total_hk,
            totals.total_dk,
        );

        self.honor_total_hk = total_hk;
        self.honor_total_dk = total_dk;
        self.honor_rank_info = HonorRankInfo::calculate(self.honor_rank_points);
        let rank = self.honor_rank_info;
        if rank.visual_rank > 0 && rank.visual_rank > self.honor_highest_rank_info().visual_rank {
            self.honor_highest_rank = i32::from(rank.rank);
        }
        let highest = self.honor_highest_rank_info();

        let v = self.unit.world.object.values_mut();
        // HIGHEST RANK (PLAYER_FIELD_BYTES byte 3).
        v.set_byte(offsets::PLAYER_FIELD_BYTES, 3, highest.rank);
        // RANK (PLAYER_BYTES_3 byte 3).
        v.set_byte(offsets::PLAYER_BYTES_3, 3, rank.rank);
        // Honor bar (PLAYER_FIELD_BYTES2 byte 0):
        // fork u8 cast of the signed fraction.
        let honor_bar_abs = (if self.honor_rank_points >= 0.0 {
            self.honor_rank_points
        } else {
            -self.honor_rank_points
        }) as u32;
        let bar = ((honor_bar_abs as f32 - rank.min_rp) / (rank.max_rp - rank.min_rp))
            * if rank.positive { 255.0 } else { -255.0 };
        v.set_byte(offsets::PLAYER_FIELD_BYTES2, 0, bar as u8);
        // TODAY.
        v.set_u16(
            offsets::PLAYER_FIELD_SESSION_KILLS,
            0,
            today_hk.min(u32::from(u16::MAX)) as u16,
        );
        v.set_u16(
            offsets::PLAYER_FIELD_SESSION_KILLS,
            1,
            today_dk.min(u32::from(u16::MAX)) as u16,
        );
        // YESTERDAY.
        v.set_u32(offsets::PLAYER_FIELD_YESTERDAY_KILLS, yesterday_kills);
        v.set_u32(
            offsets::PLAYER_FIELD_YESTERDAY_CONTRIBUTION,
            if yesterday_cp > 0.0 {
                yesterday_cp as u32
            } else {
                0
            },
        );
        // THIS WEEK.
        v.set_u32(offsets::PLAYER_FIELD_THIS_WEEK_KILLS, this_week_kills);
        v.set_u32(
            offsets::PLAYER_FIELD_THIS_WEEK_CONTRIBUTION,
            if this_week_cp > 0.0 {
                this_week_cp as u32
            } else {
                0
            },
        );
        // LAST WEEK.
        v.set_u32(
            offsets::PLAYER_FIELD_LAST_WEEK_KILLS,
            self.honor_last_week_hk.max(0) as u32,
        );
        v.set_u32(
            offsets::PLAYER_FIELD_LAST_WEEK_CONTRIBUTION,
            if self.honor_last_week_cp > 0.0 {
                self.honor_last_week_cp as u32
            } else {
                0
            },
        );
        v.set_u32(
            offsets::PLAYER_FIELD_LAST_WEEK_RANK,
            self.honor_standing.max(0) as u32,
        );
        // LIFETIME.
        v.set_u32(offsets::PLAYER_FIELD_LIFETIME_HONORBALE_KILLS, total_hk);
        v.set_u32(offsets::PLAYER_FIELD_LIFETIME_DISHONORBALE_KILLS, total_dk);
    }

    /// `ClearHonorData` (`HonorMgr.cpp:602`) minus
    /// the DB delete of `Reset` (the map/actor side
    /// owns persistence).
    pub fn honor_clear_data(&mut self) {
        self.deferred.honor_cp.clear();
        self.honor_total_hk = 0;
        self.honor_total_dk = 0;
        self.honor_stored_hk = 0;
        self.honor_stored_dk = 0;
        self.honor_standing = 0;
        self.honor_last_week_hk = 0;
        self.honor_rank_points = 0.0;
        self.honor_last_week_cp = 0.0;
        self.honor_rank_info = HonorRankInfo::init();
        self.honor_highest_rank = 0;
    }
}
