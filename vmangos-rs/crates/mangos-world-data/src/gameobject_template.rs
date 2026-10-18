//! `gameobject_template` → [`GameObjectInfo`], ported from
//! `ObjectMgr::LoadGameObjectTemplates` / `LoadGameObjectInfo`
//! / `CheckGameObjectTemplate` (`ObjectMgr.cpp:7703-7988`) and
//! `GameObjectDefines.h:25-71,211-…`, with C++ max-patch
//! selection.
//!
//! The 24 `dataN` words are kept as the raw `[i32; 24]` the C++
//! union stores; typed per-type views land with the `GameObject`
//! entity (Slice 3), which is also when most fields gain
//! consumers. The check switch below encodes the same
//! (type, index) pairs the C++ union members name.

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

use crate::load_report::LoadReport;

/// `GAMEOBJECT_TYPE_DOOR`.
pub const TYPE_DOOR: u32 = 0;
/// `GAMEOBJECT_TYPE_BUTTON`.
pub const TYPE_BUTTON: u32 = 1;
/// `GAMEOBJECT_TYPE_QUESTGIVER`.
pub const TYPE_QUESTGIVER: u32 = 2;
/// `GAMEOBJECT_TYPE_CHEST`.
pub const TYPE_CHEST: u32 = 3;
/// `GAMEOBJECT_TYPE_GENERIC`.
pub const TYPE_GENERIC: u32 = 5;
/// `GAMEOBJECT_TYPE_TRAP`.
pub const TYPE_TRAP: u32 = 6;
/// `GAMEOBJECT_TYPE_CHAIR`.
pub const TYPE_CHAIR: u32 = 7;
/// `GAMEOBJECT_TYPE_SPELL_FOCUS`.
pub const TYPE_SPELL_FOCUS: u32 = 8;
/// `GAMEOBJECT_TYPE_GOOBER`.
pub const TYPE_GOOBER: u32 = 10;
/// `GAMEOBJECT_TYPE_TRANSPORT`.
pub const TYPE_TRANSPORT: u32 = 11;
/// `GAMEOBJECT_TYPE_AREADAMAGE`.
pub const TYPE_AREADAMAGE: u32 = 12;
/// `GAMEOBJECT_TYPE_CAMERA`.
pub const TYPE_CAMERA: u32 = 13;
/// `GAMEOBJECT_TYPE_MO_TRANSPORT`.
pub const TYPE_MO_TRANSPORT: u32 = 15;
/// `GAMEOBJECT_TYPE_SPELLCASTER`.
pub const TYPE_SPELLCASTER: u32 = 22;
/// `GAMEOBJECT_TYPE_FLAGSTAND` (build > 1.5.1).
pub const TYPE_FLAGSTAND: u32 = 24;
/// `GAMEOBJECT_TYPE_FISHINGHOLE` (build > 1.6.1).
pub const TYPE_FISHINGHOLE: u32 = 25;
/// `GAMEOBJECT_TYPE_FLAGDROP` (build > 1.7.1).
pub const TYPE_FLAGDROP: u32 = 26;
/// `GAMEOBJECT_TYPE_MAX` at build 5875 (also
/// `MAX_GAMEOBJECT_TYPE`, GameObjectDefines.h:68-71).
pub const TYPE_MAX: u32 = 31;

/// `struct GameObjectInfo` (`GameObjectDefines.h:211-…`).
#[derive(Clone, Debug, Default)]
pub struct GameObjectInfo {
    pub id: u32,
    /// `type` column (`enum GameobjectTypes`).
    pub go_type: u32,
    pub display_id: u32,
    pub name: String,
    pub icon: String,
    pub faction: u32,
    pub flags: u32,
    pub size: f32,
    /// Raw union words `data0..data23` (signed in the DB read).
    pub data: [i32; 24],
    pub min_money_loot: u32,
    pub max_money_loot: u32,
    /// Raw `script_name`; `script_id` resolves with the script
    /// registry (same note as creature templates).
    pub script_name: String,
    pub script_id: u32,
}

impl GameObjectInfo {
    /// Union word as unsigned (the C++ union members are `uint32`).
    #[must_use]
    pub fn data_u32(&self, index: usize) -> u32 {
        u32::from_ne_bytes(self.data[index].to_ne_bytes())
    }

    #[must_use]
    pub fn is_transport(&self) -> bool {
        self.go_type == TYPE_TRANSPORT || self.go_type == TYPE_MO_TRANSPORT
    }

