//! `AuctionHouse.dbc` rows (`AuctionHouseEntry`):
//! house id, linked faction, deposit and consignment
//! percentages. Produced fields of fmt
//! `niiixxxxxxxxx` are those four integers.

use std::collections::HashMap;

/// One `AuctionHouse.dbc` row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuctionHouseEntry {
    pub house_id: u32,
    pub faction: u32,
    pub deposit_percent: u32,
    pub cut_percent: u32,
}

/// Loaded `AuctionHouse.dbc` (`sAuctionHouseStore`).
#[derive(Clone, Debug, Default)]
pub struct AuctionHouseDbc {
    pub entries: Vec<AuctionHouseEntry>,
    by_id: HashMap<u32, AuctionHouseEntry>,
}

impl AuctionHouseDbc {
    /// Builds the store and its id index.
    #[must_use]
    pub fn from_entries(entries: Vec<AuctionHouseEntry>) -> Self {
        let by_id = entries
            .iter()
            .map(|entry| (entry.house_id, *entry))
            .collect();
        Self { entries, by_id }
    }

    /// `LookupEntry(houseId)`.
    #[must_use]
    pub fn get(&self, house_id: u32) -> Option<AuctionHouseEntry> {
        self.by_id.get(&house_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_by_house_id() {
        let dbc = AuctionHouseDbc::from_entries(vec![AuctionHouseEntry {
            house_id: 6,
            faction: 85,
            deposit_percent: 5,
            cut_percent: 5,
        }]);
        assert_eq!(dbc.get(6).map(|e| e.faction), Some(85));
        assert_eq!(dbc.get(1), None);
    }
}
