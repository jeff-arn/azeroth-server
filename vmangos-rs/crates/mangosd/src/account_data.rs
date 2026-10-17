//! Account-data caches (`AccountData.h`, `WorldSession.cpp:
//! 872-970`): eight slots (global config/bindings/macros and
//! per-character config/bindings/macros/layout/chat), loaded
//! with mask rules and published to the client as MD5 digests
//! (`SMSG_ACCOUNT_DATA_MD5`). The write side
//! (`SetAccountData`, `CMSG_UPDATE_ACCOUNT_DATA`) is a save path
//! and stays with Phase 6.

use md5::{Digest, Md5};

use mangos_db::characters::AccountDataEntry;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

pub const NUM_ACCOUNT_DATA_TYPES: usize = 8;
pub const GLOBAL_CACHE_MASK: u32 = 0x15;
pub const PER_CHARACTER_CACHE_MASK: u32 = 0xEA;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountData {
    pub timestamp: u64,
    pub data: String,
}

#[derive(Clone, Debug, Default)]
pub struct AccountDataSet {
    entries: [AccountData; NUM_ACCOUNT_DATA_TYPES],
}

impl AccountDataSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `LoadAccountData` (`WorldSession.cpp:878-908`): masked
    /// slots reset first; out-of-range or wrong-mask rows are
    /// ignored (C++ logs them; counts don't affect behaviour).
    pub fn load(&mut self, rows: &[AccountDataEntry], mask: u32) {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *slot = AccountData::default();
            }
        }
        for row in rows {
            let Ok(index) = usize::try_from(row.data_type) else {
                continue;
            };
            if index >= NUM_ACCOUNT_DATA_TYPES || mask & (1 << index) == 0 {
                continue;
            }
            self.entries[index] = AccountData {
                timestamp: row.timestamp,
                data: row.data.clone(),
            };
        }
    }

    #[must_use]
    pub fn get(&self, data_type: usize) -> Option<&AccountData> {
        self.entries.get(data_type)
    }

    /// `SendAccountDataTimes` at the 1.12 build
    /// (`WorldSession.cpp:948-970`): one MD5 per slot, empty
    /// data hashing as the empty string.
    #[must_use]
    pub fn build_md5_packet(&self) -> WorldPacket {
        let mut writer = PacketWriter::new();
        for entry in &self.entries {
            let digest = Md5::digest(entry.data.as_bytes());
            writer.write_bytes(&digest);
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_ACCOUNT_DATA_MD5);
        packet.take_writer(writer);
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(data_type: u32, data: &str) -> AccountDataEntry {
        AccountDataEntry {
            data_type,
            timestamp: 123,
            data: data.to_string(),
        }
    }

    #[test]
    fn mask_rules() {
        let mut set = AccountDataSet::new();
        set.load(
            &[entry(0, "g0"), entry(1, "wrong-mask"), entry(9, "oob")],
            GLOBAL_CACHE_MASK,
        );
        assert_eq!(set.get(0).expect("0").data, "g0");
        assert_eq!(set.get(1).expect("1").data, "");
        // Per-character load resets its own slots, keeps global.
        set.load(&[entry(1, "p1")], PER_CHARACTER_CACHE_MASK);
        assert_eq!(set.get(0).expect("0").data, "g0");
        assert_eq!(set.get(1).expect("1").data, "p1");
    }

    #[test]
    fn md5_packet_shape() {
        let set = AccountDataSet::new();
        let packet = set.build_md5_packet();
        assert_eq!(packet.opcode(), opcode_id::SMSG_ACCOUNT_DATA_MD5);
        assert_eq!(packet.body().len(), 8 * 16);
        // MD5("") = d41d8cd98f00b204e9800998ecf8427e
        assert_eq!(
            &packet.body()[..4],
            &[0xD4, 0x1D, 0x8C, 0xD9],
            "empty slots hash as empty string"
        );
    }
}
