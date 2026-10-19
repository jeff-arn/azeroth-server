//! World-presence director (`BotDirector` core):
//! classifies human players by zone and assigns
//! crowd-bot GUID pools (33-52 horde, 53-72
//! alliance city, 73-92 alliance world) to zones,
//! emitting `playerbot.chance` updates for the
//! population manager. Pure state; host applies.

use std::collections::HashMap;

/// `BotDirectorZoneType`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ZoneType {
    Capital = 0,
    Friendly = 1,
    Contested = 2,
    Unknown = 3,
}

/// `BotDirectorZoneInfo`.
#[derive(Clone, Copy, Debug)]
pub struct ZoneInfo {
    pub zone_id: u32,
    pub name: &'static str,
    pub map_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub level_min: u32,
    pub level_max: u32,
    pub zone_type: ZoneType,
    pub faction: Option<&'static str>,
}

/// `BotAssignment`.
#[derive(Clone, Copy, Debug)]
pub struct BotAssignment {
    pub guid: u32,
    pub target_map: u32,
    pub target_x: f32,
    pub target_y: f32,
    pub target_z: f32,
    pub target_zone: u32,
    pub radius: f32,
    pub aggressive: bool,
    pub target_level: u32,
    /// 469 Alliance, 67 Horde.
    pub faction: u32,
}

/// One online human for the cycle.
#[derive(Clone, Debug)]
pub struct HumanInfo {
    pub level: u32,
    pub zone: u32,
    pub name: String,
}

pub(super) mod zones {
    include!("director_zones.rs");
}

const HORDE_GUID_MIN: u32 = 33;
const HORDE_GUID_MAX: u32 = 52;
const ALLIANCE_CITY_GUID_MIN: u32 = 53;
const ALLIANCE_CITY_GUID_MAX: u32 = 72;
const ALLIANCE_WORLD_GUID_MIN: u32 = 73;
const ALLIANCE_WORLD_GUID_MAX: u32 = 92;

type Pick<'a> = dyn FnMut(u32, u32) -> u32 + 'a;

fn urand(pick: &mut Pick<'_>, lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + pick(0, hi - lo) }
}

