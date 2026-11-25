//! `UpdateData` / `MovementData` packet assembly, ported from
//! `vmangos/src/game/Objects/UpdateData.{h,cpp}` at the 1.12.1
//! build shape (`SUPPORTED_CLIENT_BUILD > CLIENT_BUILD_1_8_4`:
//! packed GUIDs in the out-of-range section, `CREATE_OBJECT2`
//! exists).
//!
//! Semantics replicated:
//!
//! - Blocks append into sub-packets split when the current buffer
//!   **already exceeds** 32 KiB *before* a new block starts, so a
//!   single block may push a sub-packet past the cap
//!   (`UpdateData.cpp:43-57`).
//! - Packet layout: `u32 blockCount (+1 when out-of-range GUIDs
//!   are present)`, `u8 hasTransport`, optional out-of-range
//!   section (`u8 type, u32 count, packed GUIDs` in set order),
//!   then the sub-packet's blocks (`:122-147`).
//! - Payloads strictly larger than `Compression.Update.Size`
//!   (default 128) become `SMSG_COMPRESSED_UPDATE_OBJECT=502`:
//!   `u32` raw size + zlib stream at `Compression.Level`
//!   (default 1); otherwise `SMSG_UPDATE_OBJECT=169` raw
//!   (`:149-175`). Byte-identical zlib output is not required by
//!   the client; thresholds and opcode choice are the observable
//!   contract.
//! - `Send` (`:177-194`) emits the out-of-range section only in
//!   the first sub-packet; [`UpdateData::build_packets`] mirrors
//!   that sequencing without the session.
//!
//! Deviations: compression failure makes the C++ send a corrupt
//! packet anyway (its `bool` return is ignored by `Send`); here a
//! failed compression yields `None` / drops that sub-packet. With
//! a `compressBound`-sized output this is unreachable in practice.
//! The `[CRASH-CLIENT]` oversize log (`>= 900000`, `:154-155`) is
//! left to the caller (this crate does no logging).

use std::collections::BTreeSet;
use std::io::Write as _;

use mangos_guid::ObjectGuid;

use crate::buffer::PacketWriter;
use crate::world::WorldPacket;

/// `SMSG_UPDATE_OBJECT` (`Opcodes_1_12_1.h:172`).
pub const SMSG_UPDATE_OBJECT: u16 = 169;
/// `SMSG_DESTROY_OBJECT` (`Opcodes_1_12_1.h:173`).
pub const SMSG_DESTROY_OBJECT: u16 = 170;
/// `SMSG_COMPRESSED_UPDATE_OBJECT` (`Opcodes_1_12_1.h:505`).
pub const SMSG_COMPRESSED_UPDATE_OBJECT: u16 = 502;
/// `SMSG_COMPRESSED_MOVES` (`Opcodes_1_12_1.h:766`).
pub const SMSG_COMPRESSED_MOVES: u16 = 763;

/// `MAX_UNCOMPRESSED_PACKET_SIZE` (`UpdateData.cpp:33`).
pub const MAX_UNCOMPRESSED_PACKET_SIZE: usize = 0x8000;

/// `enum ObjectUpdateType` (`UpdateData.h:32-42`) at 1.12.1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ObjectUpdateType {
    Values = 0,
    Movement = 1,
    CreateObject = 2,
    CreateObject2 = 3,
    OutOfRangeObjects = 4,
    NearObjects = 5,
}

/// `enum ObjectUpdateFlags` (`UpdateData.h:45-57`), 1.12.1 shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct ObjectUpdateFlags(pub u8);

impl ObjectUpdateFlags {
    pub const NONE: u8 = 0x00;
    pub const SELF: u8 = 0x01;
    pub const TRANSPORT: u8 = 0x02;
    pub const MELEE_ATTACKING: u8 = 0x04;
    pub const HIGH_GUID: u8 = 0x08;
    pub const ALL: u8 = 0x10;
    pub const LIVING: u8 = 0x20;
    pub const HAS_POSITION: u8 = 0x40;

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    pub fn add(&mut self, flag: u8) {
        self.0 |= flag;
    }
}