    /// `IsDespawnAtAction` (`GameObjectDefines.h:537-547`):
    /// chest/goober `consumable` word (data3).
    #[must_use]
    pub fn is_despawn_at_action(&self) -> bool {
        matches!(self.go_type, TYPE_CHEST | TYPE_GOOBER) && self.data_u32(3) != 0
    }
}

/// DBC/store sets for the `CheckGameObjectTemplate` arms.
/// `None` = check skipped (reported once).
#[derive(Clone, Copy, Debug, Default)]
pub struct GameObjectValidation<'a> {
    pub lock_ids: Option<&'a HashSet<u32>>,
    pub spell_ids: Option<&'a HashSet<u32>>,
    pub spell_focus_ids: Option<&'a HashSet<u32>>,
    pub page_text_ids: Option<&'a HashSet<u32>>,
    pub taxi_path_ids: Option<&'a HashSet<u32>>,
}

/// Loaded gameobject templates by entry (`m_gameObjectInfoMap`).
#[derive(Clone, Debug, Default)]
pub struct GameObjectTemplates {
    by_entry: HashMap<u32, GameObjectInfo>,
}

impl GameObjectTemplates {
    /// `sObjectMgr.GetGameObjectTemplate(entry)`.
    #[must_use]
    pub fn get(&self, entry: u32) -> Option<&GameObjectInfo> {
        self.by_entry.get(&entry)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &GameObjectInfo)> {
        self.by_entry.iter()
    }

    /// `GetTransportDisplayIds` (`ObjectMgr.cpp:7990-8012`),
    /// including the two hardcoded removals (no pathable model).
    #[must_use]
    pub fn transport_display_ids(&self) -> HashSet<u32> {
        let mut set: HashSet<u32> = self
            .by_entry
            .values()
            .filter(|info| info.is_transport())
            .map(|info| info.display_id)
            .collect();
        set.remove(&462);
        set.remove(&562);
        set
    }
}

#[derive(Clone, Copy)]
enum GoCheck {
    Lock(usize),
    NoDamageImmune(usize),
    Consumable(usize),
    LinkedTrap(usize),
    SpellId(usize),
    SpellFocusId,
    PageId(usize),
    TaxiPathId,
    ChairHeight(usize),
}

/// The per-type check table from the `CheckGameObjectTemplate`
/// switch (`ObjectMgr.cpp:7842-7987`, 5875 shape). Indices are
/// the union's `dataN` slots.
fn checks_for(go_type: u32) -> &'static [GoCheck] {
    use GoCheck::{
        ChairHeight, Consumable, LinkedTrap, Lock, NoDamageImmune, PageId, SpellFocusId, SpellId,
        TaxiPathId,
    };
    match go_type {
        TYPE_DOOR => &[Lock(1), NoDamageImmune(3)],
        TYPE_BUTTON => &[Lock(1), LinkedTrap(3), NoDamageImmune(4)],
        TYPE_QUESTGIVER | TYPE_FLAGSTAND => &[Lock(0), NoDamageImmune(5)],
        TYPE_CHEST => &[Lock(0), Consumable(3), LinkedTrap(7)],
        TYPE_TRAP | TYPE_AREADAMAGE | TYPE_CAMERA => &[Lock(0)],
        TYPE_CHAIR => &[ChairHeight(1)],
        TYPE_SPELL_FOCUS => &[SpellFocusId, LinkedTrap(2)],
        TYPE_GOOBER => &[
            Lock(0),
            Consumable(3),
            PageId(7),
            NoDamageImmune(11),
            LinkedTrap(12),
        ],
        TYPE_MO_TRANSPORT => &[TaxiPathId],
        TYPE_SPELLCASTER => &[SpellId(0)],
        TYPE_FISHINGHOLE => &[Lock(4)],
        TYPE_FLAGDROP => &[Lock(0), NoDamageImmune(3)],
        _ => &[],
    }
}

