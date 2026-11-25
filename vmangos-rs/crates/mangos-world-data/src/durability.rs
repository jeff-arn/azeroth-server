//! `DurabilityCosts.dbc` + `DurabilityQuality.dbc`
//! rows (`DurabilityCostsEntry` /
//! `DurabilityQualityEntry`, `DBCStructure.h:279-289`)
//! and the fork repair-cost formula inputs
//! (`Player::DurabilityRepair`, `Player.cpp:4955-5035`;
//! the same lookup prices the durability deduction in
//! `HandleSellItemOpcode`).

use std::collections::HashMap;

/// One `DurabilityCosts.dbc` row: item level plus the
/// 29 subclass multipliers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurabilityCostEntry {
    pub item_level: u32,
    pub multipliers: [u32; 29],
}

/// Loaded durability tables
/// (`sDurabilityCostsStore` / `sDurabilityQualityStore`).
#[derive(Clone, Debug, Default)]
pub struct DurabilityTables {
    costs: HashMap<u32, [u32; 29]>,
    quality_mods: HashMap<u32, f32>,
}

impl DurabilityTables {
    /// Builds the tables from produced DBC fields
    /// (also the test-fixture constructor).
    #[must_use]
    pub fn from_entries(costs: Vec<DurabilityCostEntry>, quality_mods: Vec<(u32, f32)>) -> Self {
        Self {
            costs: costs
                .into_iter()
                .map(|entry| (entry.item_level, entry.multipliers))
                .collect(),
            quality_mods: quality_mods.into_iter().collect(),
        }
    }

    /// `ItemSubClassToDurabilityMultiplierId`
    /// (`ItemPrototype.h:366-375`): weapons use the
    /// subclass, armor is offset by 21, all other
    /// classes use slot 0.
    #[must_use]
    pub fn multiplier_id(item_class: u32, item_subclass: u32) -> usize {
        match item_class {
            2 => item_subclass as usize,
            4 => (item_subclass + 21) as usize,
            _ => 0,
        }
    }

    /// Repair cost before any reputation discount:
    /// `LostDurability * multiplier * quality_mod`
    /// truncated to `u32`, exactly as the fork's float
    /// expression. `None` when either DBC entry is
    /// missing (fork logs and aborts that item).
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )] // fork float formula
    // Fork float-to-uint32 conversion.
    pub fn repair_cost(
        &self,
        item_level: u32,
        quality: u32,
        item_class: u32,
        item_subclass: u32,
        lost_durability: u32,
    ) -> Option<u32> {
        let multipliers = self.costs.get(&item_level)?;
        let quality_entry_id = (quality + 1) * 2;
        let quality_mod = *self.quality_mods.get(&quality_entry_id)?;
        let multiplier = multipliers[Self::multiplier_id(item_class, item_subclass)];
        Some((lost_durability as f32 * multiplier as f32 * quality_mod) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_cost_uses_fork_indexes() {
        let mut multipliers = [0u32; 29];
        multipliers[0] = 3; // weapon subclass 0
        multipliers[21] = 5; // armor subclass 0
        let tables = DurabilityTables::from_entries(
            vec![DurabilityCostEntry {
                item_level: 10,
                multipliers,
            }],
            vec![(4, 2.0)], // quality 1 -> entry (1+1)*2
        );
        assert_eq!(tables.repair_cost(10, 1, 2, 0, 4), Some(24));
        assert_eq!(tables.repair_cost(10, 1, 4, 0, 4), Some(40));
        assert_eq!(tables.repair_cost(11, 1, 2, 0, 4), None);
    }
}
