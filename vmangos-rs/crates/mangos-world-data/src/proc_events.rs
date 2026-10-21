//! `spell_proc_event` loader (`SpellMgr::LoadSpellProcEvents`)
//! with higher-rank fill from `spell_chain` (the DB-table
//! arm of the fork's chain sources; Talent/DBC-derived
//! chains are not modeled).

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_db::row::Fields;
use mangos_spells::entry::ProcEventEntry;
use mysql::prelude::Queryable;

/// Loads all `spell_proc_event` rows visible at
/// `supported_build`, then copies each row to higher
/// ranks of its `spell_chain` chain that lack their
/// own row (`SpellRankHelper::FillHigherRanks`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_spell_proc_events(
    conn: &mut mysql::Conn,
    supported_build: u32,
) -> Result<HashMap<u32, ProcEventEntry>, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `entry`, `SchoolMask`, `SpellFamilyName`, `SpellFamilyMask0`, \
         `SpellFamilyMask1`, `SpellFamilyMask2`, `procFlags`, `procEx`, `ppmRate`, \
         `CustomChance`, `Cooldown` FROM `spell_proc_event` \
         WHERE ? BETWEEN `build_min` AND `build_max`",
        (supported_build,),
    )?;
    let mut events: HashMap<u32, ProcEventEntry> = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = Fields::from_row(row);
        events.insert(
            f.u32(0),
            ProcEventEntry {
                school_mask: f.u32(1),
                spell_family_name: f.u32(2),
                spell_family_mask: [f.u64(3), f.u64(4), f.u64(5)],
                proc_flags: f.u32(6),
                proc_ex: f.u32(7),
                ppm_rate: f.f32(8),
                custom_chance: f.f32(9),
                cooldown: f.u32(10),
            },
        );
    }

    // Rank fill: spell_id -> (first_spell, rank).
    let chain_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `spell_id`, `first_spell`, `rank` FROM `spell_chain` \
         WHERE ? BETWEEN `build_min` AND `build_max`",
        (supported_build,),
    )?;
    let mut chains: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    let mut rank_of: HashMap<u32, (u32, u32)> = HashMap::new();
    for row in chain_rows {
        let f = Fields::from_row(row);
        let (spell, first, rank) = (f.u32(0), f.u32(1), f.u32(2));
        chains.entry(first).or_default().push((rank, spell));
        rank_of.insert(spell, (first, rank));
    }
    let explicit: Vec<u32> = events.keys().copied().collect();
    for spell in explicit {
        let Some(&(first, rank)) = rank_of.get(&spell) else {
            continue;
        };
        let Some(members) = chains.get(&first) else {
            continue;
        };
        let entry = events[&spell];
        for &(member_rank, member) in members {
            if member_rank > rank {
                events.entry(member).or_insert(entry);
            }
        }
    }
    Ok(events)
}