/// `Compression.Level` / `Compression.Update.Size`
/// (`World.cpp:557-558`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompressionConfig {
    /// zlib level; C++ config enforces 1..=9.
    pub level: u32,
    /// Payloads strictly larger than this are compressed.
    pub update_size: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            level: 1,
            update_size: 128,
        }
    }
}

/// zlib-compress (`PacketCompressor::Compress`,
/// `UpdateData.cpp:59-113`); `None` on encoder failure.
fn compress(data: &[u8], level: u32) -> Option<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(data.len()),
        flate2::Compression::new(level.min(9)),
    );
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

/// `class UpdatePacket` (`UpdateData.h:59-65`): one sub-packet's
/// blocks plus their count.
#[derive(Clone, Debug, Default)]
pub struct UpdatePacket {
    data: PacketWriter,
    block_count: u32,
}

impl UpdatePacket {
    #[must_use]
    pub fn block_count(&self) -> u32 {
        self.block_count
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.data.as_slice()
    }
}

/// `class UpdateData` (`UpdateData.h:73-93`).
#[derive(Clone, Debug, Default)]
pub struct UpdateData {
    out_of_range_guids: BTreeSet<ObjectGuid>,
    datas: Vec<UpdatePacket>,
}

impl UpdateData {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `AddOutOfRangeGUID(guid)` (`UpdateData.cpp:41`).
    pub fn add_out_of_range_guid(&mut self, guid: ObjectGuid) {
        self.out_of_range_guids.insert(guid);
    }

    /// `AddOutOfRangeGUID(set)` (`UpdateData.cpp:39`).
    pub fn add_out_of_range_guids(&mut self, guids: impl IntoIterator<Item = ObjectGuid>) {
        self.out_of_range_guids.extend(guids);
    }

    /// `GetOutOfRangeGUIDs` (`UpdateData.h:88`).
    #[must_use]
    pub fn out_of_range_guids(&self) -> &BTreeSet<ObjectGuid> {
        &self.out_of_range_guids
    }

    /// `AddUpdateBlockAndGetBuffer` (`UpdateData.cpp:43-57`):
    /// start a new sub-packet when the current one already exceeds
    /// 32 KiB, count the block, and hand out its buffer.
    ///
    /// # Panics
    ///
    /// Never: a sub-packet is pushed above whenever none is usable.
    pub fn add_update_block(&mut self) -> &mut PacketWriter {
        if self
            .datas
            .last()
            .is_none_or(|p| p.bytes().len() > MAX_UNCOMPRESSED_PACKET_SIZE)
        {
            self.datas.push(UpdatePacket::default());
        }
        let packet = self.datas.last_mut().expect("packet just ensured");
        packet.block_count += 1;
        &mut packet.data
    }

    /// `HasData` (`UpdateData.h:85`).
    #[must_use]
    pub fn has_data(&self) -> bool {
        !self.datas.is_empty() || !self.out_of_range_guids.is_empty()
    }

    /// `Clear` (`UpdateData.cpp:196-200`).
    pub fn clear(&mut self) {
        self.datas.clear();
        self.out_of_range_guids.clear();
    }

    /// Sub-packets built so far, in send order.
    #[must_use]
    pub fn packets(&self) -> &[UpdatePacket] {
        &self.datas
    }

    /// `BuildPacket(packet, updPacket, hasTransport)`
    /// (`UpdateData.cpp:122-175`) for one sub-packet (or none, for
    /// a pure out-of-range packet). `None` iff compression failed.
    #[must_use]
    pub fn build_packet_for(
        &self,
        upd_packet: Option<&UpdatePacket>,
        has_transport: bool,
        compression: CompressionConfig,
    ) -> Option<WorldPacket> {
        let mut buf = PacketWriter::new();
        let block_count = upd_packet.map_or(0, UpdatePacket::block_count);
        buf.write_u32_le(if self.out_of_range_guids.is_empty() {
            block_count
        } else {
            block_count + 1
        })
        .write_u8(u8::from(has_transport));

        if !self.out_of_range_guids.is_empty() {
            buf.write_u8(ObjectUpdateType::OutOfRangeObjects as u8)
                .write_u32_le(self.out_of_range_guids.len() as u32);
            let mut packed = Vec::with_capacity(9);
            for &guid in &self.out_of_range_guids {
                packed.clear();
                guid.write_packed(&mut packed);
                buf.write_bytes(&packed);
            }
        }

        if let Some(upd) = upd_packet {
            buf.write_bytes(upd.bytes());
        }

        let payload = buf.into_vec();
        if payload.len() > compression.update_size {
            let compressed = compress(&payload, compression.level)?;
            let mut body = Vec::with_capacity(4 + compressed.len());
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(&compressed);
            Some(WorldPacket::with_body(SMSG_COMPRESSED_UPDATE_OBJECT, body))
        } else {
            Some(WorldPacket::with_body(SMSG_UPDATE_OBJECT, payload))
        }
    }

