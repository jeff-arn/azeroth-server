//! The packet-admission core of `WorldSession`: queue-class
//! resolution (`QueuePacket`, `WorldSession.cpp:252-282`), flood
//! accounting (`AllowPacket`, `:1167-1233`), and the status filter
//! (`ProcessPackets`, `:456-555`). Pure and synchronous so the
//! world actor and tests drive it identically.
//!
//! Phase 3 keeps `has_player` permanently false (empty world):
//! `LoggedIn`/`Transfer` packets therefore always drop, exactly as
//! they do in C++ before `CMSG_PLAYER_LOGIN` completes.

use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::opcodes::OpcodeMeta;
use mangos_protocol::world::{PacketProcessing, SessionStatus};

/// What `QueuePacket` decides for an inbound packet's queue class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueAdmission {
    /// Forward to the world actor (class preserved for the
    /// per-class pass order a full port needs).
    Queue(PacketProcessing),
    /// `processing >= PACKET_PROCESS_MAX_TYPE`: silently skipped
    /// on the IO side, never reaches the session
    /// (`WorldSession.cpp:270-278`).
    DropSilently,
}

/// `CMSG_MESSAGECHAT` rerouting + the `MAX_TYPE` drop, exactly as
/// `QueuePacket` computes it. Sniff/anticheat logging hooks are
/// out of Phase 3 scope.
#[must_use]
pub fn queue_admission(meta: &OpcodeMeta, opcode: u16, body: &[u8]) -> QueueAdmission {
    if opcode == opcode_id::CMSG_MESSAGECHAT && body.len() >= 4 {
        let chat_type = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        return QueueAdmission::Queue(chat_processing(chat_type));
    }
    if meta.processing == PacketProcessing::MaxType {
        return QueueAdmission::DropSilently;
    }
    QueueAdmission::Queue(meta.processing)
}

/// `GetChatPacketProcessingType` (`WorldSession.cpp:220-249`) at
/// the 1.12.1 build; chat-type values from `SharedDefines.h:1207+`.
#[must_use]
pub fn chat_processing(chat_type: u32) -> PacketProcessing {
    match chat_type {
        0x0E | 0x06 | 0x01 | 0x03 | 0x04 | 0x02 | 0x57 | 0x58 | 0x5C | 0x5D | 0x15 => {
            PacketProcessing::Async
        }
        0x00 | 0x08 | 0x05 => PacketProcessing::Map,
        _ => PacketProcessing::World,
    }
}

/// Outcome of the `ProcessPackets` status filter for one packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    /// Run the registered handler (Phase 3: class stub).
    Execute,
    /// `LogUnexpectedOpcode` (debug level) with the C++ reason.
    DropUnexpected(&'static str),
    /// Silent drop: lagging `LoggedIn` packet with no player in
    /// world, where C++ logs nothing.
    DropSilent,
    /// `STATUS_NEVER`: error log, drop (`:514-516`).
    DropNotAllowed,
    /// `STATUS_UNHANDLED`: debug log, drop (`:517-519`).
    DropUnhandled,
    /// Unknown status value: error log, drop (`:520-522`).
    DropWrongStatus,
    /// `AllowPacket` refused: stop this session's queue for the
    /// current update; the popped packet is consumed, the rest
    /// stay queued (`:463-464`).
    FloodBreak,
}

/// `m_floodPacketsCount` (`WorldSession.h`), reset per `Update`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FloodCounters {
    pub total: u32,
    pub slow: u32,
    pub very_slow: u32,
}

/// Opcodes `AllowPacket` waves through without any counting
/// (`WorldSession.cpp:1170-1182`).
const FLOOD_EXEMPT: [u16; 10] = [
    opcode_id::CMSG_GAMEOBJECT_QUERY,
    opcode_id::CMSG_CREATURE_QUERY,
    opcode_id::CMSG_QUESTGIVER_STATUS_QUERY,
    opcode_id::CMSG_ITEM_QUERY_SINGLE,
    opcode_id::CMSG_NAME_QUERY,
    opcode_id::CMSG_PET_NAME_QUERY,
    opcode_id::CMSG_GUILD_QUERY,
    opcode_id::CMSG_JOIN_CHANNEL,
    opcode_id::CMSG_AUCTION_LIST_ITEMS,
    opcode_id::CMSG_WHO,
];

/// Very-slow opcodes; they also count as slow (fallthrough,
/// `WorldSession.cpp:1189-1201`).
const FLOOD_VERY_SLOW: [u16; 11] = [
    opcode_id::CMSG_CHAR_CREATE,
    opcode_id::CMSG_CHAR_ENUM,
    opcode_id::CMSG_CHAR_DELETE,
    opcode_id::CMSG_OPEN_ITEM,
    opcode_id::CMSG_PETITION_BUY,
    opcode_id::CMSG_PETITION_SIGN,
    opcode_id::CMSG_PETITION_QUERY,
    opcode_id::MSG_PETITION_RENAME,
    opcode_id::CMSG_SEND_MAIL,
    opcode_id::CMSG_PLAYER_LOGIN,
    opcode_id::CMSG_GMTICKET_UPDATETEXT,
];

