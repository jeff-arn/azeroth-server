//! Opcode registry types over the generated 1.12.1 table
//! ([`crate::opcode_table`], from `Opcodes.cpp` via
//! `vmangos-rs/scripts/opcode_table_gen.py`).

use crate::opcode_table::OPCODE_TABLE;
use crate::world::{PacketProcessing, SessionStatus};

/// One `OpcodeHandler` registration (`Opcodes.h:150-166`),
/// without the C++ member-function pointer: dispatch targets are
/// resolved by `handler_name` until the session layer maps them
/// to typed handlers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpcodeMeta {
    pub name: &'static str,
    pub status: SessionStatus,
    pub processing: PacketProcessing,
    pub handler_name: &'static str,
    pub registered: bool,
}

/// `emptyHandler` used for unregistered slots
/// (`Opcodes.cpp:912-913`): unhandled class, droppable queue class.
pub const EMPTY_OPCODE_META: OpcodeMeta = OpcodeMeta {
    name: "<none>",
    status: SessionStatus::Unhandled,
    processing: PacketProcessing::MaxType,
    handler_name: "Handle_NULL",
    registered: false,
};

/// Metadata for any slot in range, including unregistered ones.
#[must_use]
pub fn opcode_meta(opcode: u16) -> Option<&'static OpcodeMeta> {
    OPCODE_TABLE.get(usize::from(opcode))
}

/// `LookupOpcodeHandler`: `Some` only for registered slots.
#[must_use]
pub fn lookup_opcode(opcode: u16) -> Option<&'static OpcodeMeta> {
    opcode_meta(opcode).filter(|meta| meta.registered)
}

/// Reverse lookup by exact C++ name (`LookupOpcodeValue`,
/// `Opcodes.cpp:926-935`): `None` for unknown or out-of-table
/// names, matching the C++ `-1` result.
#[must_use]
pub fn lookup_opcode_value(name: &str) -> Option<u16> {
    OPCODE_TABLE
        .iter()
        .position(|meta| meta.name == name)
        .map(|i| i as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcode_table::opcode_id;

    #[test]
    fn census_matches_cpp() {
        assert_eq!(OPCODE_TABLE.len(), 828);
        assert_eq!(OPCODE_TABLE.iter().filter(|m| m.registered).count(), 826);
        assert_eq!(
            OPCODE_TABLE
                .iter()
                .filter(|m| m.status == SessionStatus::Never)
                .count(),
            525
        );
        let mut handlers: Vec<&str> = OPCODE_TABLE
            .iter()
            .filter(|m| m.registered)
            .map(|m| m.handler_name)
            .collect();
        handlers.sort_unstable();
        handlers.dedup();
        assert_eq!(handlers.len(), 276);
    }

    #[test]
    fn spot_checks_and_gap_slot() {
        assert_eq!(opcode_id::SMSG_AUTH_CHALLENGE, 492);
        assert_eq!(opcode_id::CMSG_AUTH_SESSION, 493);
        let ping = lookup_opcode(opcode_id::CMSG_PING).unwrap();
        assert_eq!(ping.name, "CMSG_PING");
        assert_eq!(ping.handler_name, "Handle_EarlyProccess");
        // Deliberate 0x33A gap: in range, unregistered.
        assert!(lookup_opcode(0x33A).is_none());
        assert_eq!(opcode_meta(0x33A).unwrap().handler_name, "Handle_NULL");
        assert!(opcode_meta(828).is_none());
        assert_eq!(lookup_opcode_value("CMSG_CHAR_ENUM"), Some(55));
        assert_eq!(lookup_opcode_value("CMSG_DOES_NOT_EXIST"), None);
    }
}