#[allow(clippy::too_many_lines)] // single dispatch over the C++ check helpers, kept in switch order
fn run_go_checks(
    info: &mut GameObjectInfo,
    templates: &HashMap<u32, GameObjectInfo>,
    v: &GameObjectValidation<'_>,
    report: &mut LoadReport,
) {
    let (id, go_type) = (info.id, info.go_type);
    for check in checks_for(info.go_type) {
        match *check {
            GoCheck::Lock(n) => {
                let value = info.data_u32(n);
                if value != 0 {
                    if let Some(set) = v.lock_ids {
                        if !set.contains(&value) {
                            report.error(format!(
                                "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but lock (Id: {value}) not found."
                            ));
                        }
                    }
                }
            }
            GoCheck::NoDamageImmune(n) => {
                // Called unconditionally in C++ (0/1 passes).
                let value = info.data_u32(n);
                if value > 1 {
                    report.error(format!(
                        "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but expected boolean (0/1) noDamageImmune field value."
                    ));
                }
            }
            GoCheck::Consumable(n) => {
                let value = info.data_u32(n);
                if value > 1 {
                    report.error(format!(
                        "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but expected boolean (0/1) consumable field value."
                    ));
                }
            }
            GoCheck::LinkedTrap(n) => {
                let value = info.data_u32(n);
                if value != 0 {
                    match templates.get(&value) {
                        Some(trap) if trap.go_type != TYPE_TRAP => report.error(format!(
                            "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but GO (Entry {value}) have not GAMEOBJECT_TYPE_TRAP ({TYPE_TRAP}) type."
                        )),
                        Some(_) => {}
                        None => report.error(format!(
                            "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but trap GO (Entry {value}) not exist in `gameobject_template`."
                        )),
                    }
                }
            }
            GoCheck::SpellId(n) => {
                let value = info.data_u32(n);
                if let Some(set) = v.spell_ids {
                    // C++ calls unconditionally for spellcaster:
                    // a 0 spell id is also "not exist".
                    if !set.contains(&value) {
                        report.error(format!(
                            "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={value} but Spell (Entry {value}) not exist."
                        ));
                    }
                }
            }
            GoCheck::SpellFocusId => {
                let value = info.data_u32(0);
                if value != 0 {
                    if let Some(set) = v.spell_focus_ids {
                        if !set.contains(&value) {
                            report.error(format!(
                                "Gameobject (Entry: {id} GoType: {go_type}) have data0={value} but SpellFocus (Id: {value}) not exist."
                            ));
                        }
                    }
                }
            }
            GoCheck::PageId(n) => {
                let value = info.data_u32(n);
                if value != 0 {
                    if let Some(set) = v.page_text_ids {
                        if !set.contains(&value) {
                            report.error(format!(
                                "Gameobject (Entry: {id} GoType: {go_type}) have data7={value} but PageText (Entry {value}) not exist."
                            ));
                        }
                    }
                }
            }
            GoCheck::TaxiPathId => {
                let value = info.data_u32(0);
                if value != 0 {
                    if let Some(set) = v.taxi_path_ids {
                        if !set.contains(&value) {
                            report.error(format!(
                                "Gameobject (Entry: {id} GoType: {go_type}) have data0={value} but TaxiPath (Id: {value}) not exist."
                            ));
                        }
                    }
                }
            }
            GoCheck::ChairHeight(n) => {
                // SIT_HIGH(6) - SIT_LOW(4) = 2.
                if info.data_u32(n) > 2 {
                    report.error(format!(
                        "Gameobject (Entry: {id} GoType: {go_type}) have data{n}={} but correct chair height in range 0..2.",
                        info.data_u32(n)
                    ));
                    info.data[n] = 0;
                }
            }
        }
    }
}

/// `CheckGameObjectTemplate` (`ObjectMgr.cpp:7827-7988`).
/// `templates` is the fully loaded map (linked-trap lookups).
fn check_gameobject_template(
    info: &mut GameObjectInfo,
    templates: &HashMap<u32, GameObjectInfo>,
    v: &GameObjectValidation<'_>,
    report: &mut LoadReport,
) {
    let (id, go_type) = (info.id, info.go_type);
    if info.size <= 0.0 {
        report.error(format!(
            "Gameobject (Entry: {id} GoType: {go_type}) have too small size={}",
            info.size
        ));
        info.size = 1.0; // DEFAULT_OBJECT_SCALE
    }
    if info.go_type >= TYPE_MAX {
        report.error(format!(
            "Gameobject (Entry: {id}) have invalid type={go_type} in template, forcing it to type GENERIC (5) instead."
        ));
        info.go_type = TYPE_GENERIC;
        info.data = [0; 24];
    }

    run_go_checks(info, templates, v, report);
}

