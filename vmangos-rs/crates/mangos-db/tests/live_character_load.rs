//! Field-exact character-load check: the typed 21-query load
//! rendered into the canonical line format of
//! `vmangos-rs/scripts/character_load_oracle_gen.py` and diffed
//! against its output (independent SQL implementation).
//! `#[ignore]`d; needs `TEST_MYSQL_URL_CHARACTERS` and
//! `CHARACTER_LOAD_ORACLE` (path + `CHARACTER_LOAD_GUID`).

use mangos_db::CharacterDb;
use mangos_db::character_load::{CharacterLoad, ItemInstanceRow};

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\p")
        .replace('\n', "\\n")
}

fn fbits(v: f32) -> String {
    format!("bits:{:08x}", v.to_bits())
}

fn line(section: &str, fields: &[(&str, String)]) -> String {
    use std::fmt::Write as _;
    let mut out = section.to_string();
    for (k, v) in fields {
        let _ = write!(out, "|{k}={v}");
    }
    out
}

fn item_fields(item: &ItemInstanceRow) -> Vec<(&'static str, String)> {
    vec![
        ("creator", item.creator_guid.to_string()),
        ("gift", item.gift_creator_guid.to_string()),
        ("count", item.count.to_string()),
        ("duration", item.duration.to_string()),
        ("charges", esc(&item.charges)),
        ("flags", item.flags.to_string()),
        ("ench", esc(&item.enchantments)),
        ("rprop", item.random_property_id.to_string()),
        ("dura", item.durability.to_string()),
        ("text", item.text.to_string()),
        ("item_guid", item.item_guid.to_string()),
        ("item_id", item.item_id.to_string()),
        ("gen_loot", item.generated_loot.to_string()),
    ]
}

fn dump(load: &CharacterLoad) -> Vec<String> {
    let mut out = Vec::new();
    dump_row(load, &mut out);
    dump_progression(load, &mut out);
    dump_items(load, &mut out);
    dump_social(load, &mut out);
    out
}

fn dump_row(load: &CharacterLoad, out: &mut Vec<String>) {
    let r = &load.row;
    let pairs: Vec<(&str, String)> = vec![
        ("guid", r.guid.to_string()),
        ("account", r.account.to_string()),
        ("name", esc(&r.name)),
        ("race", r.race.to_string()),
        ("class", r.class.to_string()),
        ("gender", r.gender.to_string()),
        ("level", r.level.to_string()),
        ("xp", r.xp.to_string()),
        ("money", r.money.to_string()),
        ("skin", r.skin.to_string()),
        ("face", r.face.to_string()),
        ("hair_style", r.hair_style.to_string()),
        ("hair_color", r.hair_color.to_string()),
        ("facial_hair", r.facial_hair.to_string()),
        ("bank_bag_slots", r.bank_bag_slots.to_string()),
        ("character_flags", r.character_flags.to_string()),
        ("position_x", fbits(r.position_x)),
        ("position_y", fbits(r.position_y)),
        ("position_z", fbits(r.position_z)),
        ("map", r.map.to_string()),
        ("orientation", fbits(r.orientation)),
        ("known_taxi_mask", esc(&r.known_taxi_mask)),
        ("played_time_total", r.played_time_total.to_string()),
        ("played_time_level", r.played_time_level.to_string()),
        ("rest_bonus", fbits(r.rest_bonus)),
        ("logout_time", r.logout_time.to_string()),
        (
            "reset_talents_multiplier",
            r.reset_talents_multiplier.to_string(),
        ),
        ("reset_talents_time", r.reset_talents_time.to_string()),
        ("transport_guid", r.transport_guid.to_string()),
        ("transport_x", fbits(r.transport_x)),
        ("transport_y", fbits(r.transport_y)),
        ("transport_z", fbits(r.transport_z)),
        ("transport_o", fbits(r.transport_o)),
        ("extra_flags", r.extra_flags.to_string()),
        ("stable_slots", r.stable_slots.to_string()),
        ("death_expire_time", r.death_expire_time.to_string()),
        ("current_taxi_path", esc(&r.current_taxi_path)),
        ("honor_rank_points", fbits(r.honor_rank_points)),
        ("honor_highest_rank", r.honor_highest_rank.to_string()),
        ("honor_standing", r.honor_standing.to_string()),
        ("honor_last_week_hk", r.honor_last_week_hk.to_string()),
        ("honor_last_week_cp", fbits(r.honor_last_week_cp)),
        ("honor_stored_hk", r.honor_stored_hk.to_string()),
        ("honor_stored_dk", r.honor_stored_dk.to_string()),
        ("watched_faction", r.watched_faction.to_string()),
        ("drunk", r.drunk.to_string()),
        ("health", r.health.to_string()),
        ("power1", r.power1.to_string()),
        ("power2", r.power2.to_string()),
        ("power3", r.power3.to_string()),
        ("power4", r.power4.to_string()),
        ("power5", r.power5.to_string()),
        ("explored_zones", esc(&r.explored_zones)),
        ("ammo_id", r.ammo_id.to_string()),
        ("action_bars", r.action_bars.to_string()),
        ("world_phase_mask", r.world_phase_mask.to_string()),
        ("create_time", r.create_time.to_string()),
        ("instance", r.instance.to_string()),
    ];
    out.push(line("row", &pairs));
    out.push(format!(
        "group|{}",
        load.group_id.map_or("NULL".to_string(), |v| v.to_string())
    ));
}

