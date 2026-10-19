//! Quest template subset for character load/save parity
//! (`ObjectMgr::LoadQuests` derivations): the required-item
//! objectives and the time limit are all the Phase 6
//! load/save path reads. `QUEST_SPECIAL_FLAG_DELIVER` is
//! computed when any `ReqItemId` is set
//! (`ObjectMgr.cpp:5602`), `QUEST_SPECIAL_FLAG_TIMED`
//! when `LimitTime` is nonzero (`ObjectMgr.cpp:5953`).

use std::collections::HashMap;

use mangos_db::DbError;
use mysql::prelude::Queryable;

/// One `quest_template` row, load/save-relevant fields only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestTemplateLite {
    pub req_item_ids: [u32; 4],
    pub req_item_counts: [u32; 4],
    pub limit_time: u32,
}

impl QuestTemplateLite {
    /// `QUEST_SPECIAL_FLAG_DELIVER` (computed when any
    /// `ReqItemId` is nonzero).
    #[must_use]
    pub fn is_deliver(&self) -> bool {
        self.req_item_ids.iter().any(|id| *id != 0)
    }

    /// `QUEST_SPECIAL_FLAG_TIMED` (computed when
    /// `LimitTime` is nonzero).
    #[must_use]
    pub fn is_timed(&self) -> bool {
        self.limit_time != 0
    }
}

/// `quest_template` subset by entry.
#[derive(Clone, Debug, Default)]
pub struct QuestTemplatesLite {
    by_entry: HashMap<u32, QuestTemplateLite>,
}

impl QuestTemplatesLite {
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, QuestTemplateLite)>) -> Self {
        Self {
            by_entry: entries.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    #[must_use]
    pub fn get(&self, quest_id: u32) -> Option<&QuestTemplateLite> {
        self.by_entry.get(&quest_id)
    }
}

/// Loads the `quest_template` columns the load/save path
/// reads. Many world databases carry per-patch duplicate
/// quest rows; callers pass the progression patch exactly
/// as for `map_template`, but `quest_template` has no
/// `patch` column in this schema, so all rows load.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_quest_templates_lite(conn: &mut mysql::Conn) -> Result<QuestTemplatesLite, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `LimitTime`, `ReqItemId1`, `ReqItemId2`, `ReqItemId3`, \
         `ReqItemId4`, `ReqItemCount1`, `ReqItemCount2`, `ReqItemCount3`, \
         `ReqItemCount4` FROM `quest_template`",
        (),
    )?;
    let entries = rows.into_iter().map(|row| {
        let f = mangos_db::row::Fields::from_row(row);
        (
            f.u32(0),
            QuestTemplateLite {
                limit_time: f.u32(1),
                req_item_ids: [f.u32(2), f.u32(3), f.u32(4), f.u32(5)],
                req_item_counts: [f.u32(6), f.u32(7), f.u32(8), f.u32(9)],
            },
        )
    });
    Ok(QuestTemplatesLite::from_entries(entries))
}

/// Loads `areatrigger_tavern` ids (`ObjectMgr::LoadTavernAreaTriggers`):
/// client-reported trigger ids that put the player into
/// `REST_TYPE_IN_TAVERN`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_tavern_triggers(
    conn: &mut mysql::Conn,
) -> Result<std::collections::HashSet<u32>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec("SELECT `id` FROM `areatrigger_tavern`", ())?;
    Ok(rows
        .into_iter()
        .map(|row| mangos_db::row::Fields::from_row(row).u32(0))
        .collect())
}