const SELECT_COLUMNS: &str = "`entry`, `type`, `displayId`, `name`, `icon`, `faction`, `flags`, \
    `size`, `data0`, `data1`, `data2`, `data3`, `data4`, `data5`, `data6`, `data7`, `data8`, \
    `data9`, `data10`, `data11`, `data12`, `data13`, `data14`, `data15`, `data16`, `data17`, \
    `data18`, `data19`, `data20`, `data21`, `data22`, `data23`, `mingold`, `maxgold`, \
    `script_name`";

/// `LoadGameObjectInfo` (`ObjectMgr.cpp:7805-7825`).
fn load_gameobject_info(f: &mangos_db::row::Fields) -> GameObjectInfo {
    let mut data = [0i32; 24];
    for (i, slot) in data.iter_mut().enumerate() {
        *slot = f.i32(8 + i);
    }
    GameObjectInfo {
        id: f.u32(0),
        go_type: f.u32(1),
        display_id: f.u32(2),
        name: f.string(3),
        icon: f.string(4),
        faction: f.u32(5),
        flags: f.u32(6),
        size: f.f32(7),
        data,
        min_money_loot: f.u32(32),
        max_money_loot: f.u32(33),
        script_name: f.string(34),
        script_id: 0,
    }
}

fn note_skipped_checks(v: &GameObjectValidation<'_>, report: &mut LoadReport) {
    let skipped: &[(&str, bool)] = &[
        ("lock (DBC)", v.lock_ids.is_none()),
        ("spell existence", v.spell_ids.is_none()),
        ("spell focus (DBC)", v.spell_focus_ids.is_none()),
        ("page text", v.page_text_ids.is_none()),
        ("taxi path", v.taxi_path_ids.is_none()),
    ];
    for (name, is_skipped) in skipped {
        if *is_skipped {
            report.error(format!(
                "gameobject_template load: {name} validation skipped (no id set supplied)"
            ));
        }
    }
}

/// `LoadGameObjectTemplates` (`ObjectMgr.cpp:7760-7781`).
/// Checks run after the full load, as in C++ (linked-trap
/// lookups need the whole map).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_gameobject_templates(
    conn: &mut mysql::Conn,
    wow_patch: u32,
    validation: &GameObjectValidation<'_>,
) -> Result<(GameObjectTemplates, LoadReport), DbError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM `gameobject_template` t1 \
         WHERE `patch` = (SELECT max(`patch`) FROM `gameobject_template` t2 \
                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, (wow_patch,))?;
    let mut by_entry = HashMap::with_capacity(rows.len());
    for row in rows {
        let info = load_gameobject_info(&mangos_db::row::Fields::from_row(row));
        by_entry.insert(info.id, info);
    }
    let mut report = LoadReport::default();
    note_skipped_checks(validation, &mut report);
    // Check pass needs the whole map for trap lookups: work on a
    // snapshot of entries, checking cloned rows back into place
    // only where fix-ups apply (size/type/chair height).
    let entries: Vec<u32> = by_entry.keys().copied().collect();
    for entry in entries {
        let mut info = by_entry[&entry].clone();
        check_gameobject_template(&mut info, &by_entry, validation, &mut report);
        by_entry.insert(entry, info);
    }
    Ok((GameObjectTemplates { by_entry }, report))
}

/// `LoadGameObjectTemplate(entry)` (`ObjectMgr.cpp:7783-7803`).
/// Linked-trap checks see only this row (C++ sees the live map;
/// single-entry reload parity needs the loaded store, which the
/// caller does not have here — noted in the report).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_gameobject_template(
    conn: &mut mysql::Conn,
    entry: u32,
    wow_patch: u32,
    validation: &GameObjectValidation<'_>,
) -> Result<(Option<GameObjectInfo>, LoadReport), DbError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM `gameobject_template` t1 \
         WHERE `entry` = ? AND `patch` = (SELECT max(`patch`) FROM `gameobject_template` t2 \
                                          WHERE t1.`entry` = t2.`entry` AND `patch` <= ?)"
    );
    let rows: Vec<mysql::Row> = conn.exec(sql, (entry, wow_patch))?;
    let mut report = LoadReport::default();
    note_skipped_checks(validation, &mut report);
    let mut info = rows
        .into_iter()
        .next()
        .map(|row| load_gameobject_info(&mangos_db::row::Fields::from_row(row)));
    if let Some(info) = info.as_mut() {
        report.error(
            "gameobject_template single load: linked-trap checks see only this row".to_string(),
        );
        let mut single = HashMap::new();
        single.insert(info.id, info.clone());
        check_gameobject_template(info, &single, validation, &mut report);
    }
    Ok((info, report))
}
