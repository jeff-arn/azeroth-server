//! Live player-info loads against the dev-bench world DB +
//! production DBC fixtures. `#[ignore]`d; gated on
//! `TEST_MYSQL_URL_WORLD` and `MANGOS_DBC_DIR`. Read-only.
//! Spot values come from independent SQL and a raw-DBC Python
//! read (2026-10-19), not from the loaders.

use std::path::PathBuf;

use mangos_db::DbActor;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_world_data::player_info::{RaceClassDbc, RaceDbc, load_player_infos};
use mangos_world_data::spawns::load_map_ids;

fn uint(row: &[FieldValue], idx: usize) -> u32 {
    match row.get(idx) {
        Some(FieldValue::UInt(v)) => *v,
        other => panic!("field {idx}: expected UInt, got {other:?}"),
    }
}

fn race_class_dbc() -> RaceClassDbc {
    let dir: PathBuf = std::env::var_os("MANGOS_DBC_DIR")
        .expect("MANGOS_DBC_DIR must be set")
        .into();
    let races_bytes = std::fs::read(dir.join(formats::CHR_RACES.file)).expect("ChrRaces.dbc");
    let races_file = DbcFile::parse(&races_bytes, formats::CHR_RACES.fmt).expect("parse races");
    let races_store = races_file
        .produce(formats::CHR_RACES.fmt)
        .expect("produce races");
    let mut dbc = RaceClassDbc::default();
    for row in races_store.rows() {
        // Produced fields of `niixiixxiiiiixixissssssssxxxx`:
        // 0 id, 1 flags, 2 faction, 3 model_m, 4 model_f,
        // 5 team, 6 creature type, ...
        dbc.races.insert(
            uint(row, 0),
            RaceDbc {
                faction_id: uint(row, 2),
                model_m: uint(row, 3),
                model_f: uint(row, 4),
                team_id: uint(row, 5),
            },
        );
    }
    let classes_bytes = std::fs::read(dir.join(formats::CHR_CLASSES.file)).expect("ChrClasses.dbc");
    let classes_file =
        DbcFile::parse(&classes_bytes, formats::CHR_CLASSES.fmt).expect("parse classes");
    let classes_store = classes_file
        .produce(formats::CHR_CLASSES.fmt)
        .expect("produce classes");
    for row in classes_store.rows() {
        dbc.classes.insert(uint(row, 0));
    }
    dbc
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD and MANGOS_DBC_DIR"]
fn bench_player_infos_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let dbc = race_class_dbc();
    // DBC oracle: human models 49/50, team 7 (raw-bytes read).
    assert_eq!(dbc.races.get(&1).expect("human").model_m, 49);
    assert_eq!(dbc.races.get(&1).expect("human").model_f, 50);
    assert_eq!(dbc.races.get(&1).expect("human").team_id, 7);
    assert_eq!(dbc.races.get(&2).expect("orc").team_id, 1);

    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let (infos, errors) = actor
        .call(move |conn| {
            let map_ids = load_map_ids(conn)?;
            let (infos, report) = load_player_infos(conn, 60, Some(&dbc), Some(&map_ids), None)?;
            Ok((infos, report.errors))
        })
        .expect("player infos");

    assert_eq!(infos.create_count(), 40);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let orc_warrior_defaults = infos.default_spells(2, 1);
    assert!(orc_warrior_defaults.contains(&78), "orc warrior defaults");
    assert!(orc_warrior_defaults.contains(&6603), "Attack default");
    let human_warrior = infos.create_info(1, 1).expect("human warrior create");
    assert_eq!(
        (human_warrior.map_id, human_warrior.area_id),
        (0, 12),
        "Northshire"
    );
    assert_eq!(human_warrior.position_x.to_bits(), (-8949.95f32).to_bits());
    assert_eq!(human_warrior.position_y.to_bits(), (-132.493f32).to_bits());
    assert_eq!(
        (human_warrior.display_id_m, human_warrior.display_id_f),
        (49, 50)
    );

    // player_classlevelstats: warrior 1 -> hp 20/mana 0; 60 -> 1689/0.
    let w1 = infos.class_level_info(1, 1).expect("warrior 1");
    assert_eq!((w1.basehealth, w1.basemana), (20, 0));
    let w60 = infos.class_level_info(1, 60).expect("warrior 60");
    assert_eq!((w60.basehealth, w60.basemana), (1689, 0));
    // Level clamps to MaxPlayerLevel; level 0 / bad class -> None.
    assert_eq!(infos.class_level_info(1, 99), Some(w60));
    assert_eq!(infos.class_level_info(1, 0), None);
    assert_eq!(infos.class_level_info(12, 1), None);

    // player_levelstats: human warrior str/agi/sta/int/spi.
    let l1 = infos.level_info(1, 1, 1).expect("hw 1");
    assert_eq!(l1.stats, [23, 20, 22, 20, 20]);
    let l60 = infos.level_info(1, 1, 60).expect("hw 60");
    assert_eq!(l60.stats, [120, 80, 110, 30, 45]);
    // BuildPlayerLevelInfo above the table: the C++ loop
    // runs `lvl` over [MaxPlayerLevel-1, level) = {59, 60}
    // for level 61, starting from the level-60 row. Warrior:
    // str/sta +2 each (lvl>23), agi +1 each (lvl>36),
    // int/spi +1 only at even lvl 60. [124, 82, 114, 31, 46].
    let l61 = infos.level_info(1, 1, 61).expect("hw 61");
    assert_eq!(l61.stats, [124, 82, 114, 31, 46]);
    // Goblin (race 9) is out of MAX_RACES.
    assert_eq!(infos.level_info(9, 1, 1), None);

    // player_xp_for_level.
    assert_eq!(infos.xp_for_level(1), 400);
    assert_eq!(infos.xp_for_level(59), 209_800);
    assert_eq!(infos.xp_for_level(60), 217_400);
    assert_eq!(infos.xp_for_level(500), 217_400, "clamps to last index");
}