fn city_bot_level(pick: &mut Pick<'_>) -> u32 {
    let roll = urand(pick, 0, 99);
    if roll < 40 {
        60
    } else if roll < 70 {
        urand(pick, 40, 59)
    } else {
        urand(pick, 10, 39)
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_assignment(
    next: &mut HashMap<u32, BotAssignment>,
    guid: u32,
    zone: &ZoneInfo,
    radius: f32,
    aggressive: bool,
    level: u32,
    faction: u32,
) {
    if guid == 0 {
        return;
    }
    next.insert(
        guid,
        BotAssignment {
            guid,
            target_map: zone.map_id,
            target_x: zone.x,
            target_y: zone.y,
            target_z: zone.z,
            target_zone: zone.zone_id,
            radius,
            aggressive,
            target_level: level,
            faction,
        },
    );
}

/// `BotDirector` core state.
pub struct DirectorCore {
    pub enabled: bool,
    pub update_interval_ms: u64,
    deadline_ms: u64,
    assignments: HashMap<u32, BotAssignment>,
}

impl DirectorCore {
    #[must_use]
    pub fn new(enabled: bool, update_interval_ms: u64) -> Self {
        Self {
            enabled,
            update_interval_ms,
            deadline_ms: 0,
            assignments: HashMap::new(),
        }
    }

    /// `GetAssignment`.
    #[must_use]
    pub fn assignment(&self, guid: u32) -> Option<BotAssignment> {
        self.assignments.get(&guid).copied()
    }

    /// `ClassifyZone`.
    #[must_use]
    pub fn classify_zone(zone_id: u32) -> (ZoneType, Option<&'static ZoneInfo>) {
        for zone in zones::CAPITAL_CITIES {
            if zone.zone_id == zone_id {
                return (ZoneType::Capital, Some(zone));
            }
        }
        for zone in zones::FRIENDLY_ZONES {
            if zone.zone_id == zone_id {
                return (ZoneType::Friendly, Some(zone));
            }
        }
        for zone in zones::CONTESTED_ZONES {
            if zone.zone_id == zone_id {
                return (ZoneType::Contested, Some(zone));
            }
        }
        (ZoneType::Unknown, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_contested(
        contested: &[(&HumanInfo, &ZoneInfo)],
        horde: &mut Vec<u32>,
        world: &mut Vec<u32>,
        next: &mut HashMap<u32, BotAssignment>,
        logs: &mut Vec<String>,
        pick: &mut Pick<'_>,
    ) {
        for (player, zone) in contested {
            let num_horde = 5.min(horde.len());
            for _ in 0..num_horde {
                let guid = horde.pop().unwrap_or(0);
                let level = if urand(pick, 0, 99) < 10 {
                    logs.push(format!(
                        "[BotDirector]     !! Level 60 Horde ganker near {}!",
                        player.name
                    ));
                    60
                } else {
                    urand(pick, zone.level_min, zone.level_max)
                };
                insert_assignment(next, guid, zone, 200.0, true, level, 67);
            }
            let num_friendly = 3.min(world.len());
            for _ in 0..num_friendly {
                let guid = world.pop().unwrap_or(0);
                let level = if urand(pick, 0, 99) < 10 {
                    logs.push(format!(
                        "[BotDirector]     ** Level 60 Alliance hero near {}!",
                        player.name
                    ));
                    60
                } else {
                    urand(pick, zone.level_min, zone.level_max)
                };
                insert_assignment(next, guid, zone, 200.0, false, level, 469);
            }
            logs.push(format!(
                "[BotDirector]   Contested {}: {num_horde} horde + {num_friendly} alliance for {} (zone lvl {}-{})",
                zone.name, player.name, zone.level_min, zone.level_max
            ));
        }
    }

    fn assign_cities(
        city: &[(&HumanInfo, &ZoneInfo)],
        city_pool: &mut Vec<u32>,
        next: &mut HashMap<u32, BotAssignment>,
        logs: &mut Vec<String>,
        pick: &mut Pick<'_>,
    ) {
        for zone in zones::CAPITAL_CITIES {
            let count = 5.min(city_pool.len());
            for _ in 0..count {
                let guid = city_pool.pop().unwrap_or(0);
                let level = city_bot_level(pick);
                insert_assignment(next, guid, zone, 80.0, false, level, 469);
            }
            logs.push(format!(
                "[BotDirector]   Always-on: {count} city bots in {}",
                zone.name
            ));
        }
        for (player, zone) in city {
            let count = 5.min(city_pool.len());
            for _ in 0..count {
                let guid = city_pool.pop().unwrap_or(0);
                let level = city_bot_level(pick);
                insert_assignment(next, guid, zone, 80.0, false, level, 469);
            }
            logs.push(format!(
                "[BotDirector]   Capital {}: extra {count} city bots for {}",
                zone.name, player.name
            ));
        }
    }

    fn assign_friendly(
        friendly: &[(&HumanInfo, &ZoneInfo)],
        world: &mut Vec<u32>,
        next: &mut HashMap<u32, BotAssignment>,
        logs: &mut Vec<String>,
        pick: &mut Pick<'_>,
    ) {
        for (player, zone) in friendly {
            let count = 5.min(world.len());
            for _ in 0..count {
                let guid = world.pop().unwrap_or(0);
                let level = urand(pick, zone.level_min, zone.level_max);
                insert_assignment(next, guid, zone, 150.0, false, level, 469);
            }
            logs.push(format!(
                "[BotDirector]   Friendly {}: {count} world bots for {} (zone lvl {}-{})",
                zone.name, player.name, zone.level_min, zone.level_max
            ));
        }
    }

    /// Deadline gate for `Update` (first update runs
    /// immediately, like the fork's zeroed tracker).
    pub fn due(&mut self, now_ms: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if now_ms >= self.deadline_ms {
            self.deadline_ms = now_ms.saturating_add(self.update_interval_ms);
            return true;
        }
        false
    }

    /// `RunDirectorCycle` + `UpdateBotChances`.
    /// Returns (detail log lines, chance updates
    /// as (guid, chance) for all pool GUIDs).
    pub fn run_cycle(
        &mut self,
        players: &[HumanInfo],
        pick: &mut Pick<'_>,
    ) -> (Vec<String>, Vec<(u32, u32)>) {
        let mut logs = vec![format!(
            "[BotDirector] Cycle: {} players online",
            players.len()
        )];
        let mut city: Vec<(&HumanInfo, &ZoneInfo)> = Vec::new();
        let mut friendly: Vec<(&HumanInfo, &ZoneInfo)> = Vec::new();
        let mut contested: Vec<(&HumanInfo, &ZoneInfo)> = Vec::new();
        for player in players {
            let (kind, info) = Self::classify_zone(player.zone);
            match (kind, info) {
                (ZoneType::Capital, Some(zone)) => city.push((player, zone)),
                (ZoneType::Contested, Some(zone)) => contested.push((player, zone)),
                (_, Some(zone)) => friendly.push((player, zone)),
                _ => {}
            }
        }

        // Fork shuffles pools with a fresh mt19937;
        // Fisher-Yates over the host pick preserves
        // the observable distribution.
        let shuffle = |pool: &mut Vec<u32>, pick: &mut Pick<'_>| {
            for i in (1..pool.len()).rev() {
                let j = usize::try_from(urand(pick, 0, u32::try_from(i).unwrap_or(0)))
                    .unwrap_or(0)
                    .min(i);
                pool.swap(i, j);
            }
        };
        let mut horde: Vec<u32> = (HORDE_GUID_MIN..=HORDE_GUID_MAX).collect();
        let mut city_pool: Vec<u32> = (ALLIANCE_CITY_GUID_MIN..=ALLIANCE_CITY_GUID_MAX).collect();
        let mut world: Vec<u32> = (ALLIANCE_WORLD_GUID_MIN..=ALLIANCE_WORLD_GUID_MAX).collect();
        shuffle(&mut horde, pick);
        shuffle(&mut city_pool, pick);
        shuffle(&mut world, pick);

        let mut next: HashMap<u32, BotAssignment> = HashMap::new();
        Self::assign_cities(&city, &mut city_pool, &mut next, &mut logs, pick);
        Self::assign_friendly(&friendly, &mut world, &mut next, &mut logs, pick);
        Self::assign_contested(
            &contested, &mut horde, &mut world, &mut next, &mut logs, pick,
        );
        self.assignments = next;
        logs.push(format!(
            "[BotDirector] Cycle complete: {} bots assigned for {} players",
            self.assignments.len(),
            players.len()
        ));
        let mut chances = Vec::new();
        for guid in (HORDE_GUID_MIN..=HORDE_GUID_MAX)
            .chain(ALLIANCE_CITY_GUID_MIN..=ALLIANCE_CITY_GUID_MAX)
            .chain(ALLIANCE_WORLD_GUID_MIN..=ALLIANCE_WORLD_GUID_MAX)
        {
            chances.push((
                guid,
                if self.assignments.contains_key(&guid) {
                    30
                } else {
                    1
                },
            ));
        }
        (logs, chances)
    }
}