    /// `BuildPacket(packet, hasTransport)` (`UpdateData.cpp:115-120`):
    /// the first sub-packet, or a pure out-of-range packet.
    #[must_use]
    pub fn build_first_packet(
        &self,
        has_transport: bool,
        compression: CompressionConfig,
    ) -> Option<WorldPacket> {
        self.build_packet_for(self.datas.first(), has_transport, compression)
    }

    /// `Send` (`UpdateData.cpp:177-194`) minus the session: builds
    /// every packet in order, consuming the out-of-range set after
    /// the first. Sub-packets whose compression fails are dropped
    /// (see module deviations).
    pub fn build_packets(
        &mut self,
        has_transport: bool,
        compression: CompressionConfig,
    ) -> Vec<WorldPacket> {
        let mut out = Vec::new();
        if self.datas.is_empty() {
            if !self.out_of_range_guids.is_empty() {
                if let Some(packet) = self.build_packet_for(None, has_transport, compression) {
                    out.push(packet);
                }
                self.out_of_range_guids.clear();
            }
            return out;
        }
        for i in 0..self.datas.len() {
            if let Some(packet) =
                self.build_packet_for(Some(&self.datas[i]), has_transport, compression)
            {
                out.push(packet);
            }
            self.out_of_range_guids.clear();
        }
        out
    }
}

/// `class MovementData` (`UpdateData.h:96-110`): batches small
/// movement packets into one `SMSG_COMPRESSED_MOVES`.
#[derive(Clone, Debug, Default)]
pub struct MovementData {
    buffer: PacketWriter,
}

impl MovementData {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `CanAddPacket` (`UpdateData.cpp:203-213`): the per-packet
    /// length is stored in a `u8` including the 2-byte opcode, and
    /// the batch must stay under the 900000 client-crash line.
    #[must_use]
    pub fn can_add_packet(&self, data: &WorldPacket) -> bool {
        if data.body().len() + 2 > 0xFF {
            return false;
        }
        self.buffer.as_slice().len() + (data.body().len() + 2) < 900_000
    }

    /// `AddPacket` (`UpdateData.cpp:215-221`).
    ///
    /// # Panics
    ///
    /// Panics if the packet body + opcode exceeds 255 bytes (the
    /// C++ `ASSERT`), where the client would crash.
    pub fn add_packet(&mut self, data: &WorldPacket) {
        assert!(
            data.body().len() + 2 <= 0xFF,
            "movement packet too large for compressed-moves batching"
        );
        self.buffer
            .write_u8((data.body().len() + 2) as u8)
            .write_u16_le(data.opcode())
            .write_bytes(data.body());
    }

    /// `HasData` (`UpdateData.h:104`).
    #[must_use]
    pub fn has_data(&self) -> bool {
        !self.buffer.as_slice().is_empty()
    }

    /// `ClearBuffer` (`UpdateData.h:105`).
    pub fn clear(&mut self) {
        self.buffer = PacketWriter::new();
    }

    /// `BuildPacket` (`UpdateData.cpp:223-242`): always compressed,
    /// `u32` raw size + zlib stream. `None` iff compression failed.
    #[must_use]
    pub fn build_packet(&self, compression: CompressionConfig) -> Option<WorldPacket> {
        let payload = self.buffer.as_slice();
        let compressed = compress(payload, compression.level)?;
        let mut body = Vec::with_capacity(4 + compressed.len());
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&compressed);
        Some(WorldPacket::with_body(SMSG_COMPRESSED_MOVES, body))
    }
}
