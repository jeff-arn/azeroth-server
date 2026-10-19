//! Live area and weather loads against the dev-bench
//! world DB. `#[ignore]`d + gated on
//! `TEST_MYSQL_URL_WORLD` (URL must point at the bench
//! `mangos` database). Read-only.

use mangos_db::DbActor;
use mangos_world_data::areas::load_areas;
use mangos_world_data::weather::load_weather_chances;

#[test]
#[ignore = "requires TEST_MYSQL_URL_WORLD pointing at the dev fixture world DB"]
fn bench_areas_and_weather_load() {
    let url = std::env::var("TEST_MYSQL_URL_WORLD").expect("TEST_MYSQL_URL_WORLD must be set");
    let actor = DbActor::spawn("test-world", &url).expect("spawn");
    let (areas, weather) = actor
        .call(|conn| Ok((load_areas(conn, 10)?, load_weather_chances(conn)?)))
        .expect("load areas and weather");
    assert!(!areas.is_empty(), "bench area_template must not be empty");
    assert_eq!(areas.name(14), "Durotar");
    assert!(!weather.is_empty(), "bench game_weather must not be empty");
    assert!(weather.get(0).is_none(), "zone 0 weather is skipped");
}
