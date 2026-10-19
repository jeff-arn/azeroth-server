// Director zone tables generated from
// `BotDirector.cpp` (never hand-edited).

use super::ZoneType;

pub(super) const CAPITAL_CITIES: &[super::ZoneInfo] = &[
    super::ZoneInfo { zone_id: 1519, name: "Stormwind", map_id: 0, x: -8829.5, y: 625.6, z: 93.9, level_min: 10, level_max: 60, zone_type: ZoneType::Capital, faction: Some("alliance") },
    super::ZoneInfo { zone_id: 1537, name: "Ironforge", map_id: 0, x: -4928.5, y: -946.6, z: 501.6, level_min: 10, level_max: 60, zone_type: ZoneType::Capital, faction: Some("alliance") },
];

pub(super) const FRIENDLY_ZONES: &[super::ZoneInfo] = &[
    super::ZoneInfo { zone_id: 12, name: "Elwynn Forest", map_id: 0, x: -9449.0, y: 64.0, z: 56.0, level_min: 1, level_max: 10, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 40, name: "Westfall", map_id: 0, x: -10647.0, y: 1112.0, z: 35.0, level_min: 10, level_max: 20, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 38, name: "Loch Modan", map_id: 0, x: -5405.0, y: -2894.0, z: 341.0, level_min: 10, level_max: 20, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 44, name: "Redridge Mountains", map_id: 0, x: -9278.0, y: -2250.0, z: 68.0, level_min: 15, level_max: 25, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 148, name: "Darkshore", map_id: 1, x: 6463.0, y: 683.0, z: 9.0, level_min: 10, level_max: 20, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 10, name: "Duskwood", map_id: 0, x: -10531.0, y: -1154.0, z: 28.0, level_min: 20, level_max: 30, zone_type: ZoneType::Friendly, faction: None },
    super::ZoneInfo { zone_id: 11, name: "Wetlands", map_id: 0, x: -3799.0, y: -822.0, z: 10.0, level_min: 20, level_max: 30, zone_type: ZoneType::Friendly, faction: None },
];

pub(super) const CONTESTED_ZONES: &[super::ZoneInfo] = &[
    super::ZoneInfo { zone_id: 267, name: "Hillsbrad Foothills", map_id: 0, x: -40.7, y: -957.0, z: 56.3, level_min: 20, level_max: 30, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 33, name: "Stranglethorn Vale", map_id: 0, x: -12412.8, y: 171.2, z: 3.4, level_min: 30, level_max: 45, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 331, name: "Ashenvale", map_id: 1, x: 949.9, y: -1435.7, z: 62.7, level_min: 18, level_max: 30, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 3, name: "Arathi Highlands", map_id: 0, x: -1263.0, y: -2515.0, z: 22.0, level_min: 30, level_max: 40, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 4, name: "Badlands", map_id: 0, x: -6113.0, y: -3205.0, z: 255.0, level_min: 35, level_max: 45, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 440, name: "Tanaris", map_id: 1, x: -7193.4, y: -3793.5, z: 9.8, level_min: 40, level_max: 50, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 357, name: "Feralas", map_id: 1, x: -4808.0, y: 1590.0, z: 80.0, level_min: 42, level_max: 50, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 361, name: "Felwood", map_id: 1, x: 5230.0, y: -546.0, z: 343.0, level_min: 48, level_max: 55, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 28, name: "Western Plaguelands", map_id: 0, x: 1741.0, y: -1720.0, z: 59.0, level_min: 51, level_max: 58, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 139, name: "Eastern Plaguelands", map_id: 0, x: 2280.0, y: -5310.0, z: 82.0, level_min: 53, level_max: 60, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 46, name: "Burning Steppes", map_id: 0, x: -7530.0, y: -1052.0, z: 229.0, level_min: 50, level_max: 58, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 618, name: "Winterspring", map_id: 1, x: 6724.0, y: -4606.0, z: 721.0, level_min: 53, level_max: 60, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 1377, name: "Silithus", map_id: 1, x: -6815.0, y: 730.0, z: 42.0, level_min: 55, level_max: 60, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 490, name: "Un'Goro Crater", map_id: 1, x: -6204.0, y: -1061.0, z: -217.0, level_min: 48, level_max: 55, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 405, name: "Desolace", map_id: 1, x: -1573.0, y: 2860.0, z: 76.0, level_min: 30, level_max: 40, zone_type: ZoneType::Contested, faction: None },
    super::ZoneInfo { zone_id: 406, name: "Stonetalon Mountains", map_id: 1, x: 920.0, y: 920.0, z: 104.0, level_min: 25, level_max: 35, zone_type: ZoneType::Contested, faction: None },
];
