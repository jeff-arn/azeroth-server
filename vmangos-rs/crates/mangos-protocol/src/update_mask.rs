//! `class UpdateMask` (`vmangos/src/game/Objects/UpdateMask.h`):
//! a little-endian bitset over update-field word indices. Wire form
//! (written by the block builders in `mangos-entities`) is `u8`
//! block count followed by the `u32` blocks.
//!
//! Deviation: the C++ copy/assignment and `&=`/`|=` operators forget
//! to carry `mHasData` (`UpdateMask.h:69-89`), so a copied mask can
//! report "no data" while holding set bits. No C++ call site was
//! found to depend on that loss; this port maintains `has_data`
//! correctly through clone and bitwise ops instead of replicating
//! the bug.

/// Bitset over `count` field words, stored as
/// `ceil(count / 32)` `u32` blocks (bit `i` = byte `i >> 3`,
/// bit `i & 7`, matching the C++ byte-level layout).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateMask {
    has_data: bool,
    count: u32,
    blocks: Vec<u32>,
}

impl UpdateMask {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `SetCount` (`UpdateMask.h:51-60`): (re)allocate zeroed blocks
    /// for `values_count` words.
    pub fn set_count(&mut self, values_count: u32) {
        self.count = values_count;
        self.blocks = vec![0; values_count.div_ceil(32) as usize];
        self.has_data = false;
    }

    /// `SetBit` (`UpdateMask.h:35-39`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is outside the allocated blocks, where C++
    /// writes out of bounds.
    pub fn set_bit(&mut self, index: u32) {
        self.blocks[(index >> 5) as usize] |= 1 << (index & 0x1F);
        self.has_data = true;
    }

    /// `UnsetBit` (`UpdateMask.h:41`). Like C++, this does not
    /// recompute `has_data`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is outside the allocated blocks.
    pub fn unset_bit(&mut self, index: u32) {
        self.blocks[(index >> 5) as usize] &= !(1 << (index & 0x1F));
    }

    /// `GetBit` (`UpdateMask.h:43`).
    ///
    /// # Panics
    ///
    /// Panics if `index` is outside the allocated blocks.
    #[must_use]
    pub fn get_bit(&self, index: u32) -> bool {
        self.blocks[(index >> 5) as usize] & (1 << (index & 0x1F)) != 0
    }

    /// `GetBlockCount` (`UpdateMask.h:45`).
    #[must_use]
    pub fn block_count(&self) -> u32 {
        self.blocks.len() as u32
    }

    /// `GetLength` (`UpdateMask.h:46`): mask size in bytes.
    #[must_use]
    pub fn len_bytes(&self) -> u32 {
        self.block_count() << 2
    }

    /// `GetCount` (`UpdateMask.h:47`).
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The raw blocks (`GetMask` viewed as words).
    #[must_use]
    pub fn blocks(&self) -> &[u32] {
        &self.blocks
    }

    /// `HasData` (`UpdateMask.h:49`).
    #[must_use]
    pub fn has_data(&self) -> bool {
        self.has_data
    }

    /// `Clear` (`UpdateMask.h:62-67`).
    pub fn clear(&mut self) {
        self.blocks.fill(0);
        self.has_data = false;
    }

    fn refresh_has_data(&mut self) {
        self.has_data = self.blocks.iter().any(|&b| b != 0);
    }
}

impl std::ops::BitAndAssign<&UpdateMask> for UpdateMask {
    /// `operator&=` (`UpdateMask.h:77-82`): iterates this mask's
    /// blocks, so `other` must be at least as wide.
    fn bitand_assign(&mut self, other: &UpdateMask) {
        for (a, b) in self.blocks.iter_mut().zip(&other.blocks) {
            *a &= *b;
        }
        self.refresh_has_data();
    }
}

impl std::ops::BitOrAssign<&UpdateMask> for UpdateMask {
    /// `operator|=` (`UpdateMask.h:84-89`).
    fn bitor_assign(&mut self, other: &UpdateMask) {
        for (a, b) in self.blocks.iter_mut().zip(&other.blocks) {
            *a |= *b;
        }
        self.refresh_has_data();
    }
}
