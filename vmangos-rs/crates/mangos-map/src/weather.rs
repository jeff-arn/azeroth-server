//! Per-zone weather lifecycle (`Weather::Update`,
//! `Weather::ReGenerate`, and `SendWeatherUpdateToPlayer`)
//! driven from [`Map::tick`](crate::map::Map::tick).

use mangos_entities::creature::SpawnRng;
use mangos_guid::timer::ShortIntervalTimer;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::weather::WeatherZoneChances;

use crate::WorldRng;

/// `WeatherType` (`SharedDefines.h:1194-1200`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WeatherType {
    Fine = 0,
    Rain = 1,
    Snow = 2,
    Storm = 3,
}

/// One zone's live weather (`class Weather`).
#[derive(Clone, Debug)]
pub struct ZoneWeather {
    chances: Option<WeatherZoneChances>,
    pub(crate) kind: WeatherType,
    pub(crate) grade: f32,
    timer: ShortIntervalTimer,
}

impl ZoneWeather {
    #[must_use]
    pub fn new(chances: Option<WeatherZoneChances>, change_interval_ms: u32) -> Self {
        let mut timer = ShortIntervalTimer::new();
        timer.set_interval(change_interval_ms);
        Self {
            chances,
            kind: WeatherType::Fine,
            grade: 0.0,
            timer,
        }
    }

    /// `Weather::Update`: advances the regeneration timer
    /// and reports whether the visible weather changed.
    pub fn update(&mut self, diff_ms: u32, now_unix: u64, rng: &mut WorldRng) -> bool {
        self.timer.update(diff_ms);
        if !self.timer.passed() {
            return false;
        }
        self.timer.reset();
        self.regenerate(season_for_unix(now_unix), rng)
    }

    /// `Weather::ReGenerate`, transcribed including its
    /// fall-through behaviour after the "get fair" branch.
    fn regenerate(&mut self, season: usize, rng: &mut WorldRng) -> bool {
        let old_kind = self.kind;
        let old_grade = self.grade;

        let Some(chances) = self.chances else {
            self.kind = WeatherType::Fine;
            self.grade = 0.0;
            return old_kind != self.kind || old_grade.to_bits() != self.grade.to_bits();
        };

        let u = rng.urand(0, 99);
        if u < 30 {
            return false;
        }

        if u < 60 && self.grade < 0.333_333_34 {
            self.kind = WeatherType::Fine;
            self.grade = 0.0;
        }

        if u < 60 && self.kind != WeatherType::Fine {
            self.grade -= 0.333_333_34;
            return true;
        }

        if u < 90 && self.kind != WeatherType::Fine {
            self.grade += 0.333_333_34;
            return true;
        }

        if self.kind != WeatherType::Fine {
            if self.grade < 0.333_333_34 {
                self.grade = 0.9999;
                return true;
            }
            if self.grade > 0.666_666_7 && rng.urand(0, 99) < 50 {
                self.grade -= 0.666_666_7;
                return true;
            }
            self.kind = WeatherType::Fine;
            self.grade = 0.0;
        }

        let season = chances.seasons[season.min(3)];
        let rain_limit = u32::from(season.rain_chance);
        let snow_limit = rain_limit + u32::from(season.snow_chance);
        let storm_limit = snow_limit + u32::from(season.storm_chance);
        let roll = rng.urand(1, 100);
        self.kind = if roll <= rain_limit {
            WeatherType::Rain
        } else if roll <= snow_limit {
            WeatherType::Snow
        } else if roll <= storm_limit {
            WeatherType::Storm
        } else {
            WeatherType::Fine
        };

        if self.kind == WeatherType::Fine {
            self.grade = 0.0;
        } else if u < 90 {
            self.grade = rng.rand_norm() * 0.3333;
        } else if rng.urand(0, 99) < 50 {
            self.grade = rng.rand_norm() * 0.3333 + 0.3334;
        } else {
            self.grade = rng.rand_norm() * 0.3333 + 0.6667;
        }

        self.normalize_grade();
        self.kind != old_kind || self.grade.to_bits() != old_grade.to_bits()
    }

