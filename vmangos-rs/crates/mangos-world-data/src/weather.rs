//! `game_weather` seasonal chances
//! (`WeatherMgr::LoadWeatherZoneChances`).

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// `WeatherSeasonChances`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeatherSeasonChances {
    pub rain_chance: u8,
    pub snow_chance: u8,
    pub storm_chance: u8,
}

/// `WeatherZoneChances`: spring, summer, autumn, winter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeatherZoneChances {
    pub seasons: [WeatherSeasonChances; 4],
}

/// Loaded `game_weather` rows by zone id.
#[derive(Clone, Debug, Default)]
pub struct WeatherChanceStore {
    by_zone: HashMap<u32, WeatherZoneChances>,
}

impl WeatherChanceStore {
    #[must_use]
    pub fn get(&self, zone_id: u32) -> Option<WeatherZoneChances> {
        self.by_zone.get(&zone_id).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_zone.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_zone.is_empty()
    }

    fn from_rows(rows: impl IntoIterator<Item = (u32, WeatherZoneChances)>) -> Self {
        Self {
            by_zone: rows.into_iter().collect(),
        }
    }
}

/// Loads all weather chances. Zone 0 is skipped, as in C++.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_weather_chances(conn: &mut mysql::Conn) -> Result<WeatherChanceStore, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `zone`, \
         `spring_rain_chance`, `spring_snow_chance`, `spring_storm_chance`, \
         `summer_rain_chance`, `summer_snow_chance`, `summer_storm_chance`, \
         `fall_rain_chance`, `fall_snow_chance`, `fall_storm_chance`, \
         `winter_rain_chance`, `winter_snow_chance`, `winter_storm_chance` \
         FROM `game_weather`",
        (),
    )?;
    Ok(WeatherChanceStore::from_rows(rows.into_iter().filter_map(
        |row| {
            let f = mangos_db::row::Fields::from_row(row);
            let zone = f.u32(0);
            (zone != 0).then_some((
                zone,
                WeatherZoneChances {
                    seasons: [
                        WeatherSeasonChances {
                            rain_chance: f.u32(1) as u8,
                            snow_chance: f.u32(2) as u8,
                            storm_chance: f.u32(3) as u8,
                        },
                        WeatherSeasonChances {
                            rain_chance: f.u32(4) as u8,
                            snow_chance: f.u32(5) as u8,
                            storm_chance: f.u32(6) as u8,
                        },
                        WeatherSeasonChances {
                            rain_chance: f.u32(7) as u8,
                            snow_chance: f.u32(8) as u8,
                            storm_chance: f.u32(9) as u8,
                        },
                        WeatherSeasonChances {
                            rain_chance: f.u32(10) as u8,
                            snow_chance: f.u32(11) as u8,
                            storm_chance: f.u32(12) as u8,
                        },
                    ],
                },
            ))
        },
    )))
}
