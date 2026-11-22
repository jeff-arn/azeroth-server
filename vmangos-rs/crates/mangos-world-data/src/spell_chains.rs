//! `spell_chain` DB overlay rows (`SpellMgr::LoadSpellChains`
//! custom-case pass, `SpellMgr.cpp:1533-1670`): explicit
//! rank/prev/first/req nodes layered over the
//! `SkillLineAbility`-derived chains. The fork's per-row
//! consistency diagnostics are load-log only; the map
//! writes are unconditional overwrites.

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_spells::entry::SpellChainNode;
use mysql::prelude::Queryable;

/// # Errors
///
/// Returns the query error.
pub fn load_spell_chain_rows(
    conn: &mut mysql::Conn,
    client_build: u32,
) -> Result<HashMap<u32, SpellChainNode>, DbError> {
    let rows: Vec<(u32, u32, u32, u32, u32)> = conn.query_map(
        format!(
            "SELECT spell_id, prev_spell, first_spell, `rank`, req_spell \
             FROM spell_chain WHERE {client_build} BETWEEN build_min AND build_max"
        ),
        |(spell_id, prev, first, rank, req)| (spell_id, prev, first, rank, req),
    )?;
    Ok(rows
        .into_iter()
        .map(|(spell_id, prev, first, rank, req)| {
            (
                spell_id,
                SpellChainNode {
                    prev,
                    first,
                    rank,
                    req,
                },
            )
        })
        .collect())
}