    /// `Weather::GetSound` at the 1.12 build.
    #[must_use]
    pub fn sound(&self) -> u32 {
        const NO_SOUND: u32 = 0;
        const RAIN_LIGHT: u32 = 8533;
        const RAIN_MEDIUM: u32 = 8534;
        const RAIN_HEAVY: u32 = 8535;
        const SNOW_LIGHT: u32 = 8536;
        const SNOW_MEDIUM: u32 = 8537;
        const SNOW_HEAVY: u32 = 8538;
        const SAND_LIGHT: u32 = 8556;
        const SAND_MEDIUM: u32 = 8557;
        const SAND_HEAVY: u32 = 8558;

        if self.grade < 0.3 {
            return NO_SOUND;
        }
        let (light, medium, heavy) = match self.kind {
            WeatherType::Rain => (RAIN_LIGHT, RAIN_MEDIUM, RAIN_HEAVY),
            WeatherType::Snow => (SNOW_LIGHT, SNOW_MEDIUM, SNOW_HEAVY),
            WeatherType::Storm => (SAND_LIGHT, SAND_MEDIUM, SAND_HEAVY),
            WeatherType::Fine => return NO_SOUND,
        };
        if self.grade < 0.6 {
            light
        } else if self.grade < 0.9 {
            medium
        } else {
            heavy
        }
    }

    /// `SMSG_WEATHER` at 5875: type, grade, sound, smooth
    /// transition byte.
    pub fn packet(&mut self) -> WorldPacket {
        self.normalize_grade();
        build_weather_packet(self.kind, self.grade, self.sound())
    }

    fn normalize_grade(&mut self) {
        if self.grade >= 1.0 {
            self.grade = 0.9999;
        } else if self.grade < 0.0 {
            self.grade = 0.0001;
        }
    }
}

/// Builds one `SMSG_WEATHER` packet.
#[must_use]
pub fn build_weather_packet(kind: WeatherType, grade: f32, sound: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(kind as u32);
    writer.write_f32_le(grade);
    writer.write_u32_le(sound);
    writer.write_u8(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_WEATHER);
    packet.take_writer(writer);
    packet
}

/// `((tm_yday - 78 + 365) / 91) % 4` with tm's zero-based
/// day of year.
#[must_use]
pub fn season_for_day_of_year(day_of_year: u32) -> usize {
    ((day_of_year + 287) / 91 % 4) as usize
}

/// Season for a Unix timestamp, in UTC. C++ uses local
/// time; only the few hours around the Mar 20 boundary can
/// differ.
#[must_use]
pub fn season_for_unix(now_unix: u64) -> usize {
    season_for_day_of_year(day_of_year(now_unix))
}

#[must_use]
pub fn day_of_year(now_unix: u64) -> u32 {
    let mut days = now_unix / 86_400;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            return days as u32;
        }
        days -= days_in_year;
        year += 1;
    }
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_world_data::weather::WeatherSeasonChances;

    #[test]
    fn season_boundaries_match_tm_formula() {
        assert_eq!(season_for_day_of_year(0), 3); // Jan 1: winter
        assert_eq!(season_for_day_of_year(78), 0); // Mar 20: spring
        assert_eq!(season_for_day_of_year(364), 3);
    }

    #[test]
    fn sound_thresholds_match_weather_cpp() {
        let mut weather = ZoneWeather::new(None, 600_000);
        weather.kind = WeatherType::Rain;
        weather.grade = 0.29;
        assert_eq!(weather.sound(), 0);
        weather.grade = 0.30;
        assert_eq!(weather.sound(), 8533);
        weather.grade = 0.60;
        assert_eq!(weather.sound(), 8534);
        weather.grade = 0.90;
        assert_eq!(weather.sound(), 8535);
    }

    #[test]
    fn zones_without_chances_stay_fine() {
        let mut weather = ZoneWeather::new(None, 1);
        weather.kind = WeatherType::Rain;
        weather.grade = 0.5;
        let mut rng = WorldRng::from_seed(7);
        assert!(weather.update(1, 0, &mut rng));
        assert_eq!(weather.kind, WeatherType::Fine);
        assert_eq!(weather.grade.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn full_snow_chance_can_regenerate_to_snow() {
        // Deterministically find a seed whose first gate roll
        // admits regeneration; the type roll is then forced.
        let chances = WeatherZoneChances {
            seasons: [WeatherSeasonChances {
                rain_chance: 0,
                snow_chance: 100,
                storm_chance: 0,
            }; 4],
        };
        let mut saw_snow = false;
        for seed in 0..64 {
            let mut weather = ZoneWeather::new(Some(chances), 1);
            let mut rng = WorldRng::from_seed(seed);
            if weather.update(1, 0, &mut rng) && weather.kind == WeatherType::Snow {
                saw_snow = true;
                break;
            }
        }
        assert!(saw_snow);
    }
}
