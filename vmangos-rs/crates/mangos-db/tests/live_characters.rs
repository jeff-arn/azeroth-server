//! Live character-DB tests. `#[ignore]`d + gated on
//! `TEST_MYSQL_URL_CHARACTERS` (bench `characters` DB).
//! Oracle values read from the bench snapshot by
//! independent SQL (account 5 = JEFF, one character: guid 1).
//! The lifecycle test writes only to a synthetic account
//! and removes everything it creates (bench data is
//! disposable; guid 1 is never touched).

use mangos_db::CharacterDb;
use mangos_db::DbActor;
use mangos_db::characters::{DeletePlan, NewCharacter};
use mangos_db::corpse::{
    CorpseRow, delete_corpse_for_player, load_corpses, max_corpse_guid, save_corpse,
};
use mysql::prelude::Queryable as _;

fn db() -> CharacterDb {
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    CharacterDb::spawn(&url).expect("spawn")
}

/// Character creation allocates `MAX(guid)+1`
/// (single-writer on the server); parallel test
/// threads must not create concurrently.
static CREATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn char_enum_for_bench_account() {
    let rows = db().char_enum(5).expect("char enum");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.guid, 1);
    assert_eq!(row.name, "Ruikanis");
    assert_eq!(row.race, 1); // human
    assert_eq!(row.class, 1); // warrior
    assert_eq!(row.level, 19);
    assert!(row.equipment_cache.is_some());
    // Warrior: no current pet row expected in the snapshot.
    assert_eq!(row.pet_entry, None);
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn char_enum_empty_for_unknown_account() {
    let rows = db().char_enum(999_999).expect("char enum");
    assert!(rows.is_empty());
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn tutorials_and_account_data_read() {
    let db = db();
    // Shapes only: contents vary; calls must succeed read-only.
    let _tutorials = db.load_tutorials(5).expect("tutorials");
    let _global = db.load_global_account_data(5).expect("global data");
    let _per_char = db.load_character_account_data(1).expect("per-char data");
}

fn sample_new_character(name: &str) -> NewCharacter {
    NewCharacter {
        account_id: 999_001,
        name: name.to_string(),
        race: 1,
        class: 1,
        gender: 0,
        skin: 1,
        face: 2,
        hair_style: 3,
        hair_color: 4,
        facial_hair: 5,
        level: 1,
        money: 0,
        map_id: 0,
        position_x: -8949.95,
        position_y: -132.493,
        position_z: 83.5312,
        orientation: 1.5,
        taxi_mask: "0 0 0 0 0 0 0 0 ".to_string(),
        extra_flags: 0x0004,
        health: 42,
        powers: [0; 5],
        equipment_cache: "0 0 ".repeat(20),
        ammo_id: 0,
        actions: vec![(0, 6603, 0), (1, 78, 0)],
        now: 1_700_000_000,
    }
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn create_and_delete_round_trip() {
    let _serial = CREATE_LOCK.lock().expect("lock");
    let db = db();
    // Full delete (method 0).
    let guid = db
        .create_character(&sample_new_character("Tierfourtest"))
        .expect("create");
    assert!(guid > 1);
    assert!(db.name_in_use("Tierfourtest").expect("name in use"));
    assert_eq!(db.character_count(999_001).expect("count"), 1);
    let owner = db
        .character_owner(guid)
        .expect("owner read")
        .expect("owner row");
    assert_eq!(owner.account_id, 999_001);
    assert_eq!(owner.level, 1);
    let rows = db.char_enum(999_001).expect("enum");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].guid, guid);
    db.delete_character(
        guid,
        &DeletePlan {
            method: 0,
            account_id: 999_001,
            mail_delivery_delay_secs: 3600,
            now: 1_700_000_100,
        },
    )
    .expect("delete method 0");
    assert!(!db.name_in_use("Tierfourtest").expect("name freed"));
    assert_eq!(db.character_count(999_001).expect("count 0"), 0);
    assert!(
        db.character_owner(guid).expect("owner gone").is_none(),
        "method 0 removes the row"
    );

    // Soft delete (method 1): row unlinked, name freed.
    let guid = db
        .create_character(&sample_new_character("Tierfourtest"))
        .expect("create 2");
    db.delete_character(
        guid,
        &DeletePlan {
            method: 1,
            account_id: 999_001,
            mail_delivery_delay_secs: 3600,
            now: 1_700_000_200,
        },
    )
    .expect("delete method 1");
    assert_eq!(db.character_count(999_001).expect("soft count"), 0);
    assert!(!db.name_in_use("Tierfourtest").expect("soft name freed"));
    let owner = db
        .character_owner(guid)
        .expect("soft owner read")
        .expect("soft row retained");
    assert_eq!(owner.account_id, 0);
    // Keep-days sweep sees it once the stamp ages out.
    let stale = db
        .old_deleted_characters(30, 1_700_000_200 + 31 * 86_400)
        .expect("sweep read");
    assert!(stale.contains(&(guid, 999_001)));
    // Final cleanup: hard delete the retained row.
    db.delete_character(
        guid,
        &DeletePlan {
            method: 0,
            account_id: 999_001,
            mail_delivery_delay_secs: 3600,
            now: 1_700_000_300,
        },
    )
    .expect("final delete");
    assert!(db.character_owner(guid).expect("cleaned").is_none());
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn name_query_for_bench_character() {
    let db = db();
    let row = db.name_query(1).expect("name query").expect("Ruikanis row");
    assert_eq!(row.name, "Ruikanis");
    assert_eq!(row.race, 1);
    assert_eq!(row.gender, 0);
    assert_eq!(row.class, 1);
    assert!(
        db.name_query(999_999)
            .expect("missing name query")
            .is_none()
    );
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn corpse_save_load_delete_round_trip() {
    let _serial = CREATE_LOCK.lock().expect("lock");
    let db = db();
    let guid = db
        .create_character(&sample_new_character("Tierfcorpse"))
        .expect("create");
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    let actor = DbActor::spawn("test-corpse", &url).expect("actor");
    let corpse_guid = 987_654_u32;
    actor
        .call(move |conn| {
            save_corpse(
                conn,
                &CorpseRow {
                    guid: corpse_guid,
                    player_guid: guid,
                    position_x: -8949.95,
                    position_y: -132.493,
                    position_z: 83.5312,
                    orientation: 1.5,
                    map: 0,
                    time: 1_700_000_500,
                    corpse_type: 1,
                    instance: 0,
                },
            )
        })
        .expect("save corpse");
    let loaded = actor.call(load_corpses).expect("load corpses");
    let mine = loaded
        .iter()
        .find(|c| c.row.player_guid == guid)
        .expect("corpse joined to owner");
    assert_eq!(mine.row.guid, corpse_guid);
    assert_eq!(mine.row.corpse_type, 1);
    assert_eq!(mine.row.time, 1_700_000_500);
    assert_eq!(mine.race, 1);
    assert_eq!(mine.class, 1);
    assert_eq!(mine.skin, 1);
    assert_eq!(mine.face, 2);
    assert_eq!(mine.hair_style, 3);
    assert_eq!(mine.hair_color, 4);
    assert_eq!(mine.facial_hair, 5);
    assert!(mine.equipment_cache.is_some());
    let max_guid = actor.call(max_corpse_guid).expect("max corpse guid");
    assert!(max_guid >= corpse_guid, "guid seed covers saved corpse");
    actor
        .call(move |conn| delete_corpse_for_player(conn, guid))
        .expect("delete corpse");
    let loaded = actor.call(load_corpses).expect("reload corpses");
    assert!(
        loaded.iter().all(|c| c.row.player_guid != guid),
        "corpse row gone after delete"
    );
    db.delete_character(
        guid,
        &DeletePlan {
            method: 0,
            account_id: 999_001,
            mail_delivery_delay_secs: 3600,
            now: 1_700_000_600,
        },
    )
    .expect("cleanup delete");
}

#[test]
#[ignore = "requires TEST_MYSQL_URL_CHARACTERS pointing at the dev fixture characters DB"]
fn rename_gate_and_update_round_trip() {
    let _serial = CREATE_LOCK.lock().expect("lock");
    let db = db();
    let guid = db
        .create_character(&sample_new_character("Tierfrename"))
        .expect("create");
    // No pending rename flag: gate misses.
    assert_eq!(
        db.rename_character(guid, 999_001, "Tierfnewname")
            .expect("rename gate"),
        None
    );
    // GM sets CHARACTER_FLAG_RENAME (0x4000).
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    let actor = DbActor::spawn("test-rename", &url).expect("actor");
    actor
        .call(move |conn| {
            conn.exec_drop(
                "UPDATE `characters` SET `character_flags` = (`character_flags` | 16384) \
                 WHERE `guid` = ?",
                (guid,),
            )?;
            Ok(())
        })
        .expect("flag set");
    // Wrong account: gate misses.
    assert_eq!(
        db.rename_character(guid, 999_002, "Tierfnewname")
            .expect("rename wrong account"),
        None
    );
    // Taken name (guid 1 oracle): gate misses.
    assert_eq!(
        db.rename_character(guid, 999_001, "Ruikanis")
            .expect("rename taken"),
        None
    );
    // Success: old name returned, row updated, flags
    // flip RENAME(0x4000) -> NEEDS_GM_REVIEW(0x8000).
    let old = db
        .rename_character(guid, 999_001, "Tierfnewname")
        .expect("rename")
        .expect("renamed");
    assert_eq!(old, "Tierfrename");
    let rows = db.char_enum(999_001).expect("enum");
    let row = rows.iter().find(|r| r.guid == guid).expect("row");
    assert_eq!(row.name, "Tierfnewname");
    assert_eq!(row.character_flags & 0x0000_4000, 0, "rename flag cleared");
    assert_ne!(row.character_flags & 0x0000_8000, 0, "GM-review flag set");
    db.delete_character(
        guid,
        &DeletePlan {
            method: 0,
            account_id: 999_001,
            mail_delivery_delay_secs: 3600,
            now: 1_700_000_700,
        },
    )
    .expect("cleanup delete");
}
