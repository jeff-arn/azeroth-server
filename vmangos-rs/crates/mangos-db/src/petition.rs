//! Guild charter petition persistence (`GuildMgr::
//! LoadPetitions` / `Petition` / `PetitionSignature`,
//! `GuildMgr.cpp:162-447`): the `petition` and
//! `petition_sign` statements, the boot load with its
//! orphan-signature cleanup, and the charter item rows
//! (`item_instance` + `character_inventory`) the buy /
//! turn-in handlers create and destroy. The runtime
//! Item-object system is a later phase; until it lands
//! the charter exists only as these rows, which is the
//! same on-disk shape C++ produces because the buy
//! handler saves inventory immediately
//! (`HandlePetitionBuyOpcode` ends with
//! `SaveInventoryAndGoldToDB`).

use mysql::prelude::Queryable;

use crate::DbError;
use crate::row::Fields;

/// `GUILD_CHARTER` item entry (`PetitionsHandler.cpp:38`).
pub const GUILD_CHARTER_ENTRY: u32 = 5863;

/// One `petition` row plus the owner race the C++
/// loader resolves through `GetPlayerTeamByGUID`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredPetition {
    pub petition_id: u32,
    pub owner_low: u32,
    pub charter_low: u32,
    pub name: String,
    /// Owner `characters.race`, 0 when the character
    /// row is gone (C++ then defaults the team).
    pub owner_race: u8,
}

/// One `petition_sign` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredPetitionSign {
    pub petition_id: u32,
    pub player_low: u32,
    pub account_id: u32,
}

/// Boot-loaded petitions (`LoadPetitions` result).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedPetitions {
    pub petitions: Vec<StoredPetition>,
    pub signs: Vec<StoredPetitionSign>,
    /// `ObjectMgr::GeneratePetitionID` seed: one past
    /// the greatest loaded petition id, at least 1.
    pub next_petition_id: u32,
}

/// `GuildMgr::LoadPetitions`: load petitions and their
/// signatures, deleting signature rows whose petition
/// does not exist and repairing rows whose owner does
/// not match the petition owner.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn load_petitions(conn: &mut mysql::Conn) -> Result<LoadedPetitions, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT `petition`.`owner_guid`, `petition`.`petition_guid`, \
         `petition`.`charter_guid`, `petition`.`name`, \
         IFNULL(`characters`.`race`, 0) FROM `petition` \
         LEFT JOIN `characters` ON `characters`.`guid` = `petition`.`owner_guid` \
         ORDER BY `petition`.`petition_guid` ASC",
        (),
    )?;
    let mut petitions: Vec<StoredPetition> = Vec::new();
    let mut max_id = 0u32;
    for row in rows {
        let f = Fields::from_row(row);
        let petition = StoredPetition {
            owner_low: f.u32(0),
            petition_id: f.u32(1),
            charter_low: f.u32(2),
            name: f.string(3),
            owner_race: f.u8(4),
        };
        max_id = max_id.max(petition.petition_id);
        petitions.push(petition);
    }

    let sign_rows: Vec<mysql::Row> = conn.exec(
        "SELECT `owner_guid`, `petition_guid`, `player_guid`, `player_account` \
         FROM `petition_sign`",
        (),
    )?;
    let mut signs: Vec<StoredPetitionSign> = Vec::new();
    let mut orphan_ids: Vec<u32> = Vec::new();
    for row in sign_rows {
        let f = Fields::from_row(row);
        let owner_low = f.u32(0);
        let petition_id = f.u32(1);
        let Some(petition) = petitions.iter().find(|p| p.petition_id == petition_id) else {
            if !orphan_ids.contains(&petition_id) {
                orphan_ids.push(petition_id);
            }
            continue;
        };
        if owner_low != petition.owner_low {
            conn.exec_drop(
                "UPDATE `petition_sign` SET `owner_guid` = ? WHERE `petition_guid` = ?",
                (petition.owner_low, petition_id),
            )
            .map_err(DbError::Driver)?;
        }
        signs.push(StoredPetitionSign {
            petition_id,
            player_low: f.u32(2),
            account_id: f.u32(3),
        });
    }
    for petition_id in orphan_ids {
        conn.exec_drop(
            "DELETE FROM `petition_sign` WHERE `petition_guid` = ?",
            (petition_id,),
        )
        .map_err(DbError::Driver)?;
    }

    Ok(LoadedPetitions {
        petitions,
        signs,
        next_petition_id: max_id.saturating_add(1).max(1),
    })
}

