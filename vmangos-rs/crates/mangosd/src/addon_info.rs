//! `SMSG_ADDON_INFO` construction
//! (`AddonHandler::BuildAddonPacket`, `AddonHandler.cpp:48-136`)
//! over the auth-session addon tail: u32 decompressed size +
//! zlib stream of (name cstring, flags u8, modulusCRC u32,
//! urlCRC u32). Blizzard addons are hidden (with key data when
//! the modulus differs); custom addons stay visible.
//!
//! The C++ gate `!uncompress(...) != Z_OK` is a precedence
//! quirk that runs the builder on SUCCESS and logs/returns
//! false on failure; this port keeps the observable behaviour.

use std::io::Read;

use flate2::read::ZlibDecoder;

use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

const ADDON_STATUS_VISIBLE: u8 = 1;
const ADDON_STATUS_HIDDEN: u8 = 2;
const CORRECT_MODULUS_CRC: u32 = 0x4C1C_776D;
/// `tempValue > 0xFFFFF` refuses the packet.
const MAX_ADDON_DECOMPRESSED: u32 = 0xF_FFFF;

include!("addon_key_data.rs");

/// Why the addon tail produced no reply. All map to C++
/// `return false` (no packet sent); malformed decompressed
/// entries surface as `MalformedEntry`, which the caller treats
/// like the C++ `ByteBufferException` policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddonError {
    /// Fewer than 4 tail bytes, or declared size zero.
    Empty,
    /// Declared decompressed size over the cap.
    TooBig,
    /// zlib failure (also: decompressed length mismatch).
    Uncompress,
    /// A decompressed entry ran past the buffer.
    MalformedEntry,
}

/// Build `SMSG_ADDON_INFO` from the auth-session tail.
///
/// # Errors
///
/// Returns [`AddonError`] when the C++ would send nothing (or
/// throw, for `MalformedEntry`).
pub fn build_addon_packet(tail: &[u8]) -> Result<WorldPacket, AddonError> {
    if tail.len() < 4 {
        return Err(AddonError::Empty);
    }
    let Ok(size_bytes) = tail[..4].try_into() else {
        return Err(AddonError::Empty);
    };
    let declared = u32::from_le_bytes(size_bytes);
    if declared == 0 {
        return Err(AddonError::Empty);
    }
    if declared > MAX_ADDON_DECOMPRESSED {
        return Err(AddonError::TooBig);
    }
    let decoder = ZlibDecoder::new(&tail[4..]);
    let mut decompressed = Vec::with_capacity(declared as usize);
    // One extra byte detects a stream longer than declared
    // (zlib's uncompress fails on a too-small output buffer).
    decoder
        .take(u64::from(declared) + 1)
        .read_to_end(&mut decompressed)
        .map_err(|_| AddonError::Uncompress)?;
    if decompressed.len() != declared as usize {
        return Err(AddonError::Uncompress);
    }

    let mut writer = PacketWriter::new();
    let mut pos = 0usize;
    while pos < decompressed.len() {
        let name_end = decompressed[pos..]
            .iter()
            .position(|&b| b == 0)
            .ok_or(AddonError::MalformedEntry)?
            + pos;
        let name = String::from_utf8_lossy(&decompressed[pos..name_end]).into_owned();
        pos = name_end + 1;
        if pos + 9 > decompressed.len() {
            return Err(AddonError::MalformedEntry);
        }
        // Layout from pos: flags(1) modulusCRC(4) urlCRC(4);
        // only the modulus feeds the reply.
        let Some(crc_bytes): Option<[u8; 4]> = decompressed
            .get(pos + 1..pos + 5)
            .and_then(|s| s.try_into().ok())
        else {
            return Err(AddonError::MalformedEntry);
        };
        let modulus_crc = u32::from_le_bytes(crc_bytes);
        pos += 9;

        if name.contains("Blizzard") {
            writer.write_u8(ADDON_STATUS_HIDDEN);
            writer.write_u8(1); // InfoProvided
            if modulus_crc == CORRECT_MODULUS_CRC {
                writer.write_u8(0); // KeyProvided
            } else {
                writer.write_u8(1); // KeyProvided
                writer.write_bytes(&BLIZZARD_KEY_DATA);
            }
            writer.write_u32_le(0); // Revision
            writer.write_u8(0); // UrlProvided
        } else {
            writer.write_u8(ADDON_STATUS_VISIBLE);
            writer.write_u8(0); // InfoProvided
            writer.write_u8(0); // UrlProvided
        }
    }

    let mut packet = WorldPacket::new(opcode_id::SMSG_ADDON_INFO);
    packet.take_writer(writer);
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn tail_for(entries: &[(&str, u8, u32, u32)]) -> Vec<u8> {
        let mut plain = Vec::new();
        for (name, flags, modulus, url) in entries {
            plain.extend_from_slice(name.as_bytes());
            plain.push(0);
            plain.push(*flags);
            plain.extend_from_slice(&modulus.to_le_bytes());
            plain.extend_from_slice(&url.to_le_bytes());
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plain).expect("compress");
        let compressed = encoder.finish().expect("finish");
        let mut tail = (plain.len() as u32).to_le_bytes().to_vec();
        tail.extend_from_slice(&compressed);
        tail
    }

    #[test]
    fn visible_and_hidden_addons() {
        let tail = tail_for(&[
            ("MyAddon", 1, 0, 0),
            ("Blizzard_AuctionUI", 1, 0x4C1C_776D, 0),
            ("Blizzard_Modified", 1, 0xDEAD, 0),
        ]);
        let packet = build_addon_packet(&tail).expect("build");
        let body = packet.body();
        assert_eq!(body[0], 1); // visible
        assert_eq!(&body[1..3], &[0, 0]);
        assert_eq!(body[3], 2); // hidden, correct modulus
        assert_eq!(&body[4..6], &[1, 0]);
        assert_eq!(&body[6..10], &0u32.to_le_bytes());
        assert_eq!(body[10], 0);
        assert_eq!(body[11], 2); // hidden, wrong modulus -> key follows
        assert_eq!(&body[12..14], &[1, 1]);
        assert_eq!(&body[14..14 + 256], &BLIZZARD_KEY_DATA);
        assert_eq!(body.len(), 14 + 256 + 4 + 1);
    }

    #[test]
    fn error_cases() {
        assert_eq!(build_addon_packet(&[]), Err(AddonError::Empty));
        assert_eq!(build_addon_packet(&[0, 0, 0, 0]), Err(AddonError::Empty));
        assert_eq!(
            build_addon_packet(&0x20_0000u32.to_le_bytes()),
            Err(AddonError::TooBig)
        );
        let mut bad = 10u32.to_le_bytes().to_vec();
        bad.extend_from_slice(b"not zlib at all");
        assert_eq!(build_addon_packet(&bad), Err(AddonError::Uncompress));
    }
}