/// Slow-only opcodes (`WorldSession.cpp:1201-1207`).
const FLOOD_SLOW: [u16; 5] = [
    opcode_id::CMSG_LOGOUT_REQUEST,
    opcode_id::CMSG_ADD_FRIEND,
    opcode_id::CMSG_DEL_FRIEND,
    opcode_id::CMSG_BUY_ITEM,
    opcode_id::CMSG_SELL_ITEM,
];

/// Per-session admission state owned by the world actor. The
/// four booleans mirror independent C++ `WorldSession` flags.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct SessionCore {
    has_player: bool,
    player_in_world: bool,
    recently_logged_out: bool,
    in_queue: bool,
    flood: FloodCounters,
}

impl SessionCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `WorldSession::Update` prologue: flood counters reset
    /// (`WorldSession.cpp:373-374`).
    pub fn begin_update(&mut self) {
        self.flood = FloodCounters::default();
    }

    /// `AllowPacket` (`WorldSession.cpp:1167-1233`). The C++
    /// anticheat action on refusal is the caller's business; this
    /// preserves only the allow/refuse decision and counters.
    pub fn allow_packet(&mut self, opcode: u16) -> bool {
        if FLOOD_EXEMPT.contains(&opcode) {
            return true;
        }
        self.flood.total += 1;
        if FLOOD_VERY_SLOW.contains(&opcode) {
            self.flood.very_slow += 1;
            self.flood.slow += 1;
        } else if FLOOD_SLOW.contains(&opcode) {
            self.flood.slow += 1;
        }
        self.flood.very_slow <= 2 && self.flood.slow <= 8 && self.flood.total <= 300
    }

    #[must_use]
    pub fn flood_counters(&self) -> FloodCounters {
        self.flood
    }

    /// Session-state setters for the phases that own player login
    /// and the login queue. Phase 3 never sets them.
    pub fn set_player_state(&mut self, has_player: bool, in_world: bool) {
        self.has_player = has_player;
        self.player_in_world = in_world;
    }

    pub fn set_recently_logged_out(&mut self, value: bool) {
        self.recently_logged_out = value;
    }

    pub fn set_in_queue(&mut self, value: bool) {
        self.in_queue = value;
    }

    /// The `ProcessPackets` body for one already-queued packet
    /// (`WorldSession.cpp:463-523`).
    pub fn process(&mut self, meta: &OpcodeMeta, opcode: u16) -> Dispatch {
        if !self.allow_packet(opcode) {
            return Dispatch::FloodBreak;
        }
        match meta.status {
            SessionStatus::LoggedIn => {
                if !self.has_player {
                    if self.recently_logged_out {
                        Dispatch::DropSilent
                    } else {
                        Dispatch::DropUnexpected("the player has not logged in yet")
                    }
                } else if self.player_in_world {
                    Dispatch::Execute
                } else {
                    Dispatch::DropSilent
                }
            }
            SessionStatus::LoggedInOrRecentlyLoggedOut => {
                if !self.has_player && !self.recently_logged_out {
                    Dispatch::DropUnexpected(
                        "the player has not logged in yet and not recently logout",
                    )
                } else {
                    Dispatch::Execute
                }
            }
            SessionStatus::Transfer => {
                if !self.has_player {
                    Dispatch::DropUnexpected("the player has not logged in yet")
                } else if self.player_in_world {
                    Dispatch::DropUnexpected("the player is still in world")
                } else {
                    Dispatch::Execute
                }
            }
            SessionStatus::Authed => {
                if self.in_queue {
                    Dispatch::DropUnexpected("the player is still in queue")
                } else {
                    self.recently_logged_out = false;
                    Dispatch::Execute
                }
            }
            SessionStatus::Never => Dispatch::DropNotAllowed,
            SessionStatus::Unhandled => Dispatch::DropUnhandled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_protocol::opcodes::lookup_opcode;

    fn meta(opcode: u16) -> &'static OpcodeMeta {
        lookup_opcode(opcode).expect("registered opcode")
    }

    #[test]
    fn queue_admission_drops_max_type_and_gap() {
        // SMSG (server-side) rows are MAX_TYPE: a client sending
        // one is silently skipped, as is the 0x33A gap slot.
        let smsg = mangos_protocol::opcodes::opcode_meta(opcode_id::SMSG_PONG).unwrap();
        assert_eq!(
            queue_admission(smsg, opcode_id::SMSG_PONG, &[]),
            QueueAdmission::DropSilently
        );
        let gap = mangos_protocol::opcodes::opcode_meta(0x33A).unwrap();
        assert_eq!(
            queue_admission(gap, 0x33A, &[]),
            QueueAdmission::DropSilently
        );
        let enum_meta = meta(opcode_id::CMSG_CHAR_ENUM);
        assert_eq!(
            queue_admission(enum_meta, opcode_id::CMSG_CHAR_ENUM, &[]),
            QueueAdmission::Queue(enum_meta.processing)
        );
    }

    #[test]
    fn chat_rerouting() {
        assert_eq!(chat_processing(0x00), PacketProcessing::Map); // SAY
        assert_eq!(chat_processing(0x06), PacketProcessing::Async); // WHISPER
        assert_eq!(chat_processing(0x5C), PacketProcessing::Async); // BG
        assert_eq!(chat_processing(0x2A), PacketProcessing::World); // other
        let chat = meta(opcode_id::CMSG_MESSAGECHAT);
        assert_eq!(
            queue_admission(chat, opcode_id::CMSG_MESSAGECHAT, &[0x06, 0, 0, 0]),
            QueueAdmission::Queue(PacketProcessing::Async)
        );
        // Short body: no reroute, registered class applies.
        assert_eq!(
            queue_admission(chat, opcode_id::CMSG_MESSAGECHAT, &[0x06]),
            QueueAdmission::Queue(chat.processing)
        );
    }

    #[test]
    fn status_filter_in_empty_world() {
        let mut core = SessionCore::new();
        // Authed executes.
        core.begin_update();
        assert_eq!(
            core.process(meta(opcode_id::CMSG_CHAR_ENUM), opcode_id::CMSG_CHAR_ENUM),
            Dispatch::Execute
        );
        // LoggedIn without a player: unexpected, then silent once
        // recently-logged-out is set.
        let logged = meta(opcode_id::CMSG_LOGOUT_REQUEST);
        assert_eq!(logged.status, SessionStatus::LoggedIn);
        assert_eq!(
            core.process(logged, opcode_id::CMSG_LOGOUT_REQUEST),
            Dispatch::DropUnexpected("the player has not logged in yet")
        );
        core.set_recently_logged_out(true);
        assert_eq!(
            core.process(logged, opcode_id::CMSG_LOGOUT_REQUEST),
            Dispatch::DropSilent
        );
        // An Authed packet clears the recently-logged-out flag.
        assert_eq!(
            core.process(meta(opcode_id::CMSG_CHAR_ENUM), opcode_id::CMSG_CHAR_ENUM),
            Dispatch::Execute
        );
        assert_eq!(
            core.process(logged, opcode_id::CMSG_LOGOUT_REQUEST),
            Dispatch::DropUnexpected("the player has not logged in yet")
        );
    }

    #[test]
    fn never_and_unhandled_drop() {
        let mut core = SessionCore::new();
        let never = meta(opcode_id::CMSG_BOOTME);
        assert_eq!(never.status, SessionStatus::Never);
        assert_eq!(
            core.process(never, opcode_id::CMSG_BOOTME),
            Dispatch::DropNotAllowed
        );
        let unhandled = (0..828u16)
            .find_map(|op| lookup_opcode(op).filter(|m| m.status == SessionStatus::Unhandled));
        if let Some(m) = unhandled {
            let op = mangos_protocol::opcodes::lookup_opcode_value(m.name).unwrap();
            assert_eq!(core.process(m, op), Dispatch::DropUnhandled);
        }
    }

    #[test]
    fn flood_thresholds_and_exemptions() {
        let mut core = SessionCore::new();
        core.begin_update();
        // CHAR_ENUM is very-slow: third in one update breaks.
        assert!(core.allow_packet(opcode_id::CMSG_CHAR_ENUM));
        assert!(core.allow_packet(opcode_id::CMSG_CHAR_ENUM));
        assert!(!core.allow_packet(opcode_id::CMSG_CHAR_ENUM));
        assert_eq!(core.flood_counters().very_slow, 3);
        assert_eq!(core.flood_counters().slow, 3);
        assert_eq!(core.flood_counters().total, 3);

        core.begin_update();
        // Exempt opcodes never count, even 400 in a row.
        for _ in 0..400 {
            assert!(core.allow_packet(opcode_id::CMSG_NAME_QUERY));
        }
        assert_eq!(core.flood_counters().total, 0);

        // Total threshold with a neutral opcode.
        let mut allowed = true;
        for _ in 0..300 {
            allowed = core.allow_packet(opcode_id::CMSG_MESSAGECHAT);
        }
        assert!(allowed);
        assert!(!core.allow_packet(opcode_id::CMSG_MESSAGECHAT));
    }

    #[test]
    fn flood_break_via_process() {
        let mut core = SessionCore::new();
        core.begin_update();
        let m = meta(opcode_id::CMSG_CHAR_ENUM);
        assert_eq!(
            core.process(m, opcode_id::CMSG_CHAR_ENUM),
            Dispatch::Execute
        );
        assert_eq!(
            core.process(m, opcode_id::CMSG_CHAR_ENUM),
            Dispatch::Execute
        );
        assert_eq!(
            core.process(m, opcode_id::CMSG_CHAR_ENUM),
            Dispatch::FloodBreak
        );
    }
}
