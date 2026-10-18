//! Live creature-template load against the dev-bench world DB.
//! `#[ignore]`d + gated on `TEST_MYSQL_URL_WORLD`. Read-only.
//! Spot values were read from the bench by independent SQL
//! (2026-10-19), not from the loader.

use mangos_db::DbActor;
use mangos_world_data::creature_template::{
    TemplateValidation, load_creature_template, load_creature_templates,
};

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_creature_templates_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let (templates, report) = actor
        .call(|conn| load_creature_templates(conn, 10, &TemplateValidation::default()))
        .expect("load templates");

    assert_eq!(templates.len(), 10_381);
    // The 10 skipped-check notices lead the report; dirty-row
    // diagnostics may follow but the load itself must succeed.
    assert!(report.len() >= 10);
    assert!(
        report.errors[0].contains("validation skipped"),
        "first report line: {}",
        report.errors[0]
    );

    let wolf = templates.get(69).expect("Timber Wolf");
    assert_eq!(wolf.name, "Timber Wolf");
    assert_eq!((wolf.level_min, wolf.level_max), (2, 2));
    assert_eq!(wolf.faction, 32);
    assert_eq!(wolf.display_id[0], 604);
    assert_eq!(wolf.creature_type, 1, "beast");
    assert_eq!(wolf.unit_class, 1);
    assert_eq!(wolf.inhabit_type, 1);
    assert_eq!(wolf.speed_run.to_bits(), 0.857_143f32.to_bits());
    assert_eq!(wolf.is_tameable(), wolf.static_flags1 & 0x10 != 0);

    let imp = templates.get(416).expect("Imp");
    assert_eq!(imp.pet_family, 23);
    assert_eq!(imp.display_id[0], 4449);
    assert_eq!(imp.creature_type, 3);
    assert_eq!(imp.inhabit_type, 3, "ground|water on bench");

    let godric = templates.get(1213).expect("Godric Rothgar");
    assert_eq!(godric.npc_flags, 16388);
    assert_eq!(godric.display_total_probability, 100);

    // Auras field parsed (entry 89 carries spell 19483 on bench).
    let with_aura = templates.get(89).expect("entry 89");
    assert_eq!(with_aura.auras, vec![19483]);

    assert!(templates.get(999_999).is_none());

    // Single-entry form agrees.
    let (single, _) = actor
        .call(|conn| load_creature_template(conn, 416, 10, &TemplateValidation::default()))
        .expect("single load");
    let single = single.expect("entry 416");
    assert_eq!(single.name, imp.name);
    assert_eq!(single.pet_family, 23);
}