/// `Petition::SaveToDB`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_petition(
    conn: &mut mysql::Conn,
    petition_id: u32,
    owner_low: u32,
    charter_low: u32,
    name: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `petition` (`owner_guid`, `petition_guid`, `charter_guid`, `name`) \
         VALUES (?, ?, ?, ?)",
        (owner_low, petition_id, charter_low, name),
    )
    .map_err(DbError::Driver)
}

/// `PetitionSignature::SaveToDB`.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn insert_petition_sign(
    conn: &mut mysql::Conn,
    owner_low: u32,
    petition_id: u32,
    player_low: u32,
    account_id: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "INSERT INTO `petition_sign` (`owner_guid`, `petition_guid`, `player_guid`, \
         `player_account`) VALUES (?, ?, ?, ?)",
        (owner_low, petition_id, player_low, account_id),
    )
    .map_err(DbError::Driver)
}

/// `Petition::Rename` statement.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn rename_petition(
    conn: &mut mysql::Conn,
    petition_id: u32,
    name: &str,
) -> Result<(), DbError> {
    conn.exec_drop(
        "UPDATE `petition` SET `name` = ? WHERE `petition_guid` = ?",
        (name, petition_id),
    )
    .map_err(DbError::Driver)
}

/// `Player::RemovePetitionsAndSigns` statements.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn purge_player_petitions(conn: &mut mysql::Conn, player_low: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `petition` WHERE `owner_guid` = ?",
        (player_low,),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `petition_sign` WHERE (`owner_guid` = ?) || (`player_guid` = ?)",
        (player_low, player_low),
    )
    .map_err(DbError::Driver)
}

/// `Petition::Delete` statements.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_petition(conn: &mut mysql::Conn, petition_id: u32) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `petition` WHERE `petition_guid` = ?",
        (petition_id,),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "DELETE FROM `petition_sign` WHERE `petition_guid` = ?",
        (petition_id,),
    )
    .map_err(DbError::Driver)
}

/// `ObjectMgr::IsReservedName` point check against the
/// world database. C++ lowercases both sides with a
/// wide-string fold; the column collation plus a
/// `LOWER()` compare reproduces it for the ASCII names
/// the table ships with.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn is_reserved_name(conn: &mut mysql::Conn, name: &str) -> Result<bool, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT 1 FROM `reserved_name` WHERE LOWER(`name`) = LOWER(?) LIMIT 1",
        (name,),
    )?;
    Ok(!rows.is_empty())
}

/// `INVENTORY_SLOT_ITEM_START` (`Player.h`): first
/// backpack slot in bag 0.
const BACKPACK_SLOT_START: u32 = 23;
/// `INVENTORY_SLOT_ITEM_END`: one past the last
/// backpack slot.
const BACKPACK_SLOT_END: u32 = 39;

/// First free backpack slot in bag 0, or `None` when
/// full (the `CanStoreNewItem(NULL_BAG, NULL_SLOT)`
/// backpack scan). Shared with the mail item stores.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn first_free_backpack_slot(
    conn: &mut mysql::Conn,
    owner_low: u32,
) -> Result<Option<u32>, DbError> {
    let taken: Vec<mysql::Row> = conn.exec(
        "SELECT `slot` FROM `character_inventory` WHERE `guid` = ? AND `bag` = 0 \
         AND `slot` >= ? AND `slot` < ?",
        (owner_low, BACKPACK_SLOT_START, BACKPACK_SLOT_END),
    )?;
    let mut used = [false; (BACKPACK_SLOT_END - BACKPACK_SLOT_START) as usize];
    for row in taken {
        let f = Fields::from_row(row);
        let slot = f.u32(0);
        if (BACKPACK_SLOT_START..BACKPACK_SLOT_END).contains(&slot) {
            used[(slot - BACKPACK_SLOT_START) as usize] = true;
        }
    }
    Ok(used
        .iter()
        .position(|slot| !slot)
        .map(|free| BACKPACK_SLOT_START + free as u32))
}

