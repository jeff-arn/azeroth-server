//! `spell_cone` loader (`SpellMgr::LoadSpellCones`):
//! per-spell cone arc in degrees, converted to
//! radians at load (negative values mean a back
//! cone, as the fork's notifier interprets them).

use std::collections::HashMap;

use mangos_db::DbError;
use mangos_db::row::Fields;
use mysql::prelude::Queryable;

/// Loads all `spell_cone` rows.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_spell_cones(conn: &mut mysql::Conn) -> Result<HashMap<u32, f32>, DbError> {
    let rows: Vec<mysql::Row> =
        conn.exec("SELECT `entry`, `cone_degrees` FROM `spell_cone`", ())?;
    let mut cones: HashMap<u32, f32> = HashMap::with_capacity(rows.len());
    for row in rows {
        let f = Fields::from_row(row);
        #[allow(clippy::cast_possible_wrap)]
        let degrees = f.u32(1) as i32;
        if !(-360..=360).contains(&degrees) {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let angle = degrees as f32 * std::f32::consts::PI / 180.0;
        cones.insert(f.u32(0), angle);
    }
    Ok(cones)
}
