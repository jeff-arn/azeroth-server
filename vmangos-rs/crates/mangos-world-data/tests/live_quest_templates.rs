//! Live full quest-template load against the
//! dev-bench world DB. `#[ignore]`d + gated on
//! `TEST_MYSQL_URL_WORLD` (URL must point at the
//! bench `mangos` database). Read-only.

use mangos_db::DbActor;
use mangos_world_data::quest_template::load_quest_templates;

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_quest_templates_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let quests = actor
        .call(|conn| load_quest_templates(conn, 10))
        .expect("load quest templates");
    // 4725 rows across patches collapse to one row
    // per entry at patch 10.
    assert!(
        quests.len() > 3_000,
        "bench quest_template entries: {}",
        quests.len()
    );
    // A starter kill quest with XP + money reward:
    // fields must round-trip with plausible values.
    let any_with_xp = (1..quests.len() as u32).find_map(|id| {
        quests
            .get(id)
            .filter(|q| q.rew_xp > 0 && !q.title.is_empty())
    });
    let q = any_with_xp.expect("some quest with RewXP and a title");
    assert_eq!(
        q.xp_value(q.quest_level),
        q.rew_xp,
        "full XP at quest level"
    );
    assert!(
        q.xp_value(q.quest_level + 20) < q.rew_xp,
        "grey-band XP must scale down"
    );
    println!(
        "QUEST-TEMPLATES count={} sample={} '{}' rew_xp={}",
        quests.len(),
        q.entry,
        q.title,
        q.rew_xp
    );
}
