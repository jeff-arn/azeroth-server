//! Live Player materialization for the bench fixture
//! character. `#[ignore]`d; gated on `TEST_MYSQL_URL_WORLD`,
//! `TEST_MYSQL_URL_CHARACTERS`, `CHARACTER_LOAD_GUID`, and
//! `MANGOS_DBC_DIR`. Expected values are independent SQL /
//! raw-DBC oracles (2026-10-19): human warrior level 19,
//! `player_levelstats` 46/34/42/22/26, base hp 183, saved
//! health 181 -> naked max health 183 + (20 + 22*10) = 423.

use std::path::PathBuf;

use mangos_db::DbActor;
use mangos_db::character_load::load_character;
use mangos_dbc::{DbcFile, FieldValue, formats};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeInputs, Player};
use mangos_entities::unit::{POWER_ENERGY, POWER_MANA, POWER_RAGE, Team};
use mangos_protocol::update_field_table::offsets;
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
    let mut dbc = RaceClassDbc::default();
    let bytes = std::fs::read(dir.join(formats::CHR_RACES.file)).expect("ChrRaces.dbc");
    let store = DbcFile::parse(&bytes, formats::CHR_RACES.fmt)
        .expect("parse")
        .produce(formats::CHR_RACES.fmt)
        .expect("produce");
    for row in store.rows() {
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
    let bytes = std::fs::read(dir.join(formats::CHR_CLASSES.file)).expect("ChrClasses.dbc");
    let store = DbcFile::parse(&bytes, formats::CHR_CLASSES.fmt)
        .expect("parse")
        .produce(formats::CHR_CLASSES.fmt)
        .expect("produce");
    for row in store.rows() {
        dbc.classes.insert(uint(row, 0));
        dbc.class_power.insert(uint(row, 0), uint(row, 1));
    }
    dbc
}

#[test]
#[ignore = "requires bench world+characters DBs, CHARACTER_LOAD_GUID, MANGOS_DBC_DIR"]
fn bench_warrior_materializes() {
    let world_url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD");
    let chars_url = std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID")
        .parse()
        .expect("guid");
    let dbc = race_class_dbc();
    assert_eq!(dbc.class_power.get(&1), Some(&1), "warrior power type rage");

    let world = DbActor::spawn("test-world", &world_url).expect("world");
    let (infos, dbc) = world
        .call(move |conn| {
            let map_ids = load_map_ids(conn)?;
            let infos = load_player_infos(conn, 60, Some(&dbc), Some(&map_ids), None)?.0;
            Ok((infos, dbc))
        })
        .expect("infos");
    let chars = DbActor::spawn("test-chars", &chars_url).expect("chars");
    let load = chars
        .call(move |conn| load_character(conn, guid))
        .expect("load")
        .expect("fixture character exists");

    let inputs = MaterializeInputs {
        infos: &infos,
        race: dbc.races.get(&1),
        power_type: 1,
        display_scale: 1.0,
        model: None,
        saved_map_known: true,
        saved_map_is_instance: false,
        max_money: MAX_MONEY_AMOUNT,
        now_unix: 1_800_000_000,
    };
    let player = Player::materialize(load, &inputs).expect("materialize");
    let v = player.unit.world.object.values();

    // Identity/appearance.
    assert_eq!(player.unit.race(), 1);
    assert_eq!(player.unit.class(), 1);
    assert_eq!(player.unit.gender(), 0);
    assert_eq!(player.unit.power_type(), 1);
    assert_eq!(player.unit.level(), 19);
    assert_eq!(player.team, Team::Alliance);
    assert_eq!(player.unit.faction_template(), 1);
    assert_eq!(player.unit.display_id(), 49, "human male model");
    assert_eq!(player.unit.native_display_id(), 49);
    assert_eq!(v.get_u32(offsets::PLAYER_XP), 4263);
    assert_eq!(v.get_u32(offsets::PLAYER_NEXT_LEVEL_XP), 21_300);
    assert_eq!(v.get_u32(offsets::PLAYER_FIELD_COINAGE), 4410);
    // skin 1, face 9, hair style 11, hair color 4.
    assert_eq!(
        v.get_u32(offsets::PLAYER_BYTES),
        1 | (9 << 8) | (11 << 16) | (4 << 24)
    );
    // character_flags = 2 (RESTING) projects to PLAYER_FLAGS.
    assert!(v.has_flag(offsets::PLAYER_FLAGS, 0x20));
    // PLAYER_CONTROLLED always.
    assert!(v.has_flag(offsets::UNIT_FIELD_FLAGS, 0x08));

    // Stats (player_levelstats 19): 46/34/42/22/26.
    assert_eq!(
        [
            player.unit.stat(0),
            player.unit.stat(1),
            player.unit.stat(2),
            player.unit.stat(3),
            player.unit.stat(4)
        ]
        .map(f32::to_bits),
        [46.0f32, 34.0, 42.0, 22.0, 26.0].map(f32::to_bits)
    );
    // Armor = agility * 2, no item bonuses in Slice 3.
    assert_eq!(v.get_i32(offsets::UNIT_FIELD_RESISTANCES), 68);

    // Naked max health 423; saved health 181 restored.
    assert_eq!(player.unit.max_health(), 423);
    assert_eq!(player.unit.health(), 181);
    // Warrior: mana max 0, rage max 1000 with saved 0,
    // energy max 100 with saved 100.
    assert_eq!(player.unit.max_power(POWER_MANA), 0);
    assert_eq!(player.unit.max_power(POWER_RAGE), 1000);
    assert_eq!(player.unit.power(POWER_RAGE), 0);
    assert_eq!(player.unit.max_power(POWER_ENERGY), 100);
    assert_eq!(player.unit.power(POWER_ENERGY), 100);
    // Pet loyalty byte for rage classes.
    assert_eq!(v.get_byte(offsets::UNIT_FIELD_BYTES_1, 1), 0xEE);
    // Misc bytes: UNK3|UNK5.
    assert_eq!(v.get_byte(offsets::UNIT_FIELD_BYTES_2, 1), 0x28);
    // Cast speed 1.0f at 5875.
    assert_eq!(
        v.get_f32(offsets::UNIT_MOD_CAST_SPEED).to_bits(),
        1.0f32.to_bits()
    );

    // Position: saved spot (valid continent position).
    let pos = player.unit.world.position();
    assert_eq!(player.unit.world.map_id, 0);
    assert_eq!(pos.x.to_bits(), (-10641.1f32).to_bits());
    assert_eq!(pos.y.to_bits(), 1181.13f32.to_bits());
    assert_eq!(player.recall.map_id, 0);
    // Homebind row: map 0 zone 108.
    assert_eq!(player.homebind.area_id, 108);
    assert!(!player.homebind.synthesized);

    // Visible equipment: slot 4 (chest) item 2977 guid 583371,
    // slot 9 (wrists) item 3472 created by the player itself.
    let chest = offsets::PLAYER_VISIBLE_ITEM_1_0 + 4 * 12;
    assert_eq!(v.get_u32(chest), 2977);
    assert_eq!(
        v.get_guid(offsets::PLAYER_FIELD_INV_SLOT_HEAD + 4 * 2)
            .counter(),
        583_371
    );
    assert_eq!(
        v.get_guid(offsets::PLAYER_VISIBLE_ITEM_1_CREATOR + 9 * 12)
            .counter(),
        guid
    );
    // Bank bag slots byte and action bars byte ride along.
    assert_eq!(v.get_byte(offsets::PLAYER_FIELD_BYTES, 2), 15);
}
