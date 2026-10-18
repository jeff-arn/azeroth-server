//! Wire protocol types for the 1.12.1 client protocols.
//!
//! The auth/logon half landed in Phase 1: packet buffer
//! primitives mirroring `ByteBuffer` read semantics, the packed
//! auth packets (`vmangos/src/realmd/AuthPackets.h`), and SRP6
//! server math. Phase 3 adds the world half in [`world`]: header
//! framing, `AuthCrypt`, `WorldPacket`, and session-entry packets.
//! Phase 4 adds the object-update field metadata in
//! [`update_fields`] over the generated [`update_field_table`].

pub mod auth;
pub mod buffer;
pub mod opcode_table;
pub mod opcodes;
pub mod srp;
pub mod update_data;
pub mod update_field_table;
pub mod update_fields;
pub mod update_mask;
pub mod world;