fn dump_progression(load: &CharacterLoad, out: &mut Vec<String>) {
    for i in &load.bound_instances {
        out.push(line(
            "instance",
            &[
                ("id", i.id.map_or("NULL".into(), |v| v.to_string())),
                ("permanent", i.permanent.to_string()),
                ("map", i.map.map_or("NULL".into(), |v| v.to_string())),
                (
                    "reset_time",
                    i.reset_time.map_or("NULL".into(), |v| v.to_string()),
                ),
            ],
        ));
    }
    for a in &load.auras {
        out.push(line(
            "aura",
            &[
                ("caster", a.caster_guid.to_string()),
                ("item", a.item_guid.to_string()),
                ("spell", a.spell.to_string()),
                ("stacks", a.stacks.to_string()),
                ("charges", a.charges.to_string()),
                ("bp0", fbits(a.base_points[0])),
                ("bp1", fbits(a.base_points[1])),
                ("bp2", fbits(a.base_points[2])),
                ("pt0", a.periodic_time[0].to_string()),
                ("pt1", a.periodic_time[1].to_string()),
                ("pt2", a.periodic_time[2].to_string()),
                ("max_dur", a.max_duration.to_string()),
                ("dur", a.duration.to_string()),
                ("mask", a.effect_index_mask.to_string()),
            ],
        ));
    }
    for s in &load.spells {
        out.push(line(
            "spell",
            &[
                ("spell", s.spell.to_string()),
                ("active", s.active.to_string()),
                ("disabled", s.disabled.to_string()),
            ],
        ));
    }
    for q in &load.quest_status {
        out.push(line(
            "quest",
            &[
                ("f0", q.quest.to_string()),
                ("f1", q.status.to_string()),
                ("f2", q.rewarded.to_string()),
                ("f3", q.explored.to_string()),
                ("f4", q.timer.to_string()),
                ("f5", q.mob_count[0].to_string()),
                ("f6", q.mob_count[1].to_string()),
                ("f7", q.mob_count[2].to_string()),
                ("f8", q.mob_count[3].to_string()),
                ("f9", q.item_count[0].to_string()),
                ("f10", q.item_count[1].to_string()),
                ("f11", q.item_count[2].to_string()),
                ("f12", q.item_count[3].to_string()),
                ("f13", q.reward_choice.to_string()),
            ],
        ));
    }
    for h in &load.honor_cp {
        out.push(line(
            "honorcp",
            &[
                ("vt", h.victim_type.to_string()),
                ("vid", h.victim_id.to_string()),
                ("cp", fbits(h.cp)),
                ("date", h.date.to_string()),
                ("type", h.kind.to_string()),
            ],
        ));
    }
    for rep in &load.reputation {
        out.push(line(
            "rep",
            &[
                ("faction", rep.faction.to_string()),
                ("standing", rep.standing.to_string()),
                ("flags", rep.flags.to_string()),
            ],
        ));
    }
}

