//! Wire protocol types for the 1.12.1 client protocols.
//!
//! Phase 1 scope: the auth/logon half — packet buffer primitives
//! mirroring `ByteBuffer` read semantics, the packed auth packets
//! (`vmangos/src/realmd/AuthPackets.h`), and the SRP6 server math
//! (`vmangos/src/shared/Crypto/Authentication/SRP6.cpp`). The world
//! protocol (opcodes, `WorldPacket`, `AuthCrypt`) lands in Phase 3 per
//! `docs/RUST_MIGRATION_ARCHITECTURE.md` §12.

pub mod auth;
pub mod buffer;
pub mod srp;