/// Next free `item_instance` guid low
/// (`ObjectMgr::GenerateItemGuid` is seeded past
/// `MAX(guid)`; the world actor serializes callers).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn next_item_guid(conn: &mut mysql::Conn) -> Result<u32, DbError> {
    let max_rows: Vec<mysql::Row> =
        conn.exec("SELECT IFNULL(MAX(`guid`), 0) FROM `item_instance`", ())?;
    let max_guid = max_rows
        .into_iter()
        .next()
        .map_or(0, |row| Fields::from_row(row).u32(0));
    Ok(max_guid.saturating_add(1).max(1))
}

/// The `item_instance.enchantments` text for a fresh
/// charter: enchantment slot 0 carries the petition
/// id (`ITEM_FIELD_ENCHANTMENT`), the remaining six
/// slots are zero. 7 slots x 3 fields (`Item::SaveToDB`).
fn charter_enchantments(petition_id: u32) -> String {
    let mut text = petition_id.to_string();
    for _ in 0..20 {
        text.push_str(" 0");
    }
    text
}

/// Create the charter item rows for a petition buy:
/// pick the first free backpack slot (`CanStoreNewItem`
/// with `NULL_BAG`/`NULL_SLOT` finds the backpack
/// first), allocate the next item guid
/// (`ObjectMgr::GenerateItemGuid` is seeded past
/// `MAX(guid)`; the world actor serializes buys), and
/// insert `item_instance` + `character_inventory`.
/// Returns the new item guid low, or `None` when the
/// backpack is full.
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn create_charter_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    petition_id: u32,
) -> Result<Option<u32>, DbError> {
    let Some(slot) = first_free_backpack_slot(conn, owner_low)? else {
        return Ok(None);
    };
    let item_low = next_item_guid(conn)?;

    conn.exec_drop(
        "INSERT INTO `item_instance` (`guid`, `item_id`, `owner_guid`, `creator_guid`, \
         `gift_creator_guid`, `count`, `duration`, `charges`, `flags`, `enchantments`, \
         `random_property_id`, `durability`, `text`, `generated_loot`) \
         VALUES (?, ?, ?, 0, 0, 1, 0, '0 0 0 0 0 ', 0, ?, 0, 0, 0, 0)",
        (
            item_low,
            GUILD_CHARTER_ENTRY,
            owner_low,
            charter_enchantments(petition_id),
        ),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop(
        "INSERT INTO `character_inventory` (`guid`, `bag`, `slot`, `item_guid`, `item_id`) \
         VALUES (?, 0, ?, ?, ?)",
        (owner_low, slot, item_low, GUILD_CHARTER_ENTRY),
    )
    .map_err(DbError::Driver)?;
    Ok(Some(item_low))
}

/// Whether the character currently carries the item
/// (`Player::GetItemByGuid` over inventory rows).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn player_has_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
) -> Result<bool, DbError> {
    let rows: Vec<mysql::Row> = conn.exec(
        "SELECT 1 FROM `character_inventory` WHERE `guid` = ? AND `item_guid` = ? LIMIT 1",
        (owner_low, item_low),
    )?;
    Ok(!rows.is_empty())
}

/// Destroy the charter item rows (`Player::DestroyItem`
/// plus the item's `DeleteFromDB`).
///
/// # Errors
///
/// Returns [`DbError`] on driver failure.
pub fn delete_charter_item(
    conn: &mut mysql::Conn,
    owner_low: u32,
    item_low: u32,
) -> Result<(), DbError> {
    conn.exec_drop(
        "DELETE FROM `character_inventory` WHERE `guid` = ? AND `item_guid` = ?",
        (owner_low, item_low),
    )
    .map_err(DbError::Driver)?;
    conn.exec_drop("DELETE FROM `item_instance` WHERE `guid` = ?", (item_low,))
        .map_err(DbError::Driver)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::charter_enchantments;

    #[test]
    fn enchantment_text_carries_petition_id_in_slot_zero() {
        let text = charter_enchantments(42);
        let parts: Vec<&str> = text.split(' ').collect();
        assert_eq!(parts.len(), 21);
        assert_eq!(parts[0], "42");
        assert!(parts[1..].iter().all(|part| *part == "0"));
    }
}
