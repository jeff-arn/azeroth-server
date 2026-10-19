//! GM command table persistence inputs: the world-DB
//! `command` overrides (`getCommandTable` load) and
//! the login-DB RBAC rows (`LoadRbacPermissions`).
//! Validation and application live with the command
//! table in `mangosd` (the fork logs bad rows while
//! applying them there too).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// One `command` table row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOverrideRow {
    pub name: String,
    pub security: u8,
    pub help: String,
    pub flags: u8,
}

/// Raw RBAC rows from the login database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RbacRows {
    pub permissions: Vec<(u32, String)>,
    pub account_permissions: Vec<(u32, u32, bool)>,
    pub command_permissions: Vec<(String, u32)>,
}

/// `SELECT name, security, help, flags FROM command`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_command_overrides(conn: &mut mysql::Conn) -> Result<Vec<CommandOverrideRow>, DbError> {
    let rows: Vec<mysql::Row> =
        conn.query("SELECT `name`, `security`, `help`, `flags` FROM `command`")?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let f = Fields::from_row(row);
            CommandOverrideRow {
                name: f.string(0),
                security: f.u8(1),
                help: f.string(2),
                flags: f.u8(3),
            }
        })
        .collect())
}

/// The three `LoadRbacPermissions` queries.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_rbac_rows(conn: &mut mysql::Conn) -> Result<RbacRows, DbError> {
    let permission_rows: Vec<mysql::Row> =
        conn.query("SELECT `id`, `name` FROM `rbac_permissions`")?;
    let account_rows: Vec<mysql::Row> = conn
        .query("SELECT `account_id`, `permission_id`, `granted` FROM `rbac_account_permissions`")?;
    let command_rows: Vec<mysql::Row> =
        conn.query("SELECT `command`, `permission_id` FROM `rbac_command_permissions`")?;
    Ok(RbacRows {
        permissions: permission_rows
            .into_iter()
            .map(|row| {
                let f = Fields::from_row(row);
                (f.u32(0), f.string(1))
            })
            .collect(),
        account_permissions: account_rows
            .into_iter()
            .map(|row| {
                let f = Fields::from_row(row);
                (f.u32(0), f.u32(1), f.u8(2) != 0)
            })
            .collect(),
        command_permissions: command_rows
            .into_iter()
            .map(|row| {
                let f = Fields::from_row(row);
                (f.string(0), f.u32(1))
            })
            .collect(),
    })
}