fn dump_items(load: &CharacterLoad, out: &mut Vec<String>) {
    for inv in &load.inventory {
        let mut fields = item_fields(&inv.item);
        fields.push(("bag", inv.bag.to_string()));
        fields.push(("slot", inv.slot.to_string()));
        out.push(line("inv", &fields));
    }
    for l in &load.item_loot {
        out.push(line(
            "loot",
            &[
                ("guid", l.guid.to_string()),
                ("item", l.item_id.to_string()),
                ("amount", l.amount.to_string()),
                ("prop", l.property.to_string()),
            ],
        ));
    }
    for a in &load.actions {
        out.push(line(
            "action",
            &[
                ("button", a.button.to_string()),
                ("action", a.action.to_string()),
                ("type", a.kind.to_string()),
            ],
        ));
    }
    for s in &load.social {
        out.push(line(
            "social",
            &[
                ("friend", s.friend.to_string()),
                ("flags", s.flags.to_string()),
            ],
        ));
    }
    if let Some(h) = &load.homebind {
        out.push(line(
            "home",
            &[
                ("map", h.map.to_string()),
                ("zone", h.zone.to_string()),
                ("x", fbits(h.position_x)),
                ("y", fbits(h.position_y)),
                ("z", fbits(h.position_z)),
            ],
        ));
    }
    for c in &load.spell_cooldowns {
        out.push(line(
            "cd",
            &[
                ("f0", c.spell.to_string()),
                ("f1", c.spell_expire_time.to_string()),
                ("f2", c.category.to_string()),
                ("f3", c.category_expire_time.to_string()),
                ("f4", c.item_id.to_string()),
            ],
        ));
    }
}

fn dump_social(load: &CharacterLoad, out: &mut Vec<String>) {
    if let Some(g) = &load.guild {
        out.push(line(
            "guild",
            &[("id", g.guild_id.to_string()), ("rank", g.rank.to_string())],
        ));
    }
    if let Some(b) = &load.bg_data {
        out.push(line(
            "bg",
            &[
                ("inst", b.instance_id.to_string()),
                ("team", b.team.to_string()),
                ("x", fbits(b.join_x)),
                ("y", fbits(b.join_y)),
                ("z", fbits(b.join_z)),
                ("o", fbits(b.join_o)),
                ("map", b.join_map.to_string()),
            ],
        ));
    }
    for a in &load.account_data {
        out.push(line(
            "acctdata",
            &[
                ("type", a.data_type.to_string()),
                ("time", a.timestamp.to_string()),
                ("data", esc(&a.data)),
            ],
        ));
    }
    for s in &load.skills {
        out.push(line(
            "skill",
            &[
                ("skill", s.skill.to_string()),
                ("value", s.value.to_string()),
                ("max", s.max.to_string()),
            ],
        ));
    }
    for m in &load.mails {
        out.push(line(
            "mail",
            &[
                ("f0", m.id.to_string()),
                ("f1", m.message_type.to_string()),
                ("f2", m.sender_guid.to_string()),
                ("f3", m.receiver_guid.to_string()),
                ("f4", esc(&m.subject)),
                ("f5", m.item_text_id.to_string()),
                ("f6", m.expire_time.to_string()),
                ("f7", m.deliver_time.to_string()),
                ("f8", m.money.to_string()),
                ("f9", m.cod.to_string()),
                ("f10", m.checked.to_string()),
                ("f11", m.stationery.to_string()),
                ("f12", m.mail_template_id.to_string()),
                ("f13", m.has_items.to_string()),
            ],
        ));
    }
    for mi in &load.mailed_items {
        let mut fields = item_fields(&mi.item);
        fields.push(("mail_id", mi.mail_id.to_string()));
        out.push(line("mailitem", &fields));
    }
    for s in &load.forgotten_skills {
        out.push(line(
            "forgotten",
            &[
                ("skill", s.skill.to_string()),
                ("value", s.value.to_string()),
            ],
        ));
    }
}

#[test]
#[ignore = "requires bench characters DB and CHARACTER_LOAD_ORACLE"]
fn character_load_matches_oracle() {
    let url =
        std::env::var("TEST_MYSQL_URL_CHARACTERS").expect("TEST_MYSQL_URL_CHARACTERS must be set");
    let oracle_path =
        std::env::var("CHARACTER_LOAD_ORACLE").expect("CHARACTER_LOAD_ORACLE must be set");
    let guid: u32 = std::env::var("CHARACTER_LOAD_GUID")
        .expect("CHARACTER_LOAD_GUID must be set")
        .parse()
        .expect("guid");

    let expected = std::fs::read_to_string(oracle_path).expect("read oracle");
    let expected: Vec<&str> = expected.lines().collect();

    let db = CharacterDb::spawn(&url).expect("spawn");
    let load = db
        .load_character(guid)
        .expect("load")
        .expect("character exists");
    let actual = dump(&load);
    let actual: Vec<&str> = actual.iter().map(String::as_str).collect();

    assert_eq!(
        actual.len(),
        expected.len(),
        "line count differs\nactual sections: {:?}\nexpected sections: {:?}",
        actual
            .iter()
            .map(|l| l.split('|').next().unwrap_or(""))
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|l| l.split('|').next().unwrap_or(""))
            .collect::<Vec<_>>()
    );
    for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(a, e, "line {i} differs");
    }
}
