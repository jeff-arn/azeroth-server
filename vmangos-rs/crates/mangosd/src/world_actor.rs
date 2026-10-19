//! The world actor: single owner of all session state, driven
//! at the C++ 50 ms tick (`WorldRunnable`, architecture section
//! 3.3). Socket tasks reach it only through the bounded
//! [`WorldMessage`] channel; outbound packets flow back over a
//! per-session bounded control channel, mirroring `addSessQueue`
//! in and `WorldSocket::m_sendQueue` out.
//!
//! Phase 3 simplifications, recorded for the phases that undo
//! them: the five C++ per-class receive queues collapse into one
//! FIFO per session (cross-class reordering inside a tick is
//! unobservable while every handler is a stub), and a duplicate
//! login simply replaces the old session (`World::AddSession_`
//! without the save-on-logout machinery, which is Phase 6).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use mangos_db::DbError;
use mangos_db::character_load::{ActionRow, CharacterLoad, HomebindRow};
use mangos_db::character_save::{CharacterSaveBundle, CharacterStatsRow};
use mangos_db::characters::{CharEnumRow, NameQueryRow};
use mangos_entities::player::{MAX_MONEY_AMOUNT, MaterializeError, MaterializeInputs, Player};
use mangos_guid::ObjectGuid;
use mangos_log::{Level, Log};
use mangos_map::{Map, MapConfig, MapEvent, WorldRng, WorldStores};
use mangos_movement::movement_info::MovementInfo;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::opcodes::opcode_meta;
use mangos_protocol::world::{WorldPacket, auth_response, opcode};
use mangos_world_data::enum_stores::EnumStores;
use mangos_world_data::player_info::RaceClassDbc;
use mangos_world_data::spawn_support::RespawnTimes;
use mangos_world_data::strings::MangosStrings;

use crate::ServerConfig;
use crate::account_data::{AccountDataSet, PER_CHARACTER_CACHE_MASK};
use crate::auth::AcceptedAuth;
use crate::channels::{ChannelActor, ChannelMgr, ChannelOut, not_on_packet, notify};
use crate::char_enum::build_char_enum;
use crate::char_queries::CharQueries;
use crate::character_save::{
    SaveContext, build_action_writes, build_save_bundle, homebind_row, rebaseline,
    rebaseline_actions,
};
use crate::chat::{
    build_chat_packet, build_player_not_found, build_wrong_faction, chat_msg, language,
    language_allowed, language_spell, parse_message_chat, yell_range,
};
use crate::group::{
    Group, GroupFx, GroupWrite, group_decline_packet, group_invite_packet, loot_method,
    member_status, party_op, party_result, party_result_packet, player_guid, remove_method,
};
use crate::guild::{
    Guild, GuildFx, GuildWrite, NewMemberStats, RosterOnline, command_error, command_result_packet,
    emblem_result, event_log, guild_decline_packet as guild_decline_notice, guild_event,
    guild_event_packet, guild_info_packet, guild_invite_packet as guild_invite_notice, rights,
    type_command,
};
use crate::mail::{
    BODY_MAX_BYTES, COD_MAX, MAILBOX_CAP, MailListItem, MailSpamLedger, MailTracker, MailWrite,
    POSTAGE, SUBJECT_MAX_BYTES, check_mask, is_visible as mail_is_visible, mail_list_packet,
    mail_result_packet, mail_times, mail_update, message_type, next_mail_time_packet,
    perm_enchant_id, received_mail_packet, response_result, response_type, spell_charges,
};
use crate::petition::{
    CHARTER_COST, CHARTER_DISPLAY_ID, Petition, PetitionService, PetitionWrite,
    SIGNATURES_HARD_LIMIT, is_valid_charter_name, sign_result,
};
use crate::player_login::{
    LoginGate, build_action_buttons, build_bindpoint_update, build_character_login_failed,
    build_friend_list, build_ignore_list, build_initial_spells, build_initialize_factions,
    build_login_set_time_speed, build_login_verify_world_at, build_set_rest_start,
    build_sys_message, build_tutorial_flags, evaluate_load, initial_spell_cooldowns, motd_lines,
    time_bit_fields_now,
};
use crate::queries::{
    build_creature_query_response, build_gameobject_query_response, build_item_query_response,
    build_name_query_response,
};
use crate::security;
use crate::session_core::{Dispatch, SessionCore};
use crate::social::{
    FRIEND_STATUS_ONLINE, FriendOnlineInfo, FriendsResult, SOCIAL_FLAG_FRIEND, SOCIAL_FLAG_IGNORED,
    SocialWrite, add_to_social_list, build_friend_status, early_add_result, has_flag,
    normalize_player_name, remove_from_social_list,
};
use crate::who::{WhoCandidate, WhoViewer, build_who_response, filter_who, parse_who_request};

/// C++ world tick (`WORLD_SLEEP_CONST`-class default, 50 ms).
pub const WORLD_TICK: Duration = Duration::from_millis(50);

/// Socket→world channel capacity (architecture section 3 target).
pub const WORLD_CHANNEL_CAPACITY: usize = 1024;

/// Freeze-detector replacement state: the world actor stamps
/// every tick; an external watchdog (in the binary) reads the
/// age and applies the recorded Phase 3 policy (error log
/// past 10 s, non-zero exit past 30 s). The C++ detector
/// measured the same world-loop liveness (section 3.3).
pub struct WorldHealth {
    epoch: Instant,
    last_tick_ms: AtomicU64,
    tick_count: AtomicU64,
    /// `GetActiveSessionCount` (sessions incl. queued) and its
    /// high-water mark, for the uptime row.
    active_sessions: AtomicU64,
    max_active_sessions: AtomicU64,
}

impl WorldHealth {
    fn new() -> WorldHealth {
        WorldHealth {
            epoch: Instant::now(),
            last_tick_ms: AtomicU64::new(0),
            tick_count: AtomicU64::new(0),
            active_sessions: AtomicU64::new(0),
            max_active_sessions: AtomicU64::new(0),
        }
    }

    fn stamp_tick(&self) {
        self.last_tick_ms
            .store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        self.tick_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Age of the most recent world tick.
    #[must_use]
    pub fn tick_age(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_tick_ms.load(Ordering::Relaxed)))
    }

    #[must_use]
    pub fn tick_count(&self) -> u64 {
        self.tick_count.load(Ordering::Relaxed)
    }

    fn set_active_sessions(&self, count: usize) {
        self.active_sessions.store(count as u64, Ordering::Relaxed);
        self.max_active_sessions
            .fetch_max(count as u64, Ordering::Relaxed);
    }

    #[must_use]
    pub fn active_sessions(&self) -> u32 {
        self.active_sessions.load(Ordering::Relaxed) as u32
    }

    #[must_use]
    pub fn max_active_sessions(&self) -> u32 {
        self.max_active_sessions.load(Ordering::Relaxed) as u32
    }
}

/// Stall thresholds for the watchdog (recorded decision).
pub const WATCHDOG_ERROR_AFTER: Duration = Duration::from_secs(10);
pub const WATCHDOG_EXIT_AFTER: Duration = Duration::from_secs(30);

/// Per-session outbound capacity. The C++ closes when the send
/// queue already holds more than 1024 packets at push time
/// (`WorldSocket.cpp:524`), i.e. 1025 may be queued.
pub const SESSION_SEND_CAPACITY: usize = 1025;

/// `CHEAT_ACTION_KICK` bit in the antiflood sanction mask.
const SANCTION_KICK: u32 = 0x08;

/// Monotonic session identifier (`g_sessionCounter`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

/// Movement ACK handler families (`MovementHandler.cpp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AckKind {
    Speed,
    FlagToggle,
    Root,
    SplineDone,
    KnockBack,
    Teleport,
}

fn ack_kind(handler_name: &str) -> Option<AckKind> {
    Some(match handler_name {
        "HandleForceSpeedChangeAckOpcodes" => AckKind::Speed,
        "HandleMovementFlagChangeToggleAck" => AckKind::FlagToggle,
        "HandleMoveRootAck" => AckKind::Root,
        "HandleMoveSplineDoneOpcode" => AckKind::SplineDone,
        "HandleMoveKnockBackAck" => AckKind::KnockBack,
        "HandleMoveTeleportAckOpcode" => AckKind::Teleport,
        _ => return None,
    })
}

/// Initial state for a freshly authenticated session.
pub struct SessionInit {
    pub auth: AcceptedAuth,
    pub remote_ip: String,
    /// Global account-data slots loaded during auth
    /// (`LoadGlobalAccountData`).
    pub account_data: AccountDataSet,
    /// Tutorial flags loaded during auth
    /// (`LoadTutorialsData`); sent by the login packet train.
    pub tutorials: [u32; 8],
}

/// Dependencies the world actor calls out to. Grouped so the
/// spawn signature stays stable as handlers land.
pub struct WorldDeps {
    pub enum_stores: Arc<EnumStores>,
    pub char_queries: Arc<dyn CharQueries>,
    pub realm_publisher: Arc<dyn RealmPublisher>,
    /// Static world stores for map simulation (Phase 4).
    pub world_stores: Arc<WorldStores>,
    /// Extracted `.map` directory (ground heights); `None`
    /// in tests that do not exercise terrain.
    pub maps_dir: Option<std::path::PathBuf>,
    /// Extracted `mmaps` directory (navmesh); `None` in
    /// tests that do not exercise pathfinding.
    pub mmaps_dir: Option<std::path::PathBuf>,
    pub race_class_dbc: Arc<RaceClassDbc>,
    /// Persisted respawn times loaded at boot.
    pub respawn_times: RespawnTimes,
    /// Phase 4 write-register statements (presence + respawn
    /// clears; owner-approved 2026-10-19).
    pub presence: Arc<dyn PresenceWriter>,
    pub character_saver: Arc<dyn CharacterSaver>,
    /// Characters-DB social service (Phase 5 Slice A).
    pub social: Arc<dyn SocialStore>,
    /// Characters-DB group service (Phase 5 Slice B.3).
    pub group: Arc<dyn GroupStore>,
    /// Boot-loaded groups (`ObjectMgr::LoadGroups`
    /// output), consumed at spawn.
    pub initial_groups: mangos_db::group::LoadedGroups,
    /// Characters-DB guild service (Phase 5 Slice B.4).
    pub guild: Arc<dyn GuildStore>,
    /// Boot-loaded guilds (`GuildMgr::LoadGuilds`
    /// output), consumed at spawn.
    pub initial_guilds: mangos_db::guild::LoadedGuilds,
    /// Petition service (Phase 5 Slice B.4b).
    pub petition: Arc<dyn PetitionStore>,
    /// Boot-loaded petitions (`GuildMgr::LoadPetitions`
    /// output), consumed at spawn.
    pub initial_petitions: mangos_db::petition::LoadedPetitions,
    /// Mail service (Phase 5 Slice B.5).
    pub mail: Arc<dyn MailStore>,
    /// Trade item-move service (Phase 5 Slice B.6).
    pub trade: Arc<dyn TradeStore>,
    /// Auction persistence (Phase 5 Slice B.7).
    pub auction: Arc<dyn AuctionStore>,
    /// Boot-loaded auctions (`LoadAuctions` output),
    /// consumed at spawn.
    pub initial_auctions: mangos_db::auction::LoadedAuctions,
    /// Boot-built GM command table (Slice C: static
    /// table + `command` overrides + RBAC).
    pub initial_commands: crate::commands::CommandTable,
    /// `playerbot` persistence (Slice E.6 director
    /// chance writes).
    pub playerbot: Arc<dyn PlayerbotStore>,
    /// Boot-loaded `playerbot` rows
    /// (`PlayerBotMgr::Load` output), consumed at spawn.
    pub initial_playerbots: Vec<mangos_db::playerbot::PlayerbotRow>,
    /// Guild-bot persistence (Slice E.6 level sync).
    pub guild_bot: Arc<dyn GuildBotStore>,
    /// Boot-loaded guild bots (`GuildBotMgr::Load`
    /// output), consumed at spawn.
    pub initial_guild_bots: mangos_db::guild_bots::LoadedGuildBots,
    /// AH bot persistence (`AuctionHouseBotMgr`).
    pub ah_bot: Arc<dyn AhBotStore>,
    /// Boot-loaded AH bot data, consumed at spawn.
    pub initial_ah_bot: InitialAhBot,
    /// Instance binding persistence.
    pub instance: Arc<dyn InstanceStore>,
    /// Boot-loaded instances + binds, consumed at spawn.
    pub initial_instances: mangos_db::instance::LoadedInstances,
}

/// Boot-loaded AH bot rows (`AuctionHouseBotMgr::Load`
/// DB half; empty when the world table is missing).
#[derive(Clone, Debug, Default)]
pub struct InitialAhBot {
    pub items: Vec<mangos_db::auction::AhBotItemRow>,
    pub sellers: Vec<(u32, String)>,
    pub last_refresh_s: i64,
}

/// One simulated map instance (`MapID`): continent
/// and non-instanceable maps run as instance 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct MapKey {
    pub map_id: u32,
    pub instance_id: u32,
}

impl MapKey {
    /// The shared (instance 0) key for a map.
    #[must_use]
    pub fn shared(map_id: u32) -> Self {
        Self {
            map_id,
            instance_id: 0,
        }
    }
}

/// Instance binding persistence
/// (`MapPersistentStateMgr` write side).
pub trait InstanceStore: Send + Sync {
    /// `DungeonPersistentState::SaveToDB`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_instance(
        &self,
        instance_id: u32,
        map_id: u32,
        reset_time: i64,
    ) -> Result<(), DbError>;

    /// `Player::BindToInstance` insert arm.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_character_bind(
        &self,
        guid_low: u32,
        instance_id: u32,
        permanent: bool,
    ) -> Result<(), DbError>;

    /// `Player::BindToInstance` update arm.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn update_character_bind(
        &self,
        guid_low: u32,
        old_instance_id: u32,
        new_instance_id: u32,
        permanent: bool,
    ) -> Result<(), DbError>;

    /// `Player::UnbindInstance` (non-unload path).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_character_bind(&self, guid_low: u32, instance_id: u32) -> Result<(), DbError>;

    /// `Group::BindToInstance` insert arm.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_group_bind(
        &self,
        leader_low: u32,
        instance_id: u32,
        permanent: bool,
    ) -> Result<(), DbError>;
}

/// `(instance_id, permanent)` bind by map.
type BindMap = std::collections::HashMap<u32, (u32, bool)>;

/// Live instance/binding state
/// (`MapPersistentStateManager` subset: allocation +
/// binds; reset scheduling lands later).
#[derive(Clone, Debug, Default)]
pub struct InstanceRuntime {
    next_instance_id: u32,
    /// instance id -> map id.
    instance_maps: std::collections::HashMap<u32, u32>,
    /// character guid low -> binds by map.
    char_binds: std::collections::HashMap<u32, BindMap>,
    /// group leader guid low -> binds by map.
    group_binds: std::collections::HashMap<u32, BindMap>,
}

impl InstanceRuntime {
    fn from_loaded(loaded: &mangos_db::instance::LoadedInstances) -> Self {
        let instance_maps: std::collections::HashMap<u32, u32> = loaded
            .instances
            .iter()
            .map(|row| (row.id, row.map_id))
            .collect();
        let mut char_binds: std::collections::HashMap<u32, BindMap> =
            std::collections::HashMap::new();
        for bind in &loaded.character_binds {
            if let Some(map_id) = instance_maps.get(&bind.instance_id) {
                char_binds
                    .entry(bind.guid_low)
                    .or_default()
                    .insert(*map_id, (bind.instance_id, bind.permanent));
            }
        }
        let mut group_binds: std::collections::HashMap<u32, BindMap> =
            std::collections::HashMap::new();
        for bind in &loaded.group_binds {
            if let Some(map_id) = instance_maps.get(&bind.instance_id) {
                group_binds
                    .entry(bind.guid_low)
                    .or_default()
                    .insert(*map_id, (bind.instance_id, bind.permanent));
            }
        }
        Self {
            next_instance_id: loaded.max_instance_id,
            instance_maps,
            char_binds,
            group_binds,
        }
    }

    fn allocate(&mut self, map_id: u32) -> u32 {
        self.next_instance_id = self.next_instance_id.saturating_add(1);
        self.instance_maps.insert(self.next_instance_id, map_id);
        self.next_instance_id
    }

    fn char_bind(&self, guid_low: u32, map_id: u32) -> Option<(u32, bool)> {
        self.char_binds
            .get(&guid_low)
            .and_then(|binds| binds.get(&map_id))
            .copied()
    }

    fn group_bind(&self, leader_low: u32, map_id: u32) -> Option<(u32, bool)> {
        self.group_binds
            .get(&leader_low)
            .and_then(|binds| binds.get(&map_id))
            .copied()
    }

    fn set_char_bind(&mut self, guid_low: u32, map_id: u32, bind: (u32, bool)) {
        self.char_binds
            .entry(guid_low)
            .or_default()
            .insert(map_id, bind);
    }

    fn set_group_bind(&mut self, leader_low: u32, map_id: u32, bind: (u32, bool)) {
        self.group_binds
            .entry(leader_low)
            .or_default()
            .insert(map_id, bind);
    }

    fn remove_char_bind(&mut self, guid_low: u32, map_id: u32) -> Option<(u32, bool)> {
        self.char_binds
            .get_mut(&guid_low)
            .and_then(|binds| binds.remove(&map_id))
    }
}

/// A player extracted from its old map mid-teleport
/// (`Player::ExecuteTeleportFar` between `SMSG_NEW_WORLD`
/// and the worldport ACK).
struct PendingTeleport {
    player: mangos_entities::player::Player,
    source_key: MapKey,
    key: MapKey,
    /// Fresh permanent personal bind granted at entry:
    /// `SMSG_INSTANCE_SAVE_CREATED` is sent by
    /// `InstanceMap::Add`, i.e. at worldport-ACK finish.
    granted: bool,
}

/// World+characters AH bot service.
pub trait AhBotStore: Send + Sync {
    /// Reloads items + refresh stamp + sellers
    /// (`AuctionHouseBotMgr::Load` DB half, incl.
    /// seller creation up to `seller_count`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn load_ah_bot(
        &self,
        seller_account_id: u32,
        seller_count: u32,
    ) -> Result<InitialAhBot, DbError>;

    /// Writes the refresh stamp (world DB).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn save_refresh_time(&self, now: i64) -> Result<(), DbError>;

    /// Creates one bot item instance; returns its guid.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn create_ahbot_item(&self, item_id: u32, count: u32) -> Result<u32, DbError>;

    /// Deletes one item instance.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_item_instance(&self, item_guid: u32) -> Result<(), DbError>;
}

/// Characters-DB guild-bot service.
pub trait GuildBotStore: Send + Sync {
    /// `SELECT level FROM characters`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn character_level(&self, guid_low: u32) -> Result<Option<u32>, DbError>;

    /// `UPDATE characters SET level`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_character_level(&self, guid_low: u32, level: u32) -> Result<(), DbError>;
}

/// Characters-DB `playerbot` service.
pub trait PlayerbotStore: Send + Sync {
    /// Applies director chance updates.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn update_chances(&self, chances: &[(u32, u32)]) -> Result<(), DbError>;
}

/// Characters-DB guild service: the `guild`-family
/// statements `Guild` issues plus the cached-character
/// read `AddMember` uses. Errors are logged by the
/// caller (C++ `PExecute` cannot observe failure).
pub trait GuildStore: Send + Sync {
    /// Applies one guild write.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn apply_guild_write(&self, write: GuildWrite) -> Result<(), DbError>;

    /// Cached character stats for a guild join.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn guild_candidate(
        &self,
        guid_low: u32,
    ) -> Result<Option<mangos_db::guild::GuildCandidate>, DbError>;
}

/// Petition service: the `petition`-family statements
/// plus the charter item rows and the reserved-name
/// check. Charter creation is synchronous because the
/// buy handler needs the free-slot verdict (and the
/// allocated item guid) before it charges.
pub trait PetitionStore: Send + Sync {
    /// Applies one petition write.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn apply_petition_write(&self, write: PetitionWrite) -> Result<(), DbError>;

    /// Creates the charter item rows; `None` when the
    /// backpack has no free slot.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn create_charter_item(&self, owner_low: u32, petition_id: u32)
    -> Result<Option<u32>, DbError>;

    /// Whether the character carries the item.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn player_has_item(&self, owner_low: u32, item_low: u32) -> Result<bool, DbError>;

    /// Destroys the charter item rows.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_charter_item(&self, owner_low: u32, item_low: u32) -> Result<(), DbError>;

    /// `ObjectMgr::IsReservedName`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn is_reserved_name(&self, name: &str) -> Result<bool, DbError>;
}

/// Mail service: the `mail`-family statements, item
/// text, and the inventory <-> mail item moves.
/// Sends are synchronous because the handler needs
/// the allocated mail id for the online-receiver
/// push; item stores because the reply depends on
/// the free-slot verdict.
pub trait MailStore: Send + Sync {
    /// Applies one mail write.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn apply_mail_write(&self, write: MailWrite) -> Result<(), DbError>;

    /// Inserts a mail and returns its id.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_mail(&self, mail: mangos_db::mail::NewMail) -> Result<u32, DbError>;

    /// Creates an `item_text` body and returns its id.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn create_item_text(&self, text: &str) -> Result<u32, DbError>;

    /// Reads an `item_text` body (empty when unknown).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn item_text(&self, item_text_id: u32) -> Result<String, DbError>;

    /// Receiver lookup by normalized character name.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn receiver_by_name(
        &self,
        name: &str,
    ) -> Result<Option<mangos_db::mail::MailReceiverInfo>, DbError>;

    /// Mailbox size for the send cap.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn mail_count(&self, receiver_low: u32) -> Result<u32, DbError>;

    /// Account id for a character guid.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn account_id_by_guid(&self, guid_low: u32) -> Result<Option<u32>, DbError>;

    /// Whether the account has a verified email
    /// (offline trial-restriction check); `None` when
    /// the account does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn account_email_verified(&self, account_id: u32) -> Result<Option<bool>, DbError>;

    /// One carried item by guid, with bag contents.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn inventory_item(
        &self,
        owner_low: u32,
        item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError>;

    /// Stores a mailed item into a free backpack
    /// slot; `false` when full.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn store_mailed_item(
        &self,
        owner_low: u32,
        item_low: u32,
        item_id: u32,
    ) -> Result<bool, DbError>;

    /// Creates the permanent letter item; `false`
    /// when the backpack is full.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn create_text_item(
        &self,
        owner_low: u32,
        creator_low: u32,
        item_text_id: u32,
    ) -> Result<bool, DbError>;
}

/// Characters-DB trade service: position reads and
/// the cross-player item move (`TradeHandler.cpp`).
pub trait TradeStore: Send + Sync {
    /// The item at one inventory position.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn item_at_pos(
        &self,
        owner_low: u32,
        bag: u32,
        slot: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError>;

    /// One carried item by guid (accept revalidation).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn inventory_item(
        &self,
        owner_low: u32,
        item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError>;

    /// Free backpack slots (`CanStoreItems` capacity).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn free_backpack_slots(&self, owner_low: u32) -> Result<u32, DbError>;

    /// First free backpack slot, if any.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn first_free_backpack_slot(&self, owner_low: u32) -> Result<Option<u32>, DbError>;

    /// Moves an item between two inventories.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    #[allow(clippy::too_many_arguments)]
    fn move_item(
        &self,
        from_low: u32,
        to_low: u32,
        item_low: u32,
        item_id: u32,
        to_slot: u32,
        gift_creator_low: Option<u32>,
    ) -> Result<(), DbError>;
}

/// Characters-DB auction service: the `auction`
/// statements the handlers issue. Errors are logged
/// by the caller (C++ `PExecute` cannot observe
/// failure).
pub trait AuctionStore: Send + Sync {
    /// `AuctionEntry::SaveToDB`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_auction(&self, auction: mangos_db::auction::StoredAuction) -> Result<(), DbError>;

    /// `AuctionEntry::DeleteFromDB`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_auction(&self, auction_id: u32) -> Result<(), DbError>;

    /// The place-bid buyer/bid update.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn update_bid(&self, auction_id: u32, buyer_guid: u32, last_bid: u32) -> Result<(), DbError>;

    /// Detaches a sold item from the seller's
    /// inventory rows.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn detach_inventory_item(&self, owner_low: u32, item_low: u32) -> Result<(), DbError>;

    /// Rewrites an item instance owner (won mail).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_item_owner(&self, item_low: u32, new_owner_low: u32) -> Result<(), DbError>;

    /// Destroys an item instance (receiver gone).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn delete_item_instance(&self, item_low: u32) -> Result<(), DbError>;

    /// Account id for a character guid (bid-own and
    /// mail-receiver existence checks).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn account_id_by_guid(&self, guid_low: u32) -> Result<Option<u32>, DbError>;
}

/// Characters-DB group service: the `groups` /
/// `group_member` statements `Group` issues. The world
/// actor picks the statement from its in-memory group
/// state, as C++ does; errors are logged by the caller
/// (C++ queues the writes with `PExecute` and cannot
/// observe failure).
pub trait GroupStore: Send + Sync {
    /// Applies one group write.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn apply_group_write(&self, write: GroupWrite) -> Result<(), DbError>;
}

/// Phase 6 character persistence (`Player::SaveToDB`,
/// `MasterPlayer::SaveActions`, and synthesized
/// homebind insertion). Implementations execute each
/// bundle atomically with respect to other calls on
/// the characters DB actor, mirroring the fork's
/// per-character delayed-transaction lane.
pub trait CharacterSaver: Send + Sync {
    /// Save the `Player::SaveToDB` bundle and, when
    /// present, its post-commit `_SaveStats` row.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn save_character(
        &self,
        bundle: CharacterSaveBundle,
        stats: Option<CharacterStatsRow>,
    ) -> Result<(), DbError>;

    /// `MasterPlayer::SaveActions`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn save_actions(
        &self,
        guid_low: u32,
        inserts: Vec<ActionRow>,
        updates: Vec<ActionRow>,
        deletes: Vec<u8>,
    ) -> Result<(), DbError>;

    /// Insert a homebind synthesized by `_LoadHomeBind`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn insert_homebind(&self, guid_low: u32, row: HomebindRow) -> Result<(), DbError>;

    /// `SetHomeBind` row update.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn update_homebind(&self, guid_low: u32, row: HomebindRow) -> Result<(), DbError>;
}

/// Characters-DB writes from the Phase 4 write register.
pub trait PresenceWriter: Send + Sync {
    /// `UPDATE characters SET online = ? WHERE guid = ?`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_character_online(&self, guid_low: u32, online: bool) -> Result<(), DbError>;

    /// `DELETE FROM creature_respawn WHERE guid = ? AND instance = ?`
    /// (`MapPersistentState::SaveCreatureRespawnTime` clear arm;
    /// the instance is the map's persistent-state id).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn clear_creature_respawn(&self, guid_low: u32, instance_id: u32) -> Result<(), DbError>;

    /// `REPLACE INTO creature_respawn (guid, respawn_time, instance, map)
    /// VALUES (?, ?, ?, ?)` — `SaveCreatureRespawnTime` future-time arm
    /// (owner-approved write register, 2026-10-19).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn store_creature_respawn(
        &self,
        guid_low: u32,
        respawn_time: u64,
        map_id: u32,
        instance_id: u32,
    ) -> Result<(), DbError>;

    /// `DELETE FROM gameobject_respawn WHERE guid = ? AND instance = ?`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn clear_gameobject_respawn(&self, guid_low: u32, instance_id: u32) -> Result<(), DbError>;
}

/// Characters-DB social service: the name lookup and the
/// four `character_social` statements `PlayerSocial` issues
/// (`SocialMgr.cpp:50-107`). The world actor picks the
/// statement from its in-memory social list, as C++ does;
/// errors are logged by the caller (C++ queues the writes
/// with `PExecute` and cannot observe failure).
pub trait SocialStore: Send + Sync {
    /// `ObjectMgr::GetPlayerDataByName` (guid + race).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn player_data_by_name(
        &self,
        name: &str,
    ) -> Result<Option<mangos_db::social::NamedPlayerData>, DbError>;

    /// Applies one `character_social` write.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn apply_social_write(
        &self,
        guid_low: u32,
        friend_low: u32,
        write: SocialWrite,
    ) -> Result<(), DbError>;
}

/// Login-DB writes the world performs at admission
/// (`AddSession_` population update).
pub trait RealmPublisher: Send + Sync {
    /// `UPDATE realmlist SET population = ? WHERE id = ?`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on driver failure.
    fn set_realm_population(&self, realm_id: u32, population: f32) -> Result<(), DbError>;
}

/// Billing flags for trial-restricted accounts
/// (`WorldSession.h:96-105`): TRIAL 0x04 | RESTRICTED 0x40.
const BILLING_FLAG_TRIAL: u8 = 0x04;
const BILLING_FLAG_RESTRICTED: u8 = 0x40;

/// World→socket control flow for one session.
#[derive(Debug)]
pub enum SessionControl {
    Packet(WorldPacket),
    Disconnect,
}

/// Socket→world messages, processed in arrival order.
pub enum WorldMessage {
    AddSession {
        id: SessionId,
        init: Box<SessionInit>,
        control: mpsc::Sender<SessionControl>,
    },
    Packet {
        id: SessionId,
        packet: WorldPacket,
    },
    /// `WorldSession::SetLatency` from the socket's ping handler.
    SetLatency {
        id: SessionId,
        latency: u32,
    },
    RemoveSession {
        id: SessionId,
    },
    /// Async char-enum result (the `chrHandler` callback,
    /// delivered by account id like in C++ — the session may
    /// have been replaced since the query started).
    CharEnumReady {
        account_id: u32,
        rows: Result<Vec<CharEnumRow>, DbError>,
    },
    /// Async character-load result (the `LoginQueryHolder`
    /// callback). Keyed by session: a session that logged out
    /// mid-load must not be resurrected.
    PlayerLoginReady {
        id: SessionId,
        guid_low: u32,
        load: Result<Option<Box<CharacterLoad>>, DbError>,
    },
    /// Async `CMSG_NAME_QUERY` fallback result for a GUID
    /// with no live player in any map.
    NameQueryReady {
        id: SessionId,
        guid_low: u32,
        row: Result<Option<NameQueryRow>, DbError>,
    },
}

/// One headless bot player (Slice D `BotSystem`
/// minimum: session + 500 ms behaviour tick; the
/// manager singletons attach in later slices).
#[allow(clippy::struct_excessive_bools)]
struct BotState {
    session_id: SessionId,
    guid_low: u32,
    /// Outbound sink drained every actor tick (the
    /// future `OnPacketReceived` tap).
    control_rx: mpsc::Receiver<SessionControl>,
    next_tick: Instant,
    combat_behaviors: mangos_bots::BehaviorSet,
    ooc_behaviors: mangos_bots::BehaviorSet,
    /// Raid icons this bot must CC (`m_marksToCC`).
    cc_marks: Vec<u8>,
    /// Last totem cast per slot (presence
    /// approximation until totem tracking exists).
    totem_cast_at: [Option<Instant>; 4],
    /// `m_formationSlotIndex` (-1 in C++).
    formation_slot: Option<u32>,
    /// First in-combat tick (opener delay).
    combat_engaged_at: Option<Instant>,
    /// Formation leader (`.bot add` invoker);
    /// the formation manager replaces this in E.4.
    leader_low: Option<u32>,
    role: mangos_bots::BotRole,
    class_id: u8,
    kit: mangos_bots::SpellKit,
    /// Classified spellbook (`PopulateSpellData`
    /// output), resolved on world entry.
    spells: std::sync::Arc<mangos_bots::ClassSpells>,
    /// `.partybot debug` state; trace goes to the
    /// log like the fork ("Check Server.log").
    debug: bool,
    /// Explicit role override (`.partybot setrole`);
    /// auto-assign only fills `role` when unset.
    role_locked: bool,
    /// `m_wipeReacted` latch.
    wipe_reacted: bool,
    /// `m_lastCombatTarget` (boss-kill detection).
    last_combat_target: Option<ObjectGuid>,
    /// `m_marksToFocus` raid icons.
    focus_marks: Vec<u8>,
    /// Tank pull-announce internal cooldown end.
    pull_chat_until: Option<Instant>,
    /// Next director-assignment wander move.
    wander_at: Option<Instant>,
    /// Set once the player is in a map.
    in_world: bool,
}

struct SessionEntry {
    account_id: u32,
    auth: AcceptedAuth,
    remote_ip: String,
    created: Instant,
    latency: u32,
    core: SessionCore,
    queue: VecDeque<WorldPacket>,
    control: mpsc::Sender<SessionControl>,
    #[allow(dead_code)] // read by the player-login packet train
    account_data: AccountDataSet,
    #[allow(dead_code)] // sent by the player-login packet train
    tutorials: [u32; 8],
    /// `m_charactersCount` from the last enum.
    characters_count: u8,
    /// `m_characterMaxLevel` from the last enum.
    character_max_level: u32,
    in_queue: bool,
    /// `PlayerLoading()` while the async load runs (also true
    /// transiently during logout in C++; Phase 3 has no logout
    /// packet yet).
    player_loading: bool,
    /// The entered character, once login completed.
    player_guid: Option<u32>,
    /// Wall-clock anchor for `Player::Update` played
    /// time accumulation (whole seconds).
    played_last_tick: Instant,
    /// Last level observed by played-time accounting;
    /// a change mirrors `GiveLevel` resetting
    /// `PLAYED_TIME_LEVEL`.
    played_level: u32,
    /// Next periodic `SaveToDB` instant, randomized on
    /// login like fork `m_nextSave`.
    next_player_save: Option<Instant>,
    /// Last in-map zone (`GetCachedZoneId`), retained
    /// for forced saves while a far teleport is pending.
    cached_zone: u32,
    /// The read-only snapshot backing the empty-world session.
    #[allow(dead_code)] // consumed as handlers grow past stubs
    character_load: Option<CharacterLoad>,
    /// `MasterPlayer::m_chatTag` (0 none, 1 AFK, 2 DND;
    /// GM tag 3 has no state yet).
    chat_tag: u8,
    /// `MasterPlayer::afkMsg` / `dndMsg` auto-replies.
    afk_msg: String,
    dnd_msg: String,
    /// `PLAYER_EXTRA_ACCEPT_WHISPERS` (set by the GM
    /// whispers toggle; Slice C will drive it).
    accept_whispers: bool,
    /// `m_allowedWhispers`: player guid lows this session
    /// has whispered first, for the staff whisper gate.
    allowed_whisperers: HashSet<u32>,
    /// Unread/pending-delivery mail tracking
    /// (`MasterPlayer` mail fields).
    mail_tracker: crate::mail::MailTracker,
}

/// `WorldSession::SendPacket` queue-full policy: `false` means
/// close the session.
fn send_packet(entry: &SessionEntry, packet: WorldPacket) -> bool {
    entry
        .control
        .try_send(SessionControl::Packet(packet))
        .is_ok()
}

struct World {
    config: ServerConfig,
    log: Arc<Log>,
    #[allow(dead_code)] // read by handlers landing in later slices
    strings: MangosStrings,
    deps: WorldDeps,
    /// Simulated maps by (map id, instance id);
    /// continents run as instance 0.
    maps: HashMap<MapKey, Map>,
    /// Player guid low -> map key.
    player_map: HashMap<u32, MapKey>,
    /// Instance allocation + binds (Phase 6 Slice 0).
    instances: InstanceRuntime,
    /// Far teleports waiting for `MSG_MOVE_WORLDPORT_ACK`
    /// (bot sessions finish inline).
    pending_teleports: HashMap<SessionId, PendingTeleport>,
    /// Per-map area-trigger script cooldowns
    /// (`m_areaTriggerCooldowns`, unix seconds).
    area_trigger_cooldowns: HashMap<(MapKey, u32), i64>,
    /// Player guid low -> session.
    player_session: HashMap<u32, SessionId>,
    /// Boot instant for the server ms clock.
    started: Instant,
    last_tick: Instant,
    sessions: HashMap<SessionId, SessionEntry>,
    by_account: HashMap<u32, SessionId>,
    /// Login queue, front first (`m_QueuedSessions`).
    login_queue: VecDeque<SessionId>,
    /// Last disconnect per account (queue grace period).
    last_logout: HashMap<u32, Instant>,
    /// Own sender, for async DB callbacks. Set at spawn.
    self_tx: Option<mpsc::Sender<WorldMessage>>,
    health: Arc<WorldHealth>,
    /// Per-faction channel managers (`channelMgr(team)`).
    channels_alliance: ChannelMgr,
    channels_horde: ChannelMgr,
    /// All live groups by internal key (`ObjectMgr`
    /// group map, including uncreated invite shells).
    groups: HashMap<u64, Group>,
    /// Persisted group id -> internal key.
    group_by_db_id: HashMap<u32, u64>,
    /// Member guid low -> group key (`Player::GetGroup`).
    player_group: HashMap<u32, u64>,
    /// Invitee guid low -> group key
    /// (`Player::GetGroupInvite`).
    player_group_invite: HashMap<u32, u64>,
    next_group_key: u64,
    /// `ObjectMgr::GenerateGroupId` sequence.
    next_group_db_id: u32,
    /// Dice for `MSG_RANDOM_ROLL` (`urand`).
    roll_rng: WorldRng,
    /// All live guilds by persisted id (`GuildMgr` map).
    guilds: HashMap<u32, Guild>,
    /// Member guid low -> guild id (`Player::GetGuildId`).
    player_guild: HashMap<u32, u32>,
    /// Invitee guid low -> guild id
    /// (`Player::GetGuildIdInvited`).
    player_guild_invite: HashMap<u32, u32>,
    /// `GuildMgr::GenerateGuildId` sequence.
    next_guild_id: u32,
    /// Live guild charter petitions (`GuildMgr`
    /// petition map).
    petitions: PetitionService,
    /// Account mail-spam ledger
    /// (`AccountPersistentData`).
    mail_spam: MailSpamLedger,
    /// Live trades by owner guid low (`Player::m_trade`).
    trades: HashMap<u32, crate::trade::TradeData>,
    /// DBC house id -> auction object index
    /// (`m_mAuctionHouses`).
    auction_house_map: HashMap<u32, usize>,
    /// Live auctions per house object, by auction id
    /// (`AuctionHouseObject::AuctionsMap`).
    auction_houses: Vec<std::collections::BTreeMap<u32, crate::auction::AuctionEntry>>,
    /// Auctioned item rows by item guid (`mAitems`).
    auction_items: HashMap<u32, mangos_db::character_load::ItemInstanceRow>,
    /// `ObjectMgr::GenerateAuctionID` sequence.
    next_auction_id: u32,
    /// GM command table + RBAC (`ChatHandler` statics).
    commands: crate::commands::CommandTable,
    /// Headless bot players by guid low (Slice D
    /// `BotSystem` minimum; the eight C++ singletons
    /// attach here as slices land).
    bots: HashMap<u32, BotState>,
    /// Bot session id -> guid low.
    bot_sessions: HashMap<SessionId, u32>,
    /// Explicit `.partyrole` assignments by guid low
    /// (`s_playerRoleAssignments`).
    player_roles: HashMap<u32, mangos_bots::BotRole>,
    /// Party formations by leader (`BotFormationMgr`).
    formations: mangos_bots::FormationManager,
    /// Ambient bot chat (`BotChatManager`).
    bot_chat: mangos_bots::chat::ChatCore,
    /// Whisper bot services (`BotServiceMgr`).
    bot_services: mangos_bots::services::ServiceCore,
    /// World-presence director (`BotDirector`).
    bot_director: mangos_bots::director::DirectorCore,
    /// Random-bot population (`PlayerBotMgr`).
    bot_population: mangos_bots::population::PopulationCore,
    /// Population logins queued at spawn.
    pending_bot_logins: Vec<u32>,
    /// `.partybot load` attachments awaiting the
    /// bot's world entry: (bot, leader).
    pending_party_attach: Vec<(u32, u32)>,
    /// Guild bot roster/presence (`GuildBotMgr`).
    guild_bots: mangos_bots::guild_bots::GuildBotCore,
    /// Auction house bot (`AuctionHouseBotMgr`).
    ah_bot: mangos_bots::ah_bot::AhBotCore,
    /// Resolved AH bot house (entry + house index).
    ah_bot_house: Option<(mangos_world_data::auction_house::AuctionHouseEntry, usize)>,
    /// Raid-assembly attachments awaiting world
    /// entry: (bot, leader, roster role).
    pending_raid_attach: Vec<(u32, u32, mangos_bots::BotRole)>,
    /// One-shot guild-bot bootstrap (personalities
    /// + guild population) ran.
    guild_bots_bootstrapped: bool,
    /// Bot session id sequence (kept far from the
    /// socket session counter range).
    next_bot_session: u64,
}

/// Auction boot state (`LoadAuctionHouses` +
/// `LoadAuctions` placement into house objects).
#[allow(clippy::type_complexity)]
fn build_auction_state(
    deps: &mut WorldDeps,
    config: &ServerConfig,
) -> (
    HashMap<u32, usize>,
    Vec<std::collections::BTreeMap<u32, crate::auction::AuctionEntry>>,
    HashMap<u32, mangos_db::character_load::ItemInstanceRow>,
    u32,
) {
    let initial_auctions = std::mem::take(&mut deps.initial_auctions);
    let (house_map, house_count) = crate::auction::build_house_map(
        &deps.world_stores.auction_houses,
        config.allow_two_side_interaction_auction,
        config.unlinked_auction_houses,
        config.wow_patch,
    );
    let mut houses: Vec<std::collections::BTreeMap<u32, crate::auction::AuctionEntry>> = (0
        ..house_count.max(1))
        .map(|_| std::collections::BTreeMap::default())
        .collect();
    let accounts: HashMap<u32, u32> = initial_auctions.owner_accounts.iter().copied().collect();
    for stored in &initial_auctions.auctions {
        let Some(house) = deps.world_stores.auction_houses.get(stored.house_id) else {
            continue;
        };
        let Some(&index) = house_map.get(&stored.house_id) else {
            continue;
        };
        let entry = crate::auction::AuctionEntry::from_stored(
            stored,
            house,
            accounts.get(&stored.id).copied().unwrap_or(0),
        );
        houses[index].insert(entry.id, entry);
    }
    let items = initial_auctions
        .items
        .into_iter()
        .map(|item| (item.item_guid, item))
        .collect();
    (
        house_map,
        houses,
        items,
        initial_auctions.next_auction_id.max(1),
    )
}

/// Spawn the world actor task. Dropping every sender ends it.
fn guild_bot_roster(
    rows: Vec<mangos_db::guild_bots::GuildBotRosterRow>,
) -> Vec<mangos_bots::guild_bots::GuildBotInfo> {
    use mangos_bots::guild_bots::{GearTier, GuildBotInfo, GuildBotRole};
    rows.into_iter()
        .map(|row| GuildBotInfo {
            guid: row.guid,
            name: row.name,
            race: row.race,
            class_id: row.class,
            spec: row.spec,
            role: match row.role.as_str() {
                "tank" => GuildBotRole::Tank,
                "healer" => GuildBotRole::Healer,
                _ => GuildBotRole::Dps,
            },
            raid_assigned: row.raid_assigned,
            active: row.active,
            gear_score: row.gear_score,
            gear_tier: match row.gear_tier.as_str() {
                "fresh_60" => GearTier::Fresh60,
                "pre_raid_bis" => GearTier::PreRaidBis,
                _ => GearTier::DungeonGeared,
            },
            has_loot_upgrades: false,
            online: false,
            personality: mangos_bots::guild_bots::personality_from_str(&row.personality),
        })
        .collect()
}

fn guild_bot_weights(
    rows: Vec<(mangos_db::guild_bots::GuildBotWeightsRow, String)>,
) -> Vec<(u8, String, mangos_bots::guild_bots::SpecWeights)> {
    use mangos_bots::guild_bots::SpecWeights;
    rows.into_iter()
        .map(|(row, spec)| {
            let w = row.weights;
            (
                row.class,
                spec,
                SpecWeights {
                    stamina: w[0],
                    strength: w[1],
                    agility: w[2],
                    intellect: w[3],
                    spirit: w[4],
                    attack_power: w[5],
                    spell_power: w[6],
                    healing_power: w[7],
                    hit_rating: w[8],
                    crit_rating: w[9],
                    defense: w[10],
                    dodge: w[11],
                    parry: w[12],
                    block: w[13],
                    mp5: w[14],
                },
            )
        })
        .collect()
}

fn guild_bot_core(
    config: &ServerConfig,
    deps: &mut WorldDeps,
    log: &Log,
) -> mangos_bots::guild_bots::GuildBotCore {
    let loaded = std::mem::take(&mut deps.initial_guild_bots);
    if !config.guild_bot.enabled {
        return mangos_bots::guild_bots::GuildBotCore::load(
            config.guild_bot.clone(),
            Vec::new(),
            Vec::new(),
            0,
        );
    }
    let guild_id = if let Some(id) = loaded.bot_guild_id {
        log.out(
            Level::Minimal,
            &format!("[GuildBotMgr] Bots already in guild {id}."),
        );
        id
    } else if let Some(id) = loaded.player_guild_id {
        log.out(
            Level::Minimal,
            &format!("[GuildBotMgr] Found player guild {id}. Adding bots..."),
        );
        id
    } else {
        log.out(
            Level::Minimal,
            "[GuildBotMgr] No player guild found. Bots will join when a player creates a guild.",
        );
        0
    };
    let core = mangos_bots::guild_bots::GuildBotCore::load(
        config.guild_bot.clone(),
        guild_bot_roster(loaded.roster),
        guild_bot_weights(loaded.weights),
        guild_id,
    );
    if core.roster().is_empty() {
        log.out(
            Level::Error,
            "[GuildBotMgr] No guild bots found in guild_bot_roster!",
        );
    }
    log.out(
        Level::Minimal,
        &format!(
            "[GuildBotMgr] Loaded {} guild bots, {} spec weight entries. Presence: {} (peak={}, offpeak={}, tz={})",
            core.roster().len(),
            core.weights_count(),
            if core.config.presence_enabled { "ON" } else { "OFF" },
            core.config.presence_peak_count,
            core.config.presence_off_peak_count,
            core.config.presence_timezone_offset,
        ),
    );
    core
}

fn ah_bot_core(
    config: &ServerConfig,
    deps: &mut WorldDeps,
    auction_house_map: &HashMap<u32, usize>,
    log: &Log,
) -> (
    mangos_bots::ah_bot::AhBotCore,
    Option<(mangos_world_data::auction_house::AuctionHouseEntry, usize)>,
) {
    let initial = std::mem::take(&mut deps.initial_ah_bot);
    log.out(
        Level::Minimal,
        &format!(">> Loaded {} AuctionHouseBot items", initial.items.len()),
    );
    // `GetAuctionHouseEntry(ahfid)`: faction template
    // id -> house id -> DBC row.
    let house_id = crate::auction::house_id_for_faction_template(config.ah_bot.ah_fid);
    let house = deps
        .world_stores
        .auction_houses
        .entries
        .iter()
        .find(|entry| entry.house_id == house_id)
        .copied();
    let resolved = if let Some(entry) = house {
        auction_house_map
            .get(&entry.house_id)
            .map(|index| (entry, *index))
    } else {
        log.out(
            Level::Minimal,
            &format!(
                "AHBot::Load() : No auction house for faction {}.",
                config.ah_bot.ah_fid
            ),
        );
        None
    };
    let items: Vec<mangos_bots::ah_bot::AhBotEntry> = initial
        .items
        .into_iter()
        .map(|row| mangos_bots::ah_bot::AhBotEntry {
            item: row.item,
            min_stack: row.min_stack,
            max_stack: row.max_stack,
            bid: row.bid,
            buyout: row.buyout,
            weight: row.weight,
            category: mangos_bots::ah_bot::parse_category(&row.category),
            price_variance_pct: row.price_variance_pct,
        })
        .collect();
    let sellers: Vec<mangos_bots::ah_bot::SellerIdentity> = initial
        .sellers
        .into_iter()
        .map(|(guid, name)| mangos_bots::ah_bot::SellerIdentity {
            character_guid: guid,
            name,
            seller_type: mangos_bots::ah_bot::SellerType::Casual,
        })
        .collect();
    let core = mangos_bots::ah_bot::AhBotCore::load(
        config.ah_bot.clone(),
        items,
        sellers,
        initial.last_refresh_s,
        resolved.is_some(),
    );
    for (name, count, weight) in core.category_stats() {
        log.out(
            Level::Minimal,
            &format!("AHBot: Category '{name}' has {count} items, total weight {weight:.1}"),
        );
    }
    log.out(
        Level::Minimal,
        &format!("AHBot: {} seller identities loaded.", core.seller_count()),
    );
    (core, resolved)
}

fn bot_spawn_cores(
    config: &ServerConfig,
    deps: &mut WorldDeps,
    log: &Log,
) -> (
    mangos_bots::chat::ChatCore,
    mangos_bots::services::ServiceCore,
    mangos_bots::director::DirectorCore,
    mangos_bots::population::PopulationCore,
    Vec<u32>,
    mangos_bots::guild_bots::GuildBotCore,
) {
    let (chat, services) = bot_cores(config);
    let (director, population, pending) = bot_population_cores(config, deps, log);
    let guild = guild_bot_core(config, deps, log);
    (chat, services, director, population, pending, guild)
}

fn bot_population_cores(
    config: &ServerConfig,
    deps: &mut WorldDeps,
    log: &Log,
) -> (
    mangos_bots::director::DirectorCore,
    mangos_bots::population::PopulationCore,
    Vec<u32>,
) {
    use mangos_spells::SpellRng as _;
    let rows = std::mem::take(&mut deps.initial_playerbots);
    let total = rows.len();
    let mut population = mangos_bots::population::PopulationCore::load(
        config.bot_population.clone(),
        rows.into_iter()
            .map(|row| (row.char_guid, row.chance, row.ai))
            .collect(),
    );
    if total == 0 {
        log.out(Level::Minimal, "Table `playerbot` is empty.");
    } else {
        log.out(Level::Minimal, &format!("{total} bots loaded"));
    }
    if population.config.debug {
        log.out(
            Level::Minimal,
            &format!(
                "[PlayerBotMgr] Between {} and {} bots online",
                population.config.min_bots, population.config.max_bots
            ),
        );
    }
    let mut rng = WorldRng::from_entropy();
    let mut pick = |lo: u32, hi: u32| {
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        {
            rng.irand(lo as i32, hi as i32) as u32
        }
    };
    let pending = population
        .initial_bots(&mut pick)
        .into_iter()
        .filter_map(|action| match action {
            mangos_bots::population::PopulationAction::Login(guid) => Some(guid),
            mangos_bots::population::PopulationAction::Logout(_) => None,
        })
        .collect();
    (
        mangos_bots::director::DirectorCore::new(
            config.bot_director_enable,
            u64::from(config.bot_director_interval_ms),
        ),
        population,
        pending,
    )
}

fn bot_cores(
    config: &ServerConfig,
) -> (
    mangos_bots::chat::ChatCore,
    mangos_bots::services::ServiceCore,
) {
    (
        mangos_bots::chat::ChatCore::new(config.bot_chat.clone()),
        mangos_bots::services::ServiceCore::new(
            config.bot_service_enable,
            config.bot_service_buy_budget_gold,
        ),
    )
}

// World-literal construction; length is fields, not logic.
#[allow(clippy::too_many_lines)]
pub fn spawn_world(
    config: ServerConfig,
    log: Arc<Log>,
    strings: MangosStrings,
    deps: WorldDeps,
) -> (mpsc::Sender<WorldMessage>, JoinHandle<()>, Arc<WorldHealth>) {
    let (tx, rx) = mpsc::channel(WORLD_CHANNEL_CAPACITY);
    let health = Arc::new(WorldHealth::new());
    let channels_alliance = ChannelMgr::new_with_defaults(&deps.world_stores.chat_channels);
    let channels_horde = ChannelMgr::new_with_defaults(&deps.world_stores.chat_channels);
    let mut deps = deps;
    let initial_groups = std::mem::take(&mut deps.initial_groups);
    let mut groups: HashMap<u64, Group> = HashMap::new();
    let mut group_by_db_id: HashMap<u32, u64> = HashMap::new();
    let mut player_group: HashMap<u32, u64> = HashMap::new();
    let mut next_group_key: u64 = 1;
    for (stored, leader_name, members) in &initial_groups.groups {
        let key = next_group_key;
        next_group_key += 1;
        for member in members {
            player_group.insert(member.member_low, key);
        }
        group_by_db_id.insert(stored.group_id, key);
        groups.insert(
            key,
            Group::from_stored(stored, leader_name.clone(), members),
        );
    }
    let next_group_db_id = initial_groups.next_group_id.max(1);
    let initial_guilds = std::mem::take(&mut deps.initial_guilds);
    let mut guilds: HashMap<u32, Guild> = HashMap::new();
    let mut player_guild: HashMap<u32, u32> = HashMap::new();
    {
        let boot_rank_names = default_guild_rank_names(&strings, 0);
        for loaded in &initial_guilds.guilds {
            let (live, fx) = Guild::from_stored(loaded, &boot_rank_names);
            for write in fx.writes {
                if let Err(e) = deps.guild.apply_guild_write(write) {
                    log.out(Level::Error, &format!("guild boot write failed: {e}"));
                }
            }
            if let Some(guild) = live {
                for low in guild.members().keys() {
                    player_guild.insert(*low, guild.id());
                }
                guilds.insert(guild.id(), guild);
            }
        }
    }
    let next_guild_id = initial_guilds.next_guild_id.max(1);
    let initial_petitions = std::mem::take(&mut deps.initial_petitions);
    let petitions = PetitionService::from_loaded(&initial_petitions);
    let (auction_house_map, auction_houses, auction_items, next_auction_id) =
        build_auction_state(&mut deps, &config);
    let (ah_bot_core, ah_bot_house) = ah_bot_core(&config, &mut deps, &auction_house_map, &log);
    let commands = std::mem::take(&mut deps.initial_commands);
    let (
        bot_chat_core,
        bot_service_core,
        bot_director_core,
        bot_population_core,
        pending_bot_logins,
        guild_bot_core,
    ) = bot_spawn_cores(&config, &mut deps, &log);
    let instances = InstanceRuntime::from_loaded(&deps.initial_instances);
    let mut world = World {
        config,
        log,
        strings,
        deps,
        channels_alliance,
        channels_horde,
        groups,
        group_by_db_id,
        player_group,
        player_group_invite: HashMap::new(),
        next_group_key,
        next_group_db_id,
        roll_rng: WorldRng::from_entropy(),
        guilds,
        player_guild,
        player_guild_invite: HashMap::new(),
        next_guild_id,
        petitions,
        mail_spam: MailSpamLedger::default(),
        trades: HashMap::new(),
        auction_house_map,
        auction_houses,
        auction_items,
        next_auction_id,
        commands,
        bots: HashMap::new(),
        bot_sessions: HashMap::new(),
        player_roles: HashMap::new(),
        formations: mangos_bots::FormationManager::new(),
        bot_chat: bot_chat_core,
        bot_services: bot_service_core,
        bot_director: bot_director_core,
        bot_population: bot_population_core,
        pending_bot_logins,
        pending_party_attach: Vec::new(),
        guild_bots: guild_bot_core,
        ah_bot: ah_bot_core,
        ah_bot_house,
        pending_raid_attach: Vec::new(),
        guild_bots_bootstrapped: false,
        next_bot_session: 1 << 62,
        maps: HashMap::new(),
        instances,
        pending_teleports: HashMap::new(),
        area_trigger_cooldowns: HashMap::new(),
        player_map: HashMap::new(),
        player_session: HashMap::new(),
        started: Instant::now(),
        last_tick: Instant::now(),
        sessions: HashMap::new(),
        by_account: HashMap::new(),
        login_queue: VecDeque::new(),
        last_logout: HashMap::new(),
        self_tx: None,
        health: Arc::clone(&health),
    };
    world.self_tx = Some(tx.clone());
    let handle = tokio::spawn(world.run(rx));
    (tx, handle, health)
}

/// The five `CreateDefaultGuildRanks` names
/// (`LANG_GUILD_MASTER` 811..815) in one locale, with
/// the English text as fallback for missing rows.
fn default_guild_rank_names(strings: &MangosStrings, locale: u8) -> [String; 5] {
    const FALLBACK: [&str; 5] = ["Guild Master", "Officer", "Veteran", "Member", "Initiate"];
    std::array::from_fn(|index| {
        let entry = 811 + i32::try_from(index).unwrap_or(0);
        strings
            .get(entry)
            .map(|text| text.text(locale))
            .filter(|text| !text.is_empty())
            .map_or_else(|| FALLBACK[index].to_owned(), str::to_owned)
    })
}

/// Appends an `ObjectGuid` in plain wire form (the
/// C++ `ByteBuffer << ObjectGuid` writes the raw
/// u64; only `PackedGuid` streams packed, which the
/// petition/mail handlers do not use).
fn write_guid(writer: &mut PacketWriter, guid: ObjectGuid) {
    writer.write_u64_le(guid.raw());
}

/// `SMSG_PETITION_SHOW_SIGNATURES` + `BuildSignatureData`.
fn show_signatures_packet(
    item_guid: ObjectGuid,
    owner_guid: ObjectGuid,
    petition: &Petition,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, item_guid);
    write_guid(&mut writer, owner_guid);
    writer.write_u32_le(petition.id);
    writer.write_u8(petition.signature_count() as u8);
    for signature in &petition.signatures {
        write_guid(&mut writer, player_guid(signature.player_low));
        writer.write_u32_le(0);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_PETITION_SHOW_SIGNATURES);
    packet.take_writer(writer);
    packet
}

/// `SMSG_PETITION_SIGN_RESULTS`.
fn sign_results_packet(item_guid: ObjectGuid, signer: ObjectGuid, result: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, item_guid);
    write_guid(&mut writer, signer);
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_PETITION_SIGN_RESULTS);
    packet.take_writer(writer);
    packet
}

/// `SMSG_TURN_IN_PETITION_RESULTS`.
fn turn_in_results_packet(result: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_TURN_IN_PETITION_RESULTS);
    packet.take_writer(writer);
    packet
}

/// `Player::SendBuyError` for the guild charter
/// (0 = can't find item, 2 = not enough money).
fn buy_failed_packet(creature_guid: ObjectGuid, error: u8) -> WorldPacket {
    let mut writer = PacketWriter::new();
    write_guid(&mut writer, creature_guid);
    writer.write_u32_le(mangos_db::petition::GUILD_CHARTER_ENTRY);
    writer.write_u8(error);
    let mut packet = WorldPacket::new(opcode_id::SMSG_BUY_FAILED);
    packet.take_writer(writer);
    packet
}

/// A resolved mail receiver (online or offline).
struct MailReceiver {
    low: u32,
    session: Option<SessionId>,
    race: u8,
    account: u32,
}

/// Parsed `CMSG_SEND_MAIL` fields passed between the
/// handler halves.
struct SendMailRequest {
    subject: String,
    body: String,
    money: u32,
    cod: u32,
    item_raw: u64,
}

/// One outgoing mail's variable fields
/// (`MailDraft` + `MailSender` flattened).
struct OutgoingMail {
    subject: String,
    item_text_id: u32,
    mail_template_id: u16,
    stationery: u8,
    message_type: u8,
    money: u32,
    cod: u32,
    checked: u32,
    deliver_delay_secs: u32,
}

/// `SendEquipError(EQUIP_ERR_INVENTORY_FULL, ...)` (50).
fn inventory_full_packet() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(50);
    write_guid(&mut writer, ObjectGuid::EMPTY);
    write_guid(&mut writer, ObjectGuid::EMPTY);
    writer.write_u8(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_INVENTORY_CHANGE_FAILURE);
    packet.take_writer(writer);
    packet
}

impl World {
    async fn run(mut self, mut rx: mpsc::Receiver<WorldMessage>) {
        let mut tick = tokio::time::interval(WORLD_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = rx.recv() => {
                    if let Some(message) = message {
                        self.handle(message);
                    } else {
                        self.save_all_players();
                        break;
                    }
                }
                _ = tick.tick() => {
                    self.health.stamp_tick();
                    self.update_sessions();
                    self.update_auctions();
                    self.update_ah_bot(false);
                    self.update_bots();
                    self.update_maps();
                    self.update_player_saves();
                }
            }
        }
    }

    fn handle(&mut self, message: WorldMessage) {
        match message {
            WorldMessage::AddSession { id, init, control } => {
                let account_id = init.auth.account_id;
                if let Some(old_id) = self.by_account.get(&account_id).copied() {
                    if old_id != id {
                        self.disconnect(old_id);
                    }
                }
                self.by_account.insert(account_id, id);
                self.sessions.insert(
                    id,
                    SessionEntry {
                        account_id,
                        auth: init.auth,
                        remote_ip: init.remote_ip,
                        created: Instant::now(),
                        latency: 0,
                        core: SessionCore::new(),
                        queue: VecDeque::new(),
                        control,
                        account_data: init.account_data,
                        tutorials: init.tutorials,
                        characters_count: 10, // C++ session default
                        character_max_level: 0,
                        in_queue: false,
                        player_loading: false,
                        player_guid: None,
                        played_last_tick: Instant::now(),
                        played_level: 0,
                        next_player_save: None,
                        cached_zone: 0,
                        character_load: None,
                        chat_tag: 0,
                        afk_msg: String::new(),
                        dnd_msg: String::new(),
                        accept_whispers: false,
                        allowed_whisperers: HashSet::new(),
                        mail_tracker: crate::mail::MailTracker::default(),
                    },
                );
                self.admit(id);
                self.health.set_active_sessions(self.sessions.len());
            }
            WorldMessage::Packet { id, packet } => {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.queue.push_back(packet);
                }
            }
            WorldMessage::SetLatency { id, latency } => {
                if let Some(entry) = self.sessions.get_mut(&id) {
                    entry.latency = latency;
                    let _ = entry.latency; // stored like C++; consumers later
                }
            }
            WorldMessage::RemoveSession { id } => {
                self.remove(id);
                self.promote_from_queue();
                self.health.set_active_sessions(self.sessions.len());
            }
            WorldMessage::CharEnumReady { account_id, rows } => {
                self.on_char_enum_ready(account_id, rows);
            }
            WorldMessage::PlayerLoginReady { id, guid_low, load } => {
                self.on_player_login_ready(id, guid_low, load);
            }
            WorldMessage::NameQueryReady { id, guid_low, row } => {
                self.on_name_query_ready(id, guid_low, row);
            }
        }
    }

    fn remove(&mut self, id: SessionId) {
        if self.pending_teleports.contains_key(&id) {
            self.save_pending_player(id, true);
        }
        self.pending_teleports.remove(&id);
        if let Some(bot_low) = self.bot_sessions.remove(&id) {
            self.bots.remove(&bot_low);
        }
        if let Some(entry) = self.sessions.remove(&id) {
            if let Some(guid_low) = entry.player_guid {
                self.player_guild_invite.remove(&guid_low);
                // `LogoutPlayer` guild arm: refresh the
                // offline roster stats, stamp logout time,
                // and broadcast signed-off while the player
                // is still in the map. Membership persists.
                if let Some(guild_id) = self.player_guild.get(&guid_low).copied() {
                    let stats = self
                        .guilds
                        .get(&guild_id)
                        .and_then(|guild| self.guild_roster_online(guild).get(&guid_low).copied());
                    if let (Some(stats), Some(guild)) = (stats, self.guilds.get_mut(&guild_id)) {
                        guild.note_logout(&stats, guid_low, unix_now());
                    }
                    if let Some((player_name, _)) = self.player_name_race_of(guid_low)
                        && let Some(guild) = self.guilds.get(&guild_id)
                    {
                        let fx = guild.broadcast_event(
                            guild_event::SIGNED_OFF,
                            Some(guid_low),
                            &[&player_name],
                        );
                        self.apply_guild_fx(fx);
                    }
                }
                // `Player::CleanupChannels`: silent leave
                // everywhere (both faction managers cover
                // the GM opposite-faction join mirror),
                // then map removal.
                let outs = self.channels_alliance.cleanup_player(guid_low);
                self.deliver_channel_outs(outs);
                let outs = self.channels_horde.cleanup_player(guid_low);
                self.deliver_channel_outs(outs);
                // `LogoutPlayer`: pending invites die with
                // the session; membership persists offline
                // and the group sees a status update once
                // the player is out of the map (the C++
                // `UpdatePlayerOnlineStatus(player, false)`
                // `SendUpdate`, with `PlayerLogout()`
                // making the status read offline).
                self.uninvite_from_group(guid_low);
                self.leave_map(guid_low);
                if let Some(key) = self.player_group.get(&guid_low).copied() {
                    self.group_send_update(key);
                }
            }
            if self.by_account.get(&entry.account_id) == Some(&id) {
                self.by_account.remove(&entry.account_id);
            }
            self.login_queue.retain(|queued| *queued != id);
            self.last_logout.insert(entry.account_id, Instant::now());
        }
    }

    /// Sessions holding a world slot (not waiting in queue).
    fn admitted_count(&self) -> usize {
        self.sessions.values().filter(|e| !e.in_queue).count()
    }

    fn player_amount_limit(&self) -> u32 {
        if self.config.player_limit > 0 {
            self.config.player_limit as u32
        } else {
            0
        }
    }

    /// `HasTrialRestrictions` (`WorldSession.cpp:290`).
    fn billing_flags(&self, entry: &SessionEntry) -> u8 {
        if !entry.auth.verified_email
            && entry.auth.security == security::PLAYER
            && self.config.restrict_unverified_accounts
        {
            BILLING_FLAG_TRIAL | BILLING_FLAG_RESTRICTED
        } else {
            0
        }
    }

    /// `CanSkipQueue` (`World.cpp:3132-3144`): staff, or a
    /// recent logout inside the grace period.
    fn can_skip_queue(&self, entry: &SessionEntry) -> bool {
        if entry.auth.security > security::PLAYER {
            return true;
        }
        let grace = self.config.login_queue_grace_secs;
        if grace == 0 {
            return false;
        }
        self.last_logout.get(&entry.account_id).is_some_and(|at| {
            Instant::now().saturating_duration_since(*at) < Duration::from_secs(u64::from(grace))
        })
    }

    /// `SendAuthWaitQue` long forms from `AddSession_` /
    /// `AddQueuedSession`: code + billing fields (+ position).
    fn auth_response_long(
        &self,
        entry: &SessionEntry,
        code: u8,
        position: Option<u32>,
    ) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(code);
        writer.write_u32_le(0);
        writer.write_u8(self.billing_flags(entry));
        writer.write_u32_le(0);
        if let Some(position) = position {
            writer.write_u32_le(position);
        }
        let mut packet = WorldPacket::new(opcode::SMSG_AUTH_RESPONSE);
        packet.take_writer(writer);
        packet
    }

    /// Admission (`World::AddSession_`, `World.cpp:241-337`):
    /// queue when the player limit is full, else `AUTH_OK` plus
    /// the population publish.
    fn admit(&mut self, id: SessionId) {
        let limit = self.player_amount_limit();
        let others = self.admitted_count().saturating_sub(1);
        let must_queue = limit > 0
            && others as u32 >= limit
            && !self
                .sessions
                .get(&id)
                .is_some_and(|e| self.can_skip_queue(e));

        if must_queue {
            if let Some(entry) = self.sessions.get_mut(&id) {
                entry.in_queue = true;
                entry.core.set_in_queue(true);
            }
            self.login_queue.push_back(id);
            let position = self.login_queue.len() as u32;
            let packet = self
                .sessions
                .get(&id)
                .map(|e| self.auth_response_long(e, auth_response::WAIT_QUEUE, Some(position)));
            if let Some(packet) = packet {
                let alive = self
                    .sessions
                    .get(&id)
                    .is_some_and(|e| send_packet(e, packet));
                if !alive {
                    self.disconnect(id);
                }
            }
            return;
        }

        let packet = self
            .sessions
            .get(&id)
            .map(|e| self.auth_response_long(e, auth_response::OK, None));
        if let Some(packet) = packet {
            let alive = self
                .sessions
                .get(&id)
                .is_some_and(|e| send_packet(e, packet));
            if !alive {
                self.disconnect(id);
                return;
            }
        }
        if limit > 0 {
            #[allow(clippy::cast_precision_loss)]
            let population = self.admitted_count() as f32 / limit as f32 * 2.0;
            if let Err(e) = self
                .deps
                .realm_publisher
                .set_realm_population(self.config.realm_id, population)
            {
                self.log
                    .out(Level::Error, &format!("population publish failed: {e}"));
            }
        }
    }

    /// Queue promotion after a departure (`RemoveQueuedSession`
    /// accept path): short `AUTH_OK`, idle clock reset, then
    /// short position updates for those still waiting.
    fn promote_from_queue(&mut self) {
        let limit = self.player_amount_limit();
        let mut promoted = false;
        while limit == 0 || (self.admitted_count() as u32) < limit {
            let Some(next) = self.login_queue.pop_front() else {
                break;
            };
            let Some(entry) = self.sessions.get_mut(&next) else {
                continue;
            };
            entry.in_queue = false;
            entry.core.set_in_queue(false);
            entry.created = Instant::now();
            promoted = true;
            let packet = WorldPacket::with_body(opcode::SMSG_AUTH_RESPONSE, [auth_response::OK]);
            if !send_packet(entry, packet) {
                self.disconnect(next);
            }
        }
        if promoted {
            let waiting: Vec<SessionId> = self.login_queue.iter().copied().collect();
            for (index, queued) in waiting.iter().enumerate() {
                let mut writer = PacketWriter::new();
                writer.write_u8(auth_response::WAIT_QUEUE);
                writer.write_u32_le(index as u32 + 1);
                let mut packet = WorldPacket::new(opcode::SMSG_AUTH_RESPONSE);
                packet.take_writer(writer);
                if let Some(entry) = self.sessions.get(queued)
                    && !send_packet(entry, packet)
                {
                    self.disconnect(*queued);
                }
            }
        }
    }

    /// `HandleCharEnum` (`CharacterHandler.cpp:135-162`).
    fn on_char_enum_ready(&mut self, account_id: u32, rows: Result<Vec<CharEnumRow>, DbError>) {
        let Some(id) = self.by_account.get(&account_id).copied() else {
            return;
        };
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("char enum query failed: {e}"));
                Vec::new()
            }
        };
        let built = build_char_enum(&rows, &self.deps.enum_stores);
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        entry.characters_count = built.count;
        entry.character_max_level = entry.character_max_level.max(built.max_level);
        if !send_packet(entry, built.packet) {
            self.disconnect(id);
        }
    }

    /// Fire the async enum query (`AsyncPQuery` + `chrHandler`
    /// callback): one blocking job, result back as a message.
    fn start_char_enum(&self, account_id: u32) {
        let Some(tx) = self.self_tx.clone() else {
            return;
        };
        let queries = Arc::clone(&self.deps.char_queries);
        tokio::task::spawn_blocking(move || {
            let rows = queries.char_enum(account_id);
            let _ = tx.blocking_send(WorldMessage::CharEnumReady { account_id, rows });
        });
    }

    fn reply_packet(&mut self, id: SessionId, packet: WorldPacket) {
        let alive = self
            .sessions
            .get(&id)
            .is_some_and(|entry| send_packet(entry, packet));
        if !alive {
            self.disconnect(id);
        }
    }

    /// `HandleRequestAccountData` (`MiscHandler.cpp:897-945`).
    fn on_request_account_data(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(data_type) = reader.read_u32_le() else {
            self.bad_packet(id, opcode_id::CMSG_REQUEST_ACCOUNT_DATA);
            return;
        };
        if data_type >= crate::account_data::NUM_ACCOUNT_DATA_TYPES as u32 {
            self.log.out(
                Level::Error,
                &format!(
                    "Client requested invalid account data type {data_type} in \
                     CMSG_REQUEST_ACCOUNT_DATA."
                ),
            );
            return;
        }
        let response = self
            .sessions
            .get(&id)
            .and_then(|entry| entry.account_data.build_update_packet(data_type));
        if let Some(packet) = response {
            self.reply_packet(id, packet);
        }
    }

    /// The static cache queries (`QueryHandler.cpp` and
    /// the item-template query in `ItemHandler.cpp`).
    fn on_query_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        match packet.opcode() {
            opcode_id::CMSG_CREATURE_QUERY => self.on_creature_query(id, packet),
            opcode_id::CMSG_GAMEOBJECT_QUERY => self.on_gameobject_query(id, packet),
            opcode_id::CMSG_ITEM_QUERY_SINGLE => self.on_item_query(id, packet),
            opcode_id::CMSG_NAME_QUERY => self.on_name_query(id, packet),
            _ => {}
        }
    }

    /// Who list (`HandleWhoOpcode`, `MiscHandler.cpp:226-367`):
    /// gathered from every player currently in a map, in guid
    /// order (C++ iterates its player holder; the client
    /// re-sorts). Guild names are empty until the guild
    /// service lands.
    fn on_who_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let request = match parse_who_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(_) => {
                self.bad_packet(id, opcode_id::CMSG_WHO);
                return;
            }
        };
        let Some((viewer_map_id, viewer_guid)) = self.player_map_guid(id) else {
            return;
        };
        let Some((viewer_zone, viewer_instance, viewer_team)) =
            self.maps.get(&viewer_map_id).and_then(|map| {
                map.player(viewer_guid).map(|player| {
                    (
                        map.player_zone_id(viewer_guid),
                        map.instance_id(),
                        self.team_id_for_race(player.unit.race()),
                    )
                })
            })
        else {
            return;
        };
        let viewer = WhoViewer {
            team_id: viewer_team,
            zone: viewer_zone,
            instance_id: viewer_instance,
            security: self
                .sessions
                .get(&id)
                .map_or(security::PLAYER, |entry| entry.auth.security),
            allow_two_side: self.config.allow_two_side_who_list,
            gm_level_in_who_list: self.config.gm_level_in_who_list,
        };

        let mut guid_lows: Vec<u32> = self.player_map.keys().copied().collect();
        guid_lows.sort_unstable();
        let total_online = guid_lows.len();
        let mut candidates = Vec::with_capacity(total_online);
        for guid_low in guid_lows {
            let Some(map) = self
                .player_map
                .get(&guid_low)
                .and_then(|map_id| self.maps.get(map_id))
            else {
                continue;
            };
            let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
            let Some(player) = map.player(guid) else {
                continue;
            };
            let zone = map.player_zone_id(guid);
            let security = self
                .player_session
                .get(&guid_low)
                .and_then(|session_id| self.sessions.get(session_id))
                .map_or(security::PLAYER, |entry| entry.auth.security);
            candidates.push(WhoCandidate {
                name: player.unit.world.name.clone(),
                guild_name: self
                    .player_guild
                    .get(&guid_low)
                    .and_then(|guild_id| self.guilds.get(guild_id))
                    .map_or_else(String::new, |guild| guild.name().to_owned()),
                level: player.unit.level(),
                class: player.unit.class(),
                race: player.unit.race(),
                zone,
                zone_name: self.deps.world_stores.areas.name(zone).to_owned(),
                team_id: self.team_id_for_race(player.unit.race()),
                security,
                instance_id: map.instance_id(),
            });
        }
        let entries = filter_who(&request, &viewer, candidates);
        self.reply_packet(id, build_who_response(&entries, total_online));
    }

    /// Friend/ignore mutation (`MiscHandler.cpp:488-599`).
    /// The session must have a player in world; the opcode
    /// table's logged-in status gate has already run.
    fn on_social_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        match packet.opcode() {
            opcode_id::CMSG_ADD_FRIEND => self.on_add_social(id, packet, false),
            opcode_id::CMSG_ADD_IGNORE => self.on_add_social(id, packet, true),
            opcode_id::CMSG_DEL_FRIEND => self.on_del_social(id, packet, false),
            opcode_id::CMSG_DEL_IGNORE => self.on_del_social(id, packet, true),
            _ => {}
        }
    }

    fn alloc_group_key(&mut self) -> u64 {
        let key = self.next_group_key;
        self.next_group_key += 1;
        key
    }

    /// The session's entered character guid low.
    fn actor_low(&self, id: SessionId) -> Option<u32> {
        self.sessions.get(&id).and_then(|entry| entry.player_guid)
    }

    /// Online (name, race) for a guid low in a map.
    fn player_name_race_of(&self, guid_low: u32) -> Option<(String, u8)> {
        let map = self.maps.get(self.player_map.get(&guid_low)?)?;
        let player = map.player(player_guid(guid_low))?;
        Some((player.unit.world.name.clone(), player.unit.race()))
    }

    /// `GetGroupMemberStatus`: `None` when the player has
    /// no live session/player (offline).
    fn member_status_of(&self, guid_low: u32) -> Option<u8> {
        use mangos_protocol::update_field_table::offsets;
        let session_id = self.player_session.get(&guid_low).copied()?;
        let entry = self.sessions.get(&session_id)?;
        let map = self.maps.get(self.player_map.get(&guid_low)?)?;
        let player = map.player(player_guid(guid_low))?;
        let values = player.unit.world.object.values();
        let mut status = member_status::ONLINE;
        if values.get_u32(offsets::UNIT_FIELD_FLAGS) & 0x0000_1000 != 0 {
            status |= member_status::PVP;
        }
        if !player.unit.is_alive() {
            status |= member_status::DEAD;
        }
        let player_flags = values.get_u32(offsets::PLAYER_FLAGS);
        if player_flags & 0x0000_0010 != 0 {
            status |= member_status::GHOST;
        }
        if player_flags & mangos_entities::update::PLAYER_FLAGS_FFA_PVP != 0 {
            status |= member_status::PVP_FFA;
        }
        match entry.chat_tag {
            1 => status |= member_status::AFK,
            2 => status |= member_status::DND,
            _ => {}
        }
        Some(status)
    }

    /// Status snapshot for the given guid lows (online
    /// entries only), backing [`StatusView`] closures.
    fn group_statuses(&self, lows: &[u32]) -> HashMap<u32, u8> {
        lows.iter()
            .filter_map(|low| self.member_status_of(*low).map(|status| (*low, status)))
            .collect()
    }

    /// Statuses for every member and invitee of a group,
    /// plus any extra lows the operation touches.
    fn statuses_for_group(&self, key: u64, extra: &[u32]) -> HashMap<u32, u8> {
        let mut lows: Vec<u32> = extra.to_vec();
        if let Some(group) = self.groups.get(&key) {
            lows.extend(group.members().iter().map(|member| member.guid_low));
            lows.extend_from_slice(group.invitees());
        }
        self.group_statuses(&lows)
    }

    /// `UpdateGroupLeaderFlag`: bit 0x01 of `PLAYER_FLAGS`
    /// follows current group leadership (membership first
    /// — a leader invitee is not in a group yet).
    fn refresh_leader_flag(&mut self, guid_low: u32) {
        use mangos_protocol::update_field_table::offsets;
        const GROUP_LEADER: u32 = 0x0000_0001;
        let should_have = self
            .player_group
            .get(&guid_low)
            .and_then(|key| self.groups.get(key))
            .is_some_and(|group| group.is_leader(guid_low));
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            return;
        };
        if let Some(player) = self
            .maps
            .get_mut(&map_key)
            .and_then(|map| map.player_mut(player_guid(guid_low)))
        {
            let values = player.unit.values();
            let current = values.get_u32(offsets::PLAYER_FLAGS);
            let next = if should_have {
                current | GROUP_LEADER
            } else {
                current & !GROUP_LEADER
            };
            values.set_u32(offsets::PLAYER_FLAGS, next);
        }
    }

    /// Delivers one group operation's DB writes, packets,
    /// and leader-flag refreshes. Indexes must already
    /// reflect the post-operation membership.
    fn apply_group_fx(&mut self, fx: GroupFx) {
        for write in fx.writes {
            if let Err(e) = self.deps.group.apply_group_write(write) {
                self.log
                    .out(Level::Error, &format!("group write failed: {e}"));
            }
        }
        for (guid_low, packet) in fx.sends {
            if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                self.reply_packet(session_id, packet);
            }
        }
        for guid_low in fx.flag_refresh {
            self.refresh_leader_flag(guid_low);
        }
    }

    /// Removes a group and every index pointing at it
    /// (`ObjectMgr::RemoveGroup` + `delete group`).
    fn destroy_group(&mut self, key: u64) {
        let members: Vec<u32> = self
            .player_group
            .iter()
            .filter(|(_, k)| **k == key)
            .map(|(guid, _)| *guid)
            .collect();
        if let Some(group) = self.groups.remove(&key)
            && group.db_id() != 0
        {
            self.group_by_db_id.remove(&group.db_id());
        }
        self.player_group.retain(|_, k| *k != key);
        self.player_group_invite.retain(|_, k| *k != key);
        for guid in members {
            self.guild_bot_removed_from_group(guid);
        }
    }

    /// `GuildBotMgr::OnBotRemovedFromGroup`: despawn
    /// the party bot; presence re-logins it later.
    fn guild_bot_removed_from_group(&mut self, guid_low: u32) {
        if !self.guild_bots.is_presence_bot(guid_low)
            || !self.bots.contains_key(&guid_low)
            || self
                .pending_raid_attach
                .iter()
                .any(|(b, _, _)| *b == guid_low)
        {
            return;
        }
        let name = self
            .guild_bots
            .bot_info(guid_low)
            .map(|info| info.name.clone())
            .unwrap_or_default();
        self.guild_bots.set_online(guid_low, false);
        self.logout_bot(guid_low);
        self.log.out(
            Level::Minimal,
            &format!("[GuildBotMgr] {name} removed from group, will rejoin via presence"),
        );
    }

    /// `Player::UninviteFromGroup`: drop the invite, and
    /// tear the group down when it is down to one member
    /// (created: silent disband; shell: all invites die).
    fn uninvite_from_group(&mut self, guid_low: u32) {
        let Some(key) = self.player_group_invite.remove(&guid_low) else {
            return;
        };
        let Some(group) = self.groups.get_mut(&key) else {
            return;
        };
        group.remove_invite(guid_low);
        if group.members_count() > 1 {
            return;
        }
        if group.is_created() {
            let statuses = self.statuses_for_group(key, &[]);
            let view = |low: u32| statuses.get(&low).copied();
            let fx = self
                .groups
                .get_mut(&key)
                .map(|group| group.disband(true, &view))
                .unwrap_or_default();
            self.destroy_group(key);
            self.apply_group_fx(fx);
        } else {
            self.destroy_group(key);
        }
    }

    /// `Player::RemoveFromGroup` static: `RemoveMember`
    /// with `GROUP_LEAVE` semantics unless the caller is
    /// the rare kick path, destroying the group when the
    /// result drops to one (the disband branch returns 0).
    fn remove_from_group(&mut self, key: u64, guid_low: u32, method: u8) {
        let statuses = self.statuses_for_group(key, &[guid_low]);
        let view = |low: u32| statuses.get(&low).copied();
        let Some(group) = self.groups.get_mut(&key) else {
            return;
        };
        let (fx, _remaining, destroyed) = group.remove_member(guid_low, method, &view);
        if destroyed {
            self.destroy_group(key);
        } else {
            self.player_group.remove(&guid_low);
        }
        self.apply_group_fx(fx);
        self.guild_bot_removed_from_group(guid_low);
    }

    /// `Group::SendUpdate` for the online-status hooks.
    fn group_send_update(&mut self, key: u64) {
        let statuses = self.statuses_for_group(key, &[]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get(&key) {
            let fx = group.send_update(&view);
            self.apply_group_fx(fx);
        }
    }

    /// `Player::CanUninviteFromGroup`.
    fn can_uninvite(&self, actor_low: u32, target_low: u32) -> u32 {
        let Some(group) = self
            .player_group
            .get(&actor_low)
            .and_then(|key| self.groups.get(key))
        else {
            return party_result::NOT_IN_GROUP;
        };
        if group.is_leader(target_low) {
            return party_result::NOT_LEADER;
        }
        if !group.is_leader(actor_low) && !group.is_assistant(actor_low) {
            return party_result::NOT_LEADER;
        }
        party_result::OK
    }

    fn send_party_result(&mut self, id: SessionId, operation: u32, member: &str, result: u32) {
        self.reply_packet(id, party_result_packet(operation, member, result));
    }

    /// Group opcodes (`GroupHandler.cpp`). The session
    /// must have a player in world; the opcode table's
    /// logged-in status gate has already run. Deviations:
    /// `IsGameMaster()` reads as account security above
    /// player (no GM-mode flag yet), and the guild-bot
    /// presence-conversion invite hook lands in Slice E.
    fn on_group_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(actor_low) = self.actor_low(id) else {
            return;
        };
        match packet.opcode() {
            opcode_id::CMSG_GROUP_INVITE => self.on_group_invite(id, actor_low, packet),
            opcode_id::CMSG_GROUP_ACCEPT => self.on_group_accept(id, actor_low),
            opcode_id::CMSG_GROUP_DECLINE => self.on_group_decline(actor_low),
            opcode_id::CMSG_GROUP_UNINVITE_GUID => {
                self.on_group_uninvite_guid(id, actor_low, packet);
            }
            opcode_id::CMSG_GROUP_UNINVITE => self.on_group_uninvite(id, actor_low, packet),
            opcode_id::CMSG_GROUP_SET_LEADER => self.on_group_set_leader(id, actor_low, packet),
            opcode_id::CMSG_GROUP_DISBAND => self.on_group_disband(id, actor_low),
            opcode_id::CMSG_LOOT_METHOD => self.on_loot_method(id, actor_low, packet),
            opcode_id::CMSG_LOOT_ROLL => self.on_loot_roll(id, actor_low, packet),
            opcode_id::CMSG_GROUP_RAID_CONVERT => self.on_group_raid_convert(id, actor_low),
            opcode_id::CMSG_GROUP_CHANGE_SUB_GROUP => {
                self.on_group_change_sub_group(actor_low, packet);
            }
            opcode_id::CMSG_GROUP_SWAP_SUB_GROUP => {
                self.on_group_swap_sub_group(actor_low, packet);
            }
            opcode_id::CMSG_GROUP_ASSISTANT_LEADER => {
                self.on_group_assistant(id, actor_low, packet);
            }
            opcode_id::MSG_RAID_TARGET_UPDATE => self.on_raid_target_update(id, actor_low, packet),
            opcode_id::MSG_RAID_READY_CHECK => self.on_raid_ready_check(actor_low, packet),
            opcode_id::MSG_MINIMAP_PING => self.on_minimap_ping(id, actor_low, packet),
            opcode_id::MSG_RANDOM_ROLL => self.on_random_roll(id, actor_low, packet),
            opcode_id::CMSG_REQUEST_PARTY_MEMBER_STATS => {
                self.on_request_party_member_stats(id, actor_low, packet);
            }
            _ => {}
        }
    }

    /// `HandleGroupInviteOpcode`.
    fn on_group_invite(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let membername = reader.read_cstring();
        let Some(name) = normalize_player_name(&membername) else {
            self.send_party_result(
                id,
                party_op::INVITE,
                &membername,
                party_result::BAD_PLAYER_NAME_S,
            );
            return;
        };
        let Some((target_low, target_session, target_race)) =
            self.find_online_player_by_name(&name)
        else {
            self.send_party_result(id, party_op::INVITE, &name, party_result::BAD_PLAYER_NAME_S);
            return;
        };
        let actor_security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        let Some((actor_name, actor_race)) = self.player_name_race_of(actor_low) else {
            return;
        };
        if actor_security == security::PLAYER
            && !self.config.allow_two_side_interaction_group
            && self.team_id_for_race(actor_race) != self.team_id_for_race(target_race)
        {
            self.send_party_result(
                id,
                party_op::INVITE,
                &name,
                party_result::PLAYER_WRONG_FACTION,
            );
            return;
        }
        if self.player_ignores(target_low, actor_low) {
            self.send_party_result(id, party_op::INVITE, &name, party_result::IGNORING_YOU_S);
            return;
        }
        if self.player_group.contains_key(&target_low)
            || self.player_group_invite.contains_key(&target_low)
        {
            self.send_party_result(
                id,
                party_op::INVITE,
                &name,
                party_result::ALREADY_IN_GROUP_S,
            );
            return;
        }
        let key = if let Some(key) = self.player_group.get(&actor_low).copied() {
            let Some(group) = self.groups.get(&key) else {
                return;
            };
            if !group.is_leader(actor_low) && !group.is_assistant(actor_low) {
                self.send_party_result(id, party_op::INVITE, "", party_result::NOT_LEADER);
                return;
            }
            if group.is_full() {
                self.send_party_result(id, party_op::INVITE, "", party_result::GROUP_FULL);
                return;
            }
            key
        } else {
            let key = self.alloc_group_key();
            let shell = Group::new_invite_shell(actor_low, actor_name.clone());
            self.groups.insert(key, shell);
            self.player_group_invite.insert(actor_low, key);
            self.refresh_leader_flag(actor_low);
            key
        };
        let invited = self
            .groups
            .get_mut(&key)
            .is_some_and(|group| group.add_invite(target_low));
        if !invited {
            if !self.groups.get(&key).is_some_and(Group::is_created) {
                self.destroy_group(key);
                self.refresh_leader_flag(actor_low);
            }
            return;
        }
        self.player_group_invite.insert(target_low, key);
        self.reply_packet(target_session, group_invite_packet(&actor_name));
        self.send_party_result(id, party_op::INVITE, &name, party_result::OK);
        // Bots accept group invites (party glue).
        self.bot_accept_invite(target_low, actor_low);
    }

    /// Bot invite acceptance; guild bots convert
    /// presence -> party with their roster role
    /// (`ConvertPresenceToParty`, unified engine:
    /// no despawn/respawn cycle).
    fn bot_accept_invite(&mut self, target_low: u32, actor_low: u32) {
        if !self.bots.contains_key(&target_low) {
            return;
        }
        if let Some(info) = self.guild_bots.bot_info(target_low) {
            let role = mangos_bots::guild_bots::behavior_role(info);
            let name = info.name.clone();
            if let Some(bot) = self.bots.get_mut(&target_low) {
                bot.role = role;
                bot.role_locked = true;
                Self::bot_build_sets(bot);
            }
            let leader_name = self
                .player_name_race_of(actor_low)
                .map(|(name, _)| name)
                .unwrap_or_default();
            self.log.out(
                Level::Minimal,
                &format!("[GuildBotMgr] {name} converted to PartyBot for {leader_name}"),
            );
        }
        if let Some(bot_session) = self.player_session.get(&target_low).copied() {
            self.on_group_accept(bot_session, target_low);
            if self.player_group.contains_key(&target_low) {
                self.bot_party_greet(target_low);
            }
        }
        self.bot_follow_master(target_low, actor_low);
    }

    /// `HandleGroupAcceptOpcode`.
    fn on_group_accept(&mut self, id: SessionId, actor_low: u32) {
        let Some(key) = self.player_group_invite.get(&actor_low).copied() else {
            return;
        };
        let (leader_low, is_created, is_full) = {
            let Some(group) = self.groups.get(&key) else {
                return;
            };
            (group.leader_low(), group.is_created(), group.is_full())
        };
        if leader_low == actor_low {
            self.log.out(
                Level::Error,
                &format!("HandleGroupAcceptOpcode: guid {actor_low} tried to accept an invite to his own group"),
            );
            return;
        }
        if let Some(group) = self.groups.get_mut(&key) {
            group.remove_invite(actor_low);
        }
        self.player_group_invite.remove(&actor_low);
        if is_full {
            self.send_party_result(id, party_op::INVITE, "", party_result::GROUP_FULL);
            return;
        }
        if !is_created {
            if self.player_group_invite.get(&leader_low) == Some(&key) {
                self.player_group_invite.remove(&leader_low);
            }
            let db_id = self.next_group_db_id;
            self.next_group_db_id = self.next_group_db_id.saturating_add(1);
            let statuses = self.statuses_for_group(key, &[actor_low, leader_low]);
            let view = |low: u32| statuses.get(&low).copied();
            let Some(group) = self.groups.get_mut(&key) else {
                return;
            };
            group.remove_invite(leader_low);
            let Some(fx) = group.create(db_id, &view) else {
                self.destroy_group(key);
                return;
            };
            self.group_by_db_id.insert(db_id, key);
            self.player_group.insert(leader_low, key);
            self.apply_group_fx(fx);
        }
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        let statuses = self.statuses_for_group(key, &[actor_low]);
        let view = |low: u32| statuses.get(&low).copied();
        let Some(group) = self.groups.get_mut(&key) else {
            return;
        };
        if let Some(fx) = group.add_member(actor_low, &actor_name, &view) {
            self.player_group.insert(actor_low, key);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleGroupDeclineOpcode`.
    fn on_group_decline(&mut self, actor_low: u32) {
        let Some(key) = self.player_group_invite.get(&actor_low).copied() else {
            return;
        };
        let leader_low = self.groups.get(&key).map(Group::leader_low);
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        self.uninvite_from_group(actor_low);
        if let Some(leader_session) =
            leader_low.and_then(|low| self.player_session.get(&low).copied())
        {
            self.reply_packet(leader_session, group_decline_packet(&actor_name));
        }
    }

    /// `HandleGroupUninviteGuidOpcode`.
    fn on_group_uninvite_guid(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw) = reader.read_u64_le() else {
            self.bad_packet(id, opcode_id::CMSG_GROUP_UNINVITE_GUID);
            return;
        };
        let target_low = ObjectGuid::from_raw(raw).counter();
        if target_low == actor_low {
            return;
        }
        let result = self.can_uninvite(actor_low, target_low);
        if result != party_result::OK {
            self.send_party_result(id, party_op::LEAVE, "", result);
            return;
        }
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        if self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_member(target_low))
        {
            self.remove_from_group(key, target_low, remove_method::LEAVE);
        } else if self.player_group_invite.get(&target_low) == Some(&key) {
            self.uninvite_from_group(target_low);
        } else {
            self.send_party_result(id, party_op::LEAVE, "", party_result::TARGET_NOT_IN_GROUP_S);
        }
    }

    /// `HandleGroupUninviteOpcode`.
    fn on_group_uninvite(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let membername = reader.read_cstring();
        let Some(name) = normalize_player_name(&membername) else {
            return;
        };
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        if normalize_player_name(&actor_name).as_deref() == Some(&name) {
            return;
        }
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let member_low = self
            .groups
            .get(&key)
            .and_then(|group| group.member_low_by_name(&name));
        if let Some(target_low) = member_low {
            let result = self.can_uninvite(actor_low, target_low);
            if result != party_result::OK {
                self.send_party_result(id, party_op::LEAVE, "", result);
                return;
            }
            self.remove_from_group(key, target_low, remove_method::LEAVE);
            return;
        }
        let invited_low = self.groups.get(&key).and_then(|group| {
            group.invitees().iter().copied().find(|low| {
                self.player_name_race_of(*low)
                    .and_then(|(candidate, _)| normalize_player_name(&candidate))
                    .as_deref()
                    == Some(&name)
            })
        });
        if let Some(target_low) = invited_low {
            let result = self.can_uninvite(actor_low, target_low);
            if result != party_result::OK {
                self.send_party_result(id, party_op::LEAVE, "", result);
                return;
            }
            self.uninvite_from_group(target_low);
            return;
        }
        self.send_party_result(
            id,
            party_op::LEAVE,
            &name,
            party_result::TARGET_NOT_IN_GROUP_S,
        );
    }

    /// `HandleGroupSetLeaderOpcode` (1.12 guid form).
    fn on_group_set_leader(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw) = reader.read_u64_le() else {
            self.bad_packet(id, opcode_id::CMSG_GROUP_SET_LEADER);
            return;
        };
        let target_low = ObjectGuid::from_raw(raw).counter();
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let eligible = target_low != actor_low
            && self.player_map.contains_key(&target_low)
            && self
                .groups
                .get(&key)
                .is_some_and(|group| group.is_leader(actor_low))
            && self.player_group.get(&target_low) == Some(&key);
        if !eligible {
            return;
        }
        let statuses = self.statuses_for_group(key, &[]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.change_leader(target_low, &view);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleGroupDisbandOpcode` (leave party).
    fn on_group_disband(&mut self, id: SessionId, actor_low: u32) {
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        self.send_party_result(id, party_op::LEAVE, &actor_name, party_result::OK);
        self.remove_from_group(key, actor_low, remove_method::LEAVE);
    }

    /// `HandleLootMethodOpcode`.
    fn on_loot_method(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = (|| -> Result<(u32, u64, u32), mangos_protocol::buffer::ReadError> {
            Ok((
                reader.read_u32_le()?,
                reader.read_u64_le()?,
                reader.read_u32_le()?,
            ))
        })();
        let Ok((method, looter_raw, threshold)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_LOOT_METHOD);
            return;
        };
        if method > u32::from(loot_method::NEED_BEFORE_GREED) {
            return;
        }
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        if !self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_leader(actor_low))
        {
            return;
        }
        let statuses = self.statuses_for_group(key, &[]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            #[allow(clippy::cast_possible_truncation)]
            let fx = group.set_loot(method as u8, looter_raw, threshold as u8, &view);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleLootRoll`: parses and validates; no rolls
    /// exist until group loot lands, so the vote count is
    /// the C++ no-roll no-op.
    fn on_loot_roll(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = (|| -> Result<u8, mangos_protocol::buffer::ReadError> {
            let _target = reader.read_u64_le()?;
            let _slot = reader.read_u32_le()?;
            reader.read_u8()
        })();
        let Ok(roll_type) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_LOOT_ROLL);
            return;
        };
        let _valid = roll_type < 3 && self.player_group.contains_key(&actor_low);
    }

    /// `HandleGroupRaidConvertOpcode`.
    fn on_group_raid_convert(&mut self, id: SessionId, actor_low: u32) {
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let eligible = self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_leader(actor_low) && group.members_count() >= 2);
        if !eligible {
            return;
        }
        self.send_party_result(id, party_op::INVITE, "", party_result::OK);
        let statuses = self.statuses_for_group(key, &[]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.convert_to_raid(&view);
            self.apply_group_fx(fx);
        }
    }

    /// `ObjectMgr::GetPlayerGuidByName` for offline
    /// subgroup targets (characters table via the social
    /// store's by-name lookup).
    fn guid_low_by_name_offline(&self, name: &str) -> Option<u32> {
        match self.deps.social.player_data_by_name(name) {
            Ok(row) => row.map(|row| row.guid_low),
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("group name lookup failed: {e}"));
                None
            }
        }
    }

    /// `HandleGroupChangeSubGroupOpcode`.
    fn on_group_change_sub_group(&mut self, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let name = reader.read_cstring();
        let Ok(subgroup) = reader.read_u8() else {
            return;
        };
        if subgroup >= crate::group::MAX_RAID_SUBGROUPS {
            return;
        }
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let eligible = self.groups.get(&key).is_some_and(|group| {
            (group.is_leader(actor_low) || group.is_assistant(actor_low))
                && group.has_free_slot_subgroup(subgroup)
        });
        if !eligible {
            return;
        }
        let normalized = normalize_player_name(&name).unwrap_or(name);
        let target_low = match self.find_online_player_by_name(&normalized) {
            Some((guid_low, ..)) => Some(guid_low),
            None => self.guid_low_by_name_offline(&normalized),
        };
        let Some(target_low) = target_low else {
            return;
        };
        let statuses = self.statuses_for_group(key, &[target_low]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.change_member_subgroup(target_low, subgroup, &view);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleGroupSwapSubGroupOpcode`.
    fn on_group_swap_sub_group(&mut self, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let name = reader.read_cstring();
        let name_swap = reader.read_cstring();
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let eligible = self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_leader(actor_low) || group.is_assistant(actor_low));
        if !eligible {
            return;
        }
        let resolve = |raw: &str, this: &Self| -> Option<u32> {
            let normalized = normalize_player_name(raw).unwrap_or_else(|| raw.to_owned());
            match this.find_online_player_by_name(&normalized) {
                Some((guid_low, ..)) => Some(guid_low),
                None => this.guid_low_by_name_offline(&normalized),
            }
        };
        let (Some(first_low), Some(second_low)) = (resolve(&name, self), resolve(&name_swap, self))
        else {
            return;
        };
        let statuses = self.statuses_for_group(key, &[first_low, second_low]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.swap_member_subgroups(first_low, second_low, &view);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleGroupAssistantLeaderOpcode` (1.12 guid form).
    fn on_group_assistant(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = (|| -> Result<(u64, u8), mangos_protocol::buffer::ReadError> {
            Ok((reader.read_u64_le()?, reader.read_u8()?))
        })();
        let Ok((raw, flag)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_GROUP_ASSISTANT_LEADER);
            return;
        };
        let target_low = ObjectGuid::from_raw(raw).counter();
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        if !self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_leader(actor_low))
        {
            return;
        }
        let statuses = self.statuses_for_group(key, &[target_low]);
        let view = |low: u32| statuses.get(&low).copied();
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.set_assistant(target_low, flag != 0, &view);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleRaidTargetUpdateOpcode`.
    fn on_raid_target_update(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(icon) = reader.read_u8() else {
            self.bad_packet(id, opcode_id::MSG_RAID_TARGET_UPDATE);
            return;
        };
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        if icon == 0xFF {
            if let Some(group) = self.groups.get(&key) {
                let packet = group.target_icon_list_packet();
                self.reply_packet(id, packet);
            }
            return;
        }
        let eligible = self
            .groups
            .get(&key)
            .is_some_and(|group| group.is_leader(actor_low) || group.is_assistant(actor_low));
        if !eligible {
            return;
        }
        let Ok(target_raw) = reader.read_u64_le() else {
            self.bad_packet(id, opcode_id::MSG_RAID_TARGET_UPDATE);
            return;
        };
        if let Some(group) = self.groups.get_mut(&key) {
            let fx = group.set_target_icon(usize::from(icon), target_raw);
            self.apply_group_fx(fx);
        }
    }

    /// `HandleRaidReadyCheckOpcode`: empty body is the
    /// leader/assistant request broadcast; a state byte is
    /// a member answer relayed to the leader. The fork's
    /// `OfflineReadyCheck` is commented out (no-op).
    fn on_raid_ready_check(&mut self, actor_low: u32, packet: &WorldPacket) {
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        if packet.body().is_empty() {
            let eligible = self
                .groups
                .get(&key)
                .is_some_and(|group| group.is_leader(actor_low) || group.is_assistant(actor_low));
            if !eligible {
                return;
            }
            let request = WorldPacket::new(opcode_id::MSG_RAID_READY_CHECK);
            if let Some(group) = self.groups.get(&key) {
                for (guid_low, packet) in group.broadcast(&request, None, None) {
                    if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                        self.reply_packet(session_id, packet);
                    }
                }
            }
            return;
        }
        let state = packet.body()[0];
        let leader_low = self.groups.get(&key).map(Group::leader_low);
        if let Some(leader_session) =
            leader_low.and_then(|low| self.player_session.get(&low).copied())
        {
            let mut writer = PacketWriter::new();
            writer.write_u64_le(player_guid(actor_low).raw());
            writer.write_u8(state);
            let mut packet = WorldPacket::new(opcode_id::MSG_RAID_READY_CHECK);
            packet.take_writer(writer);
            self.reply_packet(leader_session, packet);
        }
    }

    /// `HandleMinimapPingOpcode`.
    fn on_minimap_ping(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = (|| -> Result<(f32, f32), mangos_protocol::buffer::ReadError> {
            Ok((reader.read_f32_le()?, reader.read_f32_le()?))
        })();
        let Ok((x, y)) = parsed else {
            self.bad_packet(id, opcode_id::MSG_MINIMAP_PING);
            return;
        };
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            return;
        };
        let mut writer = PacketWriter::new();
        writer.write_u64_le(player_guid(actor_low).raw());
        writer.write_f32_le(x);
        writer.write_f32_le(y);
        let mut ping = WorldPacket::new(opcode_id::MSG_MINIMAP_PING);
        ping.take_writer(writer);
        if let Some(group) = self.groups.get(&key) {
            for (guid_low, packet) in group.broadcast(&ping, None, Some(actor_low)) {
                if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                    self.reply_packet(session_id, packet);
                }
            }
        }
    }

    /// `HandleRandomRollOpcode` (patch-1.7 group
    /// broadcast arm).
    fn on_random_roll(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = (|| -> Result<(u32, u32), mangos_protocol::buffer::ReadError> {
            Ok((reader.read_u32_le()?, reader.read_u32_le()?))
        })();
        let Ok((minimum, maximum)) = parsed else {
            self.bad_packet(id, opcode_id::MSG_RANDOM_ROLL);
            return;
        };
        if minimum > maximum || maximum > 10_000 {
            return;
        }
        #[allow(clippy::cast_possible_wrap)]
        let roll =
            mangos_spells::SpellRng::irand(&mut self.roll_rng, minimum as i32, maximum as i32);
        let mut writer = PacketWriter::new();
        writer.write_u32_le(minimum);
        writer.write_u32_le(maximum);
        #[allow(clippy::cast_sign_loss)]
        writer.write_u32_le(roll as u32);
        writer.write_u64_le(player_guid(actor_low).raw());
        let mut roll_packet = WorldPacket::new(opcode_id::MSG_RANDOM_ROLL);
        roll_packet.take_writer(writer);
        match self.player_group.get(&actor_low).copied() {
            Some(key) => {
                if let Some(group) = self.groups.get(&key) {
                    for (guid_low, packet) in group.broadcast(&roll_packet, None, None) {
                        if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                            self.reply_packet(session_id, packet);
                        }
                    }
                }
            }
            None => self.reply_packet(id, roll_packet),
        }
    }

    /// `HandleRequestPartyMemberStatsOpcode` with
    /// `BuildPartyMemberStatsPacket(GROUP_UPDATE_FULL)`.
    /// Pet fields are zero: the Rust player has no pet
    /// aggregate yet (C++ writes the same zeros when
    /// `GetPet()` is null).
    #[allow(clippy::too_many_lines)] // one packet builder, mirrors BuildPartyMemberStatsPacket
    fn on_request_party_member_stats(
        &mut self,
        id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
    ) {
        use mangos_protocol::update_field_table::offsets;
        let mut reader = packet.reader();
        let Ok(raw) = reader.read_u64_le() else {
            self.bad_packet(id, opcode_id::CMSG_REQUEST_PARTY_MEMBER_STATS);
            return;
        };
        let target_guid = ObjectGuid::from_raw(raw);
        let target_low = target_guid.counter();
        let same_raid = self.player_group.contains_key(&actor_low)
            && self.player_group.get(&actor_low) == self.player_group.get(&target_low);
        let mut writer = PacketWriter::new();
        let mut packed_guid = Vec::new();
        target_guid.write_packed(&mut packed_guid);
        writer.write_bytes(&packed_guid);
        let snapshot = if same_raid {
            self.player_map.get(&target_low).and_then(|map_id| {
                let map = self.maps.get(map_id)?;
                let player = map.player(target_guid)?;
                let values = player.unit.world.object.values();
                let mut aura_mask: u32 = 0;
                let mut auras: Vec<u16> = Vec::new();
                for slot in 0..u32::from(mangos_entities::aura::MAX_POSITIVE_AURAS) {
                    #[allow(clippy::cast_possible_truncation)]
                    let spell = values.get_u32(offsets::UNIT_FIELD_AURA + slot as u16);
                    if spell != 0 {
                        aura_mask |= 1 << slot;
                        #[allow(clippy::cast_possible_truncation)]
                        auras.push(spell as u16);
                    }
                }
                let mut negative_mask: u16 = 0;
                let mut negative: Vec<u16> = Vec::new();
                for slot in u32::from(mangos_entities::aura::MAX_POSITIVE_AURAS)
                    ..u32::from(mangos_entities::aura::MAX_AURAS)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let spell = values.get_u32(offsets::UNIT_FIELD_AURA + slot as u16);
                    if spell != 0 {
                        negative_mask |=
                            1 << (slot - u32::from(mangos_entities::aura::MAX_POSITIVE_AURAS));
                        #[allow(clippy::cast_possible_truncation)]
                        negative.push(spell as u16);
                    }
                }
                let power_type = player.unit.power_type();
                let position = player.unit.world.position();
                Some((
                    self.member_status_of(target_low)
                        .unwrap_or(member_status::OFFLINE),
                    player.unit.health(),
                    player.unit.max_health(),
                    power_type,
                    player.unit.power(usize::from(power_type)),
                    player.unit.max_power(usize::from(power_type)),
                    player.unit.level(),
                    map.player_zone_id(target_guid),
                    position.x,
                    position.y,
                    aura_mask,
                    auras,
                    negative_mask,
                    negative,
                ))
            })
        } else {
            None
        };
        if let Some((
            status,
            health,
            max_health,
            power_type,
            power,
            max_power,
            level,
            zone,
            x,
            y,
            aura_mask,
            auras,
            negative_mask,
            negative,
        )) = snapshot
        {
            writer.write_u32_le(0x001F_FFFF); // GROUP_UPDATE_FULL
            writer.write_u8(status);
            #[allow(clippy::cast_possible_truncation)]
            {
                writer.write_u16_le(health as u16);
                writer.write_u16_le(max_health as u16);
            }
            writer.write_u8(power_type);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                writer.write_u16_le(power as u16);
                writer.write_u16_le(max_power as u16);
                writer.write_u16_le(level as u16);
                writer.write_u16_le(zone as u16);
                writer.write_u16_le((x as i16) as u16);
                writer.write_u16_le((y as i16) as u16);
            }
            writer.write_u32_le(aura_mask);
            for spell in auras {
                writer.write_u16_le(spell);
            }
            writer.write_u16_le(negative_mask);
            for spell in negative {
                writer.write_u16_le(spell);
            }
            writer.write_u64_le(0); // pet guid
            writer.write_u8(0); // pet name
            writer.write_u16_le(0); // pet model
            writer.write_u16_le(0); // pet cur hp
            writer.write_u16_le(0); // pet max hp
            writer.write_u8(0); // pet power type
            writer.write_u16_le(0); // pet cur power
            writer.write_u16_le(0); // pet max power
            writer.write_u32_le(0); // pet auras
            writer.write_u16_le(0); // pet negative auras
        } else {
            writer.write_u32_le(0x0000_0001); // GROUP_UPDATE_FLAG_STATUS
            writer.write_u8(member_status::OFFLINE);
        }
        let mut reply = WorldPacket::new(opcode_id::SMSG_PARTY_MEMBER_STATS_FULL);
        reply.take_writer(writer);
        self.reply_packet(id, reply);
    }

    /// Delivers guild writes, packets, and
    /// `PLAYER_GUILDID`/`PLAYER_GUILDRANK` field updates.
    fn apply_guild_fx(&mut self, fx: GuildFx) {
        use mangos_protocol::update_field_table::offsets;
        for write in fx.writes {
            if let Err(e) = self.deps.guild.apply_guild_write(write) {
                self.log
                    .out(Level::Error, &format!("guild write failed: {e}"));
            }
        }
        for (guid_low, packet) in fx.sends {
            if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                self.reply_packet(session_id, packet);
            }
        }
        for update in fx.fields {
            if let Some(player) = self
                .player_map
                .get(&update.guid_low)
                .copied()
                .and_then(|map_id| self.maps.get_mut(&map_id))
                .and_then(|map| map.player_mut(player_guid(update.guid_low)))
            {
                player.guild_id = update.guild_id;
                player.guild_rank = u8::try_from(update.rank_id).unwrap_or(0);
                let values = player.unit.values();
                values.set_u32(offsets::PLAYER_GUILDID, update.guild_id);
                values.set_u32(offsets::PLAYER_GUILDRANK, update.rank_id);
            }
        }
    }

    fn send_guild_result(&mut self, id: SessionId, type_command: u32, text: &str, result: u32) {
        self.reply_packet(id, command_result_packet(type_command, text, result));
    }

    /// `SendNotification(LANG_RESTRICTED_ACCOUNT)`.
    fn send_trial_notification(&mut self, id: SessionId) {
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let text = self.strings.get(184).map_or("", |entry| entry.text(locale));
        let mut writer = PacketWriter::new();
        writer.write_cstring(text);
        let mut packet = WorldPacket::new(opcode_id::SMSG_NOTIFICATION);
        packet.take_writer(writer);
        self.reply_packet(id, packet);
    }

    /// `HasTrialRestrictions` for a live session.
    fn has_trial_restrictions(&self, id: SessionId) -> bool {
        self.sessions
            .get(&id)
            .is_some_and(|entry| self.billing_flags(entry) != 0)
    }

    /// Online stats for a joining member (the online arm
    /// of `AddMember`; the C++ cache read matches these
    /// for a logged-in character except zone, where the
    /// live zone is fresher than `characters.zone`).
    fn guild_stats_of(&self, guid_low: u32) -> Option<NewMemberStats> {
        let account_id = self
            .player_session
            .get(&guid_low)
            .and_then(|session_id| self.sessions.get(session_id))
            .map(|entry| entry.auth.account_id)?;
        let map_id = self.player_map.get(&guid_low).copied()?;
        let map = self.maps.get(&map_id)?;
        let player = map.player(player_guid(guid_low))?;
        Some(NewMemberStats {
            guid_low,
            account_id,
            name: player.unit.world.name.clone(),
            level: u8::try_from(player.unit.level()).unwrap_or(0),
            class: player.unit.class(),
            zone_id: map.player_zone_id(player_guid(guid_low)),
        })
    }

    /// Live roster stats for every online member.
    fn guild_roster_online(&self, guild: &Guild) -> HashMap<u32, RosterOnline> {
        let mut online = HashMap::new();
        for low in guild.members().keys() {
            let Some(map_id) = self.player_map.get(low).copied() else {
                continue;
            };
            let Some(map) = self.maps.get(&map_id) else {
                continue;
            };
            let Some(player) = map.player(player_guid(*low)) else {
                continue;
            };
            let chat_tag = self
                .player_session
                .get(low)
                .and_then(|session_id| self.sessions.get(session_id))
                .map_or(0, |entry| entry.chat_tag);
            online.insert(
                *low,
                RosterOnline {
                    level: u8::try_from(player.unit.level()).unwrap_or(0),
                    class: player.unit.class(),
                    zone_id: map.player_zone_id(player_guid(*low)),
                    afk: chat_tag == 1,
                    dnd: chat_tag == 2,
                },
            );
        }
        online
    }

    /// `Guild::Roster(session)`: one packet honoring the
    /// viewer's officer-note right.
    fn guild_roster_to(&mut self, guild_id: u32, viewer: SessionId, viewer_rank: u32) {
        let Some(guild) = self.guilds.get(&guild_id) else {
            return;
        };
        let can_view = guild.has_rank_right(viewer_rank, rights::VIEWOFFNOTE);
        let online = self.guild_roster_online(guild);
        let packet = guild.roster_packet(&online, can_view, unix_now());
        self.reply_packet(viewer, packet);
    }

    /// `Guild::Roster()` broadcast form (null session:
    /// officer notes hidden from everyone, as in C++).
    fn guild_roster_broadcast(&mut self, guild_id: u32) {
        let Some(guild) = self.guilds.get(&guild_id) else {
            return;
        };
        let online = self.guild_roster_online(guild);
        let packet = guild.roster_packet(&online, false, unix_now());
        for (guid_low, packet) in guild.broadcast(&packet) {
            if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                self.reply_packet(session_id, packet);
            }
        }
    }

    /// `Guild::Query(session)`.
    fn guild_query_to(&mut self, guild_id: u32, viewer: SessionId) {
        if let Some(guild) = self.guilds.get(&guild_id) {
            let packet = guild.query_packet();
            self.reply_packet(viewer, packet);
        }
    }

    /// Removes a guild and every index pointing at it.
    fn destroy_guild_entry(&mut self, guild_id: u32) {
        self.guilds.remove(&guild_id);
        self.player_guild.retain(|_, id| *id != guild_id);
        self.player_guild_invite.retain(|_, id| *id != guild_id);
    }

    /// `Guild::LogGuildEvent` (write-only effects).
    fn log_guild_event(
        &mut self,
        guild_id: u32,
        event_type: u8,
        guid1: u32,
        guid2: u32,
        new_rank: u8,
    ) {
        let count = self.config.guild_event_log_count;
        if let Some(guild) = self.guilds.get_mut(&guild_id) {
            let fx = guild.log_event(event_type, guid1, guid2, new_rank, unix_now(), count);
            self.apply_guild_fx(fx);
        }
    }

    /// The actor's guild id + rank, sending the shared
    /// not-in-guild result when absent (the
    /// `GUILD_CREATE_S` quirk most handlers use).
    fn actor_guild_or_result(&mut self, id: SessionId, actor_low: u32) -> Option<(u32, u32)> {
        let guild_id = self.player_guild.get(&actor_low).copied();
        let rank = guild_id
            .and_then(|guild_id| self.guilds.get(&guild_id))
            .and_then(|guild| guild.rank_of(actor_low));
        if let (Some(guild_id), Some(rank)) = (guild_id, rank) {
            Some((guild_id, rank))
        } else {
            self.send_guild_result(
                id,
                type_command::CREATE,
                "",
                command_error::PLAYER_NOT_IN_GUILD,
            );
            None
        }
    }

    /// Guild opcodes (`GuildHandler.cpp`). Anticheat
    /// kicks for over-long strings are unported — the
    /// handlers drop them silently, like the chat length
    /// family.
    fn on_guild_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(actor_low) = self.actor_low(id) else {
            return;
        };
        match packet.opcode() {
            opcode_id::CMSG_GUILD_QUERY => self.on_guild_query(id, packet),
            opcode_id::CMSG_GUILD_CREATE => self.on_guild_create(id, actor_low, packet),
            opcode_id::CMSG_GUILD_INVITE => self.on_guild_invite(id, actor_low, packet),
            opcode_id::CMSG_GUILD_ACCEPT => self.on_guild_accept(actor_low),
            opcode_id::CMSG_GUILD_DECLINE => self.on_guild_decline(actor_low),
            opcode_id::CMSG_GUILD_INFO => self.on_guild_info(id, actor_low),
            opcode_id::CMSG_GUILD_ROSTER => self.on_guild_roster(id, actor_low),
            opcode_id::CMSG_GUILD_PROMOTE => self.on_guild_promote(id, actor_low, packet, true),
            opcode_id::CMSG_GUILD_DEMOTE => self.on_guild_promote(id, actor_low, packet, false),
            opcode_id::CMSG_GUILD_LEAVE => self.on_guild_leave(id, actor_low),
            opcode_id::CMSG_GUILD_REMOVE => self.on_guild_remove(id, actor_low, packet),
            opcode_id::CMSG_GUILD_DISBAND => self.on_guild_disband(id, actor_low),
            opcode_id::CMSG_GUILD_LEADER => self.on_guild_leader(id, actor_low, packet),
            opcode_id::CMSG_GUILD_MOTD => self.on_guild_motd(id, actor_low, packet),
            opcode_id::CMSG_GUILD_SET_PUBLIC_NOTE => {
                self.on_guild_note(id, actor_low, packet, true);
            }
            opcode_id::CMSG_GUILD_SET_OFFICER_NOTE => {
                self.on_guild_note(id, actor_low, packet, false);
            }
            opcode_id::CMSG_GUILD_RANK => self.on_guild_rank(id, actor_low, packet),
            opcode_id::CMSG_GUILD_ADD_RANK => self.on_guild_add_rank(id, actor_low, packet),
            opcode_id::CMSG_GUILD_DEL_RANK => self.on_guild_del_rank(id, actor_low),
            opcode_id::CMSG_GUILD_INFO_TEXT => self.on_guild_info_text(id, actor_low, packet),
            opcode_id::MSG_SAVE_GUILD_EMBLEM => self.on_guild_emblem(id, actor_low, packet),
            opcode_id::CMSG_PETITION_SHOWLIST => self.on_petition_showlist(id, actor_low, packet),
            opcode_id::CMSG_PETITION_BUY => self.on_petition_buy(id, actor_low, packet),
            opcode_id::CMSG_PETITION_SHOW_SIGNATURES => {
                self.on_petition_show_signatures(id, actor_low, packet);
            }
            opcode_id::CMSG_PETITION_QUERY => self.on_petition_query(id, packet),
            opcode_id::MSG_PETITION_RENAME => self.on_petition_rename(id, actor_low, packet),
            opcode_id::CMSG_PETITION_SIGN => self.on_petition_sign(id, actor_low, packet),
            opcode_id::MSG_PETITION_DECLINE => self.on_petition_decline(actor_low, packet),
            opcode_id::CMSG_OFFER_PETITION => self.on_offer_petition(id, actor_low, packet),
            opcode_id::CMSG_TURN_IN_PETITION => self.on_turn_in_petition(id, actor_low, packet),
            _ => {}
        }
    }

    /// Applies one mail persistence intent.
    fn apply_mail_write(&mut self, write: MailWrite) {
        if let Err(e) = self.deps.mail.apply_mail_write(write) {
            self.log
                .out(Level::Error, &format!("mail write failed: {e}"));
        }
    }

    /// `WorldSession::SendMailResult` (plain form).
    fn send_mail_result(&mut self, id: SessionId, mail_id: u32, action: u32, error: u32) {
        self.reply_packet(id, mail_result_packet(mail_id, action, error, 0, 0, 0));
    }

    /// `CheckMailBox`: mailbox gameobject exists in
    /// the actor's map and is in interaction range
    /// (state/spawn gates join with general GO use).
    fn mailbox_ok(&self, actor_low: u32, mailbox_guid: ObjectGuid) -> bool {
        let Some(map) = self
            .player_map
            .get(&actor_low)
            .and_then(|map_id| self.maps.get(map_id))
        else {
            return false;
        };
        let Some(go) = map.gameobject(mailbox_guid) else {
            return false;
        };
        // GAMEOBJECT_TYPE_MAILBOX = 19.
        if go.info.go_type != 19 {
            return false;
        }
        let Some(player) = map.player(player_guid(actor_low)) else {
            return false;
        };
        let actor_pos = player.unit.world.position();
        let go_pos = go.world.position();
        let dx = actor_pos.x - go_pos.x;
        let dy = actor_pos.y - go_pos.y;
        let dz = actor_pos.z - go_pos.z;
        dx * dx + dy * dy + dz * dz <= 6.0 * 6.0
    }

    /// Mutable access to a live player's mail rows
    /// (the `MasterPlayer` mail list).
    fn with_mail_deferred<R>(
        &mut self,
        guid_low: u32,
        f: impl FnOnce(
            &mut Vec<mangos_db::character_load::MailRow>,
            &mut Vec<mangos_db::character_load::MailedItemRow>,
        ) -> R,
    ) -> Option<R> {
        let map_id = *self.player_map.get(&guid_low)?;
        let player = self
            .maps
            .get_mut(&map_id)?
            .player_mut(player_guid(guid_low))?;
        Some(f(
            &mut player.deferred.mails,
            &mut player.deferred.mailed_items,
        ))
    }

    /// Mail count for a live player (0 when not in a
    /// map).
    fn live_mail_count(&self, guid_low: u32) -> usize {
        self.player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))
            .and_then(|map| map.player(player_guid(guid_low)))
            .map_or(0, |player| player.deferred.mails.len())
    }

    /// Push a freshly sent mail into an online
    /// receiver's list + delivery notice
    /// (`SendMailTo` online arm).
    fn deliver_mail_online(
        &mut self,
        receiver_low: u32,
        mail: mangos_db::character_load::MailRow,
        item: Option<mangos_db::character_load::MailedItemRow>,
    ) {
        let deliver_time = mail.deliver_time;
        let inserted = self
            .with_mail_deferred(receiver_low, |mails, items| {
                mails.insert(0, mail);
                if let Some(item) = item {
                    items.push(item);
                }
            })
            .is_some();
        if !inserted {
            return;
        }
        if let Some(session_id) = self.player_session.get(&receiver_low).copied() {
            let now = unix_now_i64();
            let notify = self
                .sessions
                .get_mut(&session_id)
                .is_some_and(|entry| entry.mail_tracker.note_new_mail(deliver_time, now));
            if notify {
                self.reply_packet(session_id, received_mail_packet());
            }
        }
    }

    /// `MailDraft::SendMailTo` flattened: insert the
    /// mail (+ attachment row), then push to an
    /// online receiver. Returns the new mail id.
    fn send_mail_to(
        &mut self,
        sender_low: u32,
        receiver_low: u32,
        out: OutgoingMail,
        item: Option<&mangos_db::mail::StoredInventoryItem>,
    ) -> Option<u32> {
        let now = unix_now_i64();
        let (deliver_time, expire_time) = mail_times(now, out.cod, out.deliver_delay_secs);
        let new_mail = mangos_db::mail::NewMail {
            message_type: out.message_type,
            stationery: out.stationery,
            mail_template_id: out.mail_template_id,
            sender_low,
            receiver_low,
            subject: out.subject.clone(),
            item_text_id: out.item_text_id,
            expire_time,
            deliver_time,
            money: out.money,
            cod: out.cod,
            checked: out.checked,
            item: item.map(|item| (item.item_guid, item.item_id)),
        };
        let mail_id = match self.deps.mail.insert_mail(new_mail) {
            Ok(id) => id,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("mail insert failed: {e}"));
                return None;
            }
        };
        let row = mangos_db::character_load::MailRow {
            id: mail_id,
            message_type: out.message_type,
            sender_guid: sender_low,
            receiver_guid: receiver_low,
            subject: out.subject,
            item_text_id: out.item_text_id,
            expire_time,
            deliver_time,
            money: money_as_i32(out.money),
            cod: money_as_i32(out.cod),
            checked: out.checked as u8,
            stationery: out.stationery,
            mail_template_id: u32::from(out.mail_template_id),
            has_items: u8::from(item.is_some()),
        };
        let mailed_item = item.map(|item| mangos_db::character_load::MailedItemRow {
            mail_id,
            item: mangos_db::character_load::ItemInstanceRow {
                creator_guid: item.creator_guid,
                gift_creator_guid: item.gift_creator_guid,
                count: item.count,
                duration: item.duration,
                charges: item.charges.clone(),
                flags: item.flags,
                enchantments: item.enchantments.clone(),
                random_property_id: item.random_property_id,
                durability: item.durability,
                text: item.text,
                item_guid: item.item_guid,
                item_id: item.item_id,
                generated_loot: item.generated_loot,
            },
        });
        self.deliver_mail_online(receiver_low, row, mailed_item);
        Some(mail_id)
    }

    /// Mail opcode dispatch (`MailHandler.cpp`).
    fn on_mail_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        match packet.opcode() {
            opcode_id::CMSG_GET_MAIL_LIST => self.on_get_mail_list(id, packet),
            opcode_id::CMSG_ITEM_TEXT_QUERY => self.on_item_text_query(id, packet),
            opcode_id::MSG_QUERY_NEXT_MAIL_TIME => self.on_query_next_mail_time(id),
            _ => {
                let Some(actor_low) = self.actor_low(id) else {
                    return;
                };
                match packet.opcode() {
                    opcode_id::CMSG_SEND_MAIL => self.on_send_mail(id, actor_low, packet),
                    opcode_id::CMSG_MAIL_TAKE_MONEY => {
                        self.on_mail_take_money(id, actor_low, packet);
                    }
                    opcode_id::CMSG_MAIL_TAKE_ITEM => self.on_mail_take_item(id, actor_low, packet),
                    opcode_id::CMSG_MAIL_MARK_AS_READ => {
                        self.on_mail_mark_as_read(id, actor_low, packet);
                    }
                    opcode_id::CMSG_MAIL_RETURN_TO_SENDER => {
                        self.on_mail_return(id, actor_low, packet);
                    }
                    opcode_id::CMSG_MAIL_DELETE => self.on_mail_delete(id, actor_low, packet),
                    opcode_id::CMSG_MAIL_CREATE_TEXT_ITEM => {
                        self.on_mail_create_text_item(id, actor_low, packet);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Trade opcode dispatch (`TradeHandler.cpp`).
    /// The stun/taxi and logging-out gates are not
    /// ported (those states live outside the actor);
    /// alive/distance/team/trial gates are.
    fn on_trade_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(actor_low) = self.actor_low(id) else {
            return;
        };
        match packet.opcode() {
            opcode_id::CMSG_INITIATE_TRADE => self.on_initiate_trade(id, actor_low, packet),
            opcode_id::CMSG_BEGIN_TRADE => self.on_begin_trade(id, actor_low),
            opcode_id::CMSG_BUSY_TRADE => {
                self.trade_cancel(actor_low, true, crate::trade::status::BUSY);
            }
            opcode_id::CMSG_IGNORE_TRADE => {
                self.trade_cancel(actor_low, true, crate::trade::status::IGNORE_YOU);
            }
            opcode_id::CMSG_CANCEL_TRADE => {
                self.trade_cancel(actor_low, true, crate::trade::status::TRADE_CANCELED);
            }
            opcode_id::CMSG_SET_TRADE_GOLD => self.on_set_trade_gold(actor_low, packet),
            opcode_id::CMSG_SET_TRADE_ITEM => self.on_set_trade_item(id, actor_low, packet),
            opcode_id::CMSG_CLEAR_TRADE_ITEM => self.on_clear_trade_item(actor_low, packet),
            opcode_id::CMSG_UNACCEPT_TRADE => {
                if self.trades.contains_key(&actor_low) {
                    self.trade_set_accepted(actor_low, false, true);
                }
            }
            opcode_id::CMSG_ACCEPT_TRADE => self.on_accept_trade(id, actor_low, packet),
            _ => {}
        }
    }

    /// Partner guid low of a live trade side.
    fn trade_partner_low(&self, owner_low: u32) -> Option<u32> {
        self.trades
            .get(&owner_low)
            .map(|trade| ObjectGuid::from_raw(trade.partner_guid).counter())
    }

    /// `SendTradeStatus` to one session.
    fn send_trade_status(&mut self, id: SessionId, status: u32) {
        self.reply_packet(id, crate::trade::status_packet(status));
    }

    /// `TradeData::SetAccepted`.
    fn trade_set_accepted(&mut self, owner_low: u32, state: bool, crosssend: bool) {
        let Some(trade) = self.trades.get_mut(&owner_low) else {
            return;
        };
        trade.accepted = state;
        if state {
            return;
        }
        let partner_low = ObjectGuid::from_raw(trade.partner_guid).counter();
        let target_low = if crosssend { partner_low } else { owner_low };
        if let Some(target) = self.player_session.get(&target_low).copied() {
            self.send_trade_status(target, crate::trade::status::BACK_TO_TRADE);
        }
    }

    /// `TradeData::Update(for_trader = true)`: the
    /// owner's window state to the partner.
    fn trade_send_update(&mut self, owner_low: u32) {
        let Some(trade) = self.trades.get(&owner_low).cloned() else {
            return;
        };
        let partner_low = ObjectGuid::from_raw(trade.partner_guid).counter();
        let Some(partner_session) = self.player_session.get(&partner_low).copied() else {
            return;
        };
        let mut views: [Option<crate::trade::TradeItemView>; crate::trade::TRADE_SLOT_COUNT] =
            std::array::from_fn(|_| None);
        for (index, offered) in trade.items.iter().enumerate() {
            let Some(offered) = offered else { continue };
            match self.deps.trade.inventory_item(owner_low, offered.item_low) {
                Ok(Some(row)) => {
                    if let Some(proto) = self
                        .deps
                        .world_stores
                        .item_prototypes
                        .get(row.item_id)
                        .cloned()
                    {
                        views[index] = Some(crate::trade::TradeItemView::from_stored(&row, &proto));
                    }
                }
                Ok(None) => {}
                Err(e) => self
                    .log
                    .out(Level::Error, &format!("trade item read failed: {e}")),
            }
        }
        self.reply_packet(
            partner_session,
            crate::trade::extended_packet(true, trade.money, &views),
        );
    }

    /// The modification prelude shared by set-item /
    /// clear-item / set-gold: partner un-accepts and
    /// both modification stamps refresh.
    fn trade_touch(&mut self, actor_low: u32, partner_low: u32, now: i64) {
        self.trade_set_accepted(partner_low, false, false);
        if let Some(trade) = self.trades.get_mut(&partner_low) {
            trade.last_modification = now;
        }
        if let Some(trade) = self.trades.get_mut(&actor_low) {
            trade.last_modification = now;
        }
    }

    /// `TradeData::SetItem` (spell clears are no-ops
    /// while trade spells are unported).
    fn trade_set_item(
        &mut self,
        actor_low: u32,
        partner_low: u32,
        slot: usize,
        item: Option<crate::trade::TradeItem>,
    ) {
        {
            let Some(trade) = self.trades.get_mut(&actor_low) else {
                return;
            };
            if trade.items[slot] == item {
                return;
            }
            trade.items[slot] = item;
        }
        self.trade_set_accepted(actor_low, false, false);
        self.trade_set_accepted(partner_low, false, false);
        self.trade_send_update(actor_low);
    }

    /// `Player::TradeCancel`.
    fn trade_cancel(&mut self, actor_low: u32, sendback: bool, status: u32) {
        let Some(trade) = self.trades.remove(&actor_low) else {
            return;
        };
        let partner_low = ObjectGuid::from_raw(trade.partner_guid).counter();
        self.trades.remove(&partner_low);
        if sendback && let Some(actor_session) = self.player_session.get(&actor_low).copied() {
            self.send_trade_status(actor_session, status);
        }
        if let Some(partner_session) = self.player_session.get(&partner_low).copied() {
            self.send_trade_status(partner_session, status);
        }
    }

    /// 3D distance between two live players (`None`
    /// when either is missing or on another map).
    fn players_distance(&self, a_low: u32, b_low: u32) -> Option<f32> {
        let map = self.maps.get(self.player_map.get(&a_low)?)?;
        if self.player_map.get(&b_low) != self.player_map.get(&a_low) {
            return None;
        }
        let a = map.player(player_guid(a_low))?.unit.world.position();
        let b = map.player(player_guid(b_low))?.unit.world.position();
        let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
        Some((dx * dx + dy * dy + dz * dz).sqrt())
    }

    /// Whether a live player is alive.
    fn player_alive(&self, guid_low: u32) -> bool {
        self.player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))
            .and_then(|map| map.player(player_guid(guid_low)))
            .is_some_and(|player| player.unit.is_alive())
    }

    /// `SendNotification(lang_id)`.
    fn send_notification(&mut self, id: SessionId, lang_id: u32) {
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let text = self
            .strings
            .get(money_as_i32(lang_id))
            .map_or("", |entry| entry.text(locale));
        let mut writer = PacketWriter::new();
        writer.write_cstring(text);
        let mut packet = WorldPacket::new(opcode_id::SMSG_NOTIFICATION);
        packet.take_writer(writer);
        self.reply_packet(id, packet);
    }

    /// `HandleInitiateTradeOpcode`.
    #[allow(clippy::too_many_lines)]
    fn on_initiate_trade(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::trade::status;
        if self.trades.contains_key(&actor_low) {
            return;
        }
        let mut reader = packet.reader();
        let Ok(raw) = reader.read_u64_le() else {
            return;
        };
        let other_guid = ObjectGuid::from_wire_raw(raw);
        if !self.player_alive(actor_low) {
            self.send_trade_status(id, status::YOU_DEAD);
            return;
        }
        // `IsLogingOut` has no actor state yet.
        let Some(&map_id) = self.player_map.get(&actor_low) else {
            self.send_trade_status(id, status::TARGET_TO_FAR);
            return;
        };
        let other = self
            .maps
            .get(&map_id)
            .and_then(|map| map.player(other_guid))
            .map(|player| (player.unit.is_alive(), player.unit.race()));
        let Some((other_alive, other_race)) = other else {
            self.send_trade_status(id, status::NO_TARGET);
            return;
        };
        let other_low = other_guid.counter();
        if other_low == actor_low || self.trades.contains_key(&other_low) {
            self.send_trade_status(id, status::BUSY);
            return;
        }
        if !other_alive {
            self.send_trade_status(id, status::TARGET_DEAD);
            return;
        }
        let Some(other_session) = self.player_session.get(&other_low).copied() else {
            self.send_trade_status(id, status::NO_TARGET);
            return;
        };
        if !self.config.allow_two_side_interaction_trade {
            let my_team = self
                .player_name_race_of(actor_low)
                .map_or(0, |(_, race)| self.team_id_for_race(race));
            if self.team_id_for_race(other_race) != my_team {
                self.send_trade_status(id, status::WRONG_FACTION);
                return;
            }
        }
        match self.players_distance(actor_low, other_low) {
            Some(distance) if distance <= crate::trade::TRADE_DISTANCE => {}
            _ => {
                self.send_trade_status(id, status::TARGET_TO_FAR);
                return;
            }
        }
        if self.has_trial_restrictions(id) || self.has_trial_restrictions(other_session) {
            self.send_trade_status(id, status::TRIAL_ACCOUNT);
            return;
        }
        self.trades.insert(
            actor_low,
            crate::trade::TradeData::new(player_guid(other_low).raw()),
        );
        self.trades.insert(
            other_low,
            crate::trade::TradeData::new(player_guid(actor_low).raw()),
        );
        self.reply_packet(
            other_session,
            crate::trade::begin_trade_packet(player_guid(actor_low).raw()),
        );
        // Bot trade auto-flow (`OnTradeRequested`):
        // service-backed requests begin + accept.
        if self.bots.contains_key(&other_low) {
            let now_s = u64::try_from(unix_now_i64()).unwrap_or(0);
            if self.bot_services.should_accept_trade(
                player_guid(other_low),
                player_guid(actor_low),
                now_s,
            ) {
                self.send_trade_status(id, status::OPEN_WINDOW);
                self.trade_set_accepted(other_low, true, false);
            }
        }
    }

    /// `HandleBeginTradeOpcode`.
    fn on_begin_trade(&mut self, id: SessionId, actor_low: u32) {
        let Some(partner_low) = self.trade_partner_low(actor_low) else {
            return;
        };
        if let Some(partner_session) = self.player_session.get(&partner_low).copied() {
            self.send_trade_status(partner_session, crate::trade::status::OPEN_WINDOW);
        }
        self.send_trade_status(id, crate::trade::status::OPEN_WINDOW);
    }

    /// Both trade sides exist (`GetTradeData` +
    /// trader's data checks in the set handlers).
    fn trade_pair(&self, actor_low: u32) -> Option<u32> {
        let partner_low = self.trade_partner_low(actor_low)?;
        self.trades
            .contains_key(&partner_low)
            .then_some(partner_low)
    }

    /// `HandleSetTradeGoldOpcode`.
    fn on_set_trade_gold(&mut self, actor_low: u32, packet: &WorldPacket) {
        let Some(partner_low) = self.trade_pair(actor_low) else {
            return;
        };
        let mut reader = packet.reader();
        let Ok(gold) = reader.read_u32_le() else {
            return;
        };
        if self
            .player_money(actor_low)
            .is_some_and(|money| gold > money)
        {
            return;
        }
        let now = unix_now_i64();
        self.trade_touch(actor_low, partner_low, now);
        // `TradeData::SetMoney`.
        {
            let Some(trade) = self.trades.get_mut(&actor_low) else {
                return;
            };
            if trade.money == gold {
                return;
            }
            trade.money = gold;
        }
        self.trade_set_accepted(actor_low, false, false);
        self.trade_set_accepted(partner_low, false, false);
        self.trade_send_update(actor_low);
    }

    /// `HandleSetTradeItemOpcode`.
    fn on_set_trade_item(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::trade::status;
        let Some(partner_low) = self.trade_pair(actor_low) else {
            return;
        };
        let mut reader = packet.reader();
        let (Ok(trade_slot), Ok(bag), Ok(slot)) =
            (reader.read_u8(), reader.read_u8(), reader.read_u8())
        else {
            return;
        };
        if usize::from(trade_slot) >= crate::trade::TRADE_SLOT_COUNT {
            self.send_trade_status(id, status::TRADE_CANCELED);
            return;
        }
        let row = match self
            .deps
            .trade
            .item_at_pos(actor_low, u32::from(bag), u32::from(slot))
        {
            Ok(row) => row,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("trade item read failed: {e}"));
                None
            }
        };
        let Some(row) = row else {
            self.send_trade_status(id, status::TRADE_CANCELED);
            return;
        };
        if usize::from(trade_slot) != crate::trade::TRADE_SLOT_NONTRADED {
            let tradeable = self
                .deps
                .world_stores
                .item_prototypes
                .get(row.item_id)
                .cloned()
                .is_some_and(|proto| crate::trade::can_be_traded(&row, &proto));
            if !tradeable {
                self.send_trade_status(id, status::TRADE_CANCELED);
                return;
            }
        }
        if crate::trade::is_bank_pos(u32::from(bag), u32::from(slot)) {
            self.send_trade_status(id, status::TRADE_CANCELED);
            return;
        }
        if self
            .trades
            .get(&actor_low)
            .is_some_and(|trade| trade.has_item(row.item_guid))
        {
            self.send_trade_status(id, status::TRADE_CANCELED);
            return;
        }
        let now = unix_now_i64();
        self.trade_touch(actor_low, partner_low, now);
        self.trade_set_item(
            actor_low,
            partner_low,
            usize::from(trade_slot),
            Some(crate::trade::TradeItem {
                item_low: row.item_guid,
                bag: row.bag,
                slot: row.slot,
            }),
        );
    }

    /// `HandleClearTradeItemOpcode`.
    fn on_clear_trade_item(&mut self, actor_low: u32, packet: &WorldPacket) {
        let Some(partner_low) = self.trade_pair(actor_low) else {
            return;
        };
        let mut reader = packet.reader();
        let Ok(trade_slot) = reader.read_u8() else {
            return;
        };
        if usize::from(trade_slot) >= crate::trade::TRADE_SLOT_COUNT {
            return;
        }
        let now = unix_now_i64();
        self.trade_touch(actor_low, partner_low, now);
        self.trade_set_item(actor_low, partner_low, usize::from(trade_slot), None);
    }

    /// `HandleAcceptTradeOpcode` (trade spells and
    /// the bot-service hook unported; see the module
    /// docs in `trade.rs`).
    #[allow(clippy::too_many_lines)]
    fn on_accept_trade(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::trade::status;
        if packet.reader().read_u32_le().is_err() {
            return;
        }
        let Some(my_trade) = self.trades.get(&actor_low).cloned() else {
            return;
        };
        let now = unix_now_i64();
        // `time_t` seconds granularity: the delay
        // compares whole seconds times 1000, exactly
        // like the fork's `difftime` expression.
        if (now - my_trade.last_modification).saturating_mul(1000)
            < crate::trade::SCAM_PREVENTION_DELAY_MS
        {
            self.send_trade_status(id, status::BACK_TO_TRADE);
            return;
        }
        if let Some(trade) = self.trades.get_mut(&actor_low) {
            trade.last_modification = now;
        }
        let partner_low = ObjectGuid::from_raw(my_trade.partner_guid).counter();
        let Some(his_trade) = self.trades.get(&partner_low).cloned() else {
            return;
        };
        let Some(partner_session) = self.player_session.get(&partner_low).copied() else {
            return;
        };
        self.trade_set_accepted(actor_low, true, false);
        match self.players_distance(actor_low, partner_low) {
            Some(distance) if distance <= crate::trade::TRADE_DISTANCE => {}
            _ => {
                self.send_trade_status(id, status::TARGET_TO_FAR);
                return;
            }
        }
        let my_money = self.player_money(actor_low).unwrap_or(0);
        let his_money = self.player_money(partner_low).unwrap_or(0);
        if my_trade.money > my_money {
            self.send_notification(id, crate::trade::LANG_NOT_ENOUGH_GOLD);
            self.trade_set_accepted(actor_low, false, true);
            return;
        }
        if his_trade.money > his_money {
            self.send_notification(partner_session, crate::trade::LANG_NOT_ENOUGH_GOLD);
            self.trade_set_accepted(partner_low, false, true);
            return;
        }
        let max_money = i64::from(MAX_MONEY_AMOUNT);
        if (i64::from(my_money) + i64::from(his_trade.money) - i64::from(my_trade.money)
            > max_money)
            || (i64::from(his_money) + i64::from(my_trade.money) - i64::from(his_trade.money)
                > max_money)
        {
            self.send_trade_status(id, status::TRADE_CANCELED);
            return;
        }
        if !self.config.gm_allow_trades {
            let is_gm = |session: SessionId| {
                self.sessions
                    .get(&session)
                    .is_some_and(|entry| entry.auth.security > security::PLAYER)
            };
            if is_gm(id) || is_gm(partner_session) {
                self.send_trade_status(id, status::TRADE_CANCELED);
                return;
            }
        }
        for (owner_low, trade) in [(actor_low, &my_trade), (partner_low, &his_trade)] {
            for offered in trade.traded_items() {
                let row = match self.deps.trade.inventory_item(owner_low, offered.item_low) {
                    Ok(row) => row,
                    Err(e) => {
                        self.log
                            .out(Level::Error, &format!("trade item read failed: {e}"));
                        None
                    }
                };
                let Some(row) = row else { continue };
                let tradeable = self
                    .deps
                    .world_stores
                    .item_prototypes
                    .get(row.item_id)
                    .cloned()
                    .is_some_and(|proto| crate::trade::can_be_traded(&row, &proto));
                if !tradeable {
                    self.send_trade_status(id, status::TRADE_CANCELED);
                    return;
                }
            }
        }
        if !his_trade.accepted {
            self.send_trade_status(partner_session, status::TRADE_ACCEPT);
            return;
        }
        self.complete_trade(id, actor_low, partner_low, partner_session, now);
    }

    /// The both-accepted tail of the accept handler:
    /// capacity test, item swap, money swap, cleanup.
    fn complete_trade(
        &mut self,
        id: SessionId,
        actor_low: u32,
        partner_low: u32,
        partner_session: SessionId,
        now: i64,
    ) {
        use crate::trade::status;
        let (Some(my_trade), Some(his_trade)) = (
            self.trades.get(&actor_low).cloned(),
            self.trades.get(&partner_low).cloned(),
        ) else {
            return;
        };
        self.send_trade_status(partner_session, status::TRADE_ACCEPT);
        let free_slots = |world: &Self, owner: u32| {
            world
                .deps
                .trade
                .free_backpack_slots(owner)
                .unwrap_or_else(|e| {
                    world
                        .log
                        .out(Level::Error, &format!("trade capacity read failed: {e}"));
                    0
                })
        };
        let my_capacity = free_slots(self, actor_low) + my_trade.traded_count();
        let his_capacity = free_slots(self, partner_low) + his_trade.traded_count();
        if his_trade.traded_count() > my_capacity {
            self.send_notification(id, crate::trade::LANG_NOT_FREE_TRADE_SLOTS);
            self.send_notification(
                partner_session,
                crate::trade::LANG_NOT_PARTNER_FREE_TRADE_SLOTS,
            );
            self.trade_reset_accepts(actor_low, partner_low, now);
            return;
        }
        if my_trade.traded_count() > his_capacity {
            self.send_notification(id, crate::trade::LANG_NOT_PARTNER_FREE_TRADE_SLOTS);
            self.send_notification(partner_session, crate::trade::LANG_NOT_FREE_TRADE_SLOTS);
            self.trade_reset_accepts(actor_low, partner_low, now);
            return;
        }
        // `MoveItems`, pairwise per slot.
        for slot in 0..crate::trade::TRADE_SLOT_TRADED_COUNT {
            self.move_traded_item(actor_low, partner_low, my_trade.items[slot]);
            self.move_traded_item(partner_low, actor_low, his_trade.items[slot]);
        }
        self.modify_money(actor_low, -i64::from(my_trade.money));
        self.modify_money(actor_low, i64::from(his_trade.money));
        self.modify_money(partner_low, -i64::from(his_trade.money));
        self.modify_money(partner_low, i64::from(my_trade.money));
        self.trades.remove(&actor_low);
        self.trades.remove(&partner_low);
        self.send_trade_status(partner_session, status::TRADE_COMPLETE);
        self.send_trade_status(id, status::TRADE_COMPLETE);
        self.bot_service_trade_completed(actor_low, partner_low, &my_trade, &his_trade);
    }

    /// `BotServiceMgr::OnTradeCompleted` delivery:
    /// portal casts triggered; enchant/transmute/
    /// mooncloth resolve to the fork's whisper +
    /// log (item application lands with item saves).
    fn bot_service_trade_completed(
        &mut self,
        actor_low: u32,
        partner_low: u32,
        my_trade: &crate::trade::TradeData,
        his_trade: &crate::trade::TradeData,
    ) {
        use mangos_bots::services::ServiceType;
        use mangos_spells::SpellRng as _;
        let (bot_low, player_low, gold_received) = if self.bots.contains_key(&partner_low) {
            (partner_low, actor_low, my_trade.money)
        } else if self.bots.contains_key(&actor_low) {
            (actor_low, partner_low, his_trade.money)
        } else {
            return;
        };
        let now_s = u64::try_from(unix_now_i64()).unwrap_or(0);
        let mut rng = WorldRng::from_entropy();
        let mut pick = |lo: u32, hi: u32| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                rng.irand(lo as i32, hi as i32) as u32
            }
        };
        let (actions, delivered) = self.bot_services.on_trade_completed(
            player_guid(bot_low),
            player_guid(player_low),
            gold_received,
            now_s,
            &mut pick,
        );
        self.apply_chat_actions(actions);
        let Some(req) = delivered else {
            return;
        };
        match req.service {
            ServiceType::Portal if req.spell_id != 0 => {
                if let Some(map) = self
                    .player_map
                    .get(&bot_low)
                    .copied()
                    .and_then(|id| self.maps.get_mut(&id))
                {
                    map.cast_spell(
                        player_guid(bot_low),
                        req.spell_id,
                        mangos_map::spells::CastTarget::SelfCast,
                        mangos_map::spells::CastOptions {
                            triggered: true,
                            skip_known: true,
                        },
                    );
                }
                self.drain_map_events();
            }
            ServiceType::Enchant | ServiceType::Transmute | ServiceType::Mooncloth => {
                self.log.out(
                    Level::Detail,
                    &format!(
                        "BotServiceMgr: {:?} delivered by bot {bot_low} (spell {})",
                        req.service, req.spell_id
                    ),
                );
            }
            _ => {}
        }
    }

    /// The capacity-failure reset: both sides
    /// un-accept, partner's stamp refreshes.
    fn trade_reset_accepts(&mut self, actor_low: u32, partner_low: u32, now: i64) {
        self.trade_set_accepted(actor_low, false, false);
        self.trade_set_accepted(partner_low, false, false);
        if let Some(trade) = self.trades.get_mut(&partner_low) {
            trade.last_modification = now;
        }
    }

    /// One `MoveItems` transfer; failures only log,
    /// like the fork's store fallbacks.
    fn move_traded_item(
        &mut self,
        from_low: u32,
        to_low: u32,
        offered: Option<crate::trade::TradeItem>,
    ) {
        let Some(offered) = offered else {
            return;
        };
        let row = match self.deps.trade.inventory_item(from_low, offered.item_low) {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("trade item read failed: {e}"));
                return;
            }
        };
        let slot = match self.deps.trade.first_free_backpack_slot(to_low) {
            Ok(Some(slot)) => slot,
            Ok(None) => {
                self.log.out(
                    Level::Error,
                    &format!("trade: no backpack slot storing item {}", offered.item_low),
                );
                return;
            }
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("trade slot read failed: {e}"));
                return;
            }
        };
        let gift_creator = (row.gift_creator_guid != 0).then_some(from_low);
        if let Err(e) = self.deps.trade.move_item(
            from_low,
            to_low,
            offered.item_low,
            row.item_id,
            slot,
            gift_creator,
        ) {
            self.log
                .out(Level::Error, &format!("trade item move failed: {e}"));
        }
    }

    /// Auction opcode dispatch
    /// (`AuctionHouseHandler.cpp`). The GM self-guid
    /// auctioneer case returns no house (auction
    /// access modes are Slice C), and list queries
    /// answer synchronously.
    fn on_auction_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(actor_low) = self.actor_low(id) else {
            return;
        };
        match packet.opcode() {
            opcode_id::MSG_AUCTION_HELLO => self.on_auction_hello(id, actor_low, packet),
            opcode_id::CMSG_AUCTION_SELL_ITEM => self.on_auction_sell(id, actor_low, packet),
            opcode_id::CMSG_AUCTION_PLACE_BID => self.on_auction_bid(id, actor_low, packet),
            opcode_id::CMSG_AUCTION_REMOVE_ITEM => {
                self.on_auction_remove(id, actor_low, packet);
            }
            opcode_id::CMSG_AUCTION_LIST_ITEMS => self.on_auction_list(id, actor_low, packet),
            opcode_id::CMSG_AUCTION_LIST_OWNER_ITEMS => {
                self.on_auction_list_owner(id, actor_low, packet);
            }
            opcode_id::CMSG_AUCTION_LIST_BIDDER_ITEMS => {
                self.on_auction_list_bidder(id, actor_low, packet);
            }
            _ => {}
        }
    }

    /// `GetCheckedAuctionHouseForAuctioneer` reduced
    /// like the petition gate: creature exists with
    /// the auctioneer flag, alive, in interaction
    /// distance. Returns the DBC house entry and the
    /// house object index.
    fn checked_auction_house(
        &self,
        actor_low: u32,
        auctioneer_guid: ObjectGuid,
    ) -> Option<(mangos_world_data::auction_house::AuctionHouseEntry, usize)> {
        use mangos_protocol::update_field_table::offsets;
        if auctioneer_guid == player_guid(actor_low) {
            return None;
        }
        let map = self
            .player_map
            .get(&actor_low)
            .and_then(|map_id| self.maps.get(map_id))?;
        let creature = map.creature(auctioneer_guid)?;
        let flags = creature
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_NPC_FLAGS);
        if flags & crate::auction::NPC_FLAG_AUCTIONEER == 0 || !creature.unit.is_alive() {
            return None;
        }
        let player = map.player(player_guid(actor_low))?;
        let a = player.unit.world.position();
        let b = creature.unit.world.position();
        let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
        if dx * dx + dy * dy + dz * dz > 6.0 * 6.0 {
            return None;
        }
        let house_id = if self.config.allow_two_side_interaction_auction {
            1
        } else {
            let faction_template = self
                .deps
                .world_stores
                .creature_templates
                .get(creature.original_entry)
                .map_or(0, |template| template.faction);
            crate::auction::house_id_for_faction_template(faction_template)
        };
        let house = self.deps.world_stores.auction_houses.get(house_id)?;
        let index = *self.auction_house_map.get(&house_id)?;
        Some((house, index))
    }

    /// One auction system mail (`MailDraft::SendMailTo`
    /// with an `AuctionEntry` sender): instant
    /// delivery, 30-day expiry, COPIED check mask.
    /// Skips receivers with no account, like the
    /// fork's existence checks; item mails re-own or
    /// destroy the item per the caller's `won` arm —
    /// housekeeping stays with the caller.
    fn send_auction_mail(
        &mut self,
        receiver_low: u32,
        house_id: u32,
        subject: String,
        body: Option<&str>,
        money: u32,
        item: Option<mangos_db::character_load::ItemInstanceRow>,
    ) -> bool {
        let account = match self.deps.auction.account_id_by_guid(receiver_low) {
            Ok(account) => account,
            Err(e) => {
                self.log.out(
                    Level::Error,
                    &format!("auction mail account read failed: {e}"),
                );
                None
            }
        };
        if account.is_none() && !self.player_session.contains_key(&receiver_low) {
            return false;
        }
        let item_text_id = match body {
            Some(text) if !text.is_empty() => match self.deps.mail.create_item_text(text) {
                Ok(id) => id,
                Err(e) => {
                    self.log
                        .out(Level::Error, &format!("auction mail text failed: {e}"));
                    0
                }
            },
            _ => 0,
        };
        let now = unix_now_i64();
        let new_mail = mangos_db::mail::NewMail {
            message_type: crate::mail::message_type::AUCTION,
            stationery: 62,
            mail_template_id: 0,
            sender_low: house_id,
            receiver_low,
            subject: subject.clone(),
            item_text_id,
            expire_time: now + 30 * 86_400,
            deliver_time: now,
            money,
            cod: 0,
            checked: crate::mail::check_mask::COPIED,
            item: item.as_ref().map(|row| (row.item_guid, row.item_id)),
        };
        let mail_id = match self.deps.mail.insert_mail(new_mail) {
            Ok(id) => id,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("auction mail insert failed: {e}"));
                return false;
            }
        };
        let row = mangos_db::character_load::MailRow {
            id: mail_id,
            message_type: crate::mail::message_type::AUCTION,
            sender_guid: house_id,
            receiver_guid: receiver_low,
            subject,
            item_text_id,
            expire_time: now + 30 * 86_400,
            deliver_time: now,
            money: money_as_i32(money),
            cod: 0,
            checked: 4,
            stationery: 62,
            mail_template_id: 0,
            has_items: u8::from(item.is_some()),
        };
        let mailed =
            item.map(|row| mangos_db::character_load::MailedItemRow { mail_id, item: row });
        self.deliver_mail_online(receiver_low, row, mailed);
        true
    }

    /// Outbid refund + notification
    /// (`SendAuctionOutbiddedMail`).
    fn send_auction_outbid_mail(&mut self, auction: &crate::auction::AuctionEntry) {
        if let Some(session) = self.player_session.get(&auction.bidder).copied() {
            let random_id = self
                .auction_items
                .get(&auction.item_guid_low)
                .map_or(0, crate::auction::random_property_of);
            self.reply_packet(
                session,
                crate::auction::bidder_notification_packet(auction, false, random_id),
            );
        }
        self.send_auction_mail(
            auction.bidder,
            auction.house.house_id,
            crate::auction::mail_subject(
                auction.item_template,
                crate::auction::mail_answer::OUTBIDDED,
            ),
            None,
            auction.bid,
            None,
        );
    }

    /// Cancel-to-bidder refund
    /// (`SendAuctionCancelledToBidderMail`).
    fn send_auction_cancelled_bidder_mail(&mut self, auction: &crate::auction::AuctionEntry) {
        if let Some(session) = self.player_session.get(&auction.bidder).copied() {
            let random_id = self
                .auction_items
                .get(&auction.item_guid_low)
                .map_or(0, crate::auction::random_property_of);
            self.reply_packet(
                session,
                crate::auction::removed_notification_packet(auction, random_id),
            );
        }
        self.send_auction_mail(
            auction.bidder,
            auction.house.house_id,
            crate::auction::mail_subject(
                auction.item_template,
                crate::auction::mail_answer::CANCELLED_TO_BIDDER,
            ),
            None,
            auction.bid,
            None,
        );
    }

    /// `SendAuctionSuccessfulMail`: profit to the
    /// seller (bid + deposit - cut).
    fn send_auction_successful_mail(&mut self, auction: &crate::auction::AuctionEntry) {
        if let Some(session) = self.player_session.get(&auction.owner).copied() {
            let random_id = self
                .auction_items
                .get(&auction.item_guid_low)
                .map_or(0, crate::auction::random_property_of);
            self.reply_packet(
                session,
                crate::auction::owner_notification_packet(auction, true, random_id),
            );
        }
        let cut = auction.auction_cut(self.config.rate_auction_cut);
        let profit = auction
            .bid
            .saturating_add(auction.deposit)
            .saturating_sub(cut);
        let body = crate::auction::successful_body(
            auction.bidder,
            auction.bid,
            auction.buyout,
            auction.deposit,
            cut,
        );
        self.send_auction_mail(
            auction.owner,
            auction.house.house_id,
            crate::auction::mail_subject(
                auction.item_template,
                crate::auction::mail_answer::SUCCESSFUL,
            ),
            Some(&body),
            profit,
            None,
        );
    }

    /// `SendAuctionWonMail`: the item to the winner.
    /// Removes the item from the auction item map;
    /// destroys it if the winner no longer exists.
    fn send_auction_won_mail(&mut self, auction: &crate::auction::AuctionEntry) {
        let Some(item) = self.auction_items.get(&auction.item_guid_low).cloned() else {
            return;
        };
        if let Err(e) = self
            .deps
            .auction
            .set_item_owner(auction.item_guid_low, auction.bidder)
        {
            self.log
                .out(Level::Error, &format!("auction won re-own failed: {e}"));
        }
        if let Some(session) = self.player_session.get(&auction.bidder).copied() {
            let random_id = crate::auction::random_property_of(&item);
            self.reply_packet(
                session,
                crate::auction::bidder_notification_packet(auction, true, random_id),
            );
        }
        let body = crate::auction::won_body(auction.owner, auction.bid, auction.buyout);
        let sent = self.send_auction_mail(
            auction.bidder,
            auction.house.house_id,
            crate::auction::mail_subject(auction.item_template, crate::auction::mail_answer::WON),
            Some(&body),
            0,
            Some(item),
        );
        self.auction_items.remove(&auction.item_guid_low);
        if !sent
            && let Err(e) = self
                .deps
                .auction
                .delete_item_instance(auction.item_guid_low)
        {
            self.log.out(
                Level::Error,
                &format!("auction won item delete failed: {e}"),
            );
        }
    }

    /// `SendAuctionExpiredMail`: the item back to the
    /// seller. Removes the item from the auction map.
    fn send_auction_expired_mail(&mut self, auction: &crate::auction::AuctionEntry) {
        let Some(item) = self.auction_items.get(&auction.item_guid_low).cloned() else {
            self.log.out(
                Level::Error,
                &format!("auction item {} not found, and lost", auction.item_guid_low),
            );
            return;
        };
        if let Some(session) = self.player_session.get(&auction.owner).copied() {
            let random_id = crate::auction::random_property_of(&item);
            self.reply_packet(
                session,
                crate::auction::owner_notification_packet(auction, false, random_id),
            );
        }
        let sent = self.send_auction_mail(
            auction.owner,
            auction.house.house_id,
            crate::auction::mail_subject(
                auction.item_template,
                crate::auction::mail_answer::EXPIRED,
            ),
            None,
            0,
            Some(item),
        );
        self.auction_items.remove(&auction.item_guid_low);
        if !sent
            && let Err(e) = self
                .deps
                .auction
                .delete_item_instance(auction.item_guid_low)
        {
            self.log.out(
                Level::Error,
                &format!("auction expired item delete failed: {e}"),
            );
        }
    }

    /// `HandleAuctionHelloOpcode`.
    fn on_auction_hello(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw) = reader.read_u64_le() else {
            return;
        };
        let guid = ObjectGuid::from_raw(raw);
        let Some((house, _)) = self.checked_auction_house(actor_low, guid) else {
            return;
        };
        self.reply_packet(id, crate::auction::hello_packet(guid, house.house_id));
    }

    /// `SendAuctionCommandResult` to one session.
    fn send_auction_result(
        &mut self,
        id: SessionId,
        auction: Option<&crate::auction::AuctionEntry>,
        action_id: u32,
        error_code: u32,
        inv_error: u32,
    ) {
        self.reply_packet(
            id,
            crate::auction::command_result_packet(auction, action_id, error_code, inv_error),
        );
    }

    /// `HandleAuctionSellItem`.
    #[allow(clippy::too_many_lines)]
    fn on_auction_sell(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::auction::{action, error};
        let mut reader = packet.reader();
        let (Ok(auctioneer_raw), Ok(item_raw), Ok(bid), Ok(buyout), Ok(etime)) = (
            reader.read_u64_le(),
            reader.read_u64_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
        ) else {
            return;
        };
        if bid == 0 || etime == 0 {
            return;
        }
        if bid > crate::auction::MAX_AUCTION_PRICE || buyout > crate::auction::MAX_AUCTION_PRICE {
            self.send_auction_result(id, None, action::STARTED, error::NOT_ENOUGH_MONEY, 0);
            return;
        }
        if buyout != 0 && bid > buyout {
            self.send_auction_result(id, None, action::STARTED, error::HIGHER_BID, 0);
            return;
        }
        let security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        if !self.config.gm_allow_trades && security > security::PLAYER {
            self.send_auction_result(id, None, action::STARTED, error::RESTRICTED_ACCOUNT, 0);
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_auction_result(id, None, action::STARTED, error::RESTRICTED_ACCOUNT, 0);
            return;
        }
        let Some((house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            self.send_auction_result(id, None, action::STARTED, error::DATABASE, 0);
            return;
        };
        let account_id = self.sessions.get(&id).map_or(0, |entry| entry.account_id);
        let limit = self.config.auction_account_concurrent_limit;
        if limit != 0
            && self.auction_houses[house_index]
                .values()
                .filter(|entry| entry.owner_account == account_id)
                .count()
                >= limit as usize
        {
            self.reply_packet(
                id,
                build_sys_message("You have reached the limit of active auctions on your account."),
            );
            self.send_auction_result(id, None, action::STARTED, error::DATABASE, 0);
            return;
        }
        let etime_secs = etime.saturating_mul(60);
        if !matches!(
            etime_secs,
            crate::auction::MIN_AUCTION_TIME | 28_800 | 86_400
        ) {
            self.send_auction_result(id, None, action::STARTED, error::DATABASE, 0);
            return;
        }
        let item_guid = ObjectGuid::from_raw(item_raw);
        if item_raw == 0 {
            self.send_auction_result(id, None, action::STARTED, error::ITEM_NOT_FOUND, 0);
            return;
        }
        let item_low = item_guid.counter();
        if self.auction_items.contains_key(&item_low) {
            self.send_auction_result(
                id,
                None,
                action::STARTED,
                error::INVENTORY,
                crate::auction::EQUIP_ERR_ITEM_NOT_FOUND,
            );
            return;
        }
        let stored = match self.deps.trade.inventory_item(actor_low, item_low) {
            Ok(stored) => stored,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("auction sell item read failed: {e}"));
                None
            }
        };
        let Some(stored) = stored else {
            self.send_auction_result(
                id,
                None,
                action::STARTED,
                error::INVENTORY,
                crate::auction::EQUIP_ERR_ITEM_NOT_FOUND,
            );
            return;
        };
        let is_bank_pos = stored.bag == 0 && stored.slot >= 39;
        let proto = self
            .deps
            .world_stores
            .item_prototypes
            .get(stored.item_id)
            .cloned();
        let tradeable = proto
            .as_ref()
            .is_some_and(|proto| crate::trade::can_be_traded(&stored, proto));
        let conjured = proto.as_ref().is_some_and(|proto| proto.flags & 0x2 != 0);
        if is_bank_pos || !tradeable || conjured || stored.duration != 0 {
            self.send_auction_result(
                id,
                None,
                action::STARTED,
                error::INVENTORY,
                crate::auction::EQUIP_ERR_ITEM_NOT_FOUND,
            );
            return;
        }
        let Some(proto) = proto else {
            return;
        };
        let deposit = crate::auction::auction_deposit(
            &house,
            etime_secs,
            proto.sell_price,
            stored.count,
            self.config.auction_deposit_min,
            self.config.rate_auction_deposit,
        );
        if self.player_money(actor_low).unwrap_or(0) < deposit {
            self.send_auction_result(id, None, action::STARTED, error::NOT_ENOUGH_MONEY, 0);
            return;
        }
        self.modify_money(actor_low, -i64::from(deposit));
        #[allow(clippy::cast_precision_loss)]
        let auction_time = (etime_secs as f32 * self.config.rate_auction_time) as u32;
        let now = unix_now_i64();
        let entry = crate::auction::AuctionEntry {
            id: self.next_auction_id,
            item_guid_low: item_low,
            item_template: stored.item_id,
            owner: actor_low,
            owner_account: account_id,
            start_bid: bid,
            bid: 0,
            buyout,
            locked_ip: self
                .sessions
                .get(&id)
                .map_or_else(String::new, |entry| entry.remote_ip.clone()),
            deposit_time: now,
            expire_time: now + i64::from(auction_time),
            bidder: 0,
            deposit,
            house,
        };
        self.next_auction_id = self.next_auction_id.saturating_add(1);
        let item_row = mangos_db::character_load::ItemInstanceRow {
            creator_guid: stored.creator_guid,
            gift_creator_guid: stored.gift_creator_guid,
            count: stored.count,
            duration: stored.duration,
            charges: stored.charges.clone(),
            flags: stored.flags,
            enchantments: stored.enchantments.clone(),
            random_property_id: stored.random_property_id,
            durability: stored.durability,
            text: stored.text,
            item_guid: stored.item_guid,
            item_id: stored.item_id,
            generated_loot: stored.generated_loot,
        };
        if let Err(e) = self.deps.auction.detach_inventory_item(actor_low, item_low) {
            self.log
                .out(Level::Error, &format!("auction detach failed: {e}"));
        }
        if let Err(e) = self.deps.auction.insert_auction(entry.to_stored()) {
            self.log
                .out(Level::Error, &format!("auction insert failed: {e}"));
        }
        self.auction_items.insert(item_low, item_row);
        self.auction_houses[house_index].insert(entry.id, entry.clone());
        self.send_auction_result(id, Some(&entry), action::STARTED, error::OK, 0);
    }

    /// `HandleAuctionPlaceBid`.
    #[allow(clippy::too_many_lines)]
    fn on_auction_bid(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::auction::{action, error};
        let mut reader = packet.reader();
        let (Ok(auctioneer_raw), Ok(auction_id), Ok(price)) = (
            reader.read_u64_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
        ) else {
            return;
        };
        let security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        if !self.config.gm_allow_trades && security > security::PLAYER {
            self.send_auction_result(id, None, action::BID_PLACED, error::RESTRICTED_ACCOUNT, 0);
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_auction_result(id, None, action::BID_PLACED, error::RESTRICTED_ACCOUNT, 0);
            return;
        }
        if auction_id == 0 || price == 0 {
            self.send_auction_result(id, None, action::BID_PLACED, error::ITEM_NOT_FOUND, 0);
            return;
        }
        let Some((_house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            self.send_auction_result(id, None, action::BID_PLACED, error::ITEM_NOT_FOUND, 0);
            return;
        };
        let Some(auction) = self.auction_houses[house_index].get(&auction_id).cloned() else {
            self.send_auction_result(id, None, action::BID_PLACED, error::ITEM_NOT_FOUND, 0);
            return;
        };
        if auction.owner == actor_low {
            self.send_auction_result(id, None, action::BID_PLACED, error::BID_OWN, 0);
            return;
        }
        let account_id = self.sessions.get(&id).map_or(0, |entry| entry.account_id);
        if !self.player_session.contains_key(&auction.owner) && auction.owner_account == account_id
        {
            self.send_auction_result(id, None, action::BID_PLACED, error::BID_OWN, 0);
            return;
        }
        if price < auction.start_bid {
            self.send_auction_result(id, None, action::BID_PLACED, error::BID_INCREMENT, 0);
            return;
        }
        if price <= auction.bid {
            self.send_auction_result(id, Some(&auction), action::BID_PLACED, error::HIGHER_BID, 0);
            return;
        }
        if (price < auction.buyout || auction.buyout == 0)
            && price < auction.bid.saturating_add(auction.out_bid())
        {
            self.send_auction_result(
                id,
                Some(&auction),
                action::BID_PLACED,
                error::BID_INCREMENT,
                0,
            );
            return;
        }
        if price > self.player_money(actor_low).unwrap_or(0) {
            return;
        }
        let is_buyout = !(price < auction.buyout || auction.buyout == 0);
        // Charge (difference only when raising an own
        // bid), refund the outbid bidder by mail.
        if is_buyout {
            if actor_low == auction.bidder {
                self.modify_money(
                    actor_low,
                    -i64::from(auction.buyout.saturating_sub(auction.bid)),
                );
            } else {
                self.modify_money(actor_low, -i64::from(auction.buyout));
                if auction.bidder != 0 {
                    self.send_auction_outbid_mail(&auction);
                }
            }
        } else if actor_low == auction.bidder {
            self.modify_money(actor_low, -i64::from(price.saturating_sub(auction.bid)));
        } else {
            self.modify_money(actor_low, -i64::from(price));
            if auction.bidder != 0 {
                self.send_auction_outbid_mail(&auction);
            }
        }
        if is_buyout {
            let mut won = auction.clone();
            won.bidder = actor_low;
            won.bid = auction.buyout;
            self.send_auction_successful_mail(&won);
            self.send_auction_won_mail(&won);
            self.send_auction_result(id, Some(&won), action::BID_PLACED, error::OK, 0);
            self.auction_houses[house_index].remove(&auction_id);
            if let Err(e) = self.deps.auction.delete_auction(auction_id) {
                self.log
                    .out(Level::Error, &format!("auction delete failed: {e}"));
            }
            return;
        }
        let updated = {
            let Some(entry) = self.auction_houses[house_index].get_mut(&auction_id) else {
                return;
            };
            entry.bidder = actor_low;
            entry.bid = price;
            entry.clone()
        };
        if let Some(owner_session) = self.player_session.get(&auction.owner).copied() {
            let random_id = self
                .auction_items
                .get(&auction.item_guid_low)
                .map_or(0, crate::auction::random_property_of);
            self.reply_packet(
                owner_session,
                crate::auction::owner_notification_packet(&updated, false, random_id),
            );
        }
        if let Err(e) = self.deps.auction.update_bid(auction_id, actor_low, price) {
            self.log
                .out(Level::Error, &format!("auction bid update failed: {e}"));
        }
        self.send_auction_result(id, Some(&updated), action::BID_PLACED, error::OK, 0);
    }

    /// `HandleAuctionRemoveItem`.
    fn on_auction_remove(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use crate::auction::{action, error};
        let mut reader = packet.reader();
        let (Ok(auctioneer_raw), Ok(auction_id)) = (reader.read_u64_le(), reader.read_u32_le())
        else {
            return;
        };
        let Some((_house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            return;
        };
        let auction = self.auction_houses[house_index].get(&auction_id).cloned();
        let Some(auction) = auction.filter(|entry| entry.owner == actor_low) else {
            self.send_auction_result(id, None, action::REMOVED, error::DATABASE, 0);
            return;
        };
        let Some(item) = self.auction_items.get(&auction.item_guid_low).cloned() else {
            self.send_auction_result(
                id,
                None,
                action::REMOVED,
                error::INVENTORY,
                crate::auction::EQUIP_ERR_ITEM_NOT_FOUND,
            );
            return;
        };
        if auction.bidder > 0 {
            let cut = auction.auction_cut(self.config.rate_auction_cut);
            if self.player_money(actor_low).unwrap_or(0) < cut {
                return;
            }
            self.send_auction_cancelled_bidder_mail(&auction);
            self.modify_money(actor_low, -i64::from(cut));
        }
        self.send_auction_mail(
            actor_low,
            auction.house.house_id,
            crate::auction::mail_subject(
                auction.item_template,
                crate::auction::mail_answer::CANCELED,
            ),
            None,
            0,
            Some(item),
        );
        self.send_auction_result(id, Some(&auction), action::REMOVED, error::OK, 0);
        if let Err(e) = self.deps.auction.delete_auction(auction_id) {
            self.log
                .out(Level::Error, &format!("auction delete failed: {e}"));
        }
        self.auction_items.remove(&auction.item_guid_low);
        self.auction_houses[house_index].remove(&auction_id);
    }

    /// House entries in `OrderedAuctionMap` order
    /// (buyout, then insertion ≈ auction id).
    fn ordered_auctions(&self, house_index: usize) -> Vec<crate::auction::AuctionEntry> {
        let mut ordered: Vec<crate::auction::AuctionEntry> =
            self.auction_houses[house_index].values().cloned().collect();
        ordered.sort_by_key(|entry| (entry.buyout, entry.id));
        ordered
    }

    /// Sends one auction list result packet.
    fn send_auction_list(
        &mut self,
        id: SessionId,
        opcode: u16,
        infos: &[Vec<u8>],
        count: u32,
        total: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u32_le(count);
        for info in infos {
            writer.write_bytes(info);
        }
        writer.write_u32_le(total);
        let mut packet = WorldPacket::new(opcode);
        packet.take_writer(writer);
        self.reply_packet(id, packet);
    }

    /// One `BuildAuctionInfo` block, if the item row
    /// exists.
    fn auction_info_bytes(
        &self,
        entry: &crate::auction::AuctionEntry,
        now: i64,
    ) -> Option<Vec<u8>> {
        let item = self.auction_items.get(&entry.item_guid_low)?;
        let mut writer = PacketWriter::new();
        crate::auction::write_auction_info(&mut writer, entry, item, now);
        Some(writer.into_vec())
    }

    /// `HandleAuctionListItems` (synchronous; see the
    /// auction module docs).
    fn on_auction_list(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(auctioneer_raw) = reader.read_u64_le() else {
            return;
        };
        let Ok(list_from) = reader.read_u32_le() else {
            return;
        };
        let searched = reader.read_cstring();
        let (Ok(level_min), Ok(level_max)) = (reader.read_u8(), reader.read_u8()) else {
            return;
        };
        let (Ok(slot_id), Ok(main), Ok(sub), Ok(quality)) = (
            reader.read_u32_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
        ) else {
            return;
        };
        let Ok(usable) = reader.read_u8() else {
            return;
        };
        let Some((_house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            return;
        };
        let query = crate::auction::ListQuery {
            list_from,
            searched_name: searched,
            level_min,
            level_max,
            slot_id,
            main_category: main,
            sub_category: sub,
            quality,
            usable,
        };
        let remote_ip = self
            .sessions
            .get(&id)
            .map_or_else(String::new, |entry| entry.remote_ip.clone());
        let now = unix_now_i64();
        let ordered = self.ordered_auctions(house_index);
        let mut infos: Vec<Vec<u8>> = Vec::new();
        let (mut count, mut total) = (0u32, 0u32);
        if query.is_unfiltered() {
            total = u32::try_from(ordered.len()).unwrap_or(u32::MAX);
            for entry in ordered.iter().skip(list_from as usize) {
                if !entry.is_available_for(&remote_ip) {
                    continue;
                }
                if let Some(bytes) = self.auction_info_bytes(entry, now) {
                    infos.push(bytes);
                }
                count += 1;
                if count >= 50 {
                    break;
                }
            }
        } else {
            for entry in &ordered {
                let Some(item) = self.auction_items.get(&entry.item_guid_low) else {
                    continue;
                };
                let Some(proto) = self
                    .deps
                    .world_stores
                    .item_prototypes
                    .get(item.item_id)
                    .cloned()
                else {
                    continue;
                };
                if !query.matches(&proto) || !entry.is_available_for(&remote_ip) {
                    continue;
                }
                if count < 50 && total >= list_from {
                    count += 1;
                    let mut writer = PacketWriter::new();
                    crate::auction::write_auction_info(&mut writer, entry, item, now);
                    infos.push(writer.into_vec());
                }
                total += 1;
            }
        }
        self.send_auction_list(
            id,
            opcode_id::SMSG_AUCTION_LIST_RESULT,
            &infos,
            count,
            total,
        );
    }

    /// `HandleAuctionListOwnerItems`.
    fn on_auction_list_owner(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let (Ok(auctioneer_raw), Ok(list_from)) = (reader.read_u64_le(), reader.read_u32_le())
        else {
            return;
        };
        let Some((_house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            return;
        };
        let account_id = self.sessions.get(&id).map_or(0, |entry| entry.account_id);
        let now = unix_now_i64();
        let mut infos: Vec<Vec<u8>> = Vec::new();
        let (mut count, mut total) = (0u32, 0u32);
        let entries: Vec<crate::auction::AuctionEntry> = self.auction_houses[house_index]
            .values()
            .filter(|entry| entry.owner_account == account_id && entry.owner == actor_low)
            .cloned()
            .collect();
        for entry in &entries {
            total += 1;
            if count < 50 && total > list_from {
                if let Some(bytes) = self.auction_info_bytes(entry, now) {
                    infos.push(bytes);
                    count += 1;
                }
            }
        }
        self.send_auction_list(
            id,
            opcode_id::SMSG_AUCTION_OWNER_LIST_RESULT,
            &infos,
            count,
            total,
        );
    }

    /// `HandleAuctionListBidderItems`.
    fn on_auction_list_bidder(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let (Ok(auctioneer_raw), Ok(list_from), Ok(outbidded_count)) = (
            reader.read_u64_le(),
            reader.read_u32_le(),
            reader.read_u32_le(),
        ) else {
            return;
        };
        let mut outbidded_ids: Vec<u32> = Vec::new();
        let expected = 16usize.saturating_add((outbidded_count as usize).saturating_mul(4));
        if packet.body().len() == expected {
            for _ in 0..outbidded_count {
                match reader.read_u32_le() {
                    Ok(auction_id) => outbidded_ids.push(auction_id),
                    Err(_) => break,
                }
            }
        }
        let Some((_house, house_index)) =
            self.checked_auction_house(actor_low, ObjectGuid::from_raw(auctioneer_raw))
        else {
            return;
        };
        let now = unix_now_i64();
        let mut infos: Vec<Vec<u8>> = Vec::new();
        let (mut count, mut total) = (0u32, 0u32);
        for auction_id in &outbidded_ids {
            if let Some(entry) = self.auction_houses[house_index].get(auction_id).cloned() {
                total += 1;
                if count < 50 && total > list_from {
                    if let Some(bytes) = self.auction_info_bytes(&entry, now) {
                        infos.push(bytes);
                        count += 1;
                    }
                }
            }
        }
        let entries: Vec<crate::auction::AuctionEntry> = self.auction_houses[house_index]
            .values()
            .filter(|entry| entry.bidder == actor_low)
            .cloned()
            .collect();
        for entry in &entries {
            total += 1;
            if count < 50 && total > list_from {
                if let Some(bytes) = self.auction_info_bytes(entry, now) {
                    infos.push(bytes);
                    count += 1;
                }
            }
        }
        self.send_auction_list(
            id,
            opcode_id::SMSG_AUCTION_BIDDER_LIST_RESULT,
            &infos,
            count,
            total,
        );
    }

    /// `AuctionHouseMgr::Update`: IP-lock aging and
    /// expiry settlement, per house object.
    /// `AuctionHouseBotMgr::Update`.
    fn update_ah_bot(&mut self, force: bool) {
        use mangos_spells::SpellRng as _;
        if !self.ah_bot.loaded {
            return;
        }
        if !(self.ah_bot.config.enable || force) {
            return;
        }
        if !self.ah_bot.ready() {
            if force {
                self.log.out(
                    Level::Error,
                    "AHBot::Update() : No items or sellers configured.",
                );
            }
            return;
        }
        let Some((house, house_index)) = self.ah_bot_house else {
            self.log.out(
                Level::Error,
                &format!(
                    "AHBot::Update() : No auction house for faction {}.",
                    self.ah_bot.config.ah_fid
                ),
            );
            return;
        };
        let now = unix_now_i64();
        let mut rng = WorldRng::from_entropy();
        let mut pick = |lo: u32, hi: u32| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                rng.irand(lo as i32, hi as i32) as u32
            }
        };
        if self.ah_bot.needs_refresh(now, force) {
            self.log
                .out(Level::Minimal, "AHBot: Performing full AH refresh...");
            self.ah_bot_wipe(house_index);
            self.ah_bot_populate(house, house_index, &mut pick);
            self.ah_bot.mark_refreshed(now);
            if let Err(e) = self.deps.ah_bot.save_refresh_time(now) {
                self.log
                    .out(Level::Error, &format!("ahbot state save failed: {e}"));
            }
            self.log.out(
                Level::Minimal,
                &format!(
                    "AHBot: Refresh complete. AH now has {} listings.",
                    self.auction_houses[house_index].len()
                ),
            );
        } else {
            let current = u32::try_from(self.auction_houses[house_index].len()).unwrap_or(u32::MAX);
            if self.ah_bot.needs_restock(current) {
                self.log.out(
                    Level::Minimal,
                    &format!(
                        "AHBot: Restocking {} items (current: {current}, target: {}).",
                        self.ah_bot.config.item_count - current,
                        self.ah_bot.config.item_count
                    ),
                );
                self.ah_bot_populate(house, house_index, &mut pick);
            }
        }
        if self.ah_bot.buyer_due(now) {
            self.ah_bot_buyers(house_index, &mut pick);
            self.ah_bot.mark_buyer_run(now);
        }
    }

    /// `WipeAllBotAuctions`.
    fn ah_bot_wipe(&mut self, house_index: usize) {
        let account_id = self.ah_bot.config.seller_account_id;
        let doomed: Vec<crate::auction::AuctionEntry> = self.auction_houses[house_index]
            .values()
            .filter(|entry| entry.owner_account == account_id || entry.owner == 0)
            .cloned()
            .collect();
        let count = doomed.len();
        for entry in doomed {
            if self.auction_items.remove(&entry.item_guid_low).is_some() {
                if let Err(e) = self.deps.ah_bot.delete_item_instance(entry.item_guid_low) {
                    self.log
                        .out(Level::Error, &format!("ahbot wipe item delete failed: {e}"));
                }
            }
            if let Err(e) = self.deps.auction.delete_auction(entry.id) {
                self.log.out(
                    Level::Error,
                    &format!("ahbot wipe auction delete failed: {e}"),
                );
            }
            self.auction_houses[house_index].remove(&entry.id);
        }
        self.log.out(
            Level::Minimal,
            &format!("AHBot: Wiped {count} bot-owned auctions."),
        );
    }

    /// `PopulateAuctions` + `AddItem`.
    fn ah_bot_populate(
        &mut self,
        house: mangos_world_data::auction_house::AuctionHouseEntry,
        house_index: usize,
        pick: &mut (dyn FnMut(u32, u32) -> u32 + '_),
    ) {
        let current = u32::try_from(self.auction_houses[house_index].len()).unwrap_or(u32::MAX);
        let plan = self.ah_bot.plan_populate(current, pick);
        let now = unix_now_i64();
        for listing in plan {
            let Some(proto) = self.deps.world_stores.item_prototypes.get(listing.item_id) else {
                self.log.out(
                    Level::Minimal,
                    &format!(
                        "AHBot::AddItem() : Item {} does not exist.",
                        listing.item_id
                    ),
                );
                continue;
            };
            let item_low = match self
                .deps
                .ah_bot
                .create_ahbot_item(listing.item_id, listing.stack_size)
            {
                Ok(guid) => guid,
                Err(e) => {
                    self.log.out(
                        Level::Minimal,
                        &format!(
                            "AHBot::AddItem() : Cannot create item {}. ({e})",
                            listing.item_id
                        ),
                    );
                    continue;
                }
            };
            let deposit = crate::auction::auction_deposit(
                &house,
                172_800,
                proto.sell_price,
                listing.stack_size,
                self.config.auction_deposit_min,
                self.config.rate_auction_deposit,
            );
            let entry = crate::auction::AuctionEntry {
                id: self.next_auction_id,
                item_guid_low: item_low,
                item_template: listing.item_id,
                owner: listing.seller_guid,
                owner_account: self.ah_bot.config.seller_account_id,
                start_bid: listing.bid,
                bid: 0,
                buyout: listing.buyout,
                locked_ip: String::new(),
                deposit_time: now,
                expire_time: now + 172_800,
                bidder: 0,
                deposit,
                house,
            };
            self.next_auction_id = self.next_auction_id.saturating_add(1);
            if let Err(e) = self.deps.auction.insert_auction(entry.to_stored()) {
                self.log
                    .out(Level::Error, &format!("ahbot auction insert failed: {e}"));
                continue;
            }
            self.auction_items.insert(
                item_low,
                mangos_db::character_load::ItemInstanceRow {
                    creator_guid: 0,
                    gift_creator_guid: 0,
                    count: listing.stack_size,
                    duration: 0,
                    charges: "0 0 0 0 0 ".to_owned(),
                    flags: 0,
                    enchantments: "0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0".to_owned(),
                    random_property_id: 0,
                    durability: 0,
                    text: 0,
                    item_guid: item_low,
                    item_id: listing.item_id,
                    generated_loot: 0,
                },
            );
            self.auction_houses[house_index].insert(entry.id, entry);
        }
    }

    /// `UpdateBotBuyers`.
    fn ah_bot_buyers(&mut self, house_index: usize, pick: &mut (dyn FnMut(u32, u32) -> u32 + '_)) {
        let candidates: Vec<mangos_bots::ah_bot::BuyerCandidate> = self.auction_houses[house_index]
            .values()
            .filter(|entry| entry.bidder == 0)
            .map(|entry| mangos_bots::ah_bot::BuyerCandidate {
                auction_id: entry.id,
                item_id: entry.item_template,
                stack_size: self
                    .auction_items
                    .get(&entry.item_guid_low)
                    .map_or(1, |item| item.count),
                buyout: entry.buyout,
                owner_guid: entry.owner,
            })
            .collect();
        let (bought, spent) = self.ah_bot.plan_buyer_purchases(candidates, pick);
        let buyer = self.ah_bot.random_seller_guid(pick);
        for auction_id in &bought {
            let Some(mut entry) = self.auction_houses[house_index].get(auction_id).cloned() else {
                continue;
            };
            entry.bidder = buyer;
            entry.bid = entry.buyout;
            self.send_auction_successful_mail(&entry);
            if self.auction_items.remove(&entry.item_guid_low).is_some() {
                if let Err(e) = self.deps.ah_bot.delete_item_instance(entry.item_guid_low) {
                    self.log.out(
                        Level::Error,
                        &format!("ahbot buyer item delete failed: {e}"),
                    );
                }
            }
            if let Err(e) = self.deps.auction.delete_auction(entry.id) {
                self.log.out(
                    Level::Error,
                    &format!("ahbot buyer auction delete failed: {e}"),
                );
            }
            self.auction_houses[house_index].remove(&entry.id);
        }
        if !bought.is_empty() {
            self.log.out(
                Level::Minimal,
                &format!(
                    "AHBot Buyer: Purchased {} player auctions (budget used: {}g {}s {}c).",
                    bought.len(),
                    spent / 10_000,
                    (spent % 10_000) / 100,
                    spent % 100
                ),
            );
        }
    }

    /// `HandleAHBotUpdateCommand`.
    fn command_ahbot_update(&mut self, id: SessionId) -> bool {
        self.update_ah_bot(true);
        self.send_sys_text(id, "[AHBot] Update finished.");
        true
    }

    /// `HandleAHBotReloadCommand`.
    fn command_ahbot_reload(&mut self, id: SessionId) -> bool {
        match self.deps.ah_bot.load_ah_bot(
            self.ah_bot.config.seller_account_id,
            self.ah_bot.config.seller_count,
        ) {
            Ok(initial) => {
                let entries: Vec<mangos_bots::ah_bot::AhBotEntry> = initial
                    .items
                    .into_iter()
                    .map(|row| mangos_bots::ah_bot::AhBotEntry {
                        item: row.item,
                        min_stack: row.min_stack,
                        max_stack: row.max_stack,
                        bid: row.bid,
                        buyout: row.buyout,
                        weight: row.weight,
                        category: mangos_bots::ah_bot::parse_category(&row.category),
                        price_variance_pct: row.price_variance_pct,
                    })
                    .collect();
                let identities: Vec<mangos_bots::ah_bot::SellerIdentity> = initial
                    .sellers
                    .into_iter()
                    .map(|(guid, name)| mangos_bots::ah_bot::SellerIdentity {
                        character_guid: guid,
                        name,
                        seller_type: mangos_bots::ah_bot::SellerType::Casual,
                    })
                    .collect();
                self.ah_bot = mangos_bots::ah_bot::AhBotCore::load(
                    self.config.ah_bot.clone(),
                    entries,
                    identities,
                    initial.last_refresh_s,
                    self.ah_bot_house.is_some(),
                );
            }
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("ahbot reload load failed: {e}"));
            }
        }
        self.update_ah_bot(true);
        self.send_sys_text(id, "[AHBot] Reload finished.");
        true
    }

    fn update_auctions(&mut self) {
        if self
            .auction_houses
            .iter()
            .all(std::collections::BTreeMap::is_empty)
        {
            return;
        }
        let now = unix_now_i64();
        for house_index in 0..self.auction_houses.len() {
            let expired: Vec<crate::auction::AuctionEntry> = {
                let house = &mut self.auction_houses[house_index];
                for entry in house.values_mut() {
                    if entry.deposit_time + crate::auction::IP_LOCK_SECS < now {
                        entry.locked_ip.clear();
                    }
                }
                house
                    .values()
                    .filter(|entry| now > entry.expire_time)
                    .cloned()
                    .collect()
            };
            for entry in expired {
                if entry.bidder == 0 {
                    self.send_auction_expired_mail(&entry);
                } else {
                    self.send_auction_successful_mail(&entry);
                    self.send_auction_won_mail(&entry);
                }
                if let Err(e) = self.deps.auction.delete_auction(entry.id) {
                    self.log
                        .out(Level::Error, &format!("auction delete failed: {e}"));
                }
                self.auction_items.remove(&entry.item_guid_low);
                self.auction_houses[house_index].remove(&entry.id);
            }
        }
    }

    /// Parses the shared `(mailbox guid, mail id)`
    /// packet head.
    fn mail_packet_head(packet: &WorldPacket) -> Option<(ObjectGuid, u32)> {
        let mut reader = packet.reader();
        let mailbox = ObjectGuid::from_wire_raw(reader.read_u64_le().ok()?);
        Some((mailbox, reader.read_u32_le().ok()?))
    }

    /// `HandleQueryNextMailTime`.
    fn on_query_next_mail_time(&mut self, id: SessionId) {
        if self.actor_low(id).is_none() {
            return;
        }
        let has_unread = self
            .sessions
            .get(&id)
            .is_some_and(|entry| entry.mail_tracker.has_unread());
        self.reply_packet(id, next_mail_time_packet(has_unread));
    }

    /// `HandleItemTextQuery`.
    fn on_item_text_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(item_text_id) = reader.read_u32_le() else {
            return;
        };
        let text = match self.deps.mail.item_text(item_text_id) {
            Ok(text) => text,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("item text read failed: {e}"));
                String::new()
            }
        };
        let mut writer = PacketWriter::new();
        writer.write_u32_le(item_text_id);
        writer.write_cstring(&text);
        let mut reply = WorldPacket::new(opcode_id::SMSG_ITEM_TEXT_QUERY_RESPONSE);
        reply.take_writer(writer);
        self.reply_packet(id, reply);
    }

    /// `HandleGetMailList`.
    fn on_get_mail_list(&mut self, id: SessionId, packet: &WorldPacket) {
        use mangos_db::character_load::{MailRow, MailedItemRow};
        let Some(actor_low) = self.actor_low(id) else {
            return;
        };
        let Ok(mailbox) = packet.reader().read_u64_le().map(ObjectGuid::from_wire_raw) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let snapshot: Vec<(MailRow, Option<MailedItemRow>)> = self
            .with_mail_deferred(actor_low, |mails, items| {
                mails
                    .iter()
                    .filter(|mail| mail_is_visible(mail, now))
                    .map(|mail| {
                        let item = items.iter().find(|item| item.mail_id == mail.id).cloned();
                        (mail.clone(), item)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<(&MailRow, Option<MailListItem>)> = snapshot
            .iter()
            .map(|(mail, item)| {
                let block = item.as_ref().map(|mailed| {
                    let proto = self
                        .deps
                        .world_stores
                        .item_prototypes
                        .get(mailed.item.item_id);
                    MailListItem {
                        entry: mailed.item.item_id,
                        perm_enchant: perm_enchant_id(&mailed.item.enchantments),
                        random_property_id: i32::from(mailed.item.random_property_id),
                        count: mailed.item.count as u8,
                        charges: spell_charges(&mailed.item.charges),
                        max_durability: proto.map_or(0, |proto| proto.max_durability),
                        durability: u32::from(mailed.item.durability),
                    }
                });
                (mail, block)
            })
            .collect();
        self.reply_packet(id, mail_list_packet(&rows, now));
        // Recompute pending delivery + unreads.
        let tracker = self
            .with_mail_deferred(actor_low, |mails, _| MailTracker::recompute(mails, now))
            .unwrap_or_default();
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.mail_tracker = tracker;
        }
    }

    /// Receiver resolution for send-mail (online name
    /// cache, then the DB); sends
    /// `RECIPIENT_NOT_FOUND` itself on a miss.
    fn resolve_mail_receiver(
        &mut self,
        id: SessionId,
        receiver_name: &str,
    ) -> Option<MailReceiver> {
        use response_result as err;
        let normalized = crate::social::normalize_player_name(receiver_name);
        if let Some((low, session, race)) = normalized
            .as_deref()
            .and_then(|name| self.find_online_player_by_name(name))
        {
            let account = self
                .sessions
                .get(&session)
                .map_or(0, |entry| entry.auth.account_id);
            return Some(MailReceiver {
                low,
                session: Some(session),
                race,
                account,
            });
        }
        if let Some(name) = normalized.as_deref() {
            match self.deps.mail.receiver_by_name(name) {
                Ok(Some(info)) => {
                    return Some(MailReceiver {
                        low: info.guid_low,
                        session: None,
                        race: info.race,
                        account: info.account_id,
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    self.log
                        .out(Level::Error, &format!("mail receiver lookup failed: {e}"));
                }
            }
        }
        self.send_mail_result(id, 0, response_type::SEND, err::RECIPIENT_NOT_FOUND);
        None
    }

    /// Attachment validation for send-mail (the
    /// row-level `CanBeTraded` subset; `None` = the
    /// internal-error reply already applies).
    fn check_mail_attachment(
        &mut self,
        actor_low: u32,
        item_low: u32,
    ) -> Option<mangos_db::mail::StoredInventoryItem> {
        let stored = match self.deps.mail.inventory_item(actor_low, item_low) {
            Ok(stored) => stored?,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("mail item read failed: {e}"));
                return None;
            }
        };
        let proto = self
            .deps
            .world_stores
            .item_prototypes
            .get(stored.item_id)
            .cloned()?;
        let is_bag_pos = stored.bag == 0 && (19..23).contains(&stored.slot);
        let is_bank_pos = stored.bag == 0 && stored.slot >= 39;
        let untradeable = stored.flags & 0x1 != 0 // soulbound
            || stored.generated_loot != 0
            || (proto.class == 1 && (is_bag_pos || stored.items_in_bag > 0))
            || proto.flags & 0x2 != 0 // ITEM_FLAG_CONJURED
            || stored.duration != 0;
        if is_bank_pos || untradeable {
            return None;
        }
        Some(stored)
    }

    /// Spam gates, body text, and delivery for a
    /// validated send (the tail of the fork's mail
    /// callback).
    fn deliver_send_mail(
        &mut self,
        id: SessionId,
        actor_low: u32,
        receiver: &MailReceiver,
        req: SendMailRequest,
        required: u32,
        attached: Option<&mangos_db::mail::StoredInventoryItem>,
    ) {
        use response_result as err;
        let SendMailRequest {
            subject,
            body: mail_body,
            money,
            mut cod,
            ..
        } = req;
        let (receiver_low, receiver_account) = (receiver.low, receiver.account);
        // Mail-spam heuristics.
        let level = self
            .player_map
            .get(&actor_low)
            .and_then(|map_id| self.maps.get(map_id))
            .and_then(|map| map.player(player_guid(actor_low)))
            .map_or(0, |player| player.unit.level());
        if level < self.config.mailspam_level
            && money < self.config.mailspam_money
            && self.config.mailspam_item
            && attached.is_none()
        {
            self.send_mail_result(id, 0, response_type::SEND, err::INTERNAL_ERROR);
            return;
        }
        let sender_account = self
            .sessions
            .get(&id)
            .map_or(0, |entry| entry.auth.account_id);
        let now = unix_now_i64();
        if !self.mail_spam.can_mail(
            sender_account,
            receiver_account,
            now,
            self.config.mailspam_expire_secs,
            self.config.mailspam_max_mails,
        ) {
            // Silent drop behind an OK (fork behavior).
            self.send_mail_result(id, 0, response_type::SEND, err::OK);
            return;
        }
        self.mail_spam
            .just_mailed(sender_account, receiver_account, now);
        let item_text_id = if mail_body.is_empty() {
            0
        } else {
            match self.deps.mail.create_item_text(&mail_body) {
                Ok(text_id) => text_id,
                Err(e) => {
                    self.log
                        .out(Level::Error, &format!("item text create failed: {e}"));
                    self.send_mail_result(id, 0, response_type::SEND, err::INTERNAL_ERROR);
                    return;
                }
            }
        };
        let deliver_delay = if attached.is_none() && money == 0 {
            0
        } else {
            self.config.mail_delivery_delay_secs
        };
        if attached.is_none() {
            cod = 0;
        }
        let checked = if mail_body.is_empty() {
            check_mask::COPIED
        } else {
            check_mask::HAS_BODY
        };
        let sent = self.send_mail_to(
            actor_low,
            receiver_low,
            OutgoingMail {
                subject,
                item_text_id,
                mail_template_id: 0,
                stationery: crate::mail::stationery::DEFAULT,
                message_type: message_type::NORMAL,
                money,
                cod,
                checked,
                deliver_delay_secs: deliver_delay,
            },
            attached,
        );
        if sent.is_none() {
            self.send_mail_result(id, 0, response_type::SEND, err::INTERNAL_ERROR);
            return;
        }
        if let Some(item) = &attached {
            self.apply_mail_write(MailWrite::MoveItemToMail {
                owner_low: actor_low,
                item_low: item.item_guid,
                new_owner_low: receiver_low,
            });
        }
        self.modify_money(actor_low, -i64::from(required));
        self.send_mail_result(id, 0, response_type::SEND, err::OK);
    }

    /// The gate + delivery half of `HandleSendMail`
    /// (the fork's async callback body).
    fn finish_send_mail(
        &mut self,
        id: SessionId,
        actor_low: u32,
        security: u8,
        receiver: &MailReceiver,
        req: SendMailRequest,
    ) {
        use response_result as err;
        let SendMailRequest {
            subject,
            body: mail_body,
            money,
            cod,
            item_raw,
        } = req;
        let (receiver_low, receiver_session, receiver_race, receiver_account) = (
            receiver.low,
            receiver.session,
            receiver.race,
            receiver.account,
        );
        if receiver_low == actor_low {
            self.send_mail_result(id, 0, response_type::SEND, err::CANNOT_SEND_TO_SELF);
            return;
        }
        if security == crate::security::PLAYER {
            let receiver_trial = if let Some(session) = receiver_session {
                self.has_trial_restrictions(session)
            } else {
                self.config.restrict_unverified_accounts
                    && matches!(
                        self.deps.mail.account_email_verified(receiver_account),
                        Ok(Some(false))
                    )
            };
            if receiver_trial {
                self.send_mail_result(id, 0, response_type::SEND, err::DISABLED_FOR_TRIAL_ACC);
                return;
            }
        }
        let Some(required) = money.checked_add(POSTAGE) else {
            self.send_mail_result(id, 0, response_type::SEND, err::NOT_ENOUGH_MONEY);
            return;
        };
        if self.player_money(actor_low).unwrap_or(0) < required {
            self.send_mail_result(id, 0, response_type::SEND, err::NOT_ENOUGH_MONEY);
            return;
        }
        // Mailbox count as the fork's uint8 (wraps).
        let mails_count = if receiver_session.is_some() {
            self.live_mail_count(receiver_low) as u8
        } else {
            match self.deps.mail.mail_count(receiver_low) {
                Ok(count) => count as u8,
                Err(e) => {
                    self.log
                        .out(Level::Error, &format!("mail count failed: {e}"));
                    0xFF
                }
            }
        };
        if mails_count > MAILBOX_CAP {
            self.send_mail_result(id, 0, response_type::SEND, err::RECIPIENT_CAP_REACHED);
            return;
        }
        if !self.config.allow_two_side_interaction_mail && security == crate::security::PLAYER {
            let actor_team = self
                .player_name_race_of(actor_low)
                .map(|(_, race)| self.team_id_for_race(race));
            if actor_team.is_some() && actor_team != Some(self.team_id_for_race(receiver_race)) {
                self.send_mail_result(id, 0, response_type::SEND, err::NOT_YOUR_TEAM);
                return;
            }
        }
        // Attachment checks (see `mail` module note
        // for the row-level CanBeTraded subset).
        let mut attached = None;
        if item_raw != 0 {
            let Some(stored) = self.check_mail_attachment(actor_low, item_raw as u32) else {
                self.send_mail_result(id, 0, response_type::SEND, err::INTERNAL_ERROR);
                return;
            };
            attached = Some(stored);
        }
        self.deliver_send_mail(
            id,
            actor_low,
            receiver,
            SendMailRequest {
                subject,
                body: mail_body,
                money,
                cod,
                item_raw,
            },
            required,
            attached.as_ref(),
        );
    }

    /// `HandleSendMail` + request/callback flattened
    /// onto the actor (the async mailbox-count query
    /// resolves synchronously here).
    fn on_send_mail(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use response_result as err;
        let body = packet.body();
        let Ok(mailbox_raw) = mangos_protocol::buffer::PacketReader::new(body).read_u64_le() else {
            return;
        };
        let mailbox = ObjectGuid::from_wire_raw(mailbox_raw);
        if !self.mailbox_ok(actor_low, mailbox) {
            self.send_mail_result(id, 0, response_type::SEND, err::INTERNAL_ERROR);
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_mail_result(id, 0, response_type::SEND, err::DISABLED_FOR_TRIAL_ACC);
            return;
        }
        let mut reader = mangos_protocol::buffer::PacketReader::new(body.get(8..).unwrap_or(&[]));
        let receiver_name = reader.read_cstring();
        let mut subject = reader.read_cstring();
        let mail_body = reader.read_cstring();
        let _ = reader.read_u32_le();
        let _ = reader.read_u32_le();
        // Item guid is a full (unpacked) guid on the wire.
        let item_raw = reader.read_u64_le().unwrap_or(0);
        let money = reader.read_u32_le().unwrap_or(0);
        let cod = reader.read_u32_le().unwrap_or(0);
        if subject.len() > SUBJECT_MAX_BYTES || mail_body.len() > BODY_MAX_BYTES {
            return;
        }
        if cod > COD_MAX {
            return;
        }
        let security = self
            .sessions
            .get(&id)
            .map_or(0, |entry| entry.auth.security);
        if cod != 0
            && self.config.cod_force_tag_max_level > 0
            && self.config.cod_force_tag_max_level
                > self
                    .sessions
                    .get(&id)
                    .map_or(0, |entry| entry.character_max_level)
        {
            subject = format!("(COD) {subject}");
        }
        if !self.config.gm_allow_trades
            && security > crate::security::PLAYER
            && (item_raw != 0 || money != 0)
        {
            return;
        }
        if receiver_name.is_empty() {
            return;
        }
        let Some(receiver) = self.resolve_mail_receiver(id, &receiver_name) else {
            return;
        };
        self.finish_send_mail(
            id,
            actor_low,
            security,
            &receiver,
            SendMailRequest {
                subject,
                body: mail_body,
                money,
                cod,
                item_raw,
            },
        );
    }

    /// Finds a mail's index in a live player's list
    /// (`MasterPlayer::GetMail` position), requiring
    /// it to be delivered by `now`.
    fn find_delivered_mail(
        mails: &[mangos_db::character_load::MailRow],
        mail_id: u32,
        now: i64,
    ) -> Option<usize> {
        mails
            .iter()
            .position(|mail| mail.id == mail_id && mail.deliver_time <= now)
    }

    /// `HandleMailTakeMoney`.
    fn on_mail_take_money(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use response_result as err;
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let taken = self.with_mail_deferred(actor_low, |mails, _| {
            let index = Self::find_delivered_mail(mails, mail_id, now)?;
            let mail = &mut mails[index];
            let amount = mail.money.max(0) as u32;
            mail.money = 0;
            Some((amount, mail_update(mail)))
        });
        let Some(Some((amount, update))) = taken else {
            self.send_mail_result(id, mail_id, response_type::MONEY_TAKEN, err::INTERNAL_ERROR);
            return;
        };
        // Gold-cap guard (checked against the live
        // coinage + the entity max).
        let current = self.player_money(actor_low).unwrap_or(0);
        if u64::from(current) + u64::from(amount)
            > u64::from(mangos_entities::player::MAX_MONEY_AMOUNT)
        {
            // Restore the amount on the mail.
            if let Some(restore) = self.with_mail_deferred(actor_low, |mails, _| {
                let index = Self::find_delivered_mail(mails, mail_id, now)?;
                mails[index].money = money_as_i32(amount);
                Some(())
            }) {
                let _ = restore;
            }
            self.send_mail_result(id, mail_id, response_type::MONEY_TAKEN, err::INTERNAL_ERROR);
            return;
        }
        self.send_mail_result(id, mail_id, response_type::MONEY_TAKEN, err::OK);
        self.modify_money(actor_low, i64::from(amount));
        self.apply_mail_write(MailWrite::Update(update));
    }

    /// Charge the COD sum and mail it to the sender
    /// (`HandleMailTakeItem` COD draft).
    fn send_cod_payment(
        &mut self,
        actor_low: u32,
        mail: &mangos_db::character_load::MailRow,
        cod: u32,
    ) {
        let sender_low = mail.sender_guid;
        let sender_exists = self.player_session.contains_key(&sender_low)
            || matches!(self.deps.mail.account_id_by_guid(sender_low), Ok(Some(_)));
        if sender_exists {
            let _ = self.send_mail_to(
                actor_low,
                sender_low,
                OutgoingMail {
                    subject: mail.subject.clone(),
                    item_text_id: 0,
                    mail_template_id: 0,
                    stationery: crate::mail::stationery::DEFAULT,
                    message_type: message_type::NORMAL,
                    money: cod,
                    cod: 0,
                    checked: check_mask::COD_PAYMENT,
                    deliver_delay_secs: 0,
                },
                None,
            );
        }
        self.modify_money(actor_low, -i64::from(cod));
    }

    /// The store + COD + completion half of
    /// `HandleMailTakeItem` (snapshot resolved).
    fn complete_mail_take_item(
        &mut self,
        id: SessionId,
        actor_low: u32,
        mail_id: u32,
        mail_snapshot: &mangos_db::character_load::MailRow,
        cod: u32,
        mailed: &mangos_db::character_load::MailedItemRow,
    ) {
        use response_result as err;
        let now = unix_now_i64();
        let item_low = mailed.item.item_guid;
        let item_id = mailed.item.item_id;
        let item_count = mailed.item.count;
        match self
            .deps
            .mail
            .store_mailed_item(actor_low, item_low, item_id)
        {
            Ok(true) => {}
            Ok(false) => {
                self.reply_packet(
                    id,
                    mail_result_packet(
                        mail_id,
                        response_type::ITEM_TAKEN,
                        err::EQUIP_ERROR,
                        50, // EQUIP_ERR_INVENTORY_FULL
                        0,
                        0,
                    ),
                );
                return;
            }
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("mail item store failed: {e}"));
                self.reply_packet(
                    id,
                    mail_result_packet(
                        mail_id,
                        response_type::ITEM_TAKEN,
                        err::EQUIP_ERROR,
                        50,
                        0,
                        0,
                    ),
                );
                return;
            }
        }
        if cod > 0 {
            self.send_cod_payment(actor_low, mail_snapshot, cod);
        }
        self.apply_mail_write(MailWrite::DeleteMailItem {
            item_guid: item_low,
        });
        let update = self.with_mail_deferred(actor_low, |mails, items| {
            items.retain(|item| item.item.item_guid != item_low);
            let index = Self::find_delivered_mail(mails, mail_id, now)?;
            mails[index].cod = 0;
            Some(mail_update(&mails[index]))
        });
        if let Some(Some(update)) = update {
            self.apply_mail_write(MailWrite::Update(update));
        }
        self.reply_packet(
            id,
            mail_result_packet(
                mail_id,
                response_type::ITEM_TAKEN,
                err::OK,
                0,
                item_id,
                item_count,
            ),
        );
    }

    /// `HandleMailTakeItem`.
    fn on_mail_take_item(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use mangos_db::character_load::MailedItemRow;
        use response_result as err;
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let found: Option<Option<(mangos_db::character_load::MailRow, Option<MailedItemRow>)>> =
            self.with_mail_deferred(actor_low, |mails, items| {
                let mail = mails
                    .iter()
                    .find(|mail| mail.id == mail_id && mail.deliver_time <= now)?
                    .clone();
                let item = items.iter().find(|item| item.mail_id == mail_id).cloned();
                Some((mail, item))
            });
        let Some(Some((mail_snapshot, mailed))) = found else {
            self.send_mail_result(id, mail_id, response_type::ITEM_TAKEN, err::INTERNAL_ERROR);
            return;
        };
        let Some(mailed) = mailed else {
            self.send_mail_result(id, mail_id, response_type::ITEM_TAKEN, err::INTERNAL_ERROR);
            return;
        };
        let cod = mail_snapshot.cod.max(0) as u32;
        if self.player_money(actor_low).unwrap_or(0) < cod {
            self.send_mail_result(
                id,
                mail_id,
                response_type::ITEM_TAKEN,
                err::NOT_ENOUGH_MONEY,
            );
            return;
        }
        let security = self
            .sessions
            .get(&id)
            .map_or(0, |entry| entry.auth.security);
        if !self.config.gm_allow_trades && security > crate::security::PLAYER && cod > 0 {
            self.send_mail_result(
                id,
                mail_id,
                response_type::ITEM_TAKEN,
                err::NOT_ENOUGH_MONEY,
            );
            return;
        }
        self.complete_mail_take_item(id, actor_low, mail_id, &mail_snapshot, cod, &mailed);
    }

    /// `HandleMailMarkAsRead`.
    fn on_mail_mark_as_read(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let update = self.with_mail_deferred(actor_low, |mails, _| {
            let mail = mails.iter_mut().find(|mail| mail.id == mail_id)?;
            mail.checked |= check_mask::READ as u8;
            if mail.expire_time - now > 3 * 86_400 {
                mail.expire_time = now + 3 * 86_400;
            }
            Some(mail_update(mail))
        });
        if let Some(Some(update)) = update {
            if let Some(entry) = self.sessions.get_mut(&id) {
                entry.mail_tracker.decrease_unread();
            }
            self.apply_mail_write(MailWrite::Update(update));
        }
    }

    /// `HandleMailDelete`.
    fn on_mail_delete(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use response_result as err;
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let removed = self.with_mail_deferred(actor_low, |mails, items| {
            let index = mails.iter().position(|mail| mail.id == mail_id)?;
            if mails[index].cod != 0 {
                return None;
            }
            let mail = mails.remove(index);
            let item_lows: Vec<u32> = items
                .iter()
                .filter(|item| item.mail_id == mail_id)
                .map(|item| item.item.item_guid)
                .collect();
            items.retain(|item| item.mail_id != mail_id);
            Some((mail, item_lows))
        });
        match removed {
            Some(Some((mail, item_lows))) => {
                self.apply_mail_write(MailWrite::DeleteFull {
                    mail_id,
                    item_lows,
                    item_text_id: mail.item_text_id,
                    stationery: mail.stationery,
                });
                self.send_mail_result(id, mail_id, response_type::DELETED, err::OK);
            }
            _ => {
                self.send_mail_result(id, mail_id, response_type::DELETED, err::INTERNAL_ERROR);
            }
        }
    }

    /// The return draft half of
    /// `HandleMailReturnToSender` (rows already
    /// deleted by the caller).
    fn forward_returned_mail(
        &mut self,
        id: SessionId,
        actor_low: u32,
        mail: &mangos_db::character_load::MailRow,
        mailed: &[mangos_db::character_load::MailedItemRow],
    ) {
        if mail.message_type == message_type::NORMAL && mail.sender_guid != 0 {
            let return_to = mail.sender_guid;
            let sender_account = if let Some(session) = self.player_session.get(&return_to).copied()
            {
                self.sessions
                    .get(&session)
                    .map(|entry| entry.auth.account_id)
            } else {
                match self.deps.mail.account_id_by_guid(return_to) {
                    Ok(account) => account,
                    Err(e) => {
                        self.log.out(
                            Level::Error,
                            &format!("mail return account lookup failed: {e}"),
                        );
                        None
                    }
                }
            };
            if let Some(return_account) = sender_account {
                for item in mailed {
                    self.apply_mail_write(MailWrite::SetItemOwner {
                        item_low: item.item.item_guid,
                        new_owner_low: return_to,
                    });
                }
                let actor_account = self
                    .sessions
                    .get(&id)
                    .map_or(0, |entry| entry.auth.account_id);
                let deliver_delay = if !mailed.is_empty() && actor_account != return_account {
                    self.config.mail_delivery_delay_secs
                } else {
                    0
                };
                // A normal mail carries at most one
                // item at 5875; forward the first.
                let first = mailed
                    .first()
                    .map(|item| mangos_db::mail::StoredInventoryItem {
                        bag: 0,
                        slot: 0,
                        item_guid: item.item.item_guid,
                        item_id: item.item.item_id,
                        count: item.item.count,
                        duration: item.item.duration,
                        charges: item.item.charges.clone(),
                        flags: item.item.flags,
                        enchantments: item.item.enchantments.clone(),
                        random_property_id: item.item.random_property_id,
                        durability: item.item.durability,
                        text: item.item.text,
                        creator_guid: item.item.creator_guid,
                        gift_creator_guid: item.item.gift_creator_guid,
                        generated_loot: item.item.generated_loot,
                        items_in_bag: 0,
                    });
                let _ = self.send_mail_to(
                    actor_low,
                    return_to,
                    OutgoingMail {
                        subject: if mail.mail_template_id != 0 {
                            String::new()
                        } else {
                            mail.subject.clone()
                        },
                        item_text_id: mail.item_text_id,
                        mail_template_id: mail.mail_template_id as u16,
                        stationery: crate::mail::stationery::DEFAULT,
                        message_type: message_type::NORMAL,
                        money: mail.money.max(0) as u32,
                        cod: 0,
                        checked: check_mask::RETURNED,
                        deliver_delay_secs: deliver_delay,
                    },
                    first.as_ref(),
                );
            } else {
                for item in mailed {
                    self.apply_mail_write(MailWrite::DeleteItemInstance {
                        item_low: item.item.item_guid,
                    });
                }
            }
        }
    }

    /// `HandleMailReturnToSender`.
    fn on_mail_return(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use response_result as err;
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let removed = self.with_mail_deferred(actor_low, |mails, items| {
            let index = Self::find_delivered_mail(mails, mail_id, now)?;
            let mail = mails.remove(index);
            let mailed: Vec<_> = items
                .iter()
                .filter(|item| item.mail_id == mail_id)
                .cloned()
                .collect();
            items.retain(|item| item.mail_id != mail_id);
            Some((mail, mailed))
        });
        let Some(Some((mail, mailed))) = removed else {
            self.send_mail_result(
                id,
                mail_id,
                response_type::RETURNED_TO_SENDER,
                err::INTERNAL_ERROR,
            );
            return;
        };
        self.apply_mail_write(MailWrite::DeleteRows { mail_id });
        self.forward_returned_mail(id, actor_low, &mail, &mailed);
        self.send_mail_result(id, mail_id, response_type::RETURNED_TO_SENDER, err::OK);
    }

    /// `HandleMailCreateTextItem`.
    fn on_mail_create_text_item(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        use response_result as err;
        let Some((mailbox, mail_id)) = Self::mail_packet_head(packet) else {
            return;
        };
        if !self.mailbox_ok(actor_low, mailbox) {
            return;
        }
        let now = unix_now_i64();
        let snapshot = self.with_mail_deferred(actor_low, |mails, _| {
            mails
                .iter()
                .find(|mail| {
                    mail.id == mail_id
                        && mail.deliver_time <= now
                        && (mail.item_text_id != 0 || mail.mail_template_id != 0)
                        && mail.checked & (check_mask::COPIED as u8) == 0
                })
                .cloned()
        });
        let Some(Some(mail)) = snapshot else {
            self.send_mail_result(
                id,
                mail_id,
                response_type::MADE_PERMANENT,
                err::INTERNAL_ERROR,
            );
            return;
        };
        match self
            .deps
            .mail
            .create_text_item(actor_low, mail.sender_guid, mail.item_text_id)
        {
            Ok(true) => {
                let update = self.with_mail_deferred(actor_low, |mails, _| {
                    let mail = mails.iter_mut().find(|mail| mail.id == mail_id)?;
                    mail.checked |= check_mask::COPIED as u8;
                    Some(mail_update(mail))
                });
                if let Some(Some(update)) = update {
                    self.apply_mail_write(MailWrite::Update(update));
                }
                self.send_mail_result(id, mail_id, response_type::MADE_PERMANENT, err::OK);
            }
            Ok(false) => {
                self.reply_packet(
                    id,
                    mail_result_packet(
                        mail_id,
                        response_type::MADE_PERMANENT,
                        err::EQUIP_ERROR,
                        50,
                        0,
                        0,
                    ),
                );
            }
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("mail text item failed: {e}"));
                self.reply_packet(
                    id,
                    mail_result_packet(
                        mail_id,
                        response_type::MADE_PERMANENT,
                        err::EQUIP_ERROR,
                        50,
                        0,
                        0,
                    ),
                );
            }
        }
    }

    /// Applies one petition persistence intent.
    fn apply_petition_write(&mut self, write: PetitionWrite) {
        if let Err(e) = self.deps.petition.apply_petition_write(write) {
            self.log
                .out(Level::Error, &format!("petition write failed: {e}"));
        }
    }

    /// `Player::RemovePetitionsAndSigns`: DB purge plus
    /// the in-memory signature sweep (the owned
    /// petition object itself stays registered, as in
    /// the fork).
    fn remove_petitions_and_signs(&mut self, guid_low: u32, except_petition_id: u32) {
        self.petitions
            .delete_signatures_by_player(guid_low, except_petition_id);
        self.apply_petition_write(PetitionWrite::PurgePlayer {
            player_low: guid_low,
        });
    }

    /// The shared `IsReservedName ||
    /// !IsValidCharterName` verdict for buy/rename.
    fn charter_name_invalid(&self, name: &str) -> bool {
        if !is_valid_charter_name(
            name,
            self.config.min_charter_name,
            self.config.strict_charter_names,
        ) {
            return true;
        }
        match self.deps.petition.is_reserved_name(name) {
            Ok(reserved) => reserved,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("reserved name check failed: {e}"));
                false
            }
        }
    }

    /// Whether any live guild already uses the name
    /// (`GuildMgr::GetGuildByName`).
    fn guild_name_taken(&self, name: &str) -> bool {
        self.guilds.values().any(|guild| guild.name() == name)
    }

    /// Member stats for a guild join, DB candidate
    /// first then live stats (`AddMember` data fill).
    fn member_stats_for(&self, guid_low: u32) -> Option<NewMemberStats> {
        match self.deps.guild.guild_candidate(guid_low) {
            Ok(Some(candidate)) => Some(NewMemberStats {
                guid_low,
                account_id: candidate.account_id,
                name: candidate.name,
                level: candidate.level,
                class: candidate.class,
                zone_id: candidate.zone_id,
            }),
            Ok(None) => self.guild_stats_of(guid_low),
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("guild candidate read failed: {e}"));
                self.guild_stats_of(guid_low)
            }
        }
    }

    /// `GetNPCIfCanInteractWith` reduced to the checks
    /// the petition handlers depend on: creature
    /// exists in the actor's map, carries the
    /// petitioner flag (plus tabard designer when
    /// buying), is alive, and is inside interaction
    /// distance. The reaction/reputation/combat gates
    /// join when general NPC interaction is ported.
    fn petition_npc_ok(&self, actor_low: u32, npc_guid: ObjectGuid, need_tabard: bool) -> bool {
        use mangos_protocol::update_field_table::offsets;
        let Some(map) = self
            .player_map
            .get(&actor_low)
            .and_then(|map_id| self.maps.get(map_id))
        else {
            return false;
        };
        let Some(creature) = map.creature(npc_guid) else {
            return false;
        };
        // UNIT_NPC_FLAG_PETITIONER / TABARDDESIGNER.
        let flags = creature
            .unit
            .world
            .object
            .values()
            .get_u32(offsets::UNIT_NPC_FLAGS);
        if flags & 0x0200 == 0 || (need_tabard && flags & 0x0400 == 0) {
            return false;
        }
        if !creature.unit.is_alive() {
            return false;
        }
        let Some(player) = map.player(player_guid(actor_low)) else {
            return false;
        };
        let actor_pos = player.unit.world.position();
        let npc_pos = creature.unit.world.position();
        let dx = actor_pos.x - npc_pos.x;
        let dy = actor_pos.y - npc_pos.y;
        let dz = actor_pos.z - npc_pos.z;
        // INTERACTION_DISTANCE (5.0) plus object sizes.
        dx * dx + dy * dy + dz * dz <= 6.0 * 6.0
    }

    /// `Player::GetMoney` off the live entity.
    fn player_money(&self, actor_low: u32) -> Option<u32> {
        use mangos_protocol::update_field_table::offsets;
        let map = self.maps.get(self.player_map.get(&actor_low)?)?;
        let player = map.player(player_guid(actor_low))?;
        Some(
            player
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_FIELD_COINAGE),
        )
    }

    /// `Player::ModifyMoney` (clamped to
    /// `[0, MAX_MONEY_AMOUNT]`).
    fn modify_money(&mut self, actor_low: u32, delta: i64) {
        use mangos_protocol::update_field_table::offsets;
        let Some(player) = self
            .player_map
            .get(&actor_low)
            .copied()
            .and_then(|map_id| self.maps.get_mut(&map_id))
            .and_then(|map| map.player_mut(player_guid(actor_low)))
        else {
            return;
        };
        let money = i64::from(
            player
                .unit
                .world
                .object
                .values()
                .get_u32(offsets::PLAYER_FIELD_COINAGE),
        )
        .saturating_add(delta)
        .clamp(0, i64::from(mangos_entities::player::MAX_MONEY_AMOUNT));
        player
            .unit
            .values()
            .set_u32(offsets::PLAYER_FIELD_COINAGE, money as u32);
    }

    /// Whether the actor carries the charter item
    /// (`Player::GetItemByGuid` over inventory rows;
    /// error reads as "no item").
    fn actor_has_item(&self, actor_low: u32, item_low: u32) -> bool {
        match self.deps.petition.player_has_item(actor_low, item_low) {
            Ok(has) => has,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("charter item check failed: {e}"));
                false
            }
        }
    }

    /// `SendPetitionShowList`.
    fn on_petition_showlist(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(npc_guid) = packet.reader().read_u64_le().map(ObjectGuid::from_wire_raw) else {
            return;
        };
        if !self.petition_npc_ok(actor_low, npc_guid, false) {
            return;
        }
        let mut writer = PacketWriter::new();
        write_guid(&mut writer, npc_guid);
        writer.write_u8(1);
        writer.write_u32_le(1);
        writer.write_u32_le(mangos_db::petition::GUILD_CHARTER_ENTRY);
        writer.write_u32_le(CHARTER_DISPLAY_ID);
        writer.write_u32_le(CHARTER_COST);
        writer.write_u32_le(1);
        let mut reply = WorldPacket::new(opcode_id::SMSG_PETITION_SHOWLIST);
        reply.take_writer(writer);
        self.reply_packet(id, reply);
    }

    /// `HandlePetitionBuyOpcode`. The charter is
    /// persisted as item/inventory rows (see
    /// `mangos_db::petition`); the client-side item
    /// push (`SendNewItem`) joins with the runtime
    /// item system.
    fn on_petition_buy(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let body = packet.body();
        let Ok(npc_raw) = mangos_protocol::buffer::PacketReader::new(body).read_u64_le() else {
            return;
        };
        let npc_guid = ObjectGuid::from_wire_raw(npc_raw);
        let mut reader = mangos_protocol::buffer::PacketReader::new(body.get(8..).unwrap_or(&[]));
        // Skipped in C++: uint32 + uint64 before the name.
        if reader.skip(12).is_err() {
            return;
        }
        let name = reader.read_cstring();
        if !self.petition_npc_ok(actor_low, npc_guid, true) {
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_trial_notification(id);
            return;
        }
        if self.player_guild.contains_key(&actor_low)
            || self.petitions.by_owner(actor_low).is_some()
        {
            return;
        }
        if self.guild_name_taken(&name) {
            self.send_guild_result(
                id,
                type_command::CREATE,
                &name,
                command_error::NAME_EXISTS_S,
            );
            return;
        }
        if self.charter_name_invalid(&name) {
            self.send_guild_result(id, type_command::CREATE, &name, command_error::NAME_INVALID);
            return;
        }
        if self
            .deps
            .world_stores
            .items
            .get(mangos_db::petition::GUILD_CHARTER_ENTRY)
            .is_none()
        {
            // BUY_ERR_CANT_FIND_ITEM with an empty guid.
            self.reply_packet(id, buy_failed_packet(ObjectGuid::EMPTY, 0));
            return;
        }
        if self.player_money(actor_low).unwrap_or(0) < CHARTER_COST {
            // BUY_ERR_NOT_ENOUGHT_MONEY.
            self.reply_packet(id, buy_failed_packet(npc_guid, 2));
            return;
        }
        let petition_id = self.petitions.peek_next_id();
        let charter_low = match self
            .deps
            .petition
            .create_charter_item(actor_low, petition_id)
        {
            Ok(Some(low)) => low,
            Ok(None) => {
                // CanStoreNewItem: EQUIP_ERR_INVENTORY_FULL.
                self.reply_packet(id, inventory_full_packet());
                return;
            }
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("charter item create failed: {e}"));
                return;
            }
        };
        self.modify_money(actor_low, -i64::from(CHARTER_COST));
        let owner_horde = self
            .player_name_race_of(actor_low)
            .is_some_and(|(_, race)| self.team_id_for_race(race) == 1);
        let write = self
            .petitions
            .create(petition_id, actor_low, charter_low, name, owner_horde);
        self.apply_petition_write(write);
    }

    /// `HandlePetitionShowSignOpcode` (owner side).
    fn on_petition_show_signatures(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(item_raw) = packet.reader().read_u64_le() else {
            return;
        };
        if self.player_guild.contains_key(&actor_low) {
            return;
        }
        let item_low = item_raw as u32;
        if !self.actor_has_item(actor_low, item_low) {
            return;
        }
        let Some(petition) = self.petitions.by_charter(item_low) else {
            return;
        };
        let packet =
            show_signatures_packet(petition.charter_guid(), player_guid(actor_low), petition);
        self.reply_packet(id, packet);
    }

    /// `HandlePetitionQueryOpcode`.
    fn on_petition_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(petition_id) = reader.read_u32_le() else {
            return;
        };
        let Some(petition) = self.petitions.by_id(petition_id) else {
            return;
        };
        let mut writer = PacketWriter::new();
        writer.write_u32_le(petition.id);
        write_guid(&mut writer, petition.owner_guid());
        writer.write_cstring(&petition.name);
        writer.write_u8(0);
        writer.write_u32_le(1); // flags
        writer.write_u32_le(9); // min signatures (hardcoded in C++)
        writer.write_u32_le(9); // max signatures
        for _ in 0..5 {
            writer.write_u32_le(0);
        }
        writer.write_u16_le(0);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let mut reply = WorldPacket::new(opcode_id::SMSG_PETITION_QUERY_RESPONSE);
        reply.take_writer(writer);
        self.reply_packet(id, reply);
    }

    /// `HandlePetitionRenameOpcode`.
    fn on_petition_rename(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let body = packet.body();
        let Ok(item_raw) = mangos_protocol::buffer::PacketReader::new(body).read_u64_le() else {
            return;
        };
        let mut reader = mangos_protocol::buffer::PacketReader::new(body.get(8..).unwrap_or(&[]));
        let new_name = reader.read_cstring();
        let item_low = item_raw as u32;
        if !self.actor_has_item(actor_low, item_low) {
            return;
        }
        if self.guild_name_taken(&new_name) {
            self.send_guild_result(
                id,
                type_command::CREATE,
                &new_name,
                command_error::NAME_EXISTS_S,
            );
            return;
        }
        if self.charter_name_invalid(&new_name) {
            self.send_guild_result(
                id,
                type_command::CREATE,
                &new_name,
                command_error::NAME_INVALID,
            );
            return;
        }
        let Some(petition_id) = self.petitions.by_charter(item_low).map(|p| p.id) else {
            return;
        };
        let Some(write) = self.petitions.rename(petition_id, new_name.clone()) else {
            return;
        };
        self.apply_petition_write(write);
        let mut writer = PacketWriter::new();
        write_guid(&mut writer, ObjectGuid::from_raw(item_raw));
        writer.write_cstring(&new_name);
        let mut reply = WorldPacket::new(opcode_id::MSG_PETITION_RENAME);
        reply.take_writer(writer);
        self.reply_packet(id, reply);
    }

    /// `HandlePetitionSignOpcode`.
    fn on_petition_sign(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(item_raw) = packet.reader().read_u64_le() else {
            return;
        };
        let min_signatures = self.config.min_petition_signs;
        let Some(snapshot) = self.petitions.by_charter(item_raw as u32).cloned() else {
            return;
        };
        if snapshot.is_complete(min_signatures) {
            return;
        }
        let actor_guid = player_guid(actor_low);
        if snapshot.owner_low == actor_low {
            self.reply_packet(
                id,
                sign_results_packet(
                    snapshot.charter_guid(),
                    actor_guid,
                    sign_result::CANT_SIGN_OWN,
                ),
            );
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_trial_notification(id);
            return;
        }
        if !self.config.allow_two_side_interaction_guild {
            let actor_horde = self
                .player_name_race_of(actor_low)
                .is_some_and(|(_, race)| self.team_id_for_race(race) == 1);
            if actor_horde != snapshot.horde {
                self.send_guild_result(id, type_command::CREATE, "", command_error::NOT_ALLIED);
                return;
            }
        }
        let actor_name = self
            .player_name_race_of(actor_low)
            .map_or_else(String::new, |(name, _)| name);
        if self.player_guild.contains_key(&actor_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &actor_name,
                command_error::ALREADY_IN_GUILD_S,
            );
            return;
        }
        if self.player_guild_invite.contains_key(&actor_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &actor_name,
                command_error::ALREADY_INVITED_TO_GUILD_S,
            );
            return;
        }
        if snapshot.signature_count() >= SIGNATURES_HARD_LIMIT {
            return;
        }
        let account_id = self
            .sessions
            .get(&id)
            .map_or(0, |entry| entry.auth.account_id);
        if snapshot.signature_for_player(actor_low, account_id) {
            self.reply_packet(
                id,
                sign_results_packet(
                    snapshot.charter_guid(),
                    actor_guid,
                    sign_result::ALREADY_SIGNED,
                ),
            );
            if let Some(owner_session) = self.player_session.get(&snapshot.owner_low).copied() {
                self.send_guild_result(
                    owner_session,
                    type_command::INVITE,
                    &actor_name,
                    command_error::ALREADY_INVITED_TO_GUILD_S,
                );
            }
            return;
        }
        let Some(write) =
            self.petitions
                .add_signature(snapshot.id, actor_low, account_id, min_signatures)
        else {
            return;
        };
        self.apply_petition_write(write);
        let results = sign_results_packet(snapshot.charter_guid(), actor_guid, sign_result::OK);
        self.reply_packet(id, results.clone());
        if let Some(owner_session) = self.player_session.get(&snapshot.owner_low).copied() {
            self.reply_packet(owner_session, results);
        }
    }

    /// `HandlePetitionDeclineOpcode`.
    fn on_petition_decline(&mut self, actor_low: u32, packet: &WorldPacket) {
        let Ok(item_raw) = packet.reader().read_u64_le() else {
            return;
        };
        let Some(owner_low) = self
            .petitions
            .by_charter(item_raw as u32)
            .map(|petition| petition.owner_low)
        else {
            return;
        };
        let Some(owner_session) = self.player_session.get(&owner_low).copied() else {
            return;
        };
        let mut writer = PacketWriter::new();
        write_guid(&mut writer, player_guid(actor_low));
        let mut reply = WorldPacket::new(opcode_id::MSG_PETITION_DECLINE);
        reply.take_writer(writer);
        self.reply_packet(owner_session, reply);
    }

    /// `HandleOfferPetitionOpcode`.
    fn on_offer_petition(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let body = packet.body();
        let Ok(item_raw) = mangos_protocol::buffer::PacketReader::new(body).read_u64_le() else {
            return;
        };
        let Ok(target_guid) =
            mangos_protocol::buffer::PacketReader::new(body.get(8..).unwrap_or(&[]))
                .read_u64_le()
                .map(ObjectGuid::from_wire_raw)
        else {
            return;
        };
        let target_low = target_guid.counter();
        let Some(target_session) = self.player_session.get(&target_low).copied() else {
            return;
        };
        if !self.config.allow_two_side_interaction_guild {
            let actor_team = self
                .player_name_race_of(actor_low)
                .map(|(_, race)| self.team_id_for_race(race));
            let target_team = self
                .player_name_race_of(target_low)
                .map(|(_, race)| self.team_id_for_race(race));
            if actor_team.is_some() && actor_team != target_team {
                self.send_guild_result(id, type_command::CREATE, "", command_error::NOT_ALLIED);
                return;
            }
        }
        // C++ reports the offerer's own name in these
        // two results (faithful quirk).
        let actor_name = self
            .player_name_race_of(actor_low)
            .map_or_else(String::new, |(name, _)| name);
        if self.player_guild.contains_key(&target_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &actor_name,
                command_error::ALREADY_IN_GUILD_S,
            );
            return;
        }
        if self.player_guild_invite.contains_key(&target_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &actor_name,
                command_error::ALREADY_INVITED_TO_GUILD_S,
            );
            return;
        }
        if self.has_trial_restrictions(target_session) {
            self.send_trial_notification(id);
            return;
        }
        let item_low = item_raw as u32;
        if !self.actor_has_item(actor_low, item_low) {
            return;
        }
        let Some(petition) = self.petitions.by_charter(item_low) else {
            return;
        };
        let packet =
            show_signatures_packet(petition.charter_guid(), player_guid(actor_low), petition);
        self.reply_packet(target_session, packet);
    }

    /// `HandleTurnInPetitionOpcode`: create the guild
    /// from the completed charter (`Guild::Create`
    /// petition overload), delete the petition and
    /// the charter item.
    fn on_turn_in_petition(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Ok(item_raw) = packet.reader().read_u64_le() else {
            return;
        };
        let item_low = item_raw as u32;
        if !self.actor_has_item(actor_low, item_low) {
            return;
        }
        let Some(snapshot) = self.petitions.by_charter(item_low).cloned() else {
            return;
        };
        if self.has_trial_restrictions(id) {
            self.send_trial_notification(id);
            return;
        }
        if self.player_guild.contains_key(&actor_low) {
            self.reply_packet(id, turn_in_results_packet(sign_result::ALREADY_IN_GUILD));
            return;
        }
        if snapshot.owner_low != actor_low {
            return;
        }
        if !snapshot.is_complete(self.config.min_petition_signs) {
            self.reply_packet(id, turn_in_results_packet(sign_result::NEED_MORE));
            return;
        }
        if self.guild_name_taken(&snapshot.name) {
            self.send_guild_result(
                id,
                type_command::CREATE,
                &snapshot.name,
                command_error::NAME_EXISTS_S,
            );
            return;
        }
        let Some(leader) = self.guild_stats_of(actor_low) else {
            return;
        };
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let rank_names = default_guild_rank_names(&self.strings, locale);
        let guild_id = self.next_guild_id;
        self.next_guild_id = self.next_guild_id.saturating_add(1);
        let now = unix_now();
        let (guild, fx) =
            Guild::create_fresh(guild_id, snapshot.name.clone(), leader, now, &rank_names);
        self.guilds.insert(guild_id, guild);
        self.player_guild.insert(actor_low, guild_id);
        self.player_guild_invite.remove(&actor_low);
        self.apply_guild_fx(fx);
        // AddMember(leader) inside Guild::Create purges
        // with no exception.
        self.remove_petitions_and_signs(actor_low, 0);
        for signature in &snapshot.signatures {
            let signer_low = signature.player_low;
            if signer_low == 0 || self.player_guild.contains_key(&signer_low) {
                continue;
            }
            let Some(stats) = self.member_stats_for(signer_low) else {
                continue;
            };
            let added = self.guilds.get_mut(&guild_id).and_then(|guild| {
                let rank = guild.lowest_rank();
                guild.add_member(stats, rank, now)
            });
            let Some(fx) = added else {
                continue;
            };
            self.player_guild.insert(signer_low, guild_id);
            self.player_guild_invite.remove(&signer_low);
            self.apply_guild_fx(fx);
            self.remove_petitions_and_signs(signer_low, snapshot.id);
        }
        if let Some(write) = self.petitions.delete(snapshot.id) {
            self.apply_petition_write(write);
        }
        if let Err(e) = self
            .deps
            .petition
            .delete_charter_item(actor_low, snapshot.charter_low)
        {
            self.log
                .out(Level::Error, &format!("charter item delete failed: {e}"));
        }
        self.reply_packet(id, turn_in_results_packet(sign_result::OK));
    }

    /// `HandleGuildQueryOpcode`.
    fn on_guild_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(guild_id) = reader.read_u32_le() else {
            self.bad_packet(id, opcode_id::CMSG_GUILD_QUERY);
            return;
        };
        if self.guilds.contains_key(&guild_id) {
            self.guild_query_to(guild_id, id);
        } else {
            self.send_guild_result(
                id,
                type_command::CREATE,
                "",
                command_error::PLAYER_NOT_IN_GUILD,
            );
        }
    }

    /// `HandleGuildCreateOpcode`.
    fn on_guild_create(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let name = reader.read_cstring();
        if self.player_guild.contains_key(&actor_low) {
            return;
        }
        if self.has_trial_restrictions(id) {
            self.send_trial_notification(id);
            return;
        }
        if name.chars().count() > crate::guild::NAME_MAX_LENGTH {
            return;
        }
        if self.guilds.values().any(|guild| guild.name() == name) {
            return; // Guild::Create fails silently on duplicates
        }
        let Some(leader) = self.guild_stats_of(actor_low) else {
            return;
        };
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let rank_names = default_guild_rank_names(&self.strings, locale);
        let guild_id = self.next_guild_id;
        self.next_guild_id = self.next_guild_id.saturating_add(1);
        let (guild, fx) = Guild::create_fresh(guild_id, name, leader, unix_now(), &rank_names);
        self.guilds.insert(guild_id, guild);
        self.player_guild.insert(actor_low, guild_id);
        self.apply_guild_fx(fx);
        // `AddMember`(leader) inside `Guild::Create`
        // purges with no exception.
        self.remove_petitions_and_signs(actor_low, 0);
    }

    /// `HandleGuildInviteOpcode`.
    fn on_guild_invite(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let invited_name = reader.read_cstring();
        let normalized = normalize_player_name(&invited_name);
        let target = normalized
            .as_deref()
            .and_then(|name| self.find_online_player_by_name(name));
        let Some((target_low, target_session, target_race)) = target else {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &invited_name,
                command_error::PLAYER_NOT_FOUND_S,
            );
            return;
        };
        let name = normalized.unwrap_or(invited_name);
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        if self.has_trial_restrictions(target_session) {
            self.send_trial_notification(id);
            return;
        }
        if self.player_ignores(target_low, actor_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::IGNORING_YOU_S,
            );
            return;
        }
        let Some((actor_name, actor_race)) = self.player_name_race_of(actor_low) else {
            return;
        };
        if !self.config.allow_two_side_interaction_guild
            && self.team_id_for_race(actor_race) != self.team_id_for_race(target_race)
        {
            self.send_guild_result(id, type_command::INVITE, &name, command_error::NOT_ALLIED);
            return;
        }
        if self.player_guild.contains_key(&target_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::ALREADY_IN_GUILD_S,
            );
            return;
        }
        if self.player_guild_invite.contains_key(&target_low) {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::ALREADY_INVITED_TO_GUILD_S,
            );
            return;
        }
        let (guild_name, allowed) = {
            let Some(guild) = self.guilds.get(&guild_id) else {
                return;
            };
            (
                guild.name().to_owned(),
                guild.has_rank_right(actor_rank, rights::INVITE),
            )
        };
        if !allowed {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        self.player_guild_invite.insert(target_low, guild_id);
        self.log_guild_event(guild_id, event_log::INVITE_PLAYER, actor_low, target_low, 0);
        self.reply_packet(
            target_session,
            guild_invite_notice(&actor_name, &guild_name),
        );
    }

    /// `HandleGuildAcceptOpcode`.
    fn on_guild_accept(&mut self, actor_low: u32) {
        let Some(guild_id) = self.player_guild_invite.get(&actor_low).copied() else {
            return;
        };
        if self.player_guild.contains_key(&actor_low) || !self.guilds.contains_key(&guild_id) {
            return;
        }
        if !self.config.allow_two_side_interaction_guild {
            let leader_race = self.guilds.get(&guild_id).and_then(|guild| {
                let leader = guild.leader_low();
                guild
                    .members()
                    .get(&leader)
                    .map(|member| member.name.clone())
            });
            let leader_team = leader_race.and_then(|leader_name| {
                match self.deps.social.player_data_by_name(&leader_name) {
                    Ok(row) => row.map(|row| self.team_id_for_race(row.race)),
                    Err(e) => {
                        self.log.out(
                            Level::Error,
                            &format!("guild accept leader lookup failed: {e}"),
                        );
                        None
                    }
                }
            });
            let actor_team = self
                .player_name_race_of(actor_low)
                .map(|(_, race)| self.team_id_for_race(race));
            if let (Some(leader_team), Some(actor_team)) = (leader_team, actor_team)
                && leader_team != actor_team
            {
                return;
            }
        }
        let stats = match self.deps.guild.guild_candidate(actor_low) {
            Ok(Some(candidate)) => Some(NewMemberStats {
                guid_low: actor_low,
                account_id: candidate.account_id,
                name: candidate.name,
                level: candidate.level,
                class: candidate.class,
                zone_id: candidate.zone_id,
            }),
            Ok(None) => self.guild_stats_of(actor_low),
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("guild candidate read failed: {e}"));
                self.guild_stats_of(actor_low)
            }
        };
        let Some(stats) = stats else {
            return;
        };
        let actor_name = stats.name.clone();
        let added = self.guilds.get_mut(&guild_id).and_then(|guild| {
            let rank = guild.lowest_rank();
            guild.add_member(stats, rank, unix_now())
        });
        let Some(fx) = added else {
            return;
        };
        self.player_guild_invite.remove(&actor_low);
        self.player_guild.insert(actor_low, guild_id);
        self.apply_guild_fx(fx);
        // `AddMember` purges the joiner's petitions/signs.
        self.remove_petitions_and_signs(actor_low, 0);
        self.log_guild_event(guild_id, event_log::JOIN_GUILD, actor_low, 0, 0);
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx = guild.broadcast_event(guild_event::JOINED, Some(actor_low), &[&actor_name]);
            self.apply_guild_fx(fx);
        }
    }

    /// `HandleGuildDeclineOpcode`.
    fn on_guild_decline(&mut self, actor_low: u32) {
        if self.player_guild.contains_key(&actor_low) {
            return;
        }
        let Some(guild_id) = self.player_guild_invite.get(&actor_low).copied() else {
            return;
        };
        let inviter = self
            .guilds
            .get(&guild_id)
            .and_then(|guild| guild.inviter_of(actor_low));
        self.player_guild_invite.remove(&actor_low);
        if let Some(inviter_session) =
            inviter.and_then(|low| self.player_session.get(&low).copied())
            && let Some((actor_name, _)) = self.player_name_race_of(actor_low)
        {
            self.reply_packet(inviter_session, guild_decline_notice(&actor_name));
        }
    }

    /// `HandleGuildInfoOpcode`.
    fn on_guild_info(&mut self, id: SessionId, actor_low: u32) {
        let Some((guild_id, _rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        if let Some(guild) = self.guilds.get(&guild_id) {
            let packet = guild_info_packet(guild);
            self.reply_packet(id, packet);
        }
    }

    /// `HandleGuildRosterOpcode`.
    fn on_guild_roster(&mut self, id: SessionId, actor_low: u32) {
        if let Some(guild_id) = self.player_guild.get(&actor_low).copied()
            && let Some(rank) = self
                .guilds
                .get(&guild_id)
                .and_then(|guild| guild.rank_of(actor_low))
        {
            self.guild_roster_to(guild_id, id, rank);
        }
    }

    /// `HandleGuildPromoteOpcode` / `HandleGuildDemoteOpcode`.
    fn on_guild_promote(
        &mut self,
        id: SessionId,
        actor_low: u32,
        packet: &WorldPacket,
        promote: bool,
    ) {
        let mut reader = packet.reader();
        let raw_name = reader.read_cstring();
        let Some(name) = normalize_player_name(&raw_name) else {
            return;
        };
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let right = if promote {
            rights::PROMOTE
        } else {
            rights::DEMOTE
        };
        // Permission check happens before the slot lookup
        // in C++; one snapshot keeps that result order.
        let snapshot = self.guilds.get(&guild_id).map(|guild| {
            (
                guild.has_rank_right(actor_rank, right),
                guild
                    .member_low_by_name(&name)
                    .map(|low| (low, guild.rank_of(low).unwrap_or(0))),
                guild.lowest_rank(),
            )
        });
        let Some((allowed, slot, lowest)) = snapshot else {
            return;
        };
        if !allowed {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        let Some((target_low, target_rank)) = slot else {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::PLAYER_NOT_IN_GUILD_S,
            );
            return;
        };
        if target_low == actor_low {
            self.send_guild_result(id, type_command::INVITE, "", command_error::NAME_INVALID);
            return;
        }
        let new_rank = if promote {
            if actor_rank + 1 >= target_rank {
                self.send_guild_result(
                    id,
                    type_command::INVITE,
                    &name,
                    command_error::RANK_TOO_HIGH_S,
                );
                return;
            }
            target_rank - 1
        } else {
            if actor_rank >= target_rank {
                self.send_guild_result(
                    id,
                    type_command::INVITE,
                    &name,
                    command_error::RANK_TOO_HIGH_S,
                );
                return;
            }
            if target_rank >= lowest {
                self.send_guild_result(
                    id,
                    type_command::INVITE,
                    &name,
                    command_error::RANK_TOO_LOW_S,
                );
                return;
            }
            target_rank + 1
        };
        self.guild_apply_rank_change(guild_id, actor_low, target_low, &name, new_rank, promote);
    }

    /// Promote/demote tail: rank write, event-log row,
    /// and the promotion/demotion broadcast.
    #[allow(clippy::too_many_arguments)]
    fn guild_apply_rank_change(
        &mut self,
        guild_id: u32,
        actor_low: u32,
        target_low: u32,
        target_name: &str,
        new_rank: u32,
        promote: bool,
    ) {
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        let (fx, new_rank_name) = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            let fx = guild.change_rank(target_low, new_rank);
            (fx, guild.rank_name(new_rank))
        };
        self.apply_guild_fx(fx);
        #[allow(clippy::cast_possible_truncation)]
        self.log_guild_event(
            guild_id,
            if promote {
                event_log::PROMOTE_PLAYER
            } else {
                event_log::DEMOTE_PLAYER
            },
            actor_low,
            target_low,
            new_rank as u8,
        );
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx = guild.broadcast_event(
                if promote {
                    guild_event::PROMOTION
                } else {
                    guild_event::DEMOTION
                },
                None,
                &[&actor_name, target_name, &new_rank_name],
            );
            self.apply_guild_fx(fx);
        }
    }

    /// `HandleGuildLeaveOpcode`.
    fn on_guild_leave(&mut self, id: SessionId, actor_low: u32) {
        let Some((guild_id, _rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let (is_leader, member_count, guild_name) = {
            let Some(guild) = self.guilds.get(&guild_id) else {
                return;
            };
            (
                guild.leader_low() == actor_low,
                guild.member_count(),
                guild.name().to_owned(),
            )
        };
        if is_leader && member_count > 1 {
            self.send_guild_result(id, type_command::QUIT, "", command_error::LEADER_LEAVE);
            return;
        }
        if is_leader {
            self.guild_disband_now(guild_id);
            return;
        }
        self.send_guild_result(
            id,
            type_command::QUIT,
            &guild_name,
            command_error::PLAYER_NO_MORE_IN_GUILD,
        );
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        let (fx, need_disband) = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.del_member(actor_low, false)
        };
        self.player_guild.remove(&actor_low);
        self.apply_guild_fx(fx);
        if need_disband {
            self.guild_disband_now(guild_id);
            return;
        }
        self.log_guild_event(guild_id, event_log::LEAVE_GUILD, actor_low, 0, 0);
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx = guild.broadcast_event(guild_event::LEFT, Some(actor_low), &[&actor_name]);
            self.apply_guild_fx(fx);
        }
    }

    /// `Guild::Disband` + `GuildMgr` removal.
    fn guild_disband_now(&mut self, guild_id: u32) {
        let fx = self
            .guilds
            .get_mut(&guild_id)
            .map(Guild::disband)
            .unwrap_or_default();
        self.destroy_guild_entry(guild_id);
        self.apply_guild_fx(fx);
    }

    /// `HandleGuildRemoveOpcode`.
    fn on_guild_remove(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let raw_name = reader.read_cstring();
        let Some(name) = normalize_player_name(&raw_name) else {
            return;
        };
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let snapshot = self.guilds.get(&guild_id).map(|guild| {
            (
                guild.has_rank_right(actor_rank, rights::REMOVE),
                guild
                    .member_low_by_name(&name)
                    .map(|low| (low, guild.rank_of(low).unwrap_or(0))),
            )
        });
        let Some((allowed, slot)) = snapshot else {
            return;
        };
        if !allowed {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        let Some((target_low, target_rank)) = slot else {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::PLAYER_NOT_IN_GUILD_S,
            );
            return;
        };
        if target_rank == crate::guild::RANK_GUILDMASTER {
            self.send_guild_result(id, type_command::QUIT, "", command_error::LEADER_LEAVE);
            return;
        }
        if actor_rank >= target_rank {
            self.send_guild_result(
                id,
                type_command::QUIT,
                &name,
                command_error::RANK_TOO_HIGH_S,
            );
            return;
        }
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        let (fx, need_disband) = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.del_member(target_low, false)
        };
        self.player_guild.remove(&target_low);
        self.apply_guild_fx(fx);
        if need_disband {
            self.guild_disband_now(guild_id);
            return;
        }
        self.log_guild_event(
            guild_id,
            event_log::UNINVITE_PLAYER,
            actor_low,
            target_low,
            0,
        );
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx = guild.broadcast_event(guild_event::REMOVED, None, &[&name, &actor_name]);
            self.apply_guild_fx(fx);
        }
    }

    /// `HandleGuildDisbandOpcode`.
    fn on_guild_disband(&mut self, id: SessionId, actor_low: u32) {
        let Some((guild_id, _rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let is_leader = self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.leader_low() == actor_low);
        if !is_leader {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        self.guild_disband_now(guild_id);
    }

    /// `HandleGuildLeaderOpcode`.
    fn on_guild_leader(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let raw_name = reader.read_cstring();
        let Some(name) = normalize_player_name(&raw_name) else {
            return;
        };
        let Some((guild_id, _rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let snapshot = self.guilds.get(&guild_id).map(|guild| {
            (
                guild.leader_low() == actor_low,
                guild.member_low_by_name(&name),
            )
        });
        let Some((is_leader, target_low)) = snapshot else {
            return;
        };
        if !is_leader {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        let Some(target_low) = target_low else {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::PLAYER_NOT_IN_GUILD_S,
            );
            return;
        };
        let Some((actor_name, _)) = self.player_name_race_of(actor_low) else {
            return;
        };
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            let mut fx = guild.set_leader(target_low);
            fx.sends.clear();
            let demote = guild.change_rank(actor_low, crate::guild::RANK_OFFICER);
            fx.writes.extend(demote.writes);
            fx.fields.extend(demote.fields);
            fx
        };
        self.apply_guild_fx(fx);
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx =
                guild.broadcast_event(guild_event::LEADER_CHANGED, None, &[&actor_name, &name]);
            self.apply_guild_fx(fx);
        }
    }

    /// `HandleGuildMOTDOpcode`.
    fn on_guild_motd(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let motd = if packet.body().is_empty() {
            String::new()
        } else {
            reader.read_cstring()
        };
        if motd.chars().count() > crate::guild::MOTD_MAX_LENGTH {
            return;
        }
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let allowed = self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.has_rank_right(actor_rank, rights::SETMOTD));
        if !allowed {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.set_motd(motd.clone())
        };
        self.apply_guild_fx(fx);
        if let Some(guild) = self.guilds.get(&guild_id) {
            let fx = guild.broadcast_event(guild_event::MOTD, None, &[&motd]);
            self.apply_guild_fx(fx);
        }
    }

    /// `HandleGuildSetPublicNoteOpcode` /
    /// `HandleGuildSetOfficerNoteOpcode`.
    fn on_guild_note(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket, public: bool) {
        let mut reader = packet.reader();
        let raw_name = reader.read_cstring();
        let Some(name) = normalize_player_name(&raw_name) else {
            return;
        };
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let right = if public {
            rights::EPNOTE
        } else {
            rights::EOFFNOTE
        };
        let snapshot = self.guilds.get(&guild_id).map(|guild| {
            (
                guild.has_rank_right(actor_rank, right),
                guild.member_low_by_name(&name),
            )
        });
        let Some((allowed, target_low)) = snapshot else {
            return;
        };
        if !allowed {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return;
        }
        let Some(target_low) = target_low else {
            self.send_guild_result(
                id,
                type_command::INVITE,
                &name,
                command_error::PLAYER_NOT_IN_GUILD_S,
            );
            return;
        };
        let note = reader.read_cstring();
        if note.chars().count() > crate::guild::NOTE_MAX_LENGTH {
            return;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            if public {
                guild.set_pnote(target_low, note)
            } else {
                guild.set_offnote(target_low, note)
            }
        };
        self.apply_guild_fx(fx);
        self.guild_roster_to(guild_id, id, actor_rank);
    }

    /// The leader gate shared by the rank handlers
    /// (`GUILD_CREATE_S` then `GUILD_INVITE_S` results).
    fn guild_leader_or_result(&mut self, id: SessionId, actor_low: u32) -> Option<u32> {
        let (guild_id, _rank) = self.actor_guild_or_result(id, actor_low)?;
        let is_leader = self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.leader_low() == actor_low);
        if !is_leader {
            self.send_guild_result(id, type_command::INVITE, "", command_error::PERMISSIONS);
            return None;
        }
        Some(guild_id)
    }

    /// `HandleGuildRankOpcode` (set rank name + rights).
    fn on_guild_rank(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let Some(guild_id) = self.guild_leader_or_result(id, actor_low) else {
            return;
        };
        let mut reader = packet.reader();
        let parsed = (|| -> Result<(u32, u32), mangos_protocol::buffer::ReadError> {
            Ok((reader.read_u32_le()?, reader.read_u32_le()?))
        })();
        let Ok((rank_id, mut row_rights)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_GUILD_RANK);
            return;
        };
        let rank_name = reader.read_cstring();
        if rank_name.chars().count() > crate::guild::RANK_NAME_MAX_LENGTH {
            return;
        }
        if rank_id == crate::guild::RANK_GUILDMASTER {
            row_rights = rights::ALL;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            let mut fx = guild.set_rank_name(rank_id, rank_name);
            let rights_fx = guild.set_rank_rights(rank_id, row_rights);
            fx.writes.extend(rights_fx.writes);
            fx
        };
        self.apply_guild_fx(fx);
        self.guild_query_to(guild_id, id);
        self.guild_roster_broadcast(guild_id);
    }

    /// `HandleGuildAddRankOpcode`.
    fn on_guild_add_rank(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let rank_name = reader.read_cstring();
        if rank_name.chars().count() > crate::guild::RANK_NAME_MAX_LENGTH {
            return;
        }
        let Some(guild_id) = self.guild_leader_or_result(id, actor_low) else {
            return;
        };
        if self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.ranks_len() >= crate::guild::RANKS_MAX_COUNT)
        {
            return;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.create_rank(rank_name, rights::GCHATLISTEN | rights::GCHATSPEAK)
        };
        self.apply_guild_fx(fx);
        self.guild_query_to(guild_id, id);
        self.guild_roster_broadcast(guild_id);
    }

    /// `HandleGuildDelRankOpcode`.
    fn on_guild_del_rank(&mut self, id: SessionId, actor_low: u32) {
        let Some(guild_id) = self.guild_leader_or_result(id, actor_low) else {
            return;
        };
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.del_rank()
        };
        self.apply_guild_fx(fx);
        self.guild_query_to(guild_id, id);
        self.guild_roster_broadcast(guild_id);
    }

    /// `HandleGuildChangeInfoTextOpcode` (note the
    /// `GUILD_CREATE_S` permissions quirk).
    fn on_guild_info_text(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let info = reader.read_cstring();
        if info.chars().count() > crate::guild::INFO_MAX_LENGTH {
            return;
        }
        let Some((guild_id, actor_rank)) = self.actor_guild_or_result(id, actor_low) else {
            return;
        };
        let allowed = self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.has_rank_right(actor_rank, rights::MODIFY_GUILD_INFO));
        if !allowed {
            self.send_guild_result(id, type_command::CREATE, "", command_error::PERMISSIONS);
            return;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.set_ginfo(info)
        };
        self.apply_guild_fx(fx);
    }

    /// `HandleSaveGuildEmblemOpcode`. The tabard-designer
    /// NPC proximity check is skipped (see the guild
    /// module deviations); guild/leader/money gates and
    /// the 10-gold charge (`PLAYER_FIELD_COINAGE`) are
    /// kept.
    fn on_guild_emblem(&mut self, id: SessionId, actor_low: u32, packet: &WorldPacket) {
        const EMBLEM_COST: u32 = 10 * 10_000; // 10 * GOLD
        use mangos_protocol::update_field_table::offsets;
        let mut reader = packet.reader();
        let parsed = (|| -> Result<[i32; 5], mangos_protocol::buffer::ReadError> {
            let _vendor = reader.read_u64_le()?;
            let mut emblem = [0i32; 5];
            for slot in &mut emblem {
                #[allow(clippy::cast_possible_wrap)]
                {
                    *slot = reader.read_u32_le()? as i32;
                }
            }
            Ok(emblem)
        })();
        let Ok(emblem) = parsed else {
            self.bad_packet(id, opcode_id::MSG_SAVE_GUILD_EMBLEM);
            return;
        };
        let reply = |world: &mut Self, result: u32| {
            let mut writer = PacketWriter::new();
            writer.write_u32_le(result);
            let mut packet = WorldPacket::new(opcode_id::MSG_SAVE_GUILD_EMBLEM);
            packet.take_writer(writer);
            world.reply_packet(id, packet);
        };
        let Some(guild_id) = self.player_guild.get(&actor_low).copied() else {
            reply(self, emblem_result::NOGUILD);
            return;
        };
        let is_leader = self
            .guilds
            .get(&guild_id)
            .is_some_and(|guild| guild.leader_low() == actor_low);
        if !is_leader {
            reply(self, emblem_result::NOTGUILDMASTER);
            return;
        }
        let Some(map_id) = self.player_map.get(&actor_low).copied() else {
            reply(self, emblem_result::FAIL_NO_MESSAGE);
            return;
        };
        let paid = self
            .maps
            .get_mut(&map_id)
            .and_then(|map| map.player_mut(player_guid(actor_low)))
            .is_some_and(|player| {
                let values = player.unit.values();
                let money = values.get_u32(offsets::PLAYER_FIELD_COINAGE);
                if money < EMBLEM_COST {
                    false
                } else {
                    values.set_u32(offsets::PLAYER_FIELD_COINAGE, money - EMBLEM_COST);
                    true
                }
            });
        if !paid {
            reply(self, emblem_result::NOTENOUGHMONEY);
            return;
        }
        let fx = {
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            guild.set_emblem(emblem)
        };
        self.apply_guild_fx(fx);
        reply(self, emblem_result::SUCCESS);
        self.guild_query_to(guild_id, id);
    }

    /// Guild/officer chat delivery (`BroadcastToGuild` /
    /// `BroadcastToOfficers`): speak right on the sender,
    /// listen right plus ignore filter on recipients,
    /// language forced universal (addon stays addon).
    fn on_guild_chat(
        &mut self,
        sender_guid: ObjectGuid,
        chat_tag: u8,
        request: &crate::chat::ChatRequest,
    ) {
        let sender_low = sender_guid.counter();
        let Some(guild_id) = self.player_guild.get(&sender_low).copied() else {
            return;
        };
        let Some(guild) = self.guilds.get(&guild_id) else {
            return;
        };
        let Some(sender_rank) = guild.rank_of(sender_low) else {
            return;
        };
        let (speak_right, listen_right) = if request.msg_type == chat_msg::OFFICER {
            (rights::OFFCHATSPEAK, rights::OFFCHATLISTEN)
        } else {
            (rights::GCHATSPEAK, rights::GCHATLISTEN)
        };
        if !guild.has_rank_right(sender_rank, speak_right) {
            return;
        }
        let lang = if request.lang == language::ADDON {
            language::ADDON
        } else {
            language::UNIVERSAL
        };
        let packet =
            build_chat_packet(request.msg_type, &request.text, lang, chat_tag, sender_guid);
        let recipients: Vec<u32> = guild
            .members()
            .values()
            .filter(|member| guild.has_rank_right(member.rank_id, listen_right))
            .map(|member| member.guid_low)
            .collect();
        for low in recipients {
            if low != sender_low && self.player_ignores(low, sender_low) {
                continue;
            }
            if let Some(session_id) = self.player_session.get(&low).copied() {
                self.reply_packet(session_id, packet.clone());
            }
        }
    }

    /// Player chat (`HandleMessagechatOpcode`,
    /// `ChatHandler.cpp:149-724`) for the types routed so
    /// far; see `chat.rs` for the contract and deviations.
    /// Guild/battleground types drop here, the same
    /// observable result as C++ with no battleground.
    #[allow(clippy::too_many_lines)] // one handler, mirrors HandleMessagechatOpcode
    /// Async character-load dispatch shared by the
    /// login opcode and bot login.
    fn begin_player_login(&mut self, id: SessionId, guid_low: u32) {
        let Some(tx) = self.self_tx.clone() else {
            return;
        };
        let queries = Arc::clone(&self.deps.char_queries);
        tokio::task::spawn_blocking(move || {
            let load = queries
                .load_character(guid_low)
                .map(|opt| opt.map(Box::new));
            let _ = tx.blocking_send(WorldMessage::PlayerLoginReady { id, guid_low, load });
        });
    }

    /// `PlayerBotMgr::AddBot` minimum: log an
    /// existing character in headlessly. The fake
    /// session bypasses admission/queue (like
    /// console-created bot sessions); global account
    /// data and tutorials default (client-UI state a
    /// headless session never surfaces).
    fn login_bot(&mut self, guid_low: u32) -> bool {
        if self.bots.contains_key(&guid_low) || self.player_session.contains_key(&guid_low) {
            return false;
        }
        let Ok(Some(real_account)) = self.deps.auction.account_id_by_guid(guid_low) else {
            self.log.out(
                Level::Detail,
                &format!("Player account for GUID {guid_low} not found..."),
            );
            return false;
        };
        let _ = real_account;
        self.next_bot_session += 1;
        let id = SessionId(self.next_bot_session);
        let account_id = u32::try_from(self.next_bot_session & 0x3FFF_FFFF)
            .unwrap_or(0)
            .saturating_add(0x4000_0000);
        let (control_tx, control_rx) = mpsc::channel(SESSION_SEND_CAPACITY);
        let auth = crate::auth::AcceptedAuth {
            account_id,
            username: format!("BOT{guid_low}"),
            security: security::PLAYER,
            mutetime: 0,
            locale: 0,
            client_build: 5875,
            account_flags: 0,
            client_os: crate::auth::ClientOs::Win,
            client_platform: crate::auth::ClientPlatform::X86,
            verified_email: true,
            email: String::new(),
            session_key: num_bigint::BigUint::ZERO,
        };
        self.by_account.insert(account_id, id);
        self.sessions.insert(
            id,
            SessionEntry {
                account_id,
                auth,
                remote_ip: "127.0.0.1".to_owned(),
                created: Instant::now(),
                latency: 0,
                core: SessionCore::new(),
                queue: VecDeque::new(),
                control: control_tx,
                account_data: AccountDataSet::default(),
                tutorials: [0; 8],
                characters_count: 10,
                character_max_level: 0,
                in_queue: false,
                player_loading: true,
                player_guid: None,
                played_last_tick: Instant::now(),
                played_level: 0,
                next_player_save: None,
                cached_zone: 0,
                character_load: None,
                chat_tag: 0,
                afk_msg: String::new(),
                dnd_msg: String::new(),
                accept_whispers: false,
                allowed_whisperers: HashSet::new(),
                mail_tracker: crate::mail::MailTracker::default(),
            },
        );
        self.bot_sessions.insert(id, guid_low);
        self.bots.insert(
            guid_low,
            BotState {
                session_id: id,
                guid_low,
                control_rx,
                next_tick: Instant::now() + std::time::Duration::from_millis(500),
                combat_behaviors: mangos_bots::BehaviorSet::new(),
                ooc_behaviors: mangos_bots::BehaviorSet::new(),
                cc_marks: Vec::new(),
                totem_cast_at: [None; 4],
                formation_slot: None,
                combat_engaged_at: None,
                leader_low: None,
                role: mangos_bots::BotRole::MeleeDps,
                class_id: 0,
                kit: mangos_bots::SpellKit::default(),
                spells: std::sync::Arc::default(),
                debug: false,
                role_locked: false,
                wipe_reacted: false,
                last_combat_target: None,
                focus_marks: Vec::new(),
                pull_chat_until: None,
                wander_at: None,
                in_world: false,
            },
        );
        self.begin_player_login(id, guid_low);
        true
    }

    /// `PlayerBotMgr::DeleteBot`: full session
    /// teardown for one bot.
    fn logout_bot(&mut self, guid_low: u32) -> bool {
        self.bot_population.on_logout(guid_low);
        let Some(bot) = self.bots.remove(&guid_low) else {
            return false;
        };
        self.bot_sessions.remove(&bot.session_id);
        self.remove(bot.session_id);
        true
    }

    /// Bot activation once the login train put the
    /// player in a map: motion master + tick from
    /// here.
    fn on_bot_entered_world(&mut self, session: SessionId, guid_low: u32, map_key: MapKey) {
        if !self.bot_sessions.contains_key(&session) {
            return;
        }
        let guid = player_guid(guid_low);
        let resolved = self.maps.get(&map_key).and_then(|map| {
            let player = map.player(guid)?;
            let class_id = player.unit.class();
            let level = player.unit.level();
            let known = self.bot_known_spells(map, guid);
            Some((class_id, level, known))
        });
        if let Some(map) = self.maps.get_mut(&map_key) {
            map.motion_initialize_player(guid);
        }
        self.bot_population.on_login(guid_low);
        self.apply_director_assignment(guid_low, map_key);
        if let Some(bot) = self.bots.get_mut(&guid_low) {
            bot.in_world = true;
            bot.next_tick = Instant::now() + std::time::Duration::from_millis(500);
            if let Some((class_id, level, known)) = resolved {
                if !bot.role_locked {
                    bot.role = mangos_bots::auto_assign_role(class_id, &|spell| {
                        known.iter().any(|info| info.id == spell)
                    });
                }
                let store = &self.deps.world_stores.spells;
                let poison_lookup = |name: &str, max_level: u32| {
                    store
                        .spells
                        .values()
                        .filter(|e| {
                            e.effects[0].effect
                                == mangos_spells::defines::effect::ENCHANT_ITEM_TEMPORARY
                                && e.name == name
                                && e.spell_level <= max_level
                        })
                        .max_by_key(|e| e.spell_level)
                        .map(|e| e.id)
                };
                let mut rng = rand::rng();
                let mut pick = |len: usize| rand::Rng::random_range(&mut rng, 0..len.max(1));
                bot.spells = std::sync::Arc::new(mangos_bots::resolve_class_spells(
                    class_id,
                    level,
                    bot.role == mangos_bots::BotRole::Tank,
                    bot.role == mangos_bots::BotRole::MeleeDps,
                    &known,
                    &poison_lookup,
                    &mut pick,
                ));
                bot.kit = mangos_bots::SpellKit::resolve(
                    &bot.spells,
                    bot.role == mangos_bots::BotRole::Tank,
                    0,
                );
                bot.class_id = class_id;
                Self::bot_build_sets(bot);
            }
        }
        self.apply_raid_attach(guid_low);
        self.apply_party_attach(guid_low);
    }

    /// Formation section of `EvaluateSituation`.
    fn evaluate_bot_formation(
        &self,
        ctx: &mut mangos_bots::SituationContext,
        map: &Map,
        guid_low: u32,
        me_pos: mangos_movement::movement_info::Position,
    ) {
        if let Some(bot) = self.bots.get(&guid_low) {
            if let (Some(leader_low), Some(slot_index)) = (bot.leader_low, bot.formation_slot) {
                let leader_guid = player_guid(leader_low);
                if let Some(state) = self.formations.get(leader_guid) {
                    if !state.trail.is_empty() {
                        let host = ActorFormationHost { map };
                        if let Some(point) =
                            state.calculate_slot_position(&host, slot_index, Some(ctx.me))
                        {
                            ctx.formation_target_x = point.x;
                            ctx.formation_target_y = point.y;
                            ctx.formation_target_z = point.z;
                            ctx.dist_to_formation_pos = ((me_pos.x - point.x).powi(2)
                                + (me_pos.y - point.y).powi(2)
                                + (me_pos.z - point.z).powi(2))
                            .sqrt();
                            ctx.has_formation_target = true;
                        }
                        if ctx.is_being_meleed {
                            if let Some(flee) = state.calculate_flee_point(
                                &host,
                                ctx.me,
                                mangos_movement::Vec3::new(me_pos.x, me_pos.y, me_pos.z),
                            ) {
                                ctx.flee_target_x = flee.x;
                                ctx.flee_target_y = flee.y;
                                ctx.flee_target_z = flee.z;
                                ctx.has_flee_target = true;
                            }
                        }
                    }
                }
            }
            if !ctx.has_formation_target {
                if let Some(leader_low) = bot.leader_low {
                    if let Some(leader) = map.player(player_guid(leader_low)) {
                        let target = leader.unit.world.position();
                        ctx.formation_target_x = target.x;
                        ctx.formation_target_y = target.y;
                        ctx.formation_target_z = target.z;
                        ctx.dist_to_formation_pos = ((me_pos.x - target.x).powi(2)
                            + (me_pos.y - target.y).powi(2)
                            + (me_pos.z - target.z).powi(2))
                        .sqrt();
                        ctx.has_formation_target = true;
                    }
                }
            }
        }
    }

    /// Party section of `EvaluateSituation`.
    fn evaluate_bot_party(
        &self,
        ctx: &mut mangos_bots::SituationContext,
        map: &Map,
        guid_low: u32,
        pct: &dyn Fn(u32, u32) -> f32,
    ) {
        ctx.tank_health_pct = 0.0;
        let tank_low = self.find_party_tank(guid_low);
        if let Some(tank_low) = tank_low {
            ctx.has_tank = true;
            if let Some(tank) = map.player(player_guid(tank_low)) {
                ctx.tank_is_alive = tank.unit.is_alive();
                ctx.tank_health_pct = if ctx.tank_is_alive {
                    pct(tank.unit.health(), tank.unit.max_health())
                } else {
                    0.0
                };
                ctx.tank_unit = Some(player_guid(tank_low));
                ctx.tank_needs_heal = ctx.tank_is_alive && ctx.tank_health_pct < 80.0;
            }
        }
        if let Some(group) = self
            .player_group
            .get(&guid_low)
            .and_then(|key| self.groups.get(key))
        {
            for slot in group.members() {
                let Some(member) = map.player(player_guid(slot.guid_low)) else {
                    continue;
                };
                if !member.unit.is_alive() {
                    ctx.dead_party_members = ctx.dead_party_members.saturating_add(1);
                    continue;
                }
                ctx.alive_party_members = ctx.alive_party_members.saturating_add(1);
                if slot.guid_low == guid_low {
                    continue;
                }
                let hp = pct(member.unit.health(), member.unit.max_health());
                if hp < ctx.lowest_ally_health_pct {
                    ctx.lowest_ally_health_pct = hp;
                    ctx.most_injured_ally = Some(player_guid(slot.guid_low));
                }
                let is_tank = Some(slot.guid_low) == tank_low;
                if (is_tank && hp < 80.0) || (!is_tank && hp < 50.0) {
                    ctx.any_ally_needs_heal = true;
                }
                if hp < 30.0 {
                    ctx.critical_heal_needed = true;
                }
                let class = member.unit.class();
                if matches!(class, 2 | 5 | 7 | 11)
                    && pct(member.unit.power(0), member.unit.max_power(0)) > 10.0
                {
                    ctx.healer_available = true;
                }
            }
            // Loose adds: attackers on the party that
            // aren't the tank's victim and aren't CC'd.
            let tank_target = tank_low
                .and_then(|t| map.player(player_guid(t)))
                .filter(|t| t.unit.is_alive())
                .and_then(|t| t.unit.melee_victim);
            'loose: for slot in group.members() {
                for attacker in map.bot_attackers_of(player_guid(slot.guid_low)) {
                    if Some(attacker) != tank_target && !map.bot_is_ccd(attacker) {
                        ctx.has_loose_adds = true;
                        break 'loose;
                    }
                }
            }
        }
    }

    /// Spellbook metadata for the bot classifier
    /// (`PopulateSpellData` inputs). Rank is 0 (the
    /// Rust spell store has no rank subtext; the
    /// classifier falls back to id order like
    /// unranked C++ spells).
    fn bot_known_spells(&self, map: &Map, guid: ObjectGuid) -> Vec<mangos_bots::KnownSpellInfo> {
        use mangos_spells::defines::{aura, effect};
        let store = &self.deps.world_stores.spells;
        map.bot_spell_ids(guid)
            .into_iter()
            .filter_map(|id| store.get(id))
            .map(|entry| {
                let mut direct: Option<i32> = None;
                let mut periodic: Option<i32> = None;
                let mut periodic_total: Option<i32> = None;
                let mut taunt_effects = 0u32;
                let mut is_resurrect = false;
                for eff in &entry.effects {
                    match eff.effect {
                        effect::HEAL => {
                            *direct.get_or_insert(0) += eff.base_points;
                        }
                        effect::ATTACK_ME => taunt_effects += 1,
                        effect::RESURRECT | effect::RESURRECT_NEW => is_resurrect = true,
                        effect::APPLY_AURA => match eff.apply_aura_name {
                            aura::PERIODIC_HEAL => {
                                *periodic.get_or_insert(0) += eff.base_points;
                                let duration = entry.duration_ms(&store.tables).max(0);
                                let ticks = if eff.amplitude > 0 {
                                    duration / i32::try_from(eff.amplitude).unwrap_or(1).max(1)
                                } else {
                                    0
                                };
                                *periodic_total.get_or_insert(0) += ticks * eff.base_points;
                            }
                            aura::MOD_TAUNT => taunt_effects += 1,
                            _ => {}
                        },
                        _ => {}
                    }
                }
                mangos_bots::KnownSpellInfo {
                    id: entry.id,
                    name: entry.name.clone(),
                    rank: 0,
                    spell_level: entry.spell_level,
                    direct_heal_points: direct,
                    periodic_heal_points: periodic,
                    periodic_heal_total: periodic_total,
                    taunt_effects,
                    is_resurrect,
                    skip: entry.attributes & (0x40 | 0x80) != 0,
                }
            })
            .collect()
    }

    /// Registers the Slice D exemplar behaviour
    /// (formation follow of `master_low`) on a bot.
    /// (Re)builds both behaviour sets for a bot
    /// (`BuildBehaviorSet` combat + OOC; party when
    /// a leader is assigned).
    fn bot_build_sets(bot: &mut BotState) {
        let is_party = bot.leader_low.is_some();
        bot.combat_behaviors =
            mangos_bots::role_config::build_behavior_set(bot.class_id, bot.role, true, is_party);
        bot.ooc_behaviors =
            mangos_bots::role_config::build_behavior_set(bot.class_id, bot.role, false, is_party);
    }

    /// `.goname <leader>` catch-up: returns true when a
    /// teleport was issued (fork clears the formation
    /// trail and stops motion; the teleport paths
    /// re-seat motion on landing).
    fn bot_catch_up_to_leader(&mut self, guid_low: u32) -> bool {
        let Some(leader_low) = self.bots.get(&guid_low).and_then(|bot| bot.leader_low) else {
            return false;
        };
        if leader_low == guid_low {
            return false;
        }
        let Some(&bot_key) = self.player_map.get(&guid_low) else {
            return false;
        };
        let Some(&leader_key) = self.player_map.get(&leader_low) else {
            return false;
        };
        let Some((leader_pos, leader_alive)) = self.maps.get(&leader_key).and_then(|map| {
            map.player(player_guid(leader_low))
                .map(|p| (p.unit.world.position(), p.unit.is_alive()))
        }) else {
            return false;
        };
        if !leader_alive {
            return false;
        }
        if leader_key == bot_key {
            let within = self.maps.get(&bot_key).is_some_and(|map| {
                map.player(player_guid(guid_low)).is_some_and(|me| {
                    let pos = me.unit.world.position();
                    let dx = pos.x - leader_pos.x;
                    let dy = pos.y - leader_pos.y;
                    let dz = pos.z - leader_pos.z;
                    dx * dx + dy * dy + dz * dz <= 100.0 * 100.0
                })
            });
            if within {
                return false;
            }
        }
        let Some(session) = self
            .bot_sessions
            .iter()
            .find(|(_, low)| **low == guid_low)
            .map(|(id, _)| *id)
        else {
            return false;
        };
        self.formations
            .formation(player_guid(leader_low))
            .trail
            .clear();
        self.teleport_player(session, leader_key.map_id, leader_pos);
        true
    }

    fn bot_follow_master(&mut self, guid_low: u32, master_low: u32) {
        if let Some(bot) = self.bots.get_mut(&guid_low) {
            bot.leader_low = Some(master_low);
            Self::bot_build_sets(bot);
        }
    }

    /// Bot scheduler tick: drain sinks, then run due
    /// behaviour sets (500 ms cadence, absolute
    /// deadlines from the last due time).
    fn update_bots(&mut self) {
        self.update_bot_population();
        if self.bots.is_empty() {
            return;
        }
        for bot in self.bots.values_mut() {
            let mut drained = 0;
            while drained < 4096 && bot.control_rx.try_recv().is_ok() {
                drained += 1;
            }
        }
        self.update_bot_chat_and_services();
        let now = Instant::now();
        let due: Vec<u32> = self
            .bots
            .values()
            .filter(|bot| bot.in_world && now >= bot.next_tick)
            .map(|bot| bot.guid_low)
            .collect();
        for guid_low in due {
            self.update_bot_formation(guid_low);
            self.bot_acquire_target(guid_low);
            self.bot_wander(guid_low);
            let ctx = self.evaluate_bot_situation(guid_low);
            // Fork party-bot catch-up (`PartyBotAI`
            // out-of-combat arm): leader beyond 100 yd
            // (or on another map/instance) teleports the
            // bot to the leader and ends the tick. The
            // duel gate has no Rust duel system yet.
            if !ctx.in_combat && self.bot_catch_up_to_leader(guid_low) {
                if let Some(bot) = self.bots.get_mut(&guid_low) {
                    bot.next_tick = now + std::time::Duration::from_millis(500);
                }
                continue;
            }
            let in_combat_set = ctx.current_target.is_some() || ctx.in_combat;
            self.bot_party_events(guid_low, &ctx, in_combat_set);
            let (mut behaviors, kit, debug_enabled, opener_blocked) = {
                let Some(bot) = self.bots.get_mut(&guid_low) else {
                    continue;
                };
                bot.next_tick = now + std::time::Duration::from_millis(500);
                // Opener delay: non-tank/healer waits
                // 2s after combat starts (fork
                // `m_combatEngageTimer`).
                let mut opener_blocked = false;
                if in_combat_set {
                    let engaged_at = *bot.combat_engaged_at.get_or_insert(now);
                    if !matches!(
                        bot.role,
                        mangos_bots::BotRole::Tank | mangos_bots::BotRole::Healer
                    ) && now.duration_since(engaged_at) < std::time::Duration::from_millis(2000)
                    {
                        opener_blocked = true;
                    }
                } else {
                    bot.combat_engaged_at = None;
                }
                let taken = if in_combat_set {
                    std::mem::take(&mut bot.combat_behaviors)
                } else {
                    std::mem::take(&mut bot.ooc_behaviors)
                };
                (taken, bot.kit.clone(), bot.debug, opener_blocked)
            };
            let bot_name = self
                .player_name_race_of(guid_low)
                .map_or_else(|| format!("Bot{guid_low}"), |(name, _)| name);
            let mut lines: Vec<String> = Vec::new();
            {
                let mut host = ActorBotHost {
                    world: self,
                    bot_low: guid_low,
                };
                // Dispatcher pre-steps (fork): caster
                // attack-stop (free), dispel scan
                // (consumes the tick).
                if mangos_bots::behaviors_shared::is_caster_role(
                    ctx.role,
                    ctx.player_class,
                    ctx.current_form,
                ) {
                    <ActorBotHost<'_> as mangos_bots::BotHost>::attack_stop(&mut host);
                }
                let dispelled =
                    mangos_bots::behaviors_shared::check_for_dispel_targets(&ctx, &kit, &mut host);
                if !dispelled && !opener_blocked {
                    let mut sink = VecDebug(&mut lines);
                    behaviors.run(&bot_name, &ctx, &kit, &mut host, &mut sink, debug_enabled);
                }
            }
            if let Some(bot) = self.bots.get_mut(&guid_low) {
                if in_combat_set {
                    bot.combat_behaviors = behaviors;
                } else {
                    bot.ooc_behaviors = behaviors;
                }
            }
            for line in lines {
                self.log.out(Level::Debug, &line);
            }
        }
    }

    /// `IsValidHostileTarget` folded to the sources
    /// acquisition draws from: alive, and creature
    /// or opposite-team player (visibility/CC/immune
    /// gates have no Rust equivalents yet).
    fn bot_valid_hostile(&self, map: &Map, me: ObjectGuid, target: ObjectGuid) -> bool {
        if target == me {
            return false;
        }
        if let Some(creature) = map.creature(target) {
            return creature.unit.is_alive();
        }
        if let Some(player) = map.player(target) {
            if !player.unit.is_alive() {
                return false;
            }
            let my_race = map.player(me).map_or(0, |p| p.unit.race());
            return self.team_id_for_race(my_race) != self.team_id_for_race(player.unit.race());
        }
        false
    }

    /// `SelectAttackTarget` + `AttackStart` switch
    /// (non-healer party bots with a leader). The
    /// fleeing-attacker step has no Rust motion type
    /// to detect yet and is skipped.
    fn bot_acquire_target(&mut self, guid_low: u32) {
        let (role, leader_low, focus_marks) = match self.bots.get(&guid_low) {
            Some(bot) => (bot.role, bot.leader_low, bot.focus_marks.clone()),
            None => return,
        };
        if role == mangos_bots::BotRole::Healer {
            return;
        }
        let Some(leader_low) = leader_low.or_else(|| {
            self.player_group
                .get(&guid_low)
                .and_then(|key| self.groups.get(key))
                .map(Group::leader_low)
        }) else {
            return;
        };
        let me = player_guid(guid_low);
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            return;
        };
        let best = self.bot_best_target(guid_low, map_key, leader_low, &focus_marks);
        let current = self
            .maps
            .get(&map_key)
            .and_then(|map| map.player(me).and_then(|p| p.unit.melee_victim));
        let current_valid = current.is_some_and(|victim| {
            self.maps
                .get(&map_key)
                .is_some_and(|map| self.bot_valid_hostile(map, me, victim))
        });
        if !current_valid {
            if current.is_some() {
                if let Some(map) = self.maps.get_mut(&map_key) {
                    map.attack_stop(me);
                }
            }
            if let Some(target) = best {
                self.bot_attack_start(guid_low, target);
            }
        } else if let Some(target) = best {
            if Some(target) != current {
                if let Some(map) = self.maps.get_mut(&map_key) {
                    map.attack_stop(me);
                }
                self.bot_attack_start(guid_low, target);
            }
        }
    }

    /// `SelectAttackTarget` body.
    fn bot_best_target(
        &self,
        guid_low: u32,
        map_key: MapKey,
        leader_low: u32,
        focus_marks: &[u8],
    ) -> Option<ObjectGuid> {
        let me = player_guid(guid_low);
        let map = self.maps.get(&map_key)?;
        let player = map.player(me)?;
        if !player.unit.is_alive() {
            return None;
        }
        let in_combat = player.unit.melee_victim.is_some();
        let mut best: Option<ObjectGuid> = None;
        // Focus marks (in combat or leader engaged).
        let leader_victim = map
            .player(player_guid(leader_low))
            .and_then(|p| p.unit.melee_victim);
        if in_combat || leader_victim.is_some() {
            if let Some(group) = self
                .player_group
                .get(&guid_low)
                .and_then(|key| self.groups.get(key))
            {
                for mark in focus_marks {
                    let raw = group.target_with_icon(usize::from(*mark));
                    if raw != 0 {
                        let target = ObjectGuid::from_raw(raw);
                        if target.is_unit() && self.bot_valid_hostile(map, me, target) {
                            best = Some(target);
                            break;
                        }
                    }
                }
            }
        }
        // Tank's target, then leader's target.
        if best.is_none() {
            if let Some(tank_low) = self.find_party_tank(guid_low) {
                if tank_low != guid_low {
                    if let Some(victim) = map
                        .player(player_guid(tank_low))
                        .and_then(|p| p.unit.melee_victim)
                    {
                        if self.bot_valid_hostile(map, me, victim) {
                            best = Some(victim);
                        }
                    }
                }
            }
        }
        if best.is_none() {
            if let Some(victim) = leader_victim {
                if self.bot_valid_hostile(map, me, victim) {
                    best = Some(victim);
                }
            }
        }
        // My attackers, then party attackers (50y).
        if best.is_none() {
            for attacker in map.bot_attackers_of(me) {
                if self.bot_valid_hostile(map, me, attacker) {
                    best = Some(attacker);
                    break;
                }
            }
        }
        if best.is_none() {
            if let Some(group) = self
                .player_group
                .get(&guid_low)
                .and_then(|key| self.groups.get(key))
            {
                let my_pos = player.unit.world.position();
                'party: for slot in group.members() {
                    if slot.guid_low == guid_low {
                        continue;
                    }
                    for attacker in map.bot_attackers_of(player_guid(slot.guid_low)) {
                        if !self.bot_valid_hostile(map, me, attacker) {
                            continue;
                        }
                        let dist_ok = map
                            .creature(attacker)
                            .map(|c| c.unit.world.position())
                            .or_else(|| map.player(attacker).map(|p| p.unit.world.position()))
                            .is_some_and(|pos| {
                                (pos.x - my_pos.x).powi(2)
                                    + (pos.y - my_pos.y).powi(2)
                                    + (pos.z - my_pos.z).powi(2)
                                    <= 50.0f32.powi(2)
                            });
                        if dist_ok {
                            best = Some(attacker);
                            break 'party;
                        }
                    }
                }
            }
        }
        best
    }

    /// `AttackStart`: melee start + role chase +
    /// tank pull announce.
    fn bot_attack_start(&mut self, guid_low: u32, target: ObjectGuid) {
        use mangos_spells::SpellRng as _;
        let me = player_guid(guid_low);
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            return;
        };
        let (role, was_in_combat) = {
            let Some(map) = self.maps.get_mut(&map_key) else {
                return;
            };
            let was = map
                .player(me)
                .is_some_and(|p| p.unit.melee_victim.is_some());
            map.attack_start(me, target);
            let role = self
                .bots
                .get(&guid_low)
                .map_or(mangos_bots::BotRole::MeleeDps, |bot| bot.role);
            let (dist, angle) = match role {
                mangos_bots::BotRole::RangeDps | mangos_bots::BotRole::Healer => (25.0, 0.0),
                mangos_bots::BotRole::MeleeDps => (1.0, 3.0),
                _ => (1.0, 0.0),
            };
            map.creature_move_chase(me, target, dist, angle);
            (role, was)
        };
        self.drain_map_events();
        // Tank pull announce (15s internal cooldown).
        if role == mangos_bots::BotRole::Tank && !was_in_combat {
            let now = Instant::now();
            let cooled = self
                .bots
                .get(&guid_low)
                .and_then(|bot| bot.pull_chat_until)
                .is_none_or(|until| now >= until);
            if cooled {
                let mut rng = WorldRng::from_entropy();
                let idx = usize::try_from(rng.irand(0, 2)).unwrap_or(0);
                let text = ["pulling", "inc", "going in"][idx];
                let total = self.bots.len();
                if let Some(action) =
                    self.bot_chat
                        .party_announce(me, text, self.bot_now_ms() / 1000, total)
                {
                    self.apply_chat_actions(vec![action]);
                    if let Some(bot) = self.bots.get_mut(&guid_low) {
                        bot.pull_chat_until = Some(now + std::time::Duration::from_secs(15));
                    }
                }
            }
        }
    }

    /// Director-assignment wander for leaderless
    /// crowd/presence bots (`DynamicBotAI` roaming
    /// reduced to radius wander; its player-versus-
    /// player ganking
    /// has no Rust combat-target source yet).
    fn bot_wander(&mut self, guid_low: u32) {
        let Some(assignment) = self.bot_director.assignment(guid_low) else {
            return;
        };
        let Some(bot) = self.bots.get(&guid_low) else {
            return;
        };
        if bot.leader_low.is_some() || !bot.in_world {
            return;
        }
        let now = Instant::now();
        if bot.wander_at.is_some_and(|at| now < at) {
            return;
        }
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            return;
        };
        if map_key.map_id != assignment.target_map {
            return;
        }
        let me = player_guid(guid_low);
        let mut rng = rand::rng();
        let dx = rand::Rng::random_range(&mut rng, -0.5..0.5) * assignment.radius;
        let dy = rand::Rng::random_range(&mut rng, -0.5..0.5) * assignment.radius;
        let dest = mangos_movement::movement_info::Position {
            x: assignment.target_x + dx,
            y: assignment.target_y + dy,
            z: assignment.target_z,
            o: 0.0,
        };
        if let Some(map) = self.maps.get_mut(&map_key) {
            let alive = map
                .player(me)
                .is_some_and(|p| p.unit.is_alive() && p.unit.melee_victim.is_none());
            if !alive {
                return;
            }
            map.creature_move_point(me, 0, dest, 0, 0.0, -10.0);
        }
        if let Some(bot) = self.bots.get_mut(&guid_low) {
            bot.wander_at = Some(
                now + std::time::Duration::from_millis(
                    u64::try_from(rand::Rng::random_range(&mut rng, 8_000..20_000))
                        .unwrap_or(10_000),
                ),
            );
        }
        self.drain_map_events();
    }

    const BOT_CLASS_NAMES: [&str; 12] = [
        "", "Warrior", "Paladin", "Hunter", "Rogue", "Priest", "", "Shaman", "Mage", "Warlock", "",
        "Druid",
    ];
    const BOT_ROLE_NAMES: [&str; 3] = ["Tank", "Healer", "DPS"];

    /// Party wipe/boss-kill chat events
    /// (`PartyBotAI` dead/combat sections).
    fn bot_party_events(
        &mut self,
        guid_low: u32,
        ctx: &mangos_bots::SituationContext,
        in_combat: bool,
    ) {
        use mangos_spells::SpellRng as _;
        let Some(group_key) = self.player_group.get(&guid_low).copied() else {
            return;
        };
        let group_bots: Vec<mangos_bots::chat::BotChatInfo> = self
            .bot_chat_infos()
            .into_iter()
            .filter(|info| info.party == Some(group_key))
            .collect();
        if group_bots.is_empty() {
            return;
        }
        let now_s = self.bot_now_ms() / 1000;
        let mut actions = Vec::new();
        {
            let mut rng = WorldRng::from_entropy();
            let mut pick = |lo: u32, hi: u32| {
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                {
                    rng.irand(lo as i32, hi as i32) as u32
                }
            };
            if in_combat {
                if let Some(bot) = self.bots.get_mut(&guid_low) {
                    bot.wipe_reacted = false;
                }
                // Boss kill: last target now a dead
                // rank>=3 creature (pattern-manager
                // half not ported).
                let last = self
                    .bots
                    .get(&guid_low)
                    .and_then(|bot| bot.last_combat_target);
                if let Some(last) = last {
                    let boss_dead = self
                        .player_map
                        .get(&guid_low)
                        .and_then(|id| self.maps.get(id))
                        .and_then(|map| map.creature(last))
                        .is_some_and(|creature| {
                            !creature.unit.is_alive()
                                && self
                                    .deps
                                    .world_stores
                                    .creature_templates
                                    .get(creature.info.entry)
                                    .is_some_and(|info| info.rank >= 3)
                        });
                    if boss_dead {
                        if let Some(bot) = self.bots.get_mut(&guid_low) {
                            bot.last_combat_target = None;
                        }
                        actions.extend(self.bot_chat.on_boss_killed(&group_bots, now_s, &mut pick));
                    }
                }
                if let Some(target) = ctx.current_target {
                    if let Some(bot) = self.bots.get_mut(&guid_low) {
                        bot.last_combat_target = Some(target);
                    }
                }
            } else if ctx.health_pct <= 0.0 {
                let reacted = self.bots.get(&guid_low).is_some_and(|bot| bot.wipe_reacted);
                if !reacted {
                    let all_dead = self.groups.get(&group_key).is_some_and(|group| {
                        group.members().iter().all(|slot| {
                            self.player_map
                                .get(&slot.guid_low)
                                .and_then(|id| self.maps.get(id))
                                .and_then(|map| map.player(player_guid(slot.guid_low)))
                                .is_some_and(|player| !player.unit.is_alive())
                        })
                    });
                    if all_dead {
                        if let Some(bot) = self.bots.get_mut(&guid_low) {
                            bot.wipe_reacted = true;
                        }
                        actions.extend(self.bot_chat.on_party_wipe(&group_bots, now_s, &mut pick));
                    }
                }
            }
        }
        self.apply_chat_actions(actions);
    }

    /// `SelectDispelTarget` + friendly
    /// `IsValidDispelTarget` arm: first group
    /// member (group order, self included as in
    /// C++) within 30y carrying an aura the spell
    /// can dispel.
    fn bot_dispel_target(&self, guid_low: u32, spell_id: u32) -> Option<ObjectGuid> {
        const EFFECT_DISPEL: u32 = 38;
        const DISPEL_ALL: u32 = 7;
        const DISPEL_ALL_MASK: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4);
        let store = &self.deps.world_stores.spells;
        let entry = store.get(spell_id)?;
        let mut mask = 0u32;
        for eff in &entry.effects {
            if eff.effect == EFFECT_DISPEL {
                let dispel_type = u32::try_from(eff.misc_value).unwrap_or(0);
                mask |= if dispel_type == DISPEL_ALL {
                    DISPEL_ALL_MASK
                } else {
                    1u32.checked_shl(dispel_type).unwrap_or(0)
                };
            }
        }
        if mask == 0 {
            return None;
        }
        let map = self
            .player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))?;
        let me_pos = map.player(player_guid(guid_low))?.unit.world.position();
        let members: Vec<u32> = match self
            .player_group
            .get(&guid_low)
            .and_then(|key| self.groups.get(key))
        {
            Some(group) => group.members().iter().map(|slot| slot.guid_low).collect(),
            None => vec![guid_low],
        };
        for member_low in members {
            let member_guid = player_guid(member_low);
            let Some(member) = map.player(member_guid) else {
                continue;
            };
            if !member.unit.is_alive() {
                continue;
            }
            let pos = member.unit.world.position();
            let dist = ((pos.x - me_pos.x).powi(2)
                + (pos.y - me_pos.y).powi(2)
                + (pos.z - me_pos.z).powi(2))
            .sqrt();
            if dist > 30.0 {
                continue;
            }
            for holder in map.aura_holders(member_guid) {
                let Some(proto) = store.get(holder.spell_id) else {
                    continue;
                };
                if (1u32.checked_shl(proto.dispel).unwrap_or(0) & mask) == 0 {
                    continue;
                }
                if matches!(proto.dispel, 1 | 3 | 4) && holder.positive {
                    continue;
                }
                return Some(member_guid);
            }
        }
        None
    }

    /// Attackers within 10y (`GetAttackersInRangeCount(10)`).
    fn bot_melee_attackers(
        map: &Map,
        me: ObjectGuid,
        me_pos: mangos_movement::movement_info::Position,
    ) -> u8 {
        let mut count = 0u8;
        for attacker in map.bot_attackers_of(me) {
            let attacker_pos = map
                .creature(attacker)
                .map(|c| c.unit.world.position())
                .or_else(|| map.player(attacker).map(|pl| pl.unit.world.position()));
            if let Some(pos) = attacker_pos {
                let dist = ((pos.x - me_pos.x).powi(2)
                    + (pos.y - me_pos.y).powi(2)
                    + (pos.z - me_pos.z).powi(2))
                .sqrt();
                if dist <= 10.0 {
                    count = count.saturating_add(1);
                }
            }
        }
        count
    }

    /// `FindPartyTank`: explicit `.partyrole`
    /// assignment, then bot role, then tanking-form
    /// heuristic (shield heuristic deferred — no
    /// equipment query on players yet).
    fn find_party_tank(&self, guid_low: u32) -> Option<u32> {
        let map = self
            .player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))?;
        let group = self
            .player_group
            .get(&guid_low)
            .and_then(|key| self.groups.get(key))?;
        for slot in group.members() {
            if slot.guid_low == guid_low {
                continue;
            }
            let Some(member) = map.player(player_guid(slot.guid_low)) else {
                continue;
            };
            if !member.unit.is_alive() {
                continue;
            }
            if let Some(role) = self.player_roles.get(&slot.guid_low) {
                if *role == mangos_bots::BotRole::Tank {
                    return Some(slot.guid_low);
                }
                continue;
            }
            if self
                .bots
                .get(&slot.guid_low)
                .is_some_and(|bot| bot.role == mangos_bots::BotRole::Tank)
            {
                return Some(slot.guid_low);
            }
            let form = member.unit.world.object.values().get_byte(
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_BYTES_1,
                2,
            );
            if matches!(form, 0x05 | 0x08 | 0x12) {
                return Some(slot.guid_low);
            }
        }
        None
    }

    /// `GetThreatRatio` + `ShouldThrottleDPS`.
    fn bot_threat_ratio(&self, map: &Map, guid_low: u32, enemy: ObjectGuid) -> (f32, bool) {
        let me = player_guid(guid_low);
        let my_threat = map.threat_on(enemy, me);
        if my_threat <= 0.0 {
            return (0.0, false);
        }
        let ratio = if let Some(tank_low) = self.find_party_tank(guid_low) {
            let tank_threat = map.threat_on(enemy, player_guid(tank_low));
            if tank_threat > 0.0 {
                my_threat / tank_threat
            } else {
                0.0
            }
        } else if let Some(victim) = map.creature(enemy).and_then(|c| c.unit.melee_victim) {
            if victim == me {
                0.0
            } else {
                let victim_threat = map.threat_on(enemy, victim);
                if victim_threat > 0.0 {
                    my_threat / victim_threat
                } else {
                    0.0
                }
            }
        } else {
            0.0
        };
        let role = self
            .bots
            .get(&guid_low)
            .map_or(mangos_bots::BotRole::Invalid, |bot| bot.role);
        let throttle = !matches!(
            role,
            mangos_bots::BotRole::Tank | mangos_bots::BotRole::Healer
        ) && ratio > 0.0
            && ratio >= 0.90;
        (ratio, throttle)
    }

    /// Formation upkeep (`UpdateTrail` +
    /// `AssignSlots` on group-size change).
    fn update_bot_formation(&mut self, guid_low: u32) {
        let Some(leader_low) = self.bots.get(&guid_low).and_then(|bot| bot.leader_low) else {
            return;
        };
        let leader_guid = player_guid(leader_low);
        let Some(map) = self
            .player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))
        else {
            return;
        };
        let Some(leader) = map.player(leader_guid) else {
            return;
        };
        let pos = leader.unit.world.position();
        let now_ms = u32::try_from(unix_now().saturating_mul(1000) & 0xFFFF_FFFF).unwrap_or(0);
        self.formations.update_trail(
            leader_guid,
            mangos_movement::Vec3::new(pos.x, pos.y, pos.z),
            pos.o,
            now_ms,
        );
        // Member list: real group when grouped,
        // else leader + sibling bots.
        let members: Vec<mangos_bots::formation::MemberInfo> = if let Some(group) = self
            .player_group
            .get(&guid_low)
            .and_then(|key| self.groups.get(key))
        {
            group
                .members()
                .iter()
                .map(|slot| mangos_bots::formation::MemberInfo {
                    guid: player_guid(slot.guid_low),
                    bot_role: self
                        .bots
                        .get(&slot.guid_low)
                        .map(|bot| mangos_bots::Role::from(bot.role)),
                })
                .collect()
        } else {
            let mut members = vec![mangos_bots::formation::MemberInfo {
                guid: leader_guid,
                bot_role: None,
            }];
            for (low, bot) in &self.bots {
                if bot.leader_low == Some(leader_low) {
                    members.push(mangos_bots::formation::MemberInfo {
                        guid: player_guid(*low),
                        bot_role: Some(mangos_bots::Role::from(bot.role)),
                    });
                }
            }
            members
        };
        let size = u8::try_from(members.len()).unwrap_or(u8::MAX);
        let state = self.formations.formation(leader_guid);
        if state.last_group_size != size {
            state.assign_slots(&members);
            let assignments: Vec<(u32, u32)> = state
                .slots
                .iter()
                .map(|slot| (slot.assigned_bot.counter(), u32::from(slot.slot_index)))
                .collect();
            for (low, index) in assignments {
                if let Some(bot) = self.bots.get_mut(&low) {
                    bot.formation_slot = Some(index);
                }
            }
        }
    }

    /// `EvaluateSituation` (BotSituationContext.cpp).
    /// Still-unfilled fields (sources land with
    /// their consumers): shield check, combo points,
    /// learned patterns, boss/pattern detection.
    fn evaluate_bot_situation(&self, guid_low: u32) -> mangos_bots::SituationContext {
        let mut ctx = mangos_bots::SituationContext {
            me: player_guid(guid_low),
            ..Default::default()
        };
        let Some(map) = self
            .player_map
            .get(&guid_low)
            .and_then(|map_id| self.maps.get(map_id))
        else {
            return ctx;
        };
        let Some(player) = map.player(player_guid(guid_low)) else {
            return ctx;
        };
        let unit = &player.unit;
        let pct = |cur: u32, max: u32| {
            if max == 0 {
                0.0
            } else {
                (f64::from(cur) * 100.0 / f64::from(max)) as f32
            }
        };
        // === Self ===
        ctx.health_pct = pct(unit.health(), unit.max_health());
        ctx.mana_pct = pct(unit.power(0), unit.max_power(0));
        ctx.power_type = unit.power_type();
        ctx.rage_pct = if ctx.power_type == 1 {
            pct(unit.power(1), unit.max_power(1))
        } else {
            0.0
        };
        ctx.energy_pct = if ctx.power_type == 3 {
            pct(unit.power(3), unit.max_power(3))
        } else {
            0.0
        };
        ctx.player_class = unit.class();
        ctx.current_form = unit.world.object.values().get_byte(
            mangos_protocol::update_field_table::offsets::UNIT_FIELD_BYTES_1,
            2,
        );
        ctx.is_casting = map.bot_casting_spell(ctx.me).is_some();
        ctx.is_moving = map.motion_type_of(ctx.me) != mangos_map::motion::IDLE_MOTION_TYPE;
        ctx.in_combat = unit.melee_victim.is_some();
        ctx.melee_attacker_count = Self::bot_melee_attackers(map, ctx.me, unit.world.position());
        ctx.is_being_meleed = ctx.melee_attacker_count > 0;
        if let Some(bot) = self.bots.get(&guid_low) {
            ctx.role = bot.role;
        }

        // === Target === (melee victim)
        if let Some(victim) = unit.melee_victim {
            ctx.current_target = Some(victim);
            let view = map.player(victim).map_or_else(
                || map.creature(victim).map(|c| (&c.unit, Some(c.info.entry))),
                |p| Some((&p.unit, None)),
            );
            if let Some((target, entry)) = view {
                ctx.target_health_pct = pct(target.health(), target.max_health());
                ctx.target_is_player = entry.is_none();
                ctx.target_creature_entry = entry.unwrap_or(0);
                let a = unit.world.position();
                let b = target.world.position();
                ctx.distance_to_target =
                    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt();
                if let Some(spell_id) = map.bot_casting_spell(victim) {
                    ctx.target_is_casting = true;
                    ctx.target_cast_spell_id = spell_id;
                }
            }
            let (ratio, throttle) = self.bot_threat_ratio(map, guid_low, victim);
            ctx.threat_ratio = ratio;
            ctx.should_throttle = throttle;
        }

        self.evaluate_bot_party(&mut ctx, map, guid_low, &pct);

        self.evaluate_bot_formation(&mut ctx, map, guid_low, unit.world.position());
        ctx
    }

    /// Bot chat delivery (`BotChatManager`
    /// senders): universal language, fork
    /// fan-outs. Party goes to the whole group
    /// (`Group::BroadcastPacket` in the fork bot
    /// path), guild through the normal guild
    /// chat path.
    /// Fork local hour for peak scaling
    /// (`gmtime` + `RandomBot.TimezoneOffset`).
    fn bot_local_hour(&self) -> u32 {
        let offset = i64::from(self.config.bot_population.timezone_offset);
        u32::try_from((unix_now_i64() / 3600 + offset).rem_euclid(24)).unwrap_or(0)
    }

    /// `PlayerBotMgr::Update` + `BotDirector::Update`
    /// (absolute deadlines; the fork drives the
    /// director off the auction timer's interval).
    fn update_bot_population(&mut self) {
        use mangos_spells::SpellRng as _;
        let mut rng = WorldRng::from_entropy();
        let mut pick = |lo: u32, hi: u32| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                rng.irand(lo as i32, hi as i32) as u32
            }
        };
        let now_ms = self.bot_now_ms();
        let pending = std::mem::take(&mut self.pending_bot_logins);
        for guid in pending {
            self.login_bot(guid);
        }
        let hour = self.bot_local_hour();
        for action in self.bot_population.update(now_ms, hour, &mut pick) {
            match action {
                mangos_bots::population::PopulationAction::Login(guid) => {
                    self.login_bot(guid);
                }
                mangos_bots::population::PopulationAction::Logout(guid) => {
                    self.logout_bot(guid);
                }
            }
        }
        if self.bot_director.due(now_ms) {
            self.run_bot_director_cycle(&mut pick);
        }
        self.update_guild_bots(now_ms, &mut pick);
    }

    /// `GuildBotMgr::Update` + one-shot bootstrap
    /// (roster personalities, `PopulateBotsIntoGuild`).
    fn update_guild_bots(&mut self, now_ms: u64, pick: &mut (dyn FnMut(u32, u32) -> u32 + '_)) {
        if !self.guild_bots_bootstrapped {
            self.guild_bots_bootstrapped = true;
            for info in self.guild_bots.roster() {
                self.bot_chat.set_personality(info.guid, info.personality);
            }
            self.populate_guild_bots();
        }
        let offset = i64::from(self.guild_bots.config.presence_timezone_offset);
        let hour = u32::try_from((unix_now_i64() / 3600 + offset).rem_euclid(24)).unwrap_or(0);
        for action in self.guild_bots.update(now_ms, hour, pick) {
            use mangos_bots::guild_bots::GuildBotAction;
            match action {
                GuildBotAction::LoginPresence(guid) => {
                    if self.player_session.contains_key(&guid) {
                        let name = self
                            .guild_bots
                            .bot_info(guid)
                            .map(|info| info.name.clone())
                            .unwrap_or_default();
                        self.log.out(
                            Level::Minimal,
                            &format!(
                                "[GuildBotMgr] Presence login skip (already in world): {name} (GUID {guid})"
                            ),
                        );
                    } else if self.login_bot(guid) {
                        self.guild_bots.set_online(guid, true);
                        let name = self
                            .guild_bots
                            .bot_info(guid)
                            .map(|info| info.name.clone())
                            .unwrap_or_default();
                        self.log.out(
                            Level::Minimal,
                            &format!("[GuildBotMgr] Presence login: {name} (GUID {guid})"),
                        );
                    } else {
                        let name = self
                            .guild_bots
                            .bot_info(guid)
                            .map(|info| info.name.clone())
                            .unwrap_or_default();
                        self.log.out(
                            Level::Minimal,
                            &format!(
                                "[GuildBotMgr] Presence login FAILED: {name} (GUID {guid}) \u{2014} AddBot returned false"
                            ),
                        );
                    }
                }
                GuildBotAction::LogoutPresence(guid) => {
                    if self.logout_bot(guid) {
                        self.guild_bots.set_online(guid, false);
                        let name = self
                            .guild_bots
                            .bot_info(guid)
                            .map(|info| info.name.clone())
                            .unwrap_or_default();
                        self.log.out(
                            Level::Minimal,
                            &format!("[GuildBotMgr] Presence logout: {name}"),
                        );
                    }
                }
                GuildBotAction::SyncLevels => self.sync_guild_bot_levels(pick),
            }
        }
    }

    /// `PopulateBotsIntoGuild` (idempotent; also the
    /// `.guildraid fill` body via `DetectAndPopulateGuild`).
    fn populate_guild_bots(&mut self) {
        let guild_id = self.guild_bots.guild_id;
        if guild_id == 0 || !self.guild_bots.config.enabled {
            return;
        }
        if !self.guilds.contains_key(&guild_id) {
            self.log.out(
                Level::Error,
                &format!("[GuildBotMgr] Guild {guild_id} not found in GuildMgr."),
            );
            return;
        }
        let mut added = 0u32;
        let roster: Vec<(u32, mangos_bots::guild_bots::GuildBotRole)> = self
            .guild_bots
            .roster()
            .iter()
            .map(|info| (info.guid, info.role))
            .collect();
        for (guid, role) in roster {
            if self.player_guild.contains_key(&guid) {
                continue;
            }
            let Some(stats) = self.member_stats_for(guid) else {
                self.log.out(
                    Level::Error,
                    &format!("[GuildBotMgr] Failed to add bot {guid} to guild: status 1"),
                );
                continue;
            };
            let note = match role {
                mangos_bots::guild_bots::GuildBotRole::Tank => "Tank",
                mangos_bots::guild_bots::GuildBotRole::Healer => "Healer",
                mangos_bots::guild_bots::GuildBotRole::Dps => "DPS",
            }
            .to_owned();
            let now = u64::try_from(unix_now_i64()).unwrap_or(0);
            let Some(guild) = self.guilds.get_mut(&guild_id) else {
                return;
            };
            if let Some(fx) = guild.add_member(stats, 2, now) {
                self.player_guild.insert(guid, guild_id);
                self.apply_guild_fx(fx);
                if let Some(guild) = self.guilds.get_mut(&guild_id) {
                    let fx = guild.set_pnote(guid, note);
                    self.apply_guild_fx(fx);
                }
                added += 1;
            }
        }
        if added > 0 {
            self.log.out(
                Level::Minimal,
                &format!("[GuildBotMgr] Added {added} bots to guild {guild_id}."),
            );
            self.guild_roster_broadcast(guild_id);
        }
    }

    /// `SyncBotLevels`: roster levels track the guild
    /// leader ±3 (DB + roster slot).
    fn sync_guild_bot_levels(&mut self, pick: &mut (dyn FnMut(u32, u32) -> u32 + '_)) {
        let guild_id = self.guild_bots.guild_id;
        let Some(guild) = self.guilds.get(&guild_id) else {
            return;
        };
        let leader_low = guild.leader_low();
        let leader_level = self
            .player_map
            .get(&leader_low)
            .and_then(|id| self.maps.get(id))
            .and_then(|map| map.player(player_guid(leader_low)))
            .map(|player| player.unit.level())
            .or_else(|| match self.deps.guild_bot.character_level(leader_low) {
                Ok(level) => level,
                Err(e) => {
                    self.log.out(
                        Level::Error,
                        &format!("guild leader level read failed: {e}"),
                    );
                    None
                }
            })
            .unwrap_or(0);
        if leader_level == 0 {
            return;
        }
        let mut updated = 0u32;
        let guids: Vec<u32> = self
            .guild_bots
            .roster()
            .iter()
            .map(|info| info.guid)
            .collect();
        for guid in guids {
            let offset = i64::from(pick(0, 6)) - 3;
            let target = u32::try_from((i64::from(leader_level) + offset).clamp(1, 60))
                .unwrap_or(leader_level);
            let current = match self.deps.guild_bot.character_level(guid) {
                Ok(Some(level)) => level,
                Ok(None) => continue,
                Err(e) => {
                    self.log
                        .out(Level::Error, &format!("guild bot level read failed: {e}"));
                    continue;
                }
            };
            if current == target {
                continue;
            }
            if let Some(guild) = self.guilds.get_mut(&guild_id) {
                guild.set_member_level(guid, u8::try_from(target).unwrap_or(255));
            }
            if let Err(e) = self.deps.guild_bot.set_character_level(guid, target) {
                self.log
                    .out(Level::Error, &format!("guild bot level write failed: {e}"));
                continue;
            }
            updated += 1;
        }
        if updated > 0 {
            self.log.out(
                Level::Detail,
                &format!(
                    "[GuildBotMgr] Synced {updated} bot levels to GM level {leader_level} (\u{b1}3)."
                ),
            );
            self.guild_roster_broadcast(guild_id);
        }
    }

    fn run_bot_director_cycle(&mut self, pick: &mut (dyn FnMut(u32, u32) -> u32 + '_)) {
        // Humans: real sessions with account < 100
        // (fork's bot test), in a map.
        let mut humans = Vec::new();
        for entry in self.sessions.values() {
            if entry.auth.account_id >= 100 {
                continue;
            }
            let Some(low) = entry.player_guid else {
                continue;
            };
            let guid = player_guid(low);
            let Some(map) = self.player_map.get(&low).and_then(|id| self.maps.get(id)) else {
                continue;
            };
            let Some(player) = map.player(guid) else {
                continue;
            };
            humans.push(mangos_bots::director::HumanInfo {
                level: player.unit.level(),
                zone: map.player_zone_id(guid),
                name: self
                    .player_name_race_of(low)
                    .map(|(name, _)| name)
                    .unwrap_or_default(),
            });
        }
        let (lines, chances) = self.bot_director.run_cycle(&humans, pick);
        for line in lines {
            self.log.out(Level::Detail, &line);
        }
        for (guid, chance) in &chances {
            self.bot_population.set_chance(*guid, *chance);
        }
        if let Err(e) = self.deps.playerbot.update_chances(&chances) {
            self.log.out(
                Level::Error,
                &format!("playerbot chance update failed: {e}"),
            );
        }
    }

    /// `DynamicBotAI::BeforeAddToMap` relocation:
    /// assigned crowd bots spawn inside their zone
    /// (same-map teleport; cross-map assignment
    /// waits for player map transfer).
    fn apply_director_assignment(&mut self, guid_low: u32, map_key: MapKey) {
        let Some(assignment) = self.bot_director.assignment(guid_low) else {
            return;
        };
        if assignment.target_map != map_key.map_id {
            self.log.out(
                Level::Detail,
                &format!(
                    "[BotDirector] bot {guid_low} assignment map {} != spawn map {}; relocation deferred",
                    assignment.target_map, map_key.map_id
                ),
            );
            return;
        }
        let guid = player_guid(guid_low);
        let target = mangos_movement::movement_info::Position {
            x: assignment.target_x,
            y: assignment.target_y,
            z: assignment.target_z,
            o: 0.0,
        };
        if let Some(map) = self.maps.get_mut(&map_key) {
            map.teleport_unit_within_map(guid, target);
        }
        self.drain_map_events();
    }

    /// Monotonic bot clock (`getMSTime`-style).
    fn bot_now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// `BotChatInfo` snapshots for every in-world bot.
    fn bot_chat_infos(&self) -> Vec<mangos_bots::chat::BotChatInfo> {
        const CITY_ZONES: [u32; 6] = [1519, 1537, 1657, 1637, 1638, 1497];
        let mut infos = Vec::new();
        for bot in self.bots.values().filter(|b| b.in_world) {
            let guid = player_guid(bot.guid_low);
            let Some(map) = self
                .player_map
                .get(&bot.guid_low)
                .and_then(|id| self.maps.get(id))
            else {
                continue;
            };
            let Some(player) = map.player(guid) else {
                continue;
            };
            let race = player.unit.race();
            infos.push(mangos_bots::chat::BotChatInfo {
                guid,
                class_id: player.unit.class(),
                level: player.unit.level(),
                is_alive: player.unit.is_alive(),
                in_city: CITY_ZONES.contains(&map.player_zone_id(guid)),
                is_alliance: self.team_id_for_race(race) == 7,
                guild_id: self.player_guild.get(&bot.guid_low).copied().unwrap_or(0),
                party: self.player_group.get(&bot.guid_low).copied(),
            });
        }
        infos
    }

    /// `BotChatManager::Update` + `BotServiceMgr::Update`.
    fn update_bot_chat_and_services(&mut self) {
        use mangos_spells::SpellRng as _;
        let now_ms = self.bot_now_ms();
        let infos = self.bot_chat_infos();
        let actions = {
            let mut rng = WorldRng::from_entropy();
            let mut pick = |lo: u32, hi: u32| {
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                {
                    rng.irand(lo as i32, hi as i32) as u32
                }
            };
            self.bot_chat.update(now_ms, &infos, &mut pick)
        };
        self.apply_chat_actions(actions);
        let now_unix = u64::try_from(unix_now_i64()).unwrap_or(0);
        self.bot_services.update(now_ms, now_unix);
    }

    /// Apply `ChatAction`s to the world.
    fn apply_chat_actions(&mut self, actions: Vec<mangos_bots::chat::ChatAction>) {
        use mangos_bots::chat::ChatAction;
        for action in actions {
            match action {
                ChatAction::Say { bot, kind, text } => {
                    self.bot_say(bot.counter(), kind, &text);
                }
                ChatAction::Whisper { bot, target, text } => {
                    self.bot_whisper(bot.counter(), target.counter(), &text);
                }
                ChatAction::TradeChannel { bot, text } => {
                    let bot_low = bot.counter();
                    let team = match self
                        .player_name_race_of(bot_low)
                        .map_or(0, |(_, race)| self.team_id_for_race(race))
                    {
                        7 => mangos_entities::unit::Team::Alliance,
                        1 => mangos_entities::unit::Team::Horde,
                        _ => mangos_entities::unit::Team::None,
                    };
                    let stores = Arc::clone(&self.deps.world_stores);
                    let mut outs = Vec::new();
                    if let Some(mgr) = self.channel_mgr_mut(team) {
                        outs = mgr
                            .get_join_channel("Trade - City", &stores.chat_channels)
                            .say_skip_check(bot_low, &text);
                    }
                    self.deliver_channel_outs(outs);
                }
                ChatAction::Emote { bot, emote_id } => {
                    if let Some(map) = self
                        .player_map
                        .get(&bot.counter())
                        .copied()
                        .and_then(|id| self.maps.get_mut(&id))
                    {
                        map.player_text_emote(bot, emote_id, emote_id, ObjectGuid::EMPTY);
                    }
                    self.drain_map_events();
                }
            }
        }
    }

    /// Whisper-to-bot routing (`OnWhisperReceived`):
    /// services, then buy, then generic delayed reply.
    fn route_bot_whisper(&mut self, sender_low: u32, bot_low: u32, text: &str) {
        use mangos_spells::SpellRng as _;
        let bot_guid = player_guid(bot_low);
        let sender_guid = player_guid(sender_low);
        let Some(info) = self
            .bot_chat_infos()
            .into_iter()
            .find(|i| i.guid.counter() == bot_low)
        else {
            return;
        };
        let svc_info = mangos_bots::services::ServiceBotInfo {
            guid: bot_guid,
            class_id: info.class_id,
            is_alliance: info.is_alliance,
        };
        let now_s = u64::try_from(unix_now_i64()).unwrap_or(0);
        let mut rng = WorldRng::from_entropy();
        let mut pick = |lo: u32, hi: u32| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                rng.irand(lo as i32, hi as i32) as u32
            }
        };
        if let Some(actions) =
            self.bot_services
                .handle_service_whisper(svc_info, sender_guid, text, now_s, &mut pick)
        {
            self.apply_chat_actions(actions);
            return;
        }
        if let Some(actions) = self.bot_services.handle_buy_whisper(
            svc_info,
            sender_guid,
            text,
            now_s,
            &|item: u32, stack: u32| self.ah_bot.fair_buyout(item, stack),
        ) {
            self.apply_chat_actions(actions);
            return;
        }
        let reply = mangos_bots::chat::ChatCore::pick_whisper_response(text, &mut pick);
        if !reply.is_empty() {
            let delay_ms = u64::from(5000 + pick(0, 15_000));
            self.bot_chat
                .queue_whisper(bot_guid, sender_guid, reply, delay_ms, self.bot_now_ms());
        }
    }

    fn bot_say(&mut self, bot_low: u32, kind: mangos_bots::BotChatKind, text: &str) {
        use crate::chat::{chat_msg, language};
        use mangos_bots::BotChatKind;
        if text.is_empty() {
            return;
        }
        let guid = player_guid(bot_low);
        let bot_name = self
            .player_name_race_of(bot_low)
            .map_or_else(|| format!("Bot{bot_low}"), |(name, _)| name);
        match kind {
            BotChatKind::Say | BotChatKind::Yell => {
                let Some(map_id) = self.player_map.get(&bot_low).copied() else {
                    return;
                };
                let Some((alive, level)) = self.maps.get(&map_id).and_then(|map| {
                    map.player(guid)
                        .map(|player| (player.unit.is_alive(), player.unit.level()))
                }) else {
                    return;
                };
                if !alive {
                    return;
                }
                let yr = yell_range(
                    self.config.listen_range_yell,
                    level,
                    self.config.yell_range_linear_scale_max_level,
                    self.config.yell_range_quadratic_scale_max_level,
                    self.config.yell_range_min,
                );
                let (msg_type, range) = match kind {
                    BotChatKind::Say => (chat_msg::SAY, self.config.listen_range_say.min(yr)),
                    _ => (chat_msg::YELL, yr),
                };
                let packet =
                    crate::chat::build_chat_packet(msg_type, text, language::UNIVERSAL, 0, guid);
                if let Some(map) = self.maps.get_mut(&map_id) {
                    map.broadcast_packet_in_range(guid, range, false, &packet);
                }
                self.drain_map_events();
            }
            BotChatKind::Party => {
                let Some(group) = self
                    .player_group
                    .get(&bot_low)
                    .and_then(|key| self.groups.get(key))
                else {
                    return;
                };
                let packet = crate::chat::build_chat_packet(
                    chat_msg::PARTY,
                    text,
                    language::UNIVERSAL,
                    0,
                    guid,
                );
                for (guid_low, packet) in group.broadcast(&packet, None, None) {
                    if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                        self.reply_packet(session_id, packet);
                    }
                }
                self.log.out(
                    Level::Detail,
                    &format!("BotChat: {bot_name} says in party: {text}"),
                );
            }
            BotChatKind::Guild => {
                let request = crate::chat::ChatRequest {
                    msg_type: chat_msg::GUILD,
                    lang: language::UNIVERSAL,
                    target: String::new(),
                    text: text.to_owned(),
                };
                self.on_guild_chat(guid, 0, &request);
                self.log.out(
                    Level::Detail,
                    &format!("BotChat: {bot_name} says in guild: {text}"),
                );
            }
            BotChatKind::Whisper(target) => {
                self.bot_whisper(bot_low, target.counter(), text);
            }
        }
    }

    /// `SendWhisperToPlayer`.
    fn bot_whisper(&mut self, bot_low: u32, target_low: u32, text: &str) {
        use crate::chat::{chat_msg, language};
        let Some(session_id) = self.player_session.get(&target_low).copied() else {
            return;
        };
        let packet = crate::chat::build_chat_packet(
            chat_msg::WHISPER,
            text,
            language::UNIVERSAL,
            0,
            player_guid(bot_low),
        );
        self.reply_packet(session_id, packet);
        let bot_name = self
            .player_name_race_of(bot_low)
            .map_or_else(|| format!("Bot{bot_low}"), |(name, _)| name);
        let target_name = self
            .player_name_race_of(target_low)
            .map_or_else(|| "?".to_owned(), |(name, _)| name);
        self.log.out(
            Level::Detail,
            &format!("BotChat: {bot_name} whispers to {target_name}: {text}"),
        );
    }

    /// Party greet for a freshly grouped bot.
    fn bot_party_greet(&mut self, bot_low: u32) {
        use mangos_spells::SpellRng as _;
        let Some(info) = self
            .bot_chat_infos()
            .into_iter()
            .find(|i| i.guid.counter() == bot_low)
        else {
            return;
        };
        let total_bots = self.bots.len();
        let now_s = self.bot_now_ms() / 1000;
        let actions = {
            let mut rng = WorldRng::from_entropy();
            let mut pick = |lo: u32, hi: u32| {
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                {
                    rng.irand(lo as i32, hi as i32) as u32
                }
            };
            self.bot_chat
                .on_bot_joined_group(info, now_s, total_bots, &mut pick)
        };
        self.apply_chat_actions(actions);
    }

    /// `PartyBotAI::AddToPlayerGroup` via the real
    /// invite/accept train so packets and DB writes
    /// match a player-accepted invite exactly.
    fn bot_join_leader_group(&mut self, bot_low: u32, leader_low: u32) {
        if bot_low == leader_low || self.player_group.contains_key(&bot_low) {
            return;
        }
        let Some(bot_session) = self.player_session.get(&bot_low).copied() else {
            return;
        };
        let key = if let Some(key) = self.player_group.get(&leader_low).copied() {
            key
        } else {
            let Some((leader_name, _)) = self.player_name_race_of(leader_low) else {
                return;
            };
            let key = self.alloc_group_key();
            self.groups
                .insert(key, Group::new_invite_shell(leader_low, leader_name));
            self.player_group_invite.insert(leader_low, key);
            self.refresh_leader_flag(leader_low);
            key
        };
        let invited = self
            .groups
            .get_mut(&key)
            .is_some_and(|group| group.add_invite(bot_low));
        if !invited {
            return;
        }
        self.player_group_invite.insert(bot_low, key);
        self.on_group_accept(bot_session, bot_low);
        if self.player_group.contains_key(&bot_low) {
            self.bot_party_greet(bot_low);
        }
    }

    /// Deferred `.partybot load` attachment, run at
    /// world entry: relocate near the leader (same
    /// map), follow, group, greet.
    fn apply_party_attach(&mut self, bot_low: u32) {
        let Some(pos) = self
            .pending_party_attach
            .iter()
            .position(|(bot, _)| *bot == bot_low)
        else {
            return;
        };
        let (_, leader_low) = self.pending_party_attach.remove(pos);
        if let (Some(bot_map), Some(leader_map)) = (
            self.player_map.get(&bot_low).copied(),
            self.player_map.get(&leader_low).copied(),
        ) {
            if bot_map == leader_map {
                let leader_pos = self.maps.get(&leader_map).and_then(|map| {
                    map.player(player_guid(leader_low))
                        .map(|p| p.unit.world.position())
                });
                if let Some(pos) = leader_pos {
                    let target = mangos_movement::movement_info::Position {
                        x: pos.x + 2.0,
                        y: pos.y,
                        z: pos.z,
                        o: pos.o,
                    };
                    if let Some(map) = self.maps.get_mut(&bot_map) {
                        map.teleport_unit_within_map(player_guid(bot_low), target);
                    }
                    self.drain_map_events();
                }
            }
        }
        self.bot_follow_master(bot_low, leader_low);
        self.bot_join_leader_group(bot_low, leader_low);
    }

    /// Raid-assembly attachment at world entry:
    /// roster role (locked) + follow + group join.
    fn apply_raid_attach(&mut self, bot_low: u32) {
        let Some(pos) = self
            .pending_raid_attach
            .iter()
            .position(|(bot, _, _)| *bot == bot_low)
        else {
            return;
        };
        let (_, leader_low, role) = self.pending_raid_attach.remove(pos);
        if let Some(bot) = self.bots.get_mut(&bot_low) {
            bot.role = role;
            bot.role_locked = true;
            Self::bot_build_sets(bot);
        }
        self.bot_follow_master(bot_low, leader_low);
        self.bot_join_leader_group(bot_low, leader_low);
    }

    /// `HandleBotAddCommand` (existing character by
    /// name; follow-the-invoker is the Slice D
    /// exemplar attachment).
    fn command_bot_add(&mut self, id: SessionId, args: &str) -> bool {
        let Some(name_token) = args.split_whitespace().next() else {
            return false;
        };
        let Some(name) = normalize_player_name(name_token) else {
            return false;
        };
        let data = match self.deps.social.player_data_by_name(&name) {
            Ok(data) => data,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("bot add lookup failed: {e}"));
                None
            }
        };
        let Some(data) = data else {
            self.send_sys_text(id, &format!("Player not found : '{name}'"));
            return true;
        };
        self.bot_population.mark_manual_add(data.guid_low);
        if !self.login_bot(data.guid_low) {
            self.send_sys_text(id, "[PlayerBotMgr] Unable to load bot.");
            return true;
        }
        if let Some(master_low) = self.actor_low(id) {
            self.bot_follow_master(data.guid_low, master_low);
        }
        // Party greet (`OnGroupMemberJoined`) when the
        // bot lands in a group immediately.
        if self.player_group.contains_key(&data.guid_low) {
            self.bot_party_greet(data.guid_low);
        }
        self.send_sys_text(
            id,
            &format!(
                "[PlayerBotMgr] Bot added : '{name}', GUID={}",
                data.guid_low
            ),
        );
        true
    }

    /// `HandleBotDeleteCommand`.
    fn command_bot_delete(&mut self, id: SessionId, args: &str) -> bool {
        let Some(name_token) = args.split_whitespace().next() else {
            self.send_sys_text(id, "Syntax : $playerName");
            return true;
        };
        let Some(name) = normalize_player_name(name_token) else {
            return false;
        };
        let data = self.deps.social.player_data_by_name(&name).ok().flatten();
        let Some(data) = data else {
            self.send_sys_text(id, &format!("Unable to find player: '{name}'"));
            return true;
        };
        if self.logout_bot(data.guid_low) {
            self.send_sys_text(
                id,
                &format!("Bot {name} (GUID:{}) disconnected.", data.guid_low),
            );
        } else {
            self.send_sys_text(
                id,
                &format!(
                    "Bot {name} (GUID:{}) : unable to disconnect.",
                    data.guid_low
                ),
            );
        }
        true
    }

    /// `HandleBotInfoCommand`.
    fn command_bot_info(&mut self, id: SessionId) -> bool {
        let online = self.sessions.values().filter(|e| !e.in_queue).count();
        let stats = self.bot_population.stats();
        self.send_sys_text(id, "-- PlayerBot stats --");
        self.send_sys_text(
            id,
            &format!(
                "Min:{} Max:{} Total:{}",
                stats.conf_min_online, stats.conf_max_online, stats.total_bots
            ),
        );
        self.send_sys_text(
            id,
            &format!(
                "Loading : {}, Online : {}, Chat : {}",
                stats.loading_count, stats.online_count, stats.online_chat
            ),
        );
        self.send_sys_text(
            id,
            &format!(
                "{}p + {}b = {}",
                online.saturating_sub(stats.online_count as usize),
                stats.online_count,
                online
            ),
        );
        true
    }

    /// `HandleBotStartCommand`.
    fn command_bot_start(&mut self, _id: SessionId) -> bool {
        self.bot_population.set_enabled(true);
        true
    }

    /// `HandleBotStopCommand` (`DeleteAll` + logout).
    fn command_bot_stop(&mut self, id: SessionId) -> bool {
        let guids: Vec<u32> = self.bots.keys().copied().collect();
        for guid in guids {
            self.logout_bot(guid);
        }
        self.send_sys_text(id, "All the bots have been unloaded.");
        true
    }

    /// `HandleBotAddRandomCommand`.
    fn command_bot_ranadd(&mut self, id: SessionId, args: &str) -> bool {
        use mangos_spells::SpellRng as _;
        let count: u32 = args
            .split_whitespace()
            .next()
            .and_then(|tok| tok.parse().ok())
            .unwrap_or(1);
        let mut rng = WorldRng::from_entropy();
        let mut pick = |lo: u32, hi: u32| {
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                rng.irand(lo as i32, hi as i32) as u32
            }
        };
        for _ in 0..count {
            if let Some(guid) = self.bot_population.add_random(&mut pick) {
                self.login_bot(guid);
            }
        }
        self.send_sys_text(id, &format!("{count} bots added"));
        true
    }

    /// `HandleBotAddAllCommand`.
    fn command_bot_add_all(&mut self, id: SessionId) -> bool {
        for guid in self.bot_population.add_all() {
            self.login_bot(guid);
        }
        self.send_sys_text(id, "All bots have been loaded.");
        true
    }

    /// `HandlePartyBotLoadCommand`.
    fn command_partybot_load(&mut self, id: SessionId, args: &str) -> bool {
        let Some(name_token) = args.split_whitespace().next() else {
            return false;
        };
        let Some(name) = normalize_player_name(name_token) else {
            return false;
        };
        let data = match self.deps.social.player_data_by_name(&name) {
            Ok(data) => data,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("partybot load lookup failed: {e}"));
                None
            }
        };
        let Some(data) = data else {
            self.send_sys_text(id, "Player not found.");
            return false;
        };
        if self.player_session.contains_key(&data.guid_low) {
            self.send_sys_text(id, "Player is already online!");
            return false;
        }
        let Some(leader_low) = self.actor_low(id) else {
            return false;
        };
        if self
            .player_group
            .get(&leader_low)
            .and_then(|key| self.groups.get(key))
            .is_some_and(Group::is_full)
        {
            self.send_sys_text(id, "Cannot add more bots. Group is full.");
            return false;
        }
        if !self.login_bot(data.guid_low) {
            self.send_sys_text(id, "Error spawning bot.");
            return false;
        }
        self.pending_party_attach.push((data.guid_low, leader_low));
        self.send_sys_text(id, &format!("Loading {name} as party bot."));
        true
    }

    /// Invoker's selected unit guid, if any.
    fn selected_unit_low(&self, actor_low: u32) -> Option<ObjectGuid> {
        let guid = self
            .player_map
            .get(&actor_low)
            .and_then(|mid| self.maps.get(mid))
            .and_then(|map| map.player(player_guid(actor_low)))
            .map(|player| {
                player
                    .unit
                    .world
                    .object
                    .values()
                    .get_guid(mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET)
            })?;
        (guid != ObjectGuid::EMPTY).then_some(guid)
    }

    /// `HandlePartyBotAttackStartCommand`.
    fn command_partybot_attackstart(&mut self, id: SessionId) -> bool {
        let Some(actor_low) = self.actor_low(id) else {
            return false;
        };
        let Some(target) = self
            .selected_unit_low(actor_low)
            .filter(|guid| guid.counter() != actor_low || !guid.is_player())
        else {
            self.send_sys_text(id, "You must select a character or creature.");
            return false;
        };
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            self.send_sys_text(id, "You are not in a group.");
            return false;
        };
        let members: Vec<u32> = self.groups.get(&key).map_or_else(Vec::new, |group| {
            group.members().iter().map(|slot| slot.guid_low).collect()
        });
        let target_name = self
            .player_map
            .get(&actor_low)
            .and_then(|mid| self.maps.get(mid))
            .map(|map| map.object_name(target))
            .unwrap_or_default();
        for member in members {
            if member == actor_low || !self.bots.contains_key(&member) {
                continue;
            }
            self.bot_attack_start(member, target);
        }
        self.send_sys_text(
            id,
            &format!("All party bots are now attacking {target_name}."),
        );
        true
    }

    /// `HandlePartyBotAttackStopCommand`.
    fn command_partybot_attackstop(&mut self, id: SessionId) -> bool {
        let Some(actor_low) = self.actor_low(id) else {
            return false;
        };
        let Some(target) = self.selected_unit_low(actor_low) else {
            self.send_sys_text(id, "You must select a character or creature.");
            return false;
        };
        let Some(key) = self.player_group.get(&actor_low).copied() else {
            self.send_sys_text(id, "You are not in a group.");
            return false;
        };
        let members: Vec<u32> = self.groups.get(&key).map_or_else(Vec::new, |group| {
            group.members().iter().map(|slot| slot.guid_low).collect()
        });
        let target_name = self
            .player_map
            .get(&actor_low)
            .and_then(|mid| self.maps.get(mid))
            .map(|map| map.object_name(target))
            .unwrap_or_default();
        for member in members {
            if member == actor_low || !self.bots.contains_key(&member) {
                continue;
            }
            if let Some(map) = self
                .player_map
                .get(&member)
                .copied()
                .and_then(|mid| self.maps.get_mut(&mid))
            {
                map.attack_stop(player_guid(member));
            }
        }
        self.drain_map_events();
        self.send_sys_text(
            id,
            &format!("All party bots have stopped attacking {target_name}."),
        );
        true
    }

    /// Mark-command target resolution: named group
    /// member, else selected player, else empty
    /// (group-wide). Errors already messaged on
    /// `Some(false)`... returns None on hard error.
    fn mark_targets(
        &mut self,
        id: SessionId,
        actor_low: u32,
        name_arg: Option<&str>,
    ) -> Option<Vec<u32>> {
        let mut targets: Vec<u32> = Vec::new();
        if let Some(name_arg) = name_arg {
            if let Some(key) = self.player_group.get(&actor_low).copied() {
                if let Some(group) = self.groups.get(&key) {
                    for slot in group.members() {
                        if slot.guid_low != actor_low
                            && self
                                .player_name_race_of(slot.guid_low)
                                .is_some_and(|(name, _)| name == name_arg)
                        {
                            targets.push(slot.guid_low);
                        }
                    }
                }
            }
            if targets.is_empty() {
                self.send_sys_text(id, &format!("Bot '{name_arg}' not found in your group."));
                return None;
            }
        } else if let Some(selected) = self
            .selected_unit_low(actor_low)
            .filter(|guid| guid.is_player() && guid.counter() != actor_low)
        {
            targets.push(selected.counter());
        }
        Some(targets)
    }

    /// Shared `ccmark`/`focusmark` body.
    fn command_partybot_mark(&mut self, id: SessionId, args: &str, focus: bool) -> bool {
        const MARKS: [(&str, u8); 8] = [
            ("star", 0),
            ("circle", 1),
            ("diamond", 2),
            ("triangle", 3),
            ("moon", 4),
            ("square", 5),
            ("cross", 6),
            ("skull", 7),
        ];
        let mut parts = args.split_whitespace();
        let Some(mark_arg) = parts.next() else {
            self.send_sys_text(
                id,
                if focus {
                    "Syntax: .partybot focusmark <mark> [botname]"
                } else {
                    "Syntax: .partybot ccmark <mark> [botname]"
                },
            );
            return false;
        };
        let Some((_, mark)) = MARKS.iter().find(|(name, _)| *name == mark_arg) else {
            self.send_sys_text(id, "Invalid mark.");
            return false;
        };
        let mark = *mark;
        let Some(actor_low) = self.actor_low(id) else {
            return false;
        };
        let Some(targets) = self.mark_targets(id, actor_low, parts.next()) else {
            return false;
        };
        let verb = if focus { "focus" } else { "CC" };
        if targets.is_empty() {
            let Some(key) = self.player_group.get(&actor_low).copied() else {
                self.send_sys_text(id, "You are not in a group.");
                return false;
            };
            let members: Vec<u32> = self.groups.get(&key).map_or_else(Vec::new, |group| {
                group.members().iter().map(|slot| slot.guid_low).collect()
            });
            for member in members {
                if member == actor_low {
                    continue;
                }
                if let Some(bot) = self.bots.get_mut(&member) {
                    let list = if focus {
                        &mut bot.focus_marks
                    } else {
                        &mut bot.cc_marks
                    };
                    if !list.contains(&mark) {
                        list.push(mark);
                    }
                }
            }
            self.send_sys_text(id, &format!("All party bots will {verb} {mark_arg}."));
            return true;
        }
        for target in targets {
            let name = self
                .player_name_race_of(target)
                .map(|(name, _)| name)
                .unwrap_or_default();
            let Some(bot) = self.bots.get_mut(&target) else {
                self.send_sys_text(id, "Target is not a party bot.");
                return false;
            };
            let list = if focus {
                &mut bot.focus_marks
            } else {
                &mut bot.cc_marks
            };
            if list.contains(&mark) {
                self.send_sys_text(id, &format!("{name} already has {verb} {mark_arg}."));
                return false;
            }
            list.push(mark);
            self.send_sys_text(id, &format!("{name} will {verb} {mark_arg}."));
        }
        true
    }

    /// `HandlePartyBotClearMarksCommand`.
    fn command_partybot_clearmarks(&mut self, id: SessionId, args: &str) -> bool {
        let Some(actor_low) = self.actor_low(id) else {
            return false;
        };
        let Some(targets) = self.mark_targets(id, actor_low, args.split_whitespace().next()) else {
            return false;
        };
        if targets.is_empty() {
            let Some(key) = self.player_group.get(&actor_low).copied() else {
                self.send_sys_text(id, "You are not in a group.");
                return false;
            };
            let members: Vec<u32> = self.groups.get(&key).map_or_else(Vec::new, |group| {
                group.members().iter().map(|slot| slot.guid_low).collect()
            });
            for member in members {
                if let Some(bot) = self.bots.get_mut(&member) {
                    bot.cc_marks.clear();
                    bot.focus_marks.clear();
                }
            }
            self.send_sys_text(id, "All mark assignments cleared for all party bots.");
            return true;
        }
        for target in targets {
            let name = self
                .player_name_race_of(target)
                .map(|(name, _)| name)
                .unwrap_or_default();
            let Some(bot) = self.bots.get_mut(&target) else {
                self.send_sys_text(id, "Target is not a party bot.");
                return false;
            };
            bot.cc_marks.clear();
            bot.focus_marks.clear();
            self.send_sys_text(id, &format!("All mark assignments cleared for {name}."));
        }
        true
    }

    /// `HandlePartyBotSetRoleCommand` (selection =
    /// invoker's `UNIT_FIELD_TARGET`).
    fn command_partybot_setrole(&mut self, id: SessionId, args: &str) -> bool {
        let role_str = args.trim();
        let Some(leader_low) = self.actor_low(id) else {
            return false;
        };
        let target_low = self
            .player_map
            .get(&leader_low)
            .and_then(|mid| self.maps.get(mid))
            .and_then(|map| map.player(player_guid(leader_low)))
            .map(|player| {
                player
                    .unit
                    .world
                    .object
                    .values()
                    .get_guid(mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET)
                    .counter()
            });
        let Some(target_low) = target_low.filter(|low| *low != 0) else {
            self.send_sys_text(id, "You must select a character.");
            return false;
        };
        let role = match role_str {
            "tank" => Some(mangos_bots::BotRole::Tank),
            "healer" => Some(mangos_bots::BotRole::Healer),
            "meleedps" => Some(mangos_bots::BotRole::MeleeDps),
            "rangedps" => Some(mangos_bots::BotRole::RangeDps),
            "dps" => self.bots.get(&target_low).map(|bot| {
                if matches!(bot.class_id, 3 | 8 | 9) {
                    mangos_bots::BotRole::RangeDps
                } else {
                    mangos_bots::BotRole::MeleeDps
                }
            }),
            _ => None,
        };
        let Some(role) = role else {
            return false;
        };
        let Some(bot) = self.bots.get_mut(&target_low) else {
            self.send_sys_text(id, "Target is not a party bot.");
            return false;
        };
        bot.role = role;
        bot.role_locked = true;
        Self::bot_build_sets(bot);
        let name = self
            .player_name_race_of(target_low)
            .map(|(name, _)| name)
            .unwrap_or_default();
        self.send_sys_text(id, &format!("{name} is now a {role_str}."));
        true
    }

    fn guild_bots_disabled(&mut self, id: SessionId) -> bool {
        if self.guild_bots.config.enabled {
            return false;
        }
        self.send_sys_text(id, "Guild bot system is disabled.");
        true
    }

    /// `HandleGuildRaidFillCommand`.
    fn command_guildraid_fill(&mut self, id: SessionId) -> bool {
        if self.guild_bots_disabled(id) {
            return true;
        }
        self.populate_guild_bots();
        if self.guild_bots.guild_id == 0 {
            self.send_sys_text(
                id,
                "No guild found. Create a guild first using a Guild Charter.",
            );
        } else {
            self.send_sys_text(
                id,
                &format!(
                    "Guild roster populated. {} bots in roster.",
                    self.guild_bots.roster().len()
                ),
            );
        }
        true
    }

    /// `HandleGuildRaidAssembleCommand` + `AssembleRaid`.
    fn command_guildraid_assemble(&mut self, id: SessionId) -> bool {
        let Some(leader_low) = self.actor_low(id) else {
            return false;
        };
        if self.guild_bots_disabled(id) {
            return true;
        }
        if self.guild_bots.guild_id == 0 {
            self.populate_guild_bots();
            if self.guild_bots.guild_id == 0 {
                self.send_sys_text(id, "No guild bots available to assemble.");
                return true;
            }
        }
        let mut spawned = 0u32;
        let roster: Vec<(u32, mangos_bots::BotRole)> = self
            .guild_bots
            .roster()
            .iter()
            .filter(|info| info.raid_assigned && info.active && !info.online)
            .map(|info| (info.guid, mangos_bots::guild_bots::behavior_role(info)))
            .collect();
        for (guid, role) in roster {
            if self.player_session.contains_key(&guid) {
                continue;
            }
            if self.login_bot(guid) {
                self.guild_bots.set_online(guid, true);
                self.pending_raid_attach.push((guid, leader_low, role));
                spawned += 1;
            }
        }
        let leader_name = self
            .player_name_race_of(leader_low)
            .map(|(name, _)| name)
            .unwrap_or_default();
        self.log.out(
            Level::Minimal,
            &format!(
                "[GuildBotMgr] Assembled raid: {spawned} guild bots spawned for {leader_name}."
            ),
        );
        if spawned > 0 {
            self.send_sys_text(
                id,
                &format!(
                    "Assembling guild raid ({} bots assigned).",
                    self.guild_bots.online_count()
                ),
            );
        } else {
            self.send_sys_text(id, "No guild bots available to assemble.");
        }
        true
    }

    /// `HandleGuildRaidDisbandCommand` + `DisbandRaid`.
    fn command_guildraid_disband(&mut self, id: SessionId) -> bool {
        if self.actor_low(id).is_none() {
            return false;
        }
        let mut dismissed = 0u32;
        let online: Vec<u32> = self
            .guild_bots
            .roster()
            .iter()
            .filter(|info| info.online)
            .map(|info| info.guid)
            .collect();
        for guid in online {
            self.pending_raid_attach.retain(|(b, _, _)| *b != guid);
            if self.logout_bot(guid) {
                self.guild_bots.set_online(guid, false);
                dismissed += 1;
            }
        }
        self.log.out(
            Level::Minimal,
            &format!("[GuildBotMgr] Disbanded raid: {dismissed} guild bots dismissed."),
        );
        if dismissed > 0 {
            self.send_sys_text(id, "Guild raid disbanded.");
        } else {
            self.send_sys_text(id, "No guild bots online to disband.");
        }
        true
    }

    /// `HandleGuildRaidCheckCommand`.
    fn command_guildraid_check(&mut self, id: SessionId) -> bool {
        if self.guild_bots_disabled(id) {
            return true;
        }
        let (tanks, healers, melee, ranged, mark, fort, kings, trueshot) =
            self.guild_bots.raid_composition();
        self.send_sys_text(
            id,
            &format!(
                "Raid composition: {tanks} tanks, {healers} healers, {melee} melee DPS, {ranged} ranged DPS ({} total)",
                tanks + healers + melee + ranged
            ),
        );
        if !fort {
            self.send_sys_text(id, "  WARNING: No Priest for Fortitude!");
        }
        if !mark {
            self.send_sys_text(id, "  WARNING: No Druid for Mark of the Wild!");
        }
        if !kings {
            self.send_sys_text(id, "  WARNING: No Paladin for Blessings!");
        }
        if !trueshot {
            self.send_sys_text(id, "  WARNING: No Hunter for Trueshot Aura!");
        }
        true
    }

    /// `HandleGuildRosterCommand`.
    fn command_guildraid_roster(&mut self, id: SessionId) -> bool {
        if self.guild_bots_disabled(id) {
            return true;
        }
        self.send_sys_text(
            id,
            &format!("Guild Roster ({} bots):", self.guild_bots.roster().len()),
        );
        let lines: Vec<String> = self
            .guild_bots
            .roster()
            .iter()
            .map(|info| {
                let class_name = Self::BOT_CLASS_NAMES
                    .get(usize::from(info.class_id))
                    .copied()
                    .unwrap_or("Unknown");
                format!(
                    "  {}{} - {} {} ({}) - GS: {:.0}",
                    if info.raid_assigned { "[R] " } else { "    " },
                    info.name,
                    class_name,
                    info.spec,
                    Self::BOT_ROLE_NAMES[info.role as usize],
                    info.gear_score
                )
            })
            .collect();
        for line in lines {
            self.send_sys_text(id, &line);
        }
        true
    }

    /// `HandleGuildBotInfoCommand`.
    fn command_guildraid_botinfo(&mut self, id: SessionId, args: &str) -> bool {
        let search = args.trim();
        if search.is_empty() {
            self.send_sys_text(id, "Usage: .guildraid botinfo <name>");
            return true;
        }
        if self.guild_bots_disabled(id) {
            return true;
        }
        let found = self
            .guild_bots
            .roster()
            .iter()
            .find(|info| info.name == search)
            .cloned();
        if let Some(info) = found {
            let class_name = Self::BOT_CLASS_NAMES
                .get(usize::from(info.class_id))
                .copied()
                .unwrap_or("Unknown");
            self.send_sys_text(id, &format!("{} - Level 60 {}", info.name, class_name));
            self.send_sys_text(
                id,
                &format!(
                    "  Spec: {} | Role: {}",
                    info.spec,
                    Self::BOT_ROLE_NAMES[info.role as usize]
                ),
            );
            self.send_sys_text(id, &format!("  Gear Score: {:.0}", info.gear_score));
            self.send_sys_text(
                id,
                &format!(
                    "  Raid Assigned: {} | Active: {}",
                    if info.raid_assigned { "Yes" } else { "No" },
                    if info.active { "Yes" } else { "No" }
                ),
            );
            self.send_sys_text(
                id,
                &format!("  Online: {}", if info.online { "Yes" } else { "No" }),
            );
        }
        true
    }

    /// `HandlePartyRoleCommand`.
    fn command_partyrole(&mut self, id: SessionId, args: &str) -> bool {
        let Some(actor_low) = self.actor_low(id) else {
            return false;
        };
        let arg = args.trim();
        if arg.is_empty() {
            match self.player_roles.get(&actor_low) {
                None => self.send_sys_text(
                    id,
                    "Your role is unset (bots use heuristics to identify you).",
                ),
                Some(mangos_bots::BotRole::Tank) => {
                    self.send_sys_text(id, "Your role is: tank.");
                }
                Some(mangos_bots::BotRole::Healer) => {
                    self.send_sys_text(id, "Your role is: healer.");
                }
                Some(_) => self.send_sys_text(id, "Your role is: dps."),
            }
            self.send_sys_text(id, "Usage: .partyrole tank|healer|dps|clear");
            return true;
        }
        match arg {
            "tank" => {
                self.player_roles
                    .insert(actor_low, mangos_bots::BotRole::Tank);
                self.send_sys_text(id, "Role set to tank. Bots will treat you as the tank.");
            }
            "healer" => {
                self.player_roles
                    .insert(actor_low, mangos_bots::BotRole::Healer);
                self.send_sys_text(
                    id,
                    "Role set to healer. Bots will not shield you in tank role.",
                );
            }
            "dps" => {
                self.player_roles
                    .insert(actor_low, mangos_bots::BotRole::MeleeDps);
                self.send_sys_text(
                    id,
                    "Role set to dps. Bots will not treat you as tank or healer.",
                );
            }
            "clear" => {
                self.player_roles.remove(&actor_low);
                self.send_sys_text(
                    id,
                    "Role cleared. Bots will use heuristics to identify your role.",
                );
            }
            _ => {
                self.send_sys_text(id, "Usage: .partyrole tank|healer|dps|clear");
                return false;
            }
        }
        true
    }

    /// `HandlePartyBotDebugCommand` minimum: toggles
    /// behaviour debug on every bot (selection-based
    /// targeting lands with selection state).
    fn command_partybot_debug(&mut self, id: SessionId) -> bool {
        if self.bots.is_empty() {
            self.send_sys_text(id, "Target is not a party bot.");
            return true;
        }
        let enabling = !self.bots.values().any(|bot| bot.debug);
        for bot in self.bots.values_mut() {
            bot.debug = enabling;
        }
        self.send_sys_text(
            id,
            &format!(
                "Behavior debug {} for all bots. Check Server.log.",
                if enabling { "ENABLED" } else { "DISABLED" }
            ),
        );
        true
    }

    /// `ChatHandler::ParseCommands` for a chat
    /// session: true when the text was consumed as a
    /// command attempt. Execution is inline (the
    /// actor is the world thread the fork defers to).
    fn parse_commands(&mut self, id: SessionId, text: &str) -> bool {
        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if first != '!' && first != '.' {
            return false;
        }
        if text.len() < 2 {
            return false;
        }
        let security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        if security == security::PLAYER && !self.config.player_commands {
            return false;
        }
        if text.starts_with("..") || text.starts_with("!!") {
            return false;
        }
        self.execute_command(id, &text[1..]);
        true
    }

    /// `SendSysMessage(const char*)`: one system chat
    /// packet per non-empty line (`LineFromMessage`
    /// strtok semantics collapse blank lines).
    fn send_sys_text(&mut self, id: SessionId, text: &str) {
        for line in text.split('\n') {
            if !line.is_empty() {
                self.reply_packet(id, build_sys_message(line));
            }
        }
    }

    /// `SendSysMessage(LANG_*)` with the session locale.
    fn send_sys_lang(&mut self, id: SessionId, lang_id: i32) {
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let text = self
            .strings
            .get(lang_id)
            .map_or("", |entry| entry.text(locale))
            .to_owned();
        self.send_sys_text(id, &text);
    }

    /// `PSendSysMessage(LANG_*, arg)`.
    fn send_sys_lang_arg(&mut self, id: SessionId, lang_id: i32, arg: &str) {
        let locale = self.sessions.get(&id).map_or(0, |entry| entry.auth.locale);
        let text = self
            .strings
            .get(lang_id)
            .map_or("", |entry| entry.text(locale))
            .replacen("%s", arg, 1);
        self.send_sys_text(id, &text);
    }

    /// Emits a `commands::HelpOutput`.
    fn send_help_output(&mut self, id: SessionId, output: &crate::commands::HelpOutput) {
        for line in &output.lines {
            match line {
                crate::commands::HelpLine::Lang(lang_id) => self.send_sys_lang(id, *lang_id),
                crate::commands::HelpLine::LangArg(lang_id, arg) => {
                    self.send_sys_lang_arg(id, *lang_id, arg);
                }
                crate::commands::HelpLine::Text(text) => self.send_sys_text(id, text),
            }
        }
    }

    /// `ShowHelpForCommand(getCommandTable(), text)`.
    fn show_help(&mut self, id: SessionId, text: &str, security: u8, account_id: u32) -> bool {
        let output = self.commands.help_for(text, security, account_id);
        let found = output.found;
        self.send_help_output(id, &output);
        found
    }

    /// `HandleHelpCommand`.
    fn command_help(&mut self, id: SessionId, args: &str, security: u8, account_id: u32) {
        if args.is_empty() {
            self.show_help(id, "help", security, account_id);
            self.show_help(id, "", security, account_id);
        } else if !self.show_help(id, args, security, account_id) {
            self.send_sys_lang(id, crate::commands::LANG_NO_CMD);
        }
    }

    /// `ExecuteCommand`: availability, GM logging,
    /// and dispatch. Handlers registered so far:
    /// `help` and `commands`; every other known
    /// command takes the fork's handler-false path
    /// (help or syntax + subcommand list) until its
    /// handler is ported. The DB-flags
    /// only-on-self/critical log extras depend on
    /// selection state the actor does not track yet.
    fn is_bot_command(name: &str) -> bool {
        name.starts_with("bot ")
            || name.starts_with("partybot ")
            || name == "partyrole"
            || name.starts_with("guildraid ")
            || name.starts_with("ahbot ")
    }

    /// Bot command family dispatch (Slice D/E handlers).
    fn execute_bot_command(&mut self, id: SessionId, name: &str, tail: &str) -> bool {
        match name {
            "ahbot update" => self.command_ahbot_update(id),
            "ahbot reload" => self.command_ahbot_reload(id),
            "bot add" => self.command_bot_add(id, tail),
            "bot delete" => self.command_bot_delete(id, tail),
            "bot info" => self.command_bot_info(id),
            "bot start" => self.command_bot_start(id),
            "bot stop" => self.command_bot_stop(id),
            "bot ranadd" => self.command_bot_ranadd(id, tail),
            "bot add_all" => self.command_bot_add_all(id),
            "partybot debug" => self.command_partybot_debug(id),
            "partybot load" => self.command_partybot_load(id, tail),
            "partybot setrole" => self.command_partybot_setrole(id, tail),
            "partybot attackstart" => self.command_partybot_attackstart(id),
            "partybot attackstop" => self.command_partybot_attackstop(id),
            "partybot ccmark" => self.command_partybot_mark(id, tail, false),
            "partybot focusmark" => self.command_partybot_mark(id, tail, true),
            "partybot clearmarks" => self.command_partybot_clearmarks(id, tail),
            "partyrole" => self.command_partyrole(id, tail),
            "guildraid fill" => self.command_guildraid_fill(id),
            "guildraid assemble" => self.command_guildraid_assemble(id),
            "guildraid disband" => self.command_guildraid_disband(id),
            "guildraid check" => self.command_guildraid_check(id),
            "guildraid roster" => self.command_guildraid_roster(id),
            "guildraid botinfo" => self.command_guildraid_botinfo(id, tail),
            _ => false,
        }
    }

    fn execute_command(&mut self, id: SessionId, text: &str) {
        use crate::commands::SearchResult;
        let (security, account_id) = self
            .sessions
            .get(&id)
            .map_or((security::PLAYER, 0), |entry| {
                (entry.auth.security, entry.account_id)
            });
        let outcome = self.commands.find(text, false);
        match outcome.result {
            SearchResult::Ok => {
                let Some(cmd) = self.commands.get(&outcome.path).cloned() else {
                    return;
                };
                if !self.commands.is_available(&cmd, security, account_id) {
                    self.send_sys_lang(id, crate::commands::LANG_COMMAND_UNAVAILABLE);
                    return;
                }
                if cmd.security > security::PLAYER {
                    self.log.out(
                        Level::Basic,
                        &format!(
                            "Command: {} [Account: {account_id}]",
                            if outcome.tail.is_empty() {
                                cmd.full_name.clone()
                            } else {
                                format!("{} {}", cmd.full_name, outcome.tail)
                            }
                        ),
                    );
                }
                let handled = match cmd.full_name.as_str() {
                    "help" => {
                        self.command_help(id, &outcome.tail, security, account_id);
                        true
                    }
                    "commands" => {
                        self.show_help(id, "", security, account_id);
                        true
                    }
                    _ if Self::is_bot_command(&cmd.full_name) => {
                        self.execute_bot_command(id, &cmd.full_name, &outcome.tail)
                    }
                    _ => false,
                };
                if !handled {
                    if cmd.help.is_empty() {
                        self.send_sys_lang(id, crate::commands::LANG_CMD_SYNTAX);
                    } else {
                        self.send_sys_text(id, &cmd.help.clone());
                    }
                    let show = if cmd.name.is_empty() && outcome.path.len() > 1 {
                        self.commands
                            .get(&outcome.path[..outcome.path.len() - 1])
                            .cloned()
                    } else {
                        Some(cmd.clone())
                    };
                    if let Some(show) = show
                        && !show.children.is_empty()
                        && let Some(list) =
                            self.commands
                                .subcommand_list(&show.children, security, account_id)
                    {
                        self.send_sys_lang_arg(id, crate::commands::LANG_SUBCMDS_LIST, show.name);
                        self.send_sys_text(id, &list);
                    }
                }
            }
            SearchResult::UnknownSubcommand => {
                self.send_sys_lang(id, crate::commands::LANG_NO_SUBCMD);
                if let Some(parent) = self.commands.get(&outcome.path).cloned() {
                    let output = self.commands.help_for_in(
                        &parent.children,
                        &outcome.tail,
                        false,
                        security,
                        account_id,
                    );
                    self.send_help_output(id, &output);
                }
            }
            SearchResult::Unknown => {
                self.send_sys_lang(id, crate::commands::LANG_NO_CMD);
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn on_message_chat(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let request = match parse_message_chat(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(_) => {
                self.bad_packet(id, opcode_id::CMSG_MESSAGECHAT);
                return;
            }
        };
        if !language_allowed(request.lang, request.msg_type) {
            return;
        }
        // Addon-language traffic: channels (B.2b), group
        // types (B.3), and guild types (B.4) carry it.
        if request.lang == language::ADDON
            && !matches!(
                request.msg_type,
                chat_msg::CHANNEL
                    | chat_msg::PARTY
                    | chat_msg::RAID
                    | chat_msg::RAID_LEADER
                    | chat_msg::RAID_WARNING
                    | chat_msg::GUILD
                    | chat_msg::OFFICER
            )
        {
            return;
        }
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        if request.lang != language::UNIVERSAL
            && request.lang != language::ADDON
            && !self.knows_language(map_key, guid, request.lang)
        {
            return;
        }
        let mut lang = request.lang;
        if self.config.allow_two_side_interaction_chat
            && matches!(lang, language::COMMON | language::ORCISH)
        {
            lang = language::UNIVERSAL;
        }
        // Two-side group interaction universalizes the
        // group chat family (`ChatHandler.cpp:204-205`).
        if self.config.allow_two_side_interaction_group
            && matches!(
                request.msg_type,
                chat_msg::PARTY | chat_msg::RAID | chat_msg::RAID_LEADER | chat_msg::RAID_WARNING
            )
            && matches!(lang, language::COMMON | language::ORCISH)
        {
            lang = language::UNIVERSAL;
        }
        // Dot commands are consumed before chat
        // delivery for the text types `ParseCommands`
        // covers (`ChatHandler.cpp` per-type parse).
        if matches!(
            request.msg_type,
            chat_msg::CHANNEL
                | chat_msg::WHISPER
                | chat_msg::SAY
                | chat_msg::EMOTE
                | chat_msg::YELL
                | chat_msg::PARTY
                | chat_msg::GUILD
                | chat_msg::OFFICER
                | chat_msg::RAID
                | chat_msg::RAID_LEADER
                | chat_msg::RAID_WARNING
        ) && self.parse_commands(id, &request.text)
        {
            return;
        }
        let chat_tag = self.sessions.get(&id).map_or(0, |entry| entry.chat_tag);
        match request.msg_type {
            chat_msg::SAY | chat_msg::YELL | chat_msg::EMOTE => {
                if request.text.is_empty() {
                    return;
                }
                let Some((alive, level)) = self.maps.get(&map_key).and_then(|map| {
                    map.player(guid)
                        .map(|player| (player.unit.is_alive(), player.unit.level()))
                }) else {
                    return;
                };
                if !alive {
                    return;
                }
                let yr = yell_range(
                    self.config.listen_range_yell,
                    level,
                    self.config.yell_range_linear_scale_max_level,
                    self.config.yell_range_quadratic_scale_max_level,
                    self.config.yell_range_min,
                );
                let (out_lang, range, own_team_only) = match request.msg_type {
                    chat_msg::SAY => (lang, self.config.listen_range_say.min(yr), false),
                    chat_msg::YELL => (lang, yr, false),
                    _ => (
                        language::UNIVERSAL,
                        self.config.listen_range_text_emote,
                        !self.config.allow_two_side_interaction_chat,
                    ),
                };
                let packet =
                    build_chat_packet(request.msg_type, &request.text, out_lang, chat_tag, guid);
                if let Some(map) = self.maps.get_mut(&map_key) {
                    map.broadcast_packet_in_range(guid, range, own_team_only, &packet);
                }
                self.drain_map_events();
            }
            chat_msg::CHANNEL => {
                if !request.text.is_empty() {
                    self.on_channel_say(id, chat_tag, &request, lang);
                }
            }
            chat_msg::WHISPER => {
                if !request.text.is_empty() {
                    self.on_whisper(id, map_key, guid, chat_tag, &request);
                }
            }
            chat_msg::PARTY | chat_msg::RAID | chat_msg::RAID_LEADER | chat_msg::RAID_WARNING => {
                if !request.text.is_empty() {
                    self.on_group_chat(guid, chat_tag, &request, lang);
                }
            }
            chat_msg::GUILD | chat_msg::OFFICER => {
                if !request.text.is_empty() {
                    self.on_guild_chat(guid, chat_tag, &request);
                }
            }
            chat_msg::AFK => self.on_afk_dnd(id, true, &request.text),
            chat_msg::DND => self.on_afk_dnd(id, false, &request.text),
            _ => {}
        }
    }

    /// Party/raid chat delivery (`ChatHandler.cpp`
    /// `CHAT_MSG_PARTY` / `CHAT_MSG_RAID` /
    /// `CHAT_MSG_RAID_LEADER` / `CHAT_MSG_RAID_WARNING`
    /// arms): party goes to the sender's subgroup, raid
    /// types to the whole raid, leader/warning gated on
    /// leader resp. leader-or-assistant.
    fn on_group_chat(
        &mut self,
        sender_guid: ObjectGuid,
        chat_tag: u8,
        request: &crate::chat::ChatRequest,
        lang: u32,
    ) {
        let sender_low = sender_guid.counter();
        let Some(key) = self.player_group.get(&sender_low).copied() else {
            return;
        };
        let Some(group) = self.groups.get(&key) else {
            return;
        };
        let subgroup = match request.msg_type {
            chat_msg::PARTY => Some(group.member_subgroup(sender_low)),
            chat_msg::RAID => {
                if !group.is_raid() {
                    return;
                }
                None
            }
            chat_msg::RAID_LEADER => {
                if !group.is_raid() || !group.is_leader(sender_low) {
                    return;
                }
                None
            }
            chat_msg::RAID_WARNING => {
                if !group.is_raid()
                    || !(group.is_leader(sender_low) || group.is_assistant(sender_low))
                {
                    return;
                }
                None
            }
            _ => return,
        };
        let packet =
            build_chat_packet(request.msg_type, &request.text, lang, chat_tag, sender_guid);
        for (guid_low, packet) in group.broadcast(&packet, subgroup, None) {
            if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                self.reply_packet(session_id, packet);
            }
        }
    }

    /// `Player::KnowsLanguage`: the mask is built from
    /// known language spells — stored active spells plus
    /// the race/class defaults merged at login.
    fn knows_language(&self, map_key: MapKey, guid: ObjectGuid, lang: u32) -> bool {
        let Some(spell) = language_spell(lang) else {
            return false;
        };
        let Some(player) = self.maps.get(&map_key).and_then(|map| map.player(guid)) else {
            return false;
        };
        if player
            .deferred
            .spells
            .iter()
            .any(|row| row.spell == spell && row.active != 0 && row.disabled == 0)
        {
            return true;
        }
        self.deps
            .world_stores
            .player_infos
            .default_spells(
                u32::from(player.unit.race()),
                u32::from(player.unit.class()),
            )
            .contains(&spell)
    }

    /// `ObjectAccessor::FindMasterPlayer`: online lookup
    /// by normalized name, yielding (guid low, session,
    /// race) in guid order.
    fn find_online_player_by_name(&self, name: &str) -> Option<(u32, SessionId, u8)> {
        let mut guid_lows: Vec<u32> = self.player_map.keys().copied().collect();
        guid_lows.sort_unstable();
        for guid_low in guid_lows {
            let Some(map) = self
                .player_map
                .get(&guid_low)
                .and_then(|mid| self.maps.get(mid))
            else {
                continue;
            };
            let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
            let Some(player) = map.player(guid) else {
                continue;
            };
            if normalize_player_name(&player.unit.world.name).as_deref() == Some(name)
                && let Some(&session_id) = self.player_session.get(&guid_low)
            {
                return Some((guid_low, session_id, player.unit.race()));
            }
        }
        None
    }

    /// Whisper delivery (`ChatHandler.cpp:467-543` +
    /// `MasterPlayer::Whisper`, `MasterPlayerChat.cpp:41-73`)
    /// under the fork's default chat configs.
    fn on_whisper(
        &mut self,
        id: SessionId,
        map_key: MapKey,
        sender_guid: ObjectGuid,
        sender_tag: u8,
        request: &crate::chat::ChatRequest,
    ) {
        let Some(name) = normalize_player_name(&request.target) else {
            self.reply_packet(id, build_player_not_found(&request.target));
            return;
        };
        let sender_security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        let Some(sender_race) = self
            .maps
            .get(&map_key)
            .and_then(|map| map.player(sender_guid))
            .map(|player| player.unit.race())
        else {
            return;
        };
        let Some((target_low, target_session, target_race)) =
            self.find_online_player_by_name(&name)
        else {
            self.reply_packet(id, build_player_not_found(&name));
            return;
        };
        let Some((target_security, target_tag, target_afk, target_dnd, target_accepts)) =
            self.sessions.get(&target_session).map(|entry| {
                (
                    entry.auth.security,
                    entry.chat_tag,
                    entry.afk_msg.clone(),
                    entry.dnd_msg.clone(),
                    entry.accept_whispers
                        || entry.allowed_whisperers.contains(&sender_guid.counter()),
                )
            })
        else {
            self.reply_packet(id, build_player_not_found(&name));
            return;
        };
        if sender_security == security::PLAYER
            && target_security > security::PLAYER
            && !target_accepts
        {
            self.reply_packet(id, build_player_not_found(&name));
            return;
        }
        if sender_security == security::PLAYER
            && target_security == security::PLAYER
            && !self.config.allow_two_side_interaction_chat
            && self.team_id_for_race(sender_race) != self.team_id_for_race(target_race)
        {
            self.reply_packet(id, build_wrong_faction());
            return;
        }
        let target_guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, target_low);
        let auto_reply = match target_tag {
            2 => Some((chat_msg::DND, target_dnd.as_str())),
            1 => Some((chat_msg::AFK, target_afk.as_str())),
            _ => None,
        };
        self.deliver_whisper(
            id,
            target_session,
            (sender_guid, target_guid),
            (sender_tag, target_tag),
            &request.text,
        );
        if let Some((reply_type, reply_text)) = auto_reply {
            self.reply_packet(
                id,
                build_chat_packet(reply_type, reply_text, language::UNIVERSAL, 0, target_guid),
            );
        }
        if let Some(entry) = self.sessions.get_mut(&id)
            && !entry.accept_whispers
        {
            entry.allowed_whisperers.insert(target_low);
        }
        // Bot whisper routing (`OnWhisperReceived`).
        if self.bots.contains_key(&target_low) {
            let sender_low = sender_guid.counter();
            let text = request.text.clone();
            self.route_bot_whisper(sender_low, target_low, &text);
        }
    }

    /// `MasterPlayer::Whisper` packet pair: the whisper to
    /// the receiver, then the inform echo to the sender,
    /// both forced to universal language.
    fn deliver_whisper(
        &mut self,
        sender_id: SessionId,
        target_session: SessionId,
        guids: (ObjectGuid, ObjectGuid),
        tags: (u8, u8),
        text: &str,
    ) {
        let (sender_guid, target_guid) = guids;
        let (sender_tag, target_tag) = tags;
        self.reply_packet(
            target_session,
            build_chat_packet(
                chat_msg::WHISPER,
                text,
                language::UNIVERSAL,
                sender_tag,
                sender_guid,
            ),
        );
        self.reply_packet(
            sender_id,
            build_chat_packet(
                chat_msg::WHISPER_INFORM,
                text,
                language::UNIVERSAL,
                target_tag,
                target_guid,
            ),
        );
    }

    /// AFK/DND toggles (`ChatHandler.cpp:683-722`) over
    /// the single chat-tag state (`MasterPlayerChat.cpp`
    /// ToggleAFK/ToggleDND), minus the AFK in-combat
    /// guard. A message updates the stored reply without
    /// untoggling; an empty message toggles.
    fn on_afk_dnd(&mut self, id: SessionId, afk: bool, text: &str) {
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        let tag = if afk { 1 } else { 2 };
        let is_set = entry.chat_tag == tag;
        if !text.is_empty() || !is_set {
            if afk {
                text.clone_into(&mut entry.afk_msg);
            } else {
                text.clone_into(&mut entry.dnd_msg);
            }
        }
        if text.is_empty() || !is_set {
            entry.chat_tag = if entry.chat_tag == tag { 0 } else { tag };
        }
    }

    /// `channelMgr(team)` (`ChannelMgr.cpp:31-42`).
    fn channel_mgr(&self, team: mangos_entities::unit::Team) -> Option<&ChannelMgr> {
        use mangos_entities::unit::Team;
        if self.config.allow_two_side_interaction_channel {
            return Some(&self.channels_alliance);
        }
        match team {
            Team::Alliance => Some(&self.channels_alliance),
            Team::Horde => Some(&self.channels_horde),
            Team::None => None,
        }
    }

    fn channel_mgr_mut(&mut self, team: mangos_entities::unit::Team) -> Option<&mut ChannelMgr> {
        use mangos_entities::unit::Team;
        if self.config.allow_two_side_interaction_channel {
            return Some(&mut self.channels_alliance);
        }
        match team {
            Team::Alliance => Some(&mut self.channels_alliance),
            Team::Horde => Some(&mut self.channels_horde),
            Team::None => None,
        }
    }

    /// The channel view of the session's player, plus
    /// level/guild/team for handler gates.
    fn channel_actor_for(
        &self,
        id: SessionId,
    ) -> Option<(ChannelActor, u32, u32, mangos_entities::unit::Team)> {
        let (map_id, guid) = self.player_map_guid(id)?;
        let entry = self.sessions.get(&id)?;
        let player = self.maps.get(&map_id)?.player(guid)?;
        let actor = ChannelActor {
            guid_low: guid.counter(),
            security: entry.auth.security,
            team_id: self.team_id_for_race(player.unit.race()),
        };
        Some((actor, player.unit.level(), player.guild_id, player.team))
    }

    /// The channel view of an online player by guid low
    /// (moderation targets).
    fn channel_actor_by_guid_low(&self, guid_low: u32) -> Option<ChannelActor> {
        let session_id = *self.player_session.get(&guid_low)?;
        let (actor, ..) = self.channel_actor_for(session_id)?;
        Some(actor)
    }

    /// Online target resolution by normalized name
    /// (`sObjectMgr.GetPlayer(name)`).
    fn channel_target_by_name(&self, normalized_name: &str) -> Option<ChannelActor> {
        let (guid_low, ..) = self.find_online_player_by_name(normalized_name)?;
        self.channel_actor_by_guid_low(guid_low)
    }

    /// `GetSocial()->HasIgnore(source)` for a recipient.
    fn player_ignores(&self, who_low: u32, source_low: u32) -> bool {
        let Some(map) = self
            .player_map
            .get(&who_low)
            .and_then(|mid| self.maps.get(mid))
        else {
            return false;
        };
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, who_low);
        map.player(guid).is_some_and(|player| {
            crate::social::has_flag(
                &player.deferred.social,
                source_low,
                crate::social::SOCIAL_FLAG_IGNORED,
            )
        })
    }

    /// Deliver `Channel` output packets (`SendToOne` /
    /// `SendToAll` with the recipient ignore filter).
    fn deliver_channel_outs(&mut self, outs: Vec<ChannelOut>) {
        for out in outs {
            match out {
                ChannelOut::ToOne { guid_low, packet } => {
                    if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                        self.reply_packet(session_id, packet);
                    }
                }
                ChannelOut::ToAll {
                    recipients,
                    packet,
                    ignore_source,
                } => {
                    for guid_low in recipients {
                        if let Some(source) = ignore_source
                            && self.player_ignores(guid_low, source)
                        {
                            continue;
                        }
                        if let Some(session_id) = self.player_session.get(&guid_low).copied() {
                            self.reply_packet(session_id, packet.clone());
                        }
                    }
                }
            }
        }
    }

    /// `GetChannel(name, player)` not-found behavior:
    /// `SMSG_CHANNEL_NOTIFY` not-member with the raw name.
    fn channel_not_found(&mut self, id: SessionId, name: &str) {
        self.reply_packet(id, not_on_packet(name));
    }

    /// Run `op` on an existing channel, delivering its
    /// output; missing channel answers not-member.
    fn with_channel(
        &mut self,
        id: SessionId,
        team: mangos_entities::unit::Team,
        name: &str,
        op: impl FnOnce(&mut crate::channels::Channel) -> Vec<ChannelOut>,
    ) {
        let Some(outs) = self
            .channel_mgr_mut(team)
            .and_then(|mgr| mgr.get_channel(name).map(op))
        else {
            self.channel_not_found(id, name);
            return;
        };
        self.deliver_channel_outs(outs);
    }

    /// Channel opcodes (`ChannelHandler.cpp`): join,
    /// leave, list, and the moderation family.
    fn on_channel_packet(&mut self, id: SessionId, opcode: u16, packet: &WorldPacket) {
        let Some((actor, level, guild_id, team)) = self.channel_actor_for(id) else {
            return;
        };
        let mut reader = packet.reader();
        let channelname = reader.read_cstring();
        let second = reader.read_cstring();
        match opcode {
            opcode_id::CMSG_JOIN_CHANNEL => {
                self.on_join_channel(id, actor, guild_id, team, &channelname, &second);
            }
            opcode_id::CMSG_LEAVE_CHANNEL => {
                if channelname.is_empty() {
                    return;
                }
                self.on_leave_channel(id, actor, team, &channelname);
            }
            opcode_id::CMSG_CHANNEL_LIST => {
                let viewer_security = actor.security;
                let visible: std::collections::HashSet<u32> = self
                    .channel_mgr(team)
                    .map(|mgr| mgr.member_guids(&channelname))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|low| {
                        self.channel_actor_by_guid_low(*low).is_some_and(|member| {
                            viewer_security > security::PLAYER
                                || member.security <= self.config.gm_level_in_who_list
                        })
                    })
                    .collect();
                self.with_channel(id, team, &channelname, |channel| {
                    channel.list(actor, &|g| visible.contains(&g))
                });
            }
            opcode_id::CMSG_CHANNEL_PASSWORD => {
                self.with_channel(id, team, &channelname, |channel| {
                    channel.password(actor, &second)
                });
            }
            opcode_id::CMSG_CHANNEL_OWNER => {
                let owner_name = self.channel_owner_name(team, &channelname);
                let Some(outs) = self.channel_mgr(team).and_then(|mgr| {
                    mgr.channel(&channelname)
                        .map(|channel| channel.send_who_owner(actor, owner_name.as_deref()))
                }) else {
                    return self.channel_not_found(id, &channelname);
                };
                self.deliver_channel_outs(outs);
            }
            opcode_id::CMSG_CHANNEL_ANNOUNCEMENTS => {
                self.with_channel(id, team, &channelname, |channel| {
                    channel.toggle_announce(actor)
                });
            }
            opcode_id::CMSG_CHANNEL_MODERATE => {
                self.with_channel(id, team, &channelname, |channel| {
                    channel.toggle_moderate(actor)
                });
            }
            _ => self.on_channel_target_packet(
                id,
                opcode,
                (actor, level, team),
                &channelname,
                &second,
            ),
        }
    }

    /// Online name of a channel's owner guid
    /// (`GetPlayerNameByGUID`, online arm).
    fn channel_owner_name(
        &self,
        team: mangos_entities::unit::Team,
        channelname: &str,
    ) -> Option<String> {
        let owner_low = self.channel_mgr(team)?.channel(channelname)?.owner_guid()?;
        let map = self
            .player_map
            .get(&owner_low)
            .and_then(|mid| self.maps.get(mid))?;
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, owner_low);
        map.player(guid)
            .map(|player| player.unit.world.name.clone())
    }

    /// Targeted channel opcodes (owner/moderator/mute/
    /// invite/kick/ban/unban).
    fn on_channel_target_packet(
        &mut self,
        id: SessionId,
        opcode: u16,
        who: (ChannelActor, u32, mangos_entities::unit::Team),
        channelname: &str,
        raw_target: &str,
    ) {
        let (actor, level, team) = who;
        let Some(target_name) = normalize_player_name(raw_target) else {
            return;
        };
        if opcode == opcode_id::CMSG_CHANNEL_INVITE && level < self.config.channel_invite_min_level
        {
            return;
        }
        let target = self.channel_target_by_name(&target_name);
        let two_side = self.config.allow_two_side_interaction_channel;
        let target_ignores =
            target.is_some_and(|t| self.player_ignores(t.guid_low, actor.guid_low));
        let Some(mgr) = self.channel_mgr_mut(team) else {
            return;
        };
        let Some(channel) = mgr.get_channel(channelname) else {
            self.channel_not_found(id, channelname);
            return;
        };
        let outs = match opcode {
            opcode_id::CMSG_CHANNEL_SET_OWNER => {
                channel.set_owner_named(actor, target, &target_name, two_side)
            }
            opcode_id::CMSG_CHANNEL_MODERATOR => {
                channel.set_mode(actor, target, &target_name, true, true, two_side)
            }
            opcode_id::CMSG_CHANNEL_UNMODERATOR => {
                channel.set_mode(actor, target, &target_name, true, false, two_side)
            }
            opcode_id::CMSG_CHANNEL_MUTE => {
                channel.set_mode(actor, target, &target_name, false, true, two_side)
            }
            opcode_id::CMSG_CHANNEL_UNMUTE => {
                channel.set_mode(actor, target, &target_name, false, false, two_side)
            }
            opcode_id::CMSG_CHANNEL_INVITE => {
                channel.invite(actor, target, &target_name, two_side, target_ignores)
            }
            opcode_id::CMSG_CHANNEL_KICK => channel.kick_or_ban(actor, target, &target_name, false),
            opcode_id::CMSG_CHANNEL_BAN => channel.kick_or_ban(actor, target, &target_name, true),
            opcode_id::CMSG_CHANNEL_UNBAN => channel.unban(actor, target, &target_name),
            _ => return,
        };
        self.deliver_channel_outs(outs);
    }

    /// `HandleJoinChannelOpcode` (+ GM opposite-faction
    /// mirror join).
    fn on_join_channel(
        &mut self,
        id: SessionId,
        actor: ChannelActor,
        guild_id: u32,
        team: mangos_entities::unit::Team,
        channelname: &str,
        password: &str,
    ) {
        use mangos_entities::unit::Team;
        let first = channelname.as_bytes().first().copied();
        if first.is_none_or(|b| b <= 127 && !b.is_ascii_alphabetic()) {
            let mut writer = mangos_protocol::buffer::PacketWriter::new();
            writer.write_u8(notify::INVALID_NAME);
            writer.write_cstring(channelname);
            let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
            packet.take_writer(writer);
            self.reply_packet(id, packet);
            return;
        }
        let silently = self.config.channel_silently_gm_join;
        let stores = Arc::clone(&self.deps.world_stores);
        let mut outs = Vec::new();
        if let Some(mgr) = self.channel_mgr_mut(team) {
            outs = mgr
                .get_join_channel(channelname, &stores.chat_channels)
                .join(actor, guild_id, password, silently);
        }
        self.deliver_channel_outs(outs);
        if actor.security > security::PLAYER && self.config.gm_join_opposite_faction_channels {
            let other = match team {
                Team::Alliance => Team::Horde,
                Team::Horde => Team::Alliance,
                Team::None => Team::None,
            };
            if let Some(mgr) = self.channel_mgr_mut(other) {
                let channel = mgr.get_join_channel(channelname, &stores.chat_channels);
                if channel.security_level() == 0 {
                    let outs = channel.join(actor, guild_id, password, silently);
                    self.deliver_channel_outs(outs);
                }
            }
        }
    }

    /// `HandleLeaveChannelOpcode` (+ GM mirror leave).
    fn on_leave_channel(
        &mut self,
        id: SessionId,
        actor: ChannelActor,
        team: mangos_entities::unit::Team,
        channelname: &str,
    ) {
        use mangos_entities::unit::Team;
        let silently = self.config.channel_silently_gm_join;
        let leave_one = |world: &mut Self, which: Team| {
            let outs = world.channel_mgr_mut(which).and_then(|mgr| {
                mgr.get_channel(channelname)
                    .map(|channel| channel.leave(actor, true, silently))
            });
            let Some(outs) = outs else {
                world.channel_not_found(id, channelname);
                if let Some(mgr) = world.channel_mgr_mut(which) {
                    mgr.left_channel(channelname);
                }
                return;
            };
            world.deliver_channel_outs(outs);
            if let Some(mgr) = world.channel_mgr_mut(which) {
                mgr.left_channel(channelname);
            }
        };
        // (borrow-friendly two-step: closure over &mut self)
        leave_one(self, team);
        if actor.security > security::PLAYER && self.config.gm_join_opposite_faction_channels {
            let other = match team {
                Team::Alliance => Team::Horde,
                Team::Horde => Team::Alliance,
                Team::None => Team::None,
            };
            leave_one(self, other);
        }
    }

    /// `CHAT_MSG_CHANNEL` apply arm (`ChatHandler.cpp:328`):
    /// level restriction, then `Channel::Say`. The
    /// antispam/trial/cooldown family is unported (see
    /// `chat.rs`); account max level is approximated by
    /// the player level for the vanish-level arm.
    fn on_channel_say(
        &mut self,
        id: SessionId,
        chat_tag: u8,
        request: &crate::chat::ChatRequest,
        lang: u32,
    ) {
        let Some((actor, level, _guild_id, team)) = self.channel_actor_for(id) else {
            return;
        };
        let min_level = self.config.world_chan_min_level;
        let two_side = self.config.allow_two_side_interaction_channel;
        let Some(mgr) = self.channel_mgr_mut(team) else {
            return;
        };
        let Some(channel) = mgr.get_channel(&request.target) else {
            self.channel_not_found(id, &request.target);
            return;
        };
        if channel.is_level_restricted() && level < min_level {
            self.reply_packet(
                id,
                build_chat_packet(
                    chat_msg::SYSTEM,
                    "You cannot use this channel yet.",
                    language::UNIVERSAL,
                    0,
                    ObjectGuid::default(),
                ),
            );
            return;
        }
        let outs = channel.say(actor, &request.text, lang, chat_tag, 0, two_side);
        self.deliver_channel_outs(outs);
    }

    fn on_add_social(&mut self, id: SessionId, packet: &WorldPacket, ignore: bool) {
        let mut reader = packet.reader();
        let raw_name = reader.read_cstring();
        // C++: unnormalizable or unknown names return
        // silently — no FRIEND_NOT_FOUND packet is sent.
        let Some(name) = normalize_player_name(&raw_name) else {
            return;
        };
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let my_guid = guid.counter();
        let Some(my_race) = self
            .maps
            .get(&map_key)
            .and_then(|map| map.player(guid))
            .map(|player| player.unit.race())
        else {
            return;
        };
        let my_security = self
            .sessions
            .get(&id)
            .map_or(security::PLAYER, |entry| entry.auth.security);
        let data = match self.deps.social.player_data_by_name(&name) {
            Ok(Some(data)) => data,
            Ok(None) => return,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("player data lookup failed: {e}"));
                return;
            }
        };
        let friend_low = data.guid_low;
        let flag = if ignore {
            SOCIAL_FLAG_IGNORED
        } else {
            SOCIAL_FLAG_FRIEND
        };

        let already = self
            .maps
            .get(&map_key)
            .and_then(|map| map.player(guid))
            .is_some_and(|player| has_flag(&player.deferred.social, friend_low, flag));

        let is_enemy = !ignore
            && self.team_id_for_race(my_race) != self.team_id_for_race(data.race)
            && !self.config.allow_two_side_add_friend
            && my_security < security::MODERATOR;
        if let Some(result) = early_add_result(ignore, friend_low == my_guid, is_enemy, already) {
            self.reply_packet(id, build_friend_status(result, friend_low, None));
            return;
        }

        let online = if ignore {
            None
        } else {
            self.online_friend_info(friend_low)
        };
        let mut result = if ignore {
            FriendsResult::IgnoreAdded
        } else if online.is_some() {
            FriendsResult::AddedOnline
        } else {
            FriendsResult::AddedOffline
        };
        let write = self
            .maps
            .get_mut(&map_key)
            .and_then(|map| map.player_mut(guid))
            .and_then(|player| add_to_social_list(&mut player.deferred.social, friend_low, ignore));
        match write {
            Some(write) => self.apply_social_write(my_guid, friend_low, write),
            None => {
                result = if ignore {
                    FriendsResult::IgnoreFull
                } else {
                    FriendsResult::ListFull
                };
            }
        }
        let online = if result == FriendsResult::AddedOnline {
            online
        } else {
            None
        };
        self.reply_packet(id, build_friend_status(result, friend_low, online));
    }

    fn on_del_social(&mut self, id: SessionId, packet: &WorldPacket, ignore: bool) {
        let mut reader = packet.reader();
        let Ok(raw_guid) = reader.read_u64_le() else {
            self.bad_packet(id, packet.opcode());
            return;
        };
        let friend_low = ObjectGuid::from_raw(raw_guid).counter();
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let my_guid = guid.counter();
        let write = self
            .maps
            .get_mut(&map_key)
            .and_then(|map| map.player_mut(guid))
            .and_then(|player| {
                remove_from_social_list(&mut player.deferred.social, friend_low, ignore)
            });
        if let Some(write) = write {
            self.apply_social_write(my_guid, friend_low, write);
        }
        let result = if ignore {
            FriendsResult::IgnoreRemoved
        } else {
            FriendsResult::Removed
        };
        self.reply_packet(id, build_friend_status(result, friend_low, None));
    }

    /// Applies one social DB write; a failure only logs
    /// (C++ `PExecute` is queued and unobservable).
    fn apply_social_write(&mut self, guid_low: u32, friend_low: u32, write: SocialWrite) {
        if let Err(e) = self
            .deps
            .social
            .apply_social_write(guid_low, friend_low, write)
        {
            self.log
                .out(Level::Error, &format!("character_social write failed: {e}"));
        }
    }

    /// `Player::TeamForRace` via the `ChrRaces` `TeamID` the
    /// materializer also uses (7 Alliance, 1 Horde, else 0).
    fn team_id_for_race(&self, race: u8) -> u32 {
        self.deps
            .race_class_dbc
            .races
            .get(&u32::from(race))
            .map_or(0, |entry| entry.team_id)
    }

    /// `SocialMgr::GetFriendInfo` online arm, minus the
    /// visibility/GM gates that have no Rust session state
    /// yet: online means a live session with a player in a
    /// map; AFK/DND are not modeled.
    fn online_friend_info(&self, friend_low: u32) -> Option<FriendOnlineInfo> {
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, friend_low);
        let map = self.maps.get(self.player_map.get(&friend_low)?)?;
        let player = map.player(guid)?;
        Some(FriendOnlineInfo {
            status: FRIEND_STATUS_ONLINE,
            area: map.player_zone_id(guid),
            level: player.unit.level(),
            class: u32::from(player.unit.class()),
        })
    }

    fn on_creature_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = reader
            .read_u32_le()
            .and_then(|entry| reader.read_u64_le().map(|guid| (entry, guid)));
        let Ok((entry, raw_guid)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_CREATURE_QUERY);
            return;
        };
        let info = self.deps.world_stores.creature_templates.get(entry);
        if info.is_none() {
            self.log.out(
                Level::Debug,
                &format!(
                    "WORLD: CMSG_CREATURE_QUERY - Guid: {} Entry: {entry} NO CREATURE INFO!",
                    ObjectGuid::from_wire_raw(raw_guid)
                ),
            );
        }
        self.reply_packet(id, build_creature_query_response(entry, info));
    }

    fn on_gameobject_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = reader
            .read_u32_le()
            .and_then(|entry| reader.read_u64_le().map(|guid| (entry, guid)));
        let Ok((entry, raw_guid)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_GAMEOBJECT_QUERY);
            return;
        };
        let info = self.deps.world_stores.gameobject_templates.get(entry);
        if info.is_none() {
            self.log.out(
                Level::Debug,
                &format!(
                    "WORLD: CMSG_GAMEOBJECT_QUERY - Guid: {} Entry: {entry} Missing gameobject info!",
                    ObjectGuid::from_wire_raw(raw_guid)
                ),
            );
        }
        self.reply_packet(id, build_gameobject_query_response(entry, info));
    }

    fn on_item_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let parsed = reader
            .read_u32_le()
            .and_then(|entry| reader.read_u64_le().map(|guid| (entry, guid)));
        let Ok((entry, _raw_guid)) = parsed else {
            self.bad_packet(id, opcode_id::CMSG_ITEM_QUERY_SINGLE);
            return;
        };
        let allow_datamined = self
            .sessions
            .get(&id)
            .is_some_and(|entry| entry.auth.security > security::PLAYER);
        let proto = self.deps.world_stores.item_prototypes.get(entry);
        self.reply_packet(
            id,
            build_item_query_response(
                entry,
                proto,
                allow_datamined,
                &self.deps.world_stores.spells.spells,
            ),
        );
    }

    fn live_name_query_row(&self, guid: ObjectGuid) -> Option<NameQueryRow> {
        if !guid.is_player() {
            return None;
        }
        let &map_id = self.player_map.get(&guid.counter())?;
        let player = self.maps.get(&map_id)?.player(guid)?;
        Some(NameQueryRow {
            name: player.unit.world.name.clone(),
            race: player.unit.race(),
            gender: player.unit.gender(),
            class: player.unit.class(),
        })
    }

    fn on_name_query(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw_guid) = reader.read_u64_le() else {
            self.bad_packet(id, opcode_id::CMSG_NAME_QUERY);
            return;
        };
        let guid = ObjectGuid::from_wire_raw(raw_guid);
        let guid_low = guid.counter();
        if let Some(row) = self.live_name_query_row(guid) {
            self.reply_packet(id, build_name_query_response(guid_low, &row));
            return;
        }

        let Some(tx) = self.self_tx.clone() else {
            return;
        };
        let queries = Arc::clone(&self.deps.char_queries);
        tokio::task::spawn_blocking(move || {
            let row = queries.name_query(guid_low);
            let _ = tx.blocking_send(WorldMessage::NameQueryReady { id, guid_low, row });
        });
    }

    fn on_name_query_ready(
        &mut self,
        id: SessionId,
        guid_low: u32,
        row: Result<Option<NameQueryRow>, DbError>,
    ) {
        match row {
            Ok(Some(row)) => {
                self.reply_packet(id, build_name_query_response(guid_low, &row));
            }
            Ok(None) => {}
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("name query failed: {e}"));
            }
        }
    }

    fn disconnect(&mut self, id: SessionId) {
        if let Some(entry) = self.sessions.get(&id) {
            let _ = entry.control.try_send(SessionControl::Disconnect);
        }
        self.remove(id);
    }

    /// `HandlePlayerLoginOpcode` (`CharacterHandler.cpp:386-420`)
    /// up to the async query dispatch.
    fn on_player_login_opcode(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(raw_guid) = reader.read_u64_le() else {
            self.bad_packet(id, opcode::CMSG_PLAYER_LOGIN);
            return;
        };
        let guid = ObjectGuid::from_raw(raw_guid);
        let refused = {
            let Some(entry) = self.sessions.get_mut(&id) else {
                return;
            };
            if (!self.config.world_available && entry.auth.security == security::PLAYER)
                || entry.player_loading
                || entry.player_guid.is_some()
                || !guid.is_player()
            {
                true
            } else {
                entry.player_loading = true;
                false
            }
        };
        if refused {
            let alive = self
                .sessions
                .get(&id)
                .is_some_and(|e| send_packet(e, build_character_login_failed()));
            if !alive {
                self.disconnect(id);
            }
            return;
        }
        self.begin_player_login(id, guid.counter());
    }

    /// `HandlePlayerLogin` (`CharacterHandler.cpp:422-577`),
    /// Phase 3 cut: gates, per-character account data, then the
    /// map-independent packet train. Everything from
    /// `SendInitialPacketsBeforeAddToMap` onward (including the
    /// online-presence writes) waits for the simulated world.
    #[allow(clippy::too_many_lines)]
    fn on_player_login_ready(
        &mut self,
        id: SessionId,
        guid_low: u32,
        load: Result<Option<Box<CharacterLoad>>, DbError>,
    ) {
        let load = match load {
            Ok(load) => load,
            Err(e) => {
                self.log
                    .out(Level::Error, &format!("character load failed: {e}"));
                None
            }
        };
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        if !entry.player_loading {
            return; // stale callback (C++ asserts this cannot happen)
        }
        entry.player_loading = false;

        // Bot sessions carry synthetic account ids
        // (fork `GenBotAccountId`); their login holder
        // is keyed to the character's real account, so
        // the ownership gate compares against the row.
        let gate_account = if self.bot_sessions.contains_key(&id) {
            load.as_deref().map_or(entry.account_id, |l| l.row.account)
        } else {
            entry.account_id
        };
        match evaluate_load(load.as_deref(), gate_account) {
            LoginGate::Proceed => {}
            gate => {
                self.log.out(
                    Level::Error,
                    &format!(
                        "SESSION: player login refused for account {} guid {guid_low}: {gate:?}",
                        entry.account_id
                    ),
                );
                self.disconnect(id);
                return;
            }
        }
        let load = *load.expect("gate proceeded");

        entry
            .account_data
            .load(&load.account_data, PER_CHARACTER_CACHE_MASK);
        entry.player_guid = Some(guid_low);
        entry.core.set_player_state(true, true);

        // Presence write (Phase 4 write register,
        // owner-approved 2026-10-19): C++ SetOnline runs
        // before the packet train.
        if let Err(e) = self.deps.presence.set_character_online(guid_low, true) {
            self.log
                .out(Level::Error, &format!("presence online write failed: {e}"));
        }

        // Materialize the aggregate and enter the map
        // (AddToMap + SendInitSelf subset). The Phase 3
        // packet order is kept: verify-world uses the
        // materialized position (homebind fallback may
        // have moved the character).
        let account_data_packet = entry.account_data.build_md5_packet();
        let tutorials_packet = build_tutorial_flags(&entry.tutorials);
        let social_packets = [
            build_friend_list(&load.social),
            build_ignore_list(&load.social),
        ];
        let load_group_id = load.group_id;
        let loaded_zone = load.row.zone;
        let load_guild_id = load.guild.as_ref().map(|guild| guild.guild_id);
        let mut player = match self.materialize_player(load) {
            Ok(player) => player,
            Err(e) => {
                self.log.out(
                    Level::Error,
                    &format!("SESSION: materialize failed for guid {guid_low}: {e:?}"),
                );
                let _ = self.deps.presence.set_character_online(guid_low, false);
                self.disconnect(id);
                return;
            }
        };
        if player.homebind.synthesized && self.character_saving_allowed(id) {
            match self
                .deps
                .character_saver
                .insert_homebind(guid_low, homebind_row(&player))
            {
                Ok(()) => player.homebind.synthesized = false,
                Err(e) => self.log.out(
                    Level::Error,
                    &format!("synthesized homebind insert failed for guid {guid_low}: {e}"),
                ),
            }
        }
        let verify_position = player.unit.world.position();
        let map_id = player.unit.world.map_id;
        let bindpoint_packet =
            build_bindpoint_update(player.homebind.location, player.homebind.area_id);
        let initial_cooldowns = initial_spell_cooldowns(
            &player.deferred.spell_cooldowns,
            unix_now(),
            |spell| self.deps.world_stores.spells.spells.contains_key(&spell),
            |item| self.deps.world_stores.items.get(item).is_some(),
        );
        let default_spells = self.deps.world_stores.player_infos.default_spells(
            u32::from(player.unit.race()),
            u32::from(player.unit.class()),
        );
        let initial_spells_packet = build_initial_spells(
            &player.deferred.spells,
            default_spells,
            &initial_cooldowns,
            |spell| self.deps.world_stores.spells.spells.contains_key(&spell),
        );
        let action_buttons_packet = build_action_buttons(
            &player.deferred.actions,
            |spell| self.deps.world_stores.spells.spells.contains_key(&spell),
            |item| self.deps.world_stores.items.get(item).is_some(),
        );
        let factions_packet =
            build_initialize_factions(&self.deps.world_stores.factions.initial_slots(
                player.unit.race(),
                player.unit.class(),
                &player.deferred.reputation,
            ));
        self.enter_map(id, player);
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.cached_zone = loaded_zone;
        }

        let mut packets = vec![
            build_login_verify_world_at(map_id, verify_position),
            account_data_packet,
        ];
        packets.extend(social_packets);
        for line in motd_lines(&self.config.motd, self.config.wow_patch) {
            packets.push(build_sys_message(&line));
        }
        // SendInitialPacketsBeforeAddToMap, in C++ order:
        // rest start, bindpoint, tutorial flags (the client's
        // tutorial bitsets are null without them), initial
        // spells, action buttons (sent twice by the C++
        // login path), and game-time speed (the local mover
        // never advances without it).
        packets.push(build_set_rest_start());
        packets.push(bindpoint_packet);
        packets.push(tutorials_packet);
        packets.push(initial_spells_packet);
        packets.push(action_buttons_packet.clone());
        packets.push(action_buttons_packet);
        packets.push(factions_packet);
        packets.push(build_login_set_time_speed(time_bit_fields_now()));

        let entry = self.sessions.get(&id).expect("present");
        for packet in packets {
            if !send_packet(entry, packet) {
                self.disconnect(id);
                return;
            }
        }
        // Map::add_player queued init-self before visibility creates.
        self.drain_map_events();
        // `Player::_LoadGroup` + `UpdatePlayerOnlineStatus`
        // at login completion: reattach the persisted
        // group (boot-loaded; the spawn index already
        // points at it, this validates against the
        // `group_member` row the load carried and updates
        // everyone), and normalize the leader flag (C++
        // `UpdateGroupLeaderFlag` runs unconditionally).
        if let Some(db_id) = load_group_id
            && let Some(key) = self.group_by_db_id.get(&db_id).copied()
            && self
                .groups
                .get(&key)
                .is_some_and(|group| group.is_member(guid_low))
        {
            self.player_group.insert(guid_low, key);
            self.group_send_update(key);
        }
        self.refresh_leader_flag(guid_low);
        // Guild login (`Player::_LoadGuild`, then the
        // `CharacterHandler.cpp:577-591` MOTD + signed-on
        // broadcasts): attach the boot-loaded guild or
        // clear the materialized fields when it is gone.
        if let Some(guild_id) = load_guild_id {
            let member = self.guilds.get(&guild_id).and_then(|guild| {
                guild
                    .rank_of(guid_low)
                    .map(|rank| (rank, guild.motd().to_owned()))
            });
            match member {
                Some((rank, motd)) => {
                    self.player_guild.insert(guid_low, guild_id);
                    self.apply_guild_fx(GuildFx {
                        fields: vec![crate::guild::GuildFieldUpdate {
                            guid_low,
                            guild_id,
                            rank_id: rank,
                        }],
                        ..GuildFx::default()
                    });
                    self.reply_packet(id, guild_event_packet(guild_event::MOTD, None, &[&motd]));
                    if let Some((player_name, _)) = self.player_name_race_of(guid_low)
                        && let Some(guild) = self.guilds.get(&guild_id)
                    {
                        let fx = guild.broadcast_event(
                            guild_event::SIGNED_ON,
                            Some(guid_low),
                            &[&player_name],
                        );
                        self.apply_guild_fx(fx);
                    }
                }
                None => {
                    self.apply_guild_fx(GuildFx {
                        fields: vec![crate::guild::GuildFieldUpdate {
                            guid_low,
                            guild_id: 0,
                            rank_id: 0,
                        }],
                        ..GuildFx::default()
                    });
                }
            }
        }
        // Mail tracking seed (login-time
        // `UpdateNextMailTimeAndUnreads`).
        let tracker = self
            .player_map
            .get(&guid_low)
            .and_then(|key| self.maps.get(key))
            .and_then(|map| map.player(player_guid(guid_low)))
            .map_or_else(MailTracker::default, |player| {
                MailTracker::recompute(&player.deferred.mails, unix_now_i64())
            });
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.mail_tracker = tracker;
        }
        self.log.out(
            Level::Debug,
            &format!("SESSION: player guid {guid_low} entered map {map_id}"),
        );
    }

    /// `Player::LoadFromDB` aggregate construction with the
    /// boot-resolved DBC inputs.
    #[allow(clippy::approx_constant)] // fork Naxxramas exit orientation literal
    fn materialize_player(&self, mut load: CharacterLoad) -> Result<Player, MaterializeError> {
        // Fork `_LoadFromDB` instance branch: an unbound
        // in-dungeon save relocates to the go-back
        // trigger destination (homebind below only when
        // no go-back trigger exists).
        if self.deps.world_stores.maps.is_instance_map(load.row.map) {
            // `GetBoundInstanceSaveForSelfOrGroup`: the
            // saved position survives only when a personal
            // or group bind still names the saved instance.
            let personal_matches = load
                .bound_instances
                .iter()
                .any(|bind| bind.id == Some(load.row.instance) && bind.map == Some(load.row.map));
            let group_matches = self
                .group_leader_of(load.row.guid)
                .and_then(|leader| self.instances.group_bind(leader, load.row.map))
                .is_some_and(|(instance_id, _)| instance_id == load.row.instance);
            if !personal_matches && !group_matches {
                let go_back = self
                    .deps
                    .world_stores
                    .maps
                    .get(load.row.map)
                    .and_then(|entry| {
                        self.deps
                            .world_stores
                            .area_triggers
                            .go_back_trigger(load.row.map, entry.ghost_entrance_map)
                    })
                    .map(|(_, tele)| {
                        (
                            tele.dest_map_id,
                            tele.dest_x,
                            tele.dest_y,
                            tele.dest_z,
                            tele.dest_o,
                        )
                    })
                    // Fork Naxxramas special case: no exit
                    // trigger; hardcoded Eastern Kingdoms exit.
                    .or_else(|| {
                        (load.row.map == 533).then_some((0, 3362.15, -3379.35, 144.782, 6.28319))
                    });
                if let Some((map, x, y, z, o)) = go_back {
                    load.row.map = map;
                    load.row.position_x = x;
                    load.row.position_y = y;
                    load.row.position_z = z;
                    load.row.orientation = o;
                    load.row.instance = 0;
                }
            }
        }
        let race = self
            .deps
            .race_class_dbc
            .races
            .get(&u32::from(load.row.race));
        let power_type = self
            .deps
            .race_class_dbc
            .class_power
            .get(&u32::from(load.row.class))
            .copied()
            .unwrap_or(0) as u8;
        let gender = load.row.gender & 0x01;
        let display_id = self
            .deps
            .world_stores
            .player_infos
            .create_info(u32::from(load.row.race), u32::from(load.row.class))
            .map_or(0, |c| {
                if gender == 1 {
                    c.display_id_f
                } else {
                    c.display_id_m
                }
            });
        let inputs = MaterializeInputs {
            infos: &self.deps.world_stores.player_infos,
            race,
            power_type,
            display_scale: self.deps.world_stores.scale_for_display(display_id),
            model: self.deps.world_stores.model_for_display(display_id),
            saved_map_known: self.deps.world_stores.maps.known(load.row.map),
            // Fork login keeps the saved in-instance
            // position only when a bound instance still
            // matches the saved instance id; the go-back
            // rewrite above already relocated unbound
            // saves, so anything still flagged here (no
            // go-back trigger) falls back to homebind in
            // the materialize position gate.
            saved_map_is_instance: self.deps.world_stores.maps.is_instance_map(load.row.map)
                && !load.bound_instances.iter().any(|bind| {
                    bind.id == Some(load.row.instance) && bind.map == Some(load.row.map)
                }),
            max_money: MAX_MONEY_AMOUNT,
            now_unix: unix_now(),
            skills: Some(&self.deps.world_stores.skills),
            item_prototypes: Some(&self.deps.world_stores.item_prototypes),
            quest_templates: Some(&self.deps.world_stores.quests),
        };
        Player::materialize(load, &inputs)
    }

    /// Group leader guid low for a player (boot groups
    /// are keyed before `player_group` is seeded at
    /// login, so fall back to a membership scan).
    fn group_leader_of(&self, guid_low: u32) -> Option<u32> {
        if let Some(key) = self.player_group.get(&guid_low)
            && let Some(group) = self.groups.get(key)
        {
            return Some(group.leader_low());
        }
        self.groups
            .values()
            .find(|group| group.is_member(guid_low))
            .map(crate::group::Group::leader_low)
    }

    /// Map-entry instance resolution (`CreateMap` +
    /// `InstanceMap::Add` binding subset): bound
    /// instance for self-or-group, else a fresh
    /// allocation with the fork entry binds.
    /// Returns the dest key plus whether this entry
    /// granted a fresh permanent personal bind
    /// (`SMSG_INSTANCE_SAVE_CREATED`).
    fn resolve_map_key(&mut self, guid_low: u32, map_id: u32, player: &Player) -> (MapKey, bool) {
        if !self.deps.world_stores.maps.is_instance_map(map_id) {
            return (MapKey::shared(map_id), false);
        }
        self.seed_char_binds(guid_low, &player.deferred.bound_instances);
        self.resolve_dest_key(guid_low, map_id)
    }

    /// Fresh instance allocation + entry binds (the
    /// no-bound-state arm of `resolve_map_key`).
    fn allocate_instance_for(&mut self, guid_low: u32, map_id: u32, leader: Option<u32>) -> MapKey {
        let instance_id = self.instances.allocate(map_id);
        let is_raid = self
            .deps
            .world_stores
            .maps
            .get(map_id)
            .is_some_and(mangos_world_data::map_template::MapEntry::is_raid);
        let reset_time = if is_raid {
            0
        } else {
            unix_now_i64().saturating_add(2 * 3600)
        };
        if let Err(e) = self
            .deps
            .instance
            .insert_instance(instance_id, map_id, reset_time)
        {
            self.log
                .out(Level::Error, &format!("instance insert failed: {e}"));
        }
        if let Some(leader) = leader {
            self.instances
                .set_group_bind(leader, map_id, (instance_id, false));
            if let Err(e) = self
                .deps
                .instance
                .insert_group_bind(leader, instance_id, false)
            {
                self.log
                    .out(Level::Error, &format!("group bind insert failed: {e}"));
            }
            // Other members' non-permanent personal
            // binds for this map are dropped.
            let members: Vec<u32> = self
                .groups
                .values()
                .find(|group| group.leader_low() == leader)
                .map_or_else(Vec::new, |group| {
                    group.members().iter().map(|slot| slot.guid_low).collect()
                });
            for member in members {
                if member != guid_low
                    && let Some((old_id, false)) = self.instances.char_bind(member, map_id)
                {
                    self.instances.remove_char_bind(member, map_id);
                    if let Err(e) = self.deps.instance.delete_character_bind(member, old_id) {
                        self.log
                            .out(Level::Error, &format!("instance unbind delete failed: {e}"));
                    }
                }
            }
        } else {
            self.instances
                .set_char_bind(guid_low, map_id, (instance_id, false));
            if let Err(e) = self
                .deps
                .instance
                .insert_character_bind(guid_low, instance_id, false)
            {
                self.log
                    .out(Level::Error, &format!("instance bind insert failed: {e}"));
            }
        }
        MapKey {
            map_id,
            instance_id,
        }
    }

    /// `Player::SendSavedInstances` (permanent binds only).
    fn send_saved_instances(&mut self, id: SessionId, guid_low: u32) {
        use crate::player_login::{build_update_instance_ownership, build_update_last_instance};
        let permanent_maps: Vec<u32> = self
            .instances
            .char_binds
            .get(&guid_low)
            .map(|binds| {
                binds
                    .iter()
                    .filter(|(_, (_, perm))| *perm)
                    .map(|(map_id, _)| *map_id)
                    .collect()
            })
            .unwrap_or_default();
        self.reply_packet(
            id,
            build_update_instance_ownership(!permanent_maps.is_empty()),
        );
        for map_id in permanent_maps {
            self.reply_packet(id, build_update_last_instance(map_id));
        }
    }

    /// `Player::TeleportTo` subset (area-trigger driven):
    /// same-instance teleports relocate in place;
    /// cross-instance teleports run the far transfer
    /// (`SMSG_TRANSFER_PENDING`/`SMSG_NEW_WORLD`, completed by the
    /// worldport ACK; bot sessions finish inline).
    /// Death/transport/duel side effects of the fork
    /// entry checks land with their systems.
    fn teleport_player(
        &mut self,
        id: SessionId,
        dest_map_id: u32,
        dest: mangos_movement::movement_info::Position,
    ) {
        let Some(guid_low) = self.sessions.get(&id).and_then(|entry| entry.player_guid) else {
            return;
        };
        if self.pending_teleports.contains_key(&id) {
            return;
        }
        let Some(&current_key) = self.player_map.get(&guid_low) else {
            return;
        };
        let guid = player_guid(guid_low);
        // Peek the player for bind seeding, then resolve.
        let (dest_key, granted) = {
            let Some(player_ref) = self
                .maps
                .get(&current_key)
                .and_then(|map| map.player(guid))
                .map(|p| {
                    // resolve_map_key only reads deferred
                    // binds; pass through a temporary by
                    // resolving with the live reference is
                    // impossible while &mut self is held
                    // elsewhere, so snapshot the rows.
                    p.deferred.bound_instances.clone()
                })
            else {
                return;
            };
            self.seed_char_binds(guid_low, &player_ref);
            self.resolve_dest_key(guid_low, dest_map_id)
        };
        if dest_key == current_key {
            if let Some(map) = self.maps.get_mut(&current_key) {
                map.teleport_player_in_map(guid, dest);
            }
            self.drain_map_events();
            return;
        }
        // Far transfer.
        let Some(mut player) = self
            .maps
            .get_mut(&current_key)
            .and_then(|map| map.remove_player(guid))
        else {
            return;
        };
        self.drain_map_events();
        self.player_map.remove(&guid_low);
        player.unit.world.map_id = dest_map_id;
        player.unit.world.relocate(dest);
        player.unit.world.movement_info.pos = dest;
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.core.set_player_state(true, false);
        }
        // Fork order: TRANSFER_PENDING (TeleportTo),
        // NEW_WORLD then SendSavedInstances (SendNewWorld).
        self.reply_packet(id, crate::player_login::build_transfer_pending(dest_map_id));
        self.reply_packet(id, crate::player_login::build_new_world(dest_map_id, dest));
        self.send_saved_instances(id, guid_low);
        self.pending_teleports.insert(
            id,
            PendingTeleport {
                player,
                source_key: current_key,
                key: dest_key,
                granted,
            },
        );
        if self.bot_sessions.contains_key(&id) {
            self.finish_teleport(id);
        }
    }

    /// Seeds runtime binds from a character-load
    /// `bound_instances` snapshot.
    fn seed_char_binds(
        &mut self,
        guid_low: u32,
        rows: &[mangos_db::character_load::BoundInstanceRow],
    ) {
        for bind in rows {
            if let (Some(instance_id), Some(bind_map)) = (bind.id, bind.map)
                && self.instances.char_bind(guid_low, bind_map).is_none()
            {
                self.instances.set_char_bind(
                    guid_low,
                    bind_map,
                    (instance_id, bind.permanent != 0),
                );
            }
        }
    }

    /// Bound-or-fresh dest key (the `resolve_map_key`
    /// tail without the load-time seeding).
    fn resolve_dest_key(&mut self, guid_low: u32, map_id: u32) -> (MapKey, bool) {
        if !self.deps.world_stores.maps.is_instance_map(map_id) {
            return (MapKey::shared(map_id), false);
        }
        let leader = self.group_leader_of(guid_low);
        let personal = self.instances.char_bind(guid_low, map_id);
        let grouped = leader.and_then(|l| self.instances.group_bind(l, map_id));
        if let Some((instance_id, _)) = personal.or(grouped) {
            let mut granted = false;
            if personal.is_none()
                && let Some((_, true)) = grouped
            {
                granted = true;
                self.instances
                    .set_char_bind(guid_low, map_id, (instance_id, true));
                if let Err(e) =
                    self.deps
                        .instance
                        .insert_character_bind(guid_low, instance_id, true)
                {
                    self.log
                        .out(Level::Error, &format!("instance bind insert failed: {e}"));
                }
            }
            return (
                MapKey {
                    map_id,
                    instance_id,
                },
                granted,
            );
        }
        (self.allocate_instance_for(guid_low, map_id, leader), false)
    }

    /// `HandleMoveWorldportAckOpcode` completion.
    fn finish_teleport(&mut self, id: SessionId) {
        let Some(pending) = self.pending_teleports.remove(&id) else {
            return;
        };
        self.add_player_to_map(id, pending.player, pending.key, false);
        if pending.granted {
            self.reply_packet(id, crate::player_login::build_instance_save_created());
        }
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.core.set_player_state(true, true);
        }
        self.drain_map_events();
    }

    fn enter_map(&mut self, session: SessionId, player: Player) {
        let map_id = player.unit.world.map_id;
        let guid_low = player.unit.guid().counter();
        let (map_key, granted) = self.resolve_map_key(guid_low, map_id, &player);
        self.add_player_to_map(session, player, map_key, true);
        if granted {
            self.reply_packet(session, crate::player_login::build_instance_save_created());
        }
        // Fork sends saved instances only from
        // `SendNewWorld` (far teleport), never at login.
    }

    /// `PlayerBot.AllowSaving` gate. Non-bot sessions
    /// always save; bot sessions save only when the
    /// fork config explicitly allows it.
    fn character_saving_allowed(&self, session_id: SessionId) -> bool {
        !self.bot_sessions.contains_key(&session_id) || self.config.player_bot_allow_saving
    }

    fn initial_player_save_instant(&self, guid_low: u32) -> Option<Instant> {
        let interval = self.config.player_save_interval_ms;
        if interval == 0 {
            return None;
        }
        // Fork: urand(interval / 2, interval * 3 / 2).
        // A splitmix64 stagger keeps the actor free of
        // an RNG dependency while preserving the range.
        let mut state = u64::from(guid_low) ^ unix_now().wrapping_mul(0x9E37_79B9_7F4A_7C15);
        state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        state ^= state >> 31;
        let delay = u64::from(interval / 2) + state % (u64::from(interval) + 1);
        Some(Instant::now() + Duration::from_millis(delay))
    }

    fn should_save_stats(&self, level: u32, logout: bool) -> bool {
        let min_level = self.config.player_save_stats_min_level;
        min_level > 0
            && level >= min_level
            && (logout || !self.config.player_save_stats_only_on_logout)
    }

    /// Fork played-time tick: both counters gain whole
    /// in-world seconds; an observed level change first
    /// resets the level segment (`GiveLevel`).
    fn advance_played_time(&mut self, guid_low: u32) {
        let Some(session_id) = self.player_session.get(&guid_low).copied() else {
            return;
        };
        let now = Instant::now();
        let (elapsed, observed_level) = {
            let Some(entry) = self.sessions.get(&session_id) else {
                return;
            };
            (
                now.saturating_duration_since(entry.played_last_tick)
                    .as_secs(),
                entry.played_level,
            )
        };
        if elapsed == 0 {
            return;
        }
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            if let Some(entry) = self.sessions.get_mut(&session_id) {
                entry.played_last_tick = now;
            }
            return;
        };
        let guid = player_guid(guid_low);
        if let Some(player) = self
            .maps
            .get_mut(&map_key)
            .and_then(|map| map.player_mut(guid))
        {
            if player.unit.level() != observed_level {
                player.played_time_level = 0;
            }
            #[allow(clippy::cast_possible_truncation)]
            let seconds = elapsed as u32;
            player.played_time_total = player.played_time_total.saturating_add(seconds);
            player.played_time_level = player.played_time_level.saturating_add(seconds);
            let level = player.unit.level();
            if let Some(entry) = self.sessions.get_mut(&session_id) {
                entry.played_level = level;
                entry.played_last_tick += Duration::from_secs(elapsed);
            }
        }
    }

    fn advance_pending_played_time(&mut self, session_id: SessionId) {
        let now = Instant::now();
        let (elapsed, observed_level) = {
            let Some(entry) = self.sessions.get(&session_id) else {
                return;
            };
            (
                now.saturating_duration_since(entry.played_last_tick)
                    .as_secs(),
                entry.played_level,
            )
        };
        if elapsed == 0 {
            return;
        }
        if let Some(pending) = self.pending_teleports.get_mut(&session_id) {
            if pending.player.unit.level() != observed_level {
                pending.player.played_time_level = 0;
            }
            #[allow(clippy::cast_possible_truncation)]
            let seconds = elapsed as u32;
            pending.player.played_time_total =
                pending.player.played_time_total.saturating_add(seconds);
            pending.player.played_time_level =
                pending.player.played_time_level.saturating_add(seconds);
            let level = pending.player.unit.level();
            if let Some(entry) = self.sessions.get_mut(&session_id) {
                entry.played_level = level;
                entry.played_last_tick += Duration::from_secs(elapsed);
            }
        }
    }

    /// `Player::SaveToDB` for an in-map player.
    /// Returns true when the saver accepted the bundle.
    fn save_player(&mut self, guid_low: u32, online: bool, logout: bool) -> bool {
        let Some(session_id) = self.player_session.get(&guid_low).copied() else {
            return false;
        };
        if !self.character_saving_allowed(session_id) {
            return false;
        }
        if self.pending_teleports.contains_key(&session_id) {
            return logout && self.save_pending_player(session_id, logout);
        }
        if let Some(entry) = self.sessions.get_mut(&session_id)
            && self.config.player_save_interval_ms > 0
        {
            entry.next_player_save = Some(
                Instant::now()
                    + Duration::from_millis(u64::from(self.config.player_save_interval_ms)),
            );
        }
        self.advance_played_time(guid_low);
        let Some(map_key) = self.player_map.get(&guid_low).copied() else {
            return false;
        };
        let guid = player_guid(guid_low);
        let now_unix = unix_now();
        let can_speak = self
            .sessions
            .get(&session_id)
            .is_none_or(|entry| entry.auth.mutetime <= now_unix);
        let Some((auras, cooldowns, zone, level)) = self.maps.get(&map_key).map(|map| {
            (
                map.player_savable_auras(guid),
                map.player_cooldown_rows(guid, unix_now_i64()),
                map.player_zone_id(guid),
                map.player(guid).map_or(0, |player| player.unit.level()),
            )
        }) else {
            return false;
        };
        if let Some(entry) = self.sessions.get_mut(&session_id) {
            entry.cached_zone = zone;
        }
        let ctx = SaveContext {
            map_id: map_key.map_id,
            instance_id: map_key.instance_id,
            zone,
            online,
            logout_time: now_unix,
            save_stats: self.should_save_stats(level, logout),
            can_speak,
        };
        let Some((bundle, stats, action_writes, projected_quests)) = self
            .maps
            .get(&map_key)
            .and_then(|map| map.player(guid))
            .map(|player| {
                let action_writes = logout.then(|| build_action_writes(player));
                let (bundle, stats, projected_quests) =
                    build_save_bundle(player, auras, cooldowns, &ctx);
                (bundle, stats, action_writes, projected_quests)
            })
        else {
            return false;
        };
        if let Err(e) = self.deps.character_saver.save_character(bundle, stats) {
            self.log.out(
                Level::Error,
                &format!("character save failed for guid {guid_low}: {e}"),
            );
            return false;
        }
        let actions_saved = action_writes.is_none_or(|(inserts, updates, deletes)| {
            match self
                .deps
                .character_saver
                .save_actions(guid_low, inserts, updates, deletes)
            {
                Ok(()) => true,
                Err(e) => {
                    self.log.out(
                        Level::Error,
                        &format!("character action save failed for guid {guid_low}: {e}"),
                    );
                    false
                }
            }
        });
        if let Some(player) = self
            .maps
            .get_mut(&map_key)
            .and_then(|map| map.player_mut(guid))
        {
            rebaseline(player, now_unix, projected_quests);
            if actions_saved {
                rebaseline_actions(player);
            }
        }
        true
    }

    /// Forced `SaveToDB` while far teleport is pending:
    /// destination map/position with instance 0, per fork.
    fn save_pending_player(&mut self, session_id: SessionId, logout: bool) -> bool {
        if !self.character_saving_allowed(session_id) {
            return false;
        }
        self.advance_pending_played_time(session_id);
        let Some((guid_low, dest_key, source_key, level)) =
            self.pending_teleports.get(&session_id).map(|pending| {
                (
                    pending.player.unit.guid().counter(),
                    pending.key,
                    pending.source_key,
                    pending.player.unit.level(),
                )
            })
        else {
            return false;
        };
        let now_unix = unix_now();
        let can_speak = self
            .sessions
            .get(&session_id)
            .is_none_or(|entry| entry.auth.mutetime <= now_unix);
        let zone = self
            .sessions
            .get(&session_id)
            .map_or(0, |entry| entry.cached_zone);
        let guid = player_guid(guid_low);
        let (auras, cooldowns) = match (
            self.pending_teleports.get(&session_id),
            self.maps.get(&source_key),
        ) {
            (Some(pending), Some(map)) => (
                map.savable_auras_for_player(&pending.player),
                map.player_cooldown_rows(guid, unix_now_i64()),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        let ctx = SaveContext {
            map_id: dest_key.map_id,
            instance_id: 0,
            zone,
            online: false,
            logout_time: now_unix,
            save_stats: self.should_save_stats(level, logout),
            can_speak,
        };
        let Some((bundle, stats, action_writes, projected_quests)) =
            self.pending_teleports.get(&session_id).map(|pending| {
                let action_writes = logout.then(|| build_action_writes(&pending.player));
                let (bundle, stats, projected_quests) =
                    build_save_bundle(&pending.player, auras, cooldowns, &ctx);
                (bundle, stats, action_writes, projected_quests)
            })
        else {
            return false;
        };
        if let Err(e) = self.deps.character_saver.save_character(bundle, stats) {
            self.log.out(
                Level::Error,
                &format!("character save failed for guid {guid_low}: {e}"),
            );
            return false;
        }
        let actions_saved = action_writes.is_none_or(|(inserts, updates, deletes)| {
            match self
                .deps
                .character_saver
                .save_actions(guid_low, inserts, updates, deletes)
            {
                Ok(()) => true,
                Err(e) => {
                    self.log.out(
                        Level::Error,
                        &format!("character action save failed for guid {guid_low}: {e}"),
                    );
                    false
                }
            }
        });
        if let Some(pending) = self.pending_teleports.get_mut(&session_id) {
            rebaseline(&mut pending.player, now_unix, projected_quests);
            if actions_saved {
                rebaseline_actions(&mut pending.player);
            }
        }
        true
    }

    fn update_player_saves(&mut self) {
        let now = Instant::now();
        let players: Vec<(SessionId, u32, bool)> = self
            .sessions
            .iter()
            .filter_map(|(id, entry)| {
                entry.player_guid.map(|guid_low| {
                    (
                        *id,
                        guid_low,
                        entry.next_player_save.is_some_and(|next| next <= now),
                    )
                })
            })
            .collect();
        for (_id, guid_low, due) in players {
            self.advance_played_time(guid_low);
            if due {
                self.save_player(guid_low, true, false);
            }
        }
    }

    fn save_all_players(&mut self) {
        let guids: Vec<u32> = self.player_session.keys().copied().collect();
        for guid_low in guids {
            self.save_player(guid_low, false, true);
        }
        let pending_sessions: Vec<SessionId> = self.pending_teleports.keys().copied().collect();
        for session_id in pending_sessions {
            if !self
                .sessions
                .get(&session_id)
                .and_then(|entry| entry.player_guid)
                .is_some_and(|guid_low| self.player_session.contains_key(&guid_low))
            {
                self.save_pending_player(session_id, true);
            }
        }
    }

    /// Shared map-entry body (login + teleport finish).
    fn add_player_to_map(
        &mut self,
        session: SessionId,
        player: Player,
        map_key: MapKey,
        apply_auras: bool,
    ) {
        let map_id = map_key.map_id;
        let guid_low = player.unit.guid().counter();
        let player_level = player.unit.level();
        let initial_player_save = self.initial_player_save_instant(guid_low);
        let now_ms = self.server_ms();
        let now_unix = unix_now();
        let maps_dir = self.deps.maps_dir.clone();
        let navmesh_dir = self.deps.mmaps_dir.clone();
        let map_config = MapConfig {
            // `IsPvPRealm()`: REALM_TYPE_PVP (1), RPPVP (8), FFA_PVP (16).
            pvp_realm: matches!(self.config.game_type, 1 | 8 | 16),
            activate_weather: self.config.activate_weather,
            change_weather_interval_ms: self.config.change_weather_interval_ms,
            ..MapConfig::default()
        };
        let map = self.maps.entry(map_key).or_insert_with(|| {
            let mut map = Map::new(
                map_id,
                map_key.instance_id,
                map_config,
                Arc::clone(&self.deps.world_stores),
                WorldRng::from_entropy(),
            );
            map.set_maps_dir(maps_dir);
            map.set_mmaps_dir(navmesh_dir);
            map
        });
        map.add_player(player, &self.deps.respawn_times, now_unix, now_ms);
        if apply_auras {
            // Saved auras land after the player is in
            // the map (`Player::_LoadAuras` at login
            // completion; teleports keep live auras).
            map.apply_saved_auras(ObjectGuid::with_entry(
                mangos_guid::HighGuid::PLAYER,
                0,
                guid_low,
            ));
        }
        self.player_map.insert(guid_low, map_key);
        self.player_session.insert(guid_low, session);
        if let Some(entry) = self.sessions.get_mut(&session) {
            entry.played_last_tick = Instant::now();
            if entry.next_player_save.is_none() {
                entry.played_level = player_level;
                entry.next_player_save = initial_player_save;
            }
        }
        self.on_bot_entered_world(session, guid_low, map_key);
    }

    /// Removes the session's player from its map (logout
    /// and disconnect share this; character save is
    /// Phase 6).
    fn leave_map(&mut self, guid_low: u32) {
        // `LogoutPlayer`: save while the aggregate is
        // still in its map, then remove it.
        self.save_player(guid_low, false, true);
        // `Player::~Player` / logout `TradeCancel`.
        self.trade_cancel(guid_low, true, crate::trade::status::TRADE_CANCELED);
        if let Some(map_id) = self.player_map.remove(&guid_low) {
            if let Some(map) = self.maps.get_mut(&map_id) {
                let _ = map.remove_player(ObjectGuid::with_entry(
                    mangos_guid::HighGuid::PLAYER,
                    0,
                    guid_low,
                ));
            }
        }
        self.player_session.remove(&guid_low);
        if let Err(e) = self.deps.presence.set_character_online(guid_low, false) {
            self.log
                .out(Level::Error, &format!("presence offline write failed: {e}"));
        }
        self.drain_map_events();
    }

    /// Routes queued [`MapEvent`]s: packets to sessions,
    /// respawn clears to the characters DB.
    #[allow(clippy::too_many_lines)] // flat MapEvent dispatcher
    fn drain_map_events(&mut self) {
        let mut events: Vec<MapEvent> = Vec::new();
        for map in self.maps.values_mut() {
            events.extend(map.drain_events());
        }
        let mut dead: Vec<SessionId> = Vec::new();
        for event in events {
            match event {
                MapEvent::Packet { to, packet } => {
                    if let Some(id) = self.player_session.get(&to.counter()).copied()
                        && let Some(entry) = self.sessions.get(&id)
                        && !send_packet(entry, packet)
                    {
                        dead.push(id);
                    }
                }
                MapEvent::ClearCreatureRespawn {
                    guid_low,
                    instance_id,
                } => {
                    if let Err(e) = self
                        .deps
                        .presence
                        .clear_creature_respawn(guid_low, instance_id)
                    {
                        self.log
                            .out(Level::Error, &format!("creature respawn clear failed: {e}"));
                    }
                }
                MapEvent::StoreCreatureRespawn {
                    guid_low,
                    respawn_time,
                    map_id,
                    instance_id,
                } => {
                    if let Err(e) = self.deps.presence.store_creature_respawn(
                        guid_low,
                        respawn_time,
                        map_id,
                        instance_id,
                    ) {
                        self.log
                            .out(Level::Error, &format!("creature respawn store failed: {e}"));
                    }
                }
                MapEvent::UnitDied { guid, killer } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "UnitDied {} killer={}",
                            guid.raw(),
                            killer.map_or(0, ObjectGuid::raw)
                        ),
                    );
                }
                // AI/script consumers land in Slices 6-7;
                // until then these are observable only.
                MapEvent::MovementInform {
                    guid,
                    motion_type,
                    point_id,
                } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "MovementInform {} type={} point={}",
                            guid.raw(),
                            motion_type,
                            point_id
                        ),
                    );
                }
                MapEvent::ChaseReached { guid, target } => {
                    self.log.out(
                        Level::Detail,
                        &format!("ChaseReached {} -> {}", guid.raw(), target.raw()),
                    );
                }
                MapEvent::JustReachedHome { guid } => {
                    self.log
                        .out(Level::Detail, &format!("JustReachedHome {}", guid.raw()));
                }
                MapEvent::WaypointScript {
                    guid,
                    script_id,
                    point_id,
                } => {
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "WaypointScript {} script={} point={}",
                            guid.raw(),
                            script_id,
                            point_id
                        ),
                    );
                }
                MapEvent::ClearGameObjectRespawn {
                    guid_low,
                    instance_id,
                } => {
                    if let Err(e) = self
                        .deps
                        .presence
                        .clear_gameobject_respawn(guid_low, instance_id)
                    {
                        self.log.out(
                            Level::Error,
                            &format!("gameobject respawn clear failed: {e}"),
                        );
                    }
                }
            }
        }
        for id in dead {
            self.disconnect(id);
        }
    }

    /// Map ticks with real elapsed diff (tick contract).
    fn update_maps(&mut self) {
        let now = Instant::now();
        let diff = u32::try_from(now.saturating_duration_since(self.last_tick).as_millis())
            .unwrap_or(u32::MAX);
        self.last_tick = now;
        let now_ms = self.server_ms();
        let now_unix = unix_now();
        for map in self.maps.values_mut() {
            map.tick(diff, now_ms, now_unix);
        }
        self.drain_map_events();
    }

    /// Server ms clock (mstime): monotonic from boot.
    fn server_ms(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_millis() & 0xFFFF_FFFF).unwrap_or(0)
    }

    fn player_map_guid(&self, id: SessionId) -> Option<(MapKey, ObjectGuid)> {
        let entry = self.sessions.get(&id)?;
        let guid_low = entry.player_guid?;
        let &map_id = self.player_map.get(&guid_low)?;
        Some((
            map_id,
            ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low),
        ))
    }

    /// `HandleCastSpellOpcode` subset: spell id +
    /// `SpellCastTargets` (u16 mask; SELF=0 or UNIT
    /// packed guid). Unknown/unsupported target forms
    /// drop silently like the unknown-spell arm.
    fn on_cast_spell(&mut self, id: SessionId, packet: &WorldPacket) {
        use mangos_map::spells::{CastOptions, CastTarget};
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 6 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let spell_id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let mask = u16::from_le_bytes([body[4], body[5]]);
        let target = if mask == 0 {
            CastTarget::SelfCast
        } else if mask & 0x0002 != 0 {
            let Ok((target_guid, _)) = ObjectGuid::read_packed(&body[6..]) else {
                self.bad_packet(id, packet.opcode());
                return;
            };
            CastTarget::Unit(target_guid)
        } else {
            return; // item/GO/location targets unsupported
        };
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.cast_spell(guid, spell_id, target, CastOptions::default());
        self.drain_map_events();
    }

    /// `HandleCancelCastOpcode` subset.
    fn on_cancel_cast(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        if packet.body().len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.cancel_cast(guid, false);
        self.drain_map_events();
    }

    /// `HandleCancelAuraOpcode` subset.
    fn on_cancel_aura(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let spell_id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.cancel_aura(guid, spell_id);
        self.drain_map_events();
    }

    /// `HandleAttackSwingOpcode`: raw target guid; the
    /// map starts melee when the target is attackable.
    fn on_attack_swing(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.attack_start(guid, target);
        self.drain_map_events();
    }

    /// `HandleAttackStopOpcode` (empty body).
    fn on_attack_stop(&mut self, id: SessionId) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.attack_stop(guid);
        self.drain_map_events();
    }

    fn on_set_action_bar_toggles(&mut self, id: SessionId, packet: &WorldPacket) {
        let body = packet.body();
        if body.is_empty() {
            self.bad_packet(id, packet.opcode());
            return;
        }
        if let Some((map_key, guid)) = self.player_map_guid(id)
            && let Some(player) = self
                .maps
                .get_mut(&map_key)
                .and_then(|map| map.player_mut(guid))
        {
            player.unit.world.object.values_mut().set_byte(
                mangos_protocol::update_field_table::offsets::PLAYER_FIELD_BYTES,
                2,
                body[0],
            );
        }
    }

    fn on_played_time(&mut self, id: SessionId) {
        let Some(guid_low) = self.sessions.get(&id).and_then(|e| e.player_guid) else {
            return;
        };
        self.advance_played_time(guid_low);
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let Some((total, level)) = self.maps.get(&map_key).and_then(|map| {
            map.player(guid)
                .map(|player| (player.played_time_total, player.played_time_level))
        }) else {
            return;
        };
        let mut writer = PacketWriter::new();
        writer.write_u32_le(total);
        writer.write_u32_le(level);
        let mut response = WorldPacket::new(opcode_id::SMSG_PLAYED_TIME);
        response.take_writer(writer);
        if let Some(entry) = self.sessions.get(&id)
            && !send_packet(entry, response)
        {
            self.disconnect(id);
        }
    }

    /// `HandleSetActionButtonOpcode`
    /// (`MiscHandler.cpp:944`): update the live action
    /// rows; persistence happens at logout through
    /// `MasterPlayer::SaveActions`.
    fn on_set_action_button(&mut self, id: SessionId, packet: &WorldPacket) {
        match packet.opcode() {
            opcode_id::CMSG_SET_ACTIONBAR_TOGGLES => {
                return self.on_set_action_bar_toggles(id, packet);
            }
            opcode_id::CMSG_PLAYED_TIME => return self.on_played_time(id),
            _ => {}
        }
        let body = packet.body();
        if body.len() < 5 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let button = body[0];
        let packet_data = u32::from_le_bytes(body[1..5].try_into().expect("4 bytes"));
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        let Some(player) = map.player_mut(guid) else {
            return;
        };
        if packet_data == 0 {
            player.deferred.actions.retain(|row| row.button != button);
            return;
        }
        let action = packet_data & 0x00FF_FFFF;
        #[allow(clippy::cast_possible_truncation)]
        let kind = (packet_data >> 24) as u8;
        if !matches!(kind, 0x00 | 0x40 | 0x41 | 0x80) || button >= 120 {
            return;
        }
        let valid = match kind {
            0x00 => self
                .deps
                .world_stores
                .spells
                .get(action)
                .is_some_and(|spell| {
                    spell.attributes & 0x0000_0040 == 0
                        && player.deferred.spells.iter().any(|row| row.spell == action)
                }),
            0x80 => self.deps.world_stores.item_prototypes.get(action).is_some(),
            _ => true,
        };
        if !valid {
            return;
        }
        if let Some(row) = player
            .deferred
            .actions
            .iter_mut()
            .find(|row| row.button == button)
        {
            row.action = action;
            row.kind = kind;
        } else {
            let pos = player
                .deferred
                .actions
                .partition_point(|row| row.button < button);
            player.deferred.actions.insert(
                pos,
                ActionRow {
                    button,
                    action,
                    kind,
                },
            );
        }
    }

    /// `HandleSetSelectionOpcode`: raw guid into
    /// `UNIT_FIELD_TARGET`.
    fn on_set_selection(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        if let Some(player) = map.player_mut(guid) {
            player.unit.values().set_guid(
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET,
                target,
            );
        }
        self.drain_map_events();
    }

    /// `HandleEmoteOpcode`: only the hardcoded client
    /// emotes (0 = none, 3 = oneshot wave) pass, as in
    /// C++; mute/anim-cancel gates are not modeled.
    fn on_emote(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 4 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let emote = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
        if emote != 0 && emote != 3 {
            return;
        }
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.handle_emote_command(guid, emote);
        self.drain_map_events();
    }

    /// `HandleTextEmoteOpcode`: textEmote + emoteNum +
    /// target guid.
    fn on_text_emote(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 16 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let text_emote = u32::from_le_bytes(body[..4].try_into().expect("4 bytes"));
        let emote_num = u32::from_le_bytes(body[4..8].try_into().expect("4 bytes"));
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[8..16].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.player_text_emote(guid, text_emote, emote_num, target);
        self.drain_map_events();
    }

    /// `HandleGameObjectUseOpcode`: one gameobject guid.
    fn on_gameobj_use(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let body = packet.body();
        if body.len() < 8 {
            self.bad_packet(id, packet.opcode());
            return;
        }
        let target =
            ObjectGuid::from_raw(u64::from_le_bytes(body[..8].try_into().expect("8 bytes")));
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        map.gameobject_use(guid, target);
        self.drain_map_events();
    }

    /// `WorldSession::SendAreaTriggerMessage`.
    fn send_area_trigger_message(&mut self, id: SessionId, text: &str) {
        self.reply_packet(id, crate::player_login::build_area_trigger_message(text));
    }

    /// `HandleAreaTriggerOpcode` (`MiscHandler.cpp:675-851`)
    /// subset: script gate evaluation (execution is a
    /// recorded deviation — no DB-script engine and the
    /// C++ `OnAreaTrigger` content scripts are unported),
    /// quest-explore credit (deviation: no quest
    /// progression system yet), tavern rest flag
    /// (rest-type/bonus-rate distinction lands with
    /// rest accrual), ghost-dungeon message (fork
    /// teleports the ghost in when the corpse lies in
    /// the dungeon chain; corpse tracking is unported,
    /// so all ghosts are refused), level/condition
    /// gates, teleport. Unported fork branches whose
    /// state does not exist yet: cheat-option and
    /// taxi-flight early-outs, battleground entrance /
    /// in-BG / zone-script handling (no BG subsystem),
    /// localized teleport messages.
    #[allow(clippy::too_many_lines)]
    fn on_area_trigger(&mut self, id: SessionId, packet: &WorldPacket) {
        let mut reader = packet.reader();
        let Ok(trigger_id) = reader.read_u32_le() else {
            self.bad_packet(id, opcode_id::CMSG_AREATRIGGER);
            return;
        };
        let Some((map_key, guid)) = self.player_map_guid(id) else {
            return;
        };
        let guid_low = guid.counter();
        let stores = &self.deps.world_stores;
        let Some(trigger) = stores.area_triggers.get(trigger_id).cloned() else {
            return;
        };
        let Some((alive, level, position, is_gm)) = self.maps.get(&map_key).and_then(|map| {
            map.player(guid).map(|player| {
                (
                    player.unit.is_alive(),
                    player.unit.level(),
                    player.unit.world.position(),
                    player.extra_flags & 0x0000_0001 != 0,
                )
            })
        }) else {
            return;
        };
        if !trigger.contains(map_key.map_id, position.x, position.y, position.z, 5.0) {
            return;
        }
        // Tavern triggers (`areatrigger_tavern`): the fork
        // puts the player into tavern rest and stops
        // (`HandleAreaTriggerOpcode`).
        if stores.tavern_triggers.contains(&trigger_id) {
            if let Some(map) = self.maps.get_mut(&map_key) {
                map.set_player_rest_tavern(guid, trigger_id);
            }
            return;
        }
        // Script gate (`StartAreaTriggerScript` head):
        // condition + per-map cooldown; execution of DB
        // and C++ trigger scripts is unported.
        if trigger.has_script() {
            let condition_ok = trigger.condition_id == 0
                || self.maps.get(&map_key).is_some_and(|map| {
                    map.condition_satisfied(trigger.condition_id, Some(guid), Some(guid))
                });
            if condition_ok {
                let now = unix_now_i64();
                let cooling = trigger.cooldown != 0
                    && self
                        .area_trigger_cooldowns
                        .get(&(map_key, trigger_id))
                        .is_some_and(|until| *until > now);
                if !cooling {
                    if trigger.cooldown != 0 {
                        self.area_trigger_cooldowns.insert(
                            (map_key, trigger_id),
                            now.saturating_add(i64::from(trigger.cooldown)),
                        );
                    }
                    self.log.out(
                        Level::Detail,
                        &format!(
                            "area trigger {trigger_id} script (id {}, name '{}') not executed: trigger scripts unported",
                            trigger.script_id, trigger.script_name
                        ),
                    );
                }
            }
        }
        // Quest explore credit: progression system absent.
        let quest_id = stores.area_triggers.quest_for_trigger(trigger_id);
        if quest_id != 0 && alive {
            self.log.out(
                Level::Detail,
                &format!(
                    "area trigger {trigger_id} quest {quest_id} explore credit skipped: quest progression unported"
                ),
            );
        }
        // Tavern rest: flag only (rested-XP accrual
        // lands with character save).
        if stores.area_triggers.is_tavern(trigger_id) {
            if let Some(map) = self.maps.get_mut(&map_key)
                && let Some(player) = map.player_mut(guid)
            {
                use mangos_protocol::update_field_table::offsets;
                player
                    .unit
                    .world
                    .object
                    .values_mut()
                    .set_flag(offsets::PLAYER_FLAGS, 0x0000_0020);
            }
            return;
        }
        let Some(teleport) = stores.area_triggers.teleport(trigger_id).cloned() else {
            return;
        };
        let Some(target_entry) = stores.maps.get(teleport.dest_map_id) else {
            return;
        };
        if !alive && target_entry.is_dungeon() {
            let text = format!(
                "You cannot enter {} while in ghost form.",
                target_entry.name
            );
            self.send_area_trigger_message(id, &text);
            return;
        }
        if !is_gm {
            let level_check = level < u32::from(teleport.required_level)
                && !self.config.instance_ignore_level
                // `WOW_PATCH_104` = 2 on the 0..10 patch scale.
                && self.config.wow_patch >= 2;
            let condition_check = teleport.required_condition != 0
                && !self.maps.get(&map_key).is_some_and(|map| {
                    map.condition_satisfied(teleport.required_condition, Some(guid), Some(guid))
                });
            if level_check || condition_check {
                if teleport.message.is_empty() {
                    if level_check {
                        let template = self
                            .strings
                            .get(49)
                            .map_or("You must be level %u to enter.", |entry| entry.text(0));
                        let text = template.replacen("%u", &teleport.required_level.to_string(), 1);
                        self.send_area_trigger_message(id, &text);
                    }
                } else {
                    self.send_area_trigger_message(id, &teleport.message.clone());
                }
                return;
            }
        }
        let _ = guid_low;
        self.teleport_player(
            id,
            teleport.dest_map_id,
            mangos_movement::movement_info::Position::new(
                teleport.dest_x,
                teleport.dest_y,
                teleport.dest_z,
                teleport.dest_o,
            ),
        );
    }

    /// `HandleMovementOpcodes` relay half: parse with the
    /// player's seeded movement info (jump latch), forward
    /// to the map. Anticheat/ack opcodes are Slice 4.
    fn on_movement_packet(&mut self, id: SessionId, packet: &WorldPacket) {
        let Some(entry) = self.sessions.get(&id) else {
            return;
        };
        let Some(guid_low) = entry.player_guid else {
            return;
        };
        let Some(&map_key) = self.player_map.get(&guid_low) else {
            return;
        };
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
        let server_ms = self.server_ms();
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        let Some(player) = map.player_mut(guid) else {
            return;
        };
        // Server is moving the unit: client movement is
        // dropped until the spline completes
        // (`HasPendingSplineDone` / not-finalized gates).
        if player.unit.pending_spline_done || !player.unit.spline.finalized() {
            return;
        }
        let mut info: MovementInfo = player.unit.world.movement_info;
        let mut reader = packet.reader();
        if info.read(&mut reader, server_ms).is_err() {
            self.bad_packet(id, packet.opcode());
            return;
        }
        map.apply_movement(guid, packet.opcode(), info, &self.deps.respawn_times);
        self.drain_map_events();
    }

    /// The movement ACK family (`MovementHandler.cpp`):
    /// force-speed, flag-toggle, root, spline-done,
    /// knockback, teleport. Common shape at 1.12.1: raw
    /// u64 mover GUID, movement counter, seeded
    /// `MovementInfo`, then the change payload. Pending
    /// changes must match exactly (counter + type +
    /// value) or the packet is dropped (C++ also feeds
    /// the anticheat wrong-ack counter — no anticheat
    /// yet, recorded). Knockback/teleport have no
    /// senders in this slice, so their pending queues
    /// are always empty and packets drop.
    #[allow(clippy::too_many_lines)] // one handler family, mirrors MovementHandler.cpp
    fn on_movement_ack(&mut self, id: SessionId, packet: &WorldPacket, kind: AckKind) {
        use mangos_entities::unit::{
            BASE_MOVE_SPEED, MOVEMENT_CHANGE_FEATHER_FALL, MOVEMENT_CHANGE_ROOT,
            MOVEMENT_CHANGE_SET_HOVER, MOVEMENT_CHANGE_WATER_WALK,
        };
        use mangos_movement::movement_info::MovementFlags;
        use mangos_protocol::opcode_table::opcode_id as op;

        let Some(entry) = self.sessions.get(&id) else {
            return;
        };
        let Some(guid_low) = entry.player_guid else {
            return;
        };
        let Some(&map_key) = self.player_map.get(&guid_low) else {
            return;
        };
        let guid = ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, guid_low);
        let server_ms = self.server_ms();
        let Some(map) = self.maps.get_mut(&map_key) else {
            return;
        };
        let opcode = packet.opcode();
        let mut reader = packet.reader();

        // Spline-done has no GUID/counter prefix.
        if kind == AckKind::SplineDone {
            let Some(player) = map.player_mut(guid) else {
                return;
            };
            let mut info: MovementInfo = player.unit.world.movement_info;
            let parsed = info
                .read(&mut reader, server_ms)
                .and_then(|()| reader.read_u32_le())
                .and_then(|spline_id| reader.read_f32_le().map(|_| spline_id));
            let Ok(spline_id) = parsed else {
                self.bad_packet(id, opcode);
                return;
            };
            if player.unit.spline.id() != spline_id || !player.unit.pending_spline_done {
                return;
            }
            player.unit.pending_spline_done = false;
            map.relocate_from_client(guid, info, &self.deps.respawn_times);
            self.drain_map_events();
            return;
        }

        let Some(player) = map.player_mut(guid) else {
            return;
        };
        let mut info: MovementInfo = player.unit.world.movement_info;
        let parsed: Result<(ObjectGuid, u32, f32, bool), mangos_protocol::buffer::ReadError> =
            (|| {
                let raw = reader.read_u64_le()?;
                let counter = reader.read_u32_le()?;
                let mover = ObjectGuid::from_wire_raw(raw);
                if kind == AckKind::Teleport {
                    // guid, counter, time — no MovementInfo.
                    let _time = reader.read_u32_le()?;
                    return Ok((mover, counter, 0.0f32, false));
                }
                info.read(&mut reader, server_ms)?;
                let (value, apply) = match kind {
                    AckKind::Speed => (reader.read_f32_le()?, false),
                    AckKind::FlagToggle => (0.0, reader.read_u32_le()? != 0),
                    _ => (0.0, false),
                };
                Ok((mover, counter, value, apply))
            })();
        let Ok((mover, counter, value, apply)) = parsed else {
            self.bad_packet(id, opcode);
            return;
        };
        // GetMoverFromGuid: only the session's own player
        // is a mover in this slice (no possession).
        if mover != guid {
            return;
        }

        let wrong_ack = |log: &Log, what: &str| {
            log.out(
                Level::Error,
                &format!("{what}: unexpected ack opcode {opcode} counter {counter}"),
            );
        };
        match kind {
            AckKind::Speed => {
                let move_type = match opcode {
                    op::CMSG_FORCE_WALK_SPEED_CHANGE_ACK => 0,
                    op::CMSG_FORCE_RUN_SPEED_CHANGE_ACK => 1,
                    op::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK => 2,
                    op::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK => 3,
                    op::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK => 4,
                    op::CMSG_FORCE_TURN_RATE_CHANGE_ACK => 5,
                    _ => return,
                };
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_speed_change(value, counter, move_type)
                {
                    wrong_ack(&self.log, "HandleForceSpeedChangeAck");
                    return;
                }
                // Speed applies before relocation.
                map.set_speed_rate_real(guid, move_type, value / BASE_MOVE_SPEED[move_type]);
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.send_speed_change_to_observers(guid, move_type, value);
            }
            AckKind::FlagToggle => {
                let (change_type, flag) = match opcode {
                    op::CMSG_MOVE_WATER_WALK_ACK => {
                        (MOVEMENT_CHANGE_WATER_WALK, MovementFlags::WATER_WALKING)
                    }
                    op::CMSG_MOVE_HOVER_ACK => (MOVEMENT_CHANGE_SET_HOVER, MovementFlags::HOVER),
                    op::CMSG_MOVE_FEATHER_FALL_ACK => {
                        (MOVEMENT_CHANGE_FEATHER_FALL, MovementFlags::SAFE_FALL)
                    }
                    _ => return,
                };
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_flag_change(change_type, counter, apply)
                {
                    wrong_ack(&self.log, "HandleMovementFlagChangeToggleAck");
                    return;
                }
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.set_movement_flag_real(guid, flag, apply);
                map.send_flag_change_to_observers(guid, flag, apply);
            }
            AckKind::Root => {
                let apply = opcode == op::CMSG_FORCE_MOVE_ROOT_ACK;
                let Some(player) = map.player_mut(guid) else {
                    return;
                };
                if !player
                    .unit
                    .find_pending_flag_change(MOVEMENT_CHANGE_ROOT, counter, apply)
                {
                    wrong_ack(&self.log, "HandleMoveRootAck");
                    return;
                }
                // 1.14-client root-on-landing workaround
                // (kick when the ack lacks ROOT and the
                // unit is not falling) is anticheat-side;
                // recorded as not reproduced.
                map.relocate_from_client(guid, info, &self.deps.respawn_times);
                map.set_movement_flag_real(guid, MovementFlags::ROOT, apply);
                map.send_flag_change_to_observers(guid, MovementFlags::ROOT, apply);
            }
            AckKind::KnockBack | AckKind::Teleport => {
                // No senders yet: nothing can be pending.
                wrong_ack(&self.log, "HandleMoveAck(no sender)");
                return;
            }
            AckKind::SplineDone => unreachable!("handled above"),
        }
        self.drain_map_events();
    }

    /// `HandleLogoutRequestOpcode` immediate path (combat
    /// and duels do not exist yet): logout complete +
    /// leave map + back to character screen state.
    fn on_logout_request(&mut self, id: SessionId) {
        let guid_low = self.sessions.get(&id).and_then(|e| e.player_guid);
        let Some(guid_low) = guid_low else {
            return;
        };
        self.leave_map(guid_low);
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.player_guid = None;
            entry.character_load = None;
            entry.core.set_player_state(false, false);
            entry.core.set_recently_logged_out(true);
            let packet = WorldPacket::new(opcode::SMSG_LOGOUT_COMPLETE);
            if !send_packet(entry, packet) {
                self.disconnect(id);
            }
        }
    }

    /// Bad-packet policy for handler parse failures
    /// (`ProcessPackets` `ByteBufferException` catch).
    fn bad_packet(&mut self, id: SessionId, opcode: u16) {
        self.log.out(
            Level::Error,
            &format!("SESSION: ByteBufferException while handling opcode {opcode}"),
        );
        if self.config.kick_on_bad_packet {
            self.disconnect(id);
        }
    }

    /// One `World::UpdateSessions` pass over sessions with work,
    /// plus the character-screen idle kick.
    #[allow(clippy::too_many_lines)] // one dispatch pass, mirrors ProcessPackets
    fn update_sessions(&mut self) {
        // Undelivered-mail due notices
        // (`MasterPlayer::Update`).
        let now_unix = unix_now_i64();
        let due: Vec<SessionId> = self
            .sessions
            .iter_mut()
            .filter_map(|(id, entry)| {
                (entry.player_guid.is_some() && entry.mail_tracker.poll_delivery(now_unix))
                    .then_some(*id)
            })
            .collect();
        for id in due {
            self.reply_packet(id, received_mail_packet());
        }
        let now = Instant::now();
        let idle_limit = self.config.character_screen_max_idle_secs;
        let mut kicked: Vec<SessionId> = Vec::new();
        let mut enum_requests: Vec<u32> = Vec::new();
        let mut login_requests: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut query_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut social_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut who_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut chat_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut channel_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut group_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut guild_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut mail_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut trade_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut auction_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut account_data_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut movement_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut ack_packets: Vec<(SessionId, WorldPacket, AckKind)> = Vec::new();
        let mut cast_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut cancel_cast_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut cancel_aura_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut attack_swing_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut attack_stop_packets: Vec<SessionId> = Vec::new();
        let mut set_selection_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut action_button_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut emote_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut text_emote_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut gameobj_use_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut areatrigger_packets: Vec<(SessionId, WorldPacket)> = Vec::new();
        let mut worldport_packets: Vec<SessionId> = Vec::new();
        let mut logout_requests: Vec<SessionId> = Vec::new();

        for (id, entry) in &mut self.sessions {
            if idle_limit > 0
                && now.saturating_duration_since(entry.created)
                    >= Duration::from_secs(u64::from(idle_limit))
            {
                self.log.out(
                    Level::Debug,
                    &format!(
                        "SESSION: Kicking session [{}] from character selection",
                        entry.remote_ip
                    ),
                );
                kicked.push(*id);
                continue;
            }
            if entry.queue.is_empty() {
                continue;
            }
            entry.core.begin_update();
            while let Some(packet) = entry.queue.pop_front() {
                let opcode = packet.opcode();
                let Some(meta) = opcode_meta(opcode) else {
                    continue; // socket layer already rejects >= 828
                };
                match entry.core.process(meta, opcode) {
                    Dispatch::Execute if opcode == opcode::CMSG_CHAR_ENUM => {
                        enum_requests.push(entry.account_id);
                    }
                    Dispatch::Execute if opcode == opcode::CMSG_PLAYER_LOGIN => {
                        login_requests.push((*id, packet));
                    }
                    Dispatch::Execute if opcode == opcode::CMSG_LOGOUT_REQUEST => {
                        logout_requests.push(*id);
                    }
                    Dispatch::Execute if opcode == opcode_id::CMSG_REQUEST_ACCOUNT_DATA => {
                        account_data_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_CREATURE_QUERY
                                | opcode_id::CMSG_GAMEOBJECT_QUERY
                                | opcode_id::CMSG_ITEM_QUERY_SINGLE
                                | opcode_id::CMSG_NAME_QUERY
                        ) =>
                    {
                        query_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_ADD_FRIEND
                                | opcode_id::CMSG_DEL_FRIEND
                                | opcode_id::CMSG_ADD_IGNORE
                                | opcode_id::CMSG_DEL_IGNORE
                        ) =>
                    {
                        social_packets.push((*id, packet));
                    }
                    Dispatch::Execute if opcode == opcode_id::CMSG_WHO => {
                        who_packets.push((*id, packet));
                    }
                    Dispatch::Execute if opcode == opcode_id::CMSG_MESSAGECHAT => {
                        chat_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_JOIN_CHANNEL
                                | opcode_id::CMSG_LEAVE_CHANNEL
                                | opcode_id::CMSG_CHANNEL_LIST
                                | opcode_id::CMSG_CHANNEL_PASSWORD
                                | opcode_id::CMSG_CHANNEL_SET_OWNER
                                | opcode_id::CMSG_CHANNEL_OWNER
                                | opcode_id::CMSG_CHANNEL_MODERATOR
                                | opcode_id::CMSG_CHANNEL_UNMODERATOR
                                | opcode_id::CMSG_CHANNEL_MUTE
                                | opcode_id::CMSG_CHANNEL_UNMUTE
                                | opcode_id::CMSG_CHANNEL_INVITE
                                | opcode_id::CMSG_CHANNEL_KICK
                                | opcode_id::CMSG_CHANNEL_BAN
                                | opcode_id::CMSG_CHANNEL_UNBAN
                                | opcode_id::CMSG_CHANNEL_ANNOUNCEMENTS
                                | opcode_id::CMSG_CHANNEL_MODERATE
                        ) =>
                    {
                        channel_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_GUILD_QUERY
                                | opcode_id::CMSG_GUILD_CREATE
                                | opcode_id::CMSG_GUILD_INVITE
                                | opcode_id::CMSG_GUILD_ACCEPT
                                | opcode_id::CMSG_GUILD_DECLINE
                                | opcode_id::CMSG_GUILD_INFO
                                | opcode_id::CMSG_GUILD_ROSTER
                                | opcode_id::CMSG_GUILD_PROMOTE
                                | opcode_id::CMSG_GUILD_DEMOTE
                                | opcode_id::CMSG_GUILD_LEAVE
                                | opcode_id::CMSG_GUILD_REMOVE
                                | opcode_id::CMSG_GUILD_DISBAND
                                | opcode_id::CMSG_GUILD_LEADER
                                | opcode_id::CMSG_GUILD_MOTD
                                | opcode_id::CMSG_GUILD_SET_PUBLIC_NOTE
                                | opcode_id::CMSG_GUILD_SET_OFFICER_NOTE
                                | opcode_id::CMSG_GUILD_RANK
                                | opcode_id::CMSG_GUILD_ADD_RANK
                                | opcode_id::CMSG_GUILD_DEL_RANK
                                | opcode_id::CMSG_GUILD_INFO_TEXT
                                | opcode_id::MSG_SAVE_GUILD_EMBLEM
                                | opcode_id::CMSG_PETITION_SHOWLIST
                                | opcode_id::CMSG_PETITION_BUY
                                | opcode_id::CMSG_PETITION_SHOW_SIGNATURES
                                | opcode_id::CMSG_PETITION_QUERY
                                | opcode_id::MSG_PETITION_RENAME
                                | opcode_id::CMSG_PETITION_SIGN
                                | opcode_id::MSG_PETITION_DECLINE
                                | opcode_id::CMSG_OFFER_PETITION
                                | opcode_id::CMSG_TURN_IN_PETITION
                        ) =>
                    {
                        guild_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_GROUP_INVITE
                                | opcode_id::CMSG_GROUP_ACCEPT
                                | opcode_id::CMSG_GROUP_DECLINE
                                | opcode_id::CMSG_GROUP_UNINVITE
                                | opcode_id::CMSG_GROUP_UNINVITE_GUID
                                | opcode_id::CMSG_GROUP_SET_LEADER
                                | opcode_id::CMSG_GROUP_DISBAND
                                | opcode_id::CMSG_LOOT_METHOD
                                | opcode_id::CMSG_LOOT_ROLL
                                | opcode_id::CMSG_GROUP_RAID_CONVERT
                                | opcode_id::CMSG_GROUP_CHANGE_SUB_GROUP
                                | opcode_id::CMSG_GROUP_SWAP_SUB_GROUP
                                | opcode_id::CMSG_GROUP_ASSISTANT_LEADER
                                | opcode_id::MSG_RAID_TARGET_UPDATE
                                | opcode_id::MSG_RAID_READY_CHECK
                                | opcode_id::MSG_MINIMAP_PING
                                | opcode_id::MSG_RANDOM_ROLL
                                | opcode_id::CMSG_REQUEST_PARTY_MEMBER_STATS
                        ) =>
                    {
                        group_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_SEND_MAIL
                                | opcode_id::CMSG_GET_MAIL_LIST
                                | opcode_id::CMSG_ITEM_TEXT_QUERY
                                | opcode_id::CMSG_MAIL_TAKE_MONEY
                                | opcode_id::CMSG_MAIL_TAKE_ITEM
                                | opcode_id::CMSG_MAIL_MARK_AS_READ
                                | opcode_id::CMSG_MAIL_RETURN_TO_SENDER
                                | opcode_id::CMSG_MAIL_DELETE
                                | opcode_id::CMSG_MAIL_CREATE_TEXT_ITEM
                                | opcode_id::MSG_QUERY_NEXT_MAIL_TIME
                        ) =>
                    {
                        mail_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::CMSG_INITIATE_TRADE
                                | opcode_id::CMSG_BEGIN_TRADE
                                | opcode_id::CMSG_BUSY_TRADE
                                | opcode_id::CMSG_IGNORE_TRADE
                                | opcode_id::CMSG_ACCEPT_TRADE
                                | opcode_id::CMSG_UNACCEPT_TRADE
                                | opcode_id::CMSG_CANCEL_TRADE
                                | opcode_id::CMSG_SET_TRADE_ITEM
                                | opcode_id::CMSG_CLEAR_TRADE_ITEM
                                | opcode_id::CMSG_SET_TRADE_GOLD
                        ) =>
                    {
                        trade_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            opcode,
                            opcode_id::MSG_AUCTION_HELLO
                                | opcode_id::CMSG_AUCTION_SELL_ITEM
                                | opcode_id::CMSG_AUCTION_REMOVE_ITEM
                                | opcode_id::CMSG_AUCTION_LIST_ITEMS
                                | opcode_id::CMSG_AUCTION_LIST_OWNER_ITEMS
                                | opcode_id::CMSG_AUCTION_PLACE_BID
                                | opcode_id::CMSG_AUCTION_LIST_BIDDER_ITEMS
                        ) =>
                    {
                        auction_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleMovementOpcodes" => {
                        movement_packets.push((*id, packet));
                    }
                    Dispatch::Execute if ack_kind(meta.handler_name).is_some() => {
                        let kind = ack_kind(meta.handler_name).expect("checked");
                        ack_packets.push((*id, packet, kind));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCastSpellOpcode" => {
                        cast_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCancelCastOpcode" => {
                        cancel_cast_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleCancelAuraOpcode" => {
                        cancel_aura_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleAttackSwingOpcode" => {
                        attack_swing_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleAttackStopOpcode" => {
                        attack_stop_packets.push(*id);
                    }
                    Dispatch::Execute if meta.handler_name == "HandleSetSelectionOpcode" => {
                        set_selection_packets.push((*id, packet));
                    }
                    Dispatch::Execute
                        if matches!(
                            meta.handler_name,
                            "HandleSetActionButtonOpcode"
                                | "HandleSetActionBarTogglesOpcode"
                                | "HandlePlayedTime"
                        ) =>
                    {
                        action_button_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleEmoteOpcode" => {
                        emote_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleTextEmoteOpcode" => {
                        text_emote_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleAreaTriggerOpcode" => {
                        areatrigger_packets.push((*id, packet));
                    }
                    Dispatch::Execute if meta.handler_name == "HandleMoveWorldportAckOpcode" => {
                        worldport_packets.push(*id);
                    }
                    Dispatch::Execute if meta.handler_name == "HandleGameObjectUseOpcode" => {
                        gameobj_use_packets.push((*id, packet));
                    }
                    Dispatch::Execute => execute_stub(&self.log, entry, &packet),
                    Dispatch::DropUnexpected(reason) => self.log.out(
                        Level::Debug,
                        &format!(
                            "SESSION: received unexpected opcode {} (0x{opcode:04X}) {reason}",
                            meta.name
                        ),
                    ),
                    Dispatch::DropSilent => {}
                    Dispatch::DropNotAllowed => self.log.out(
                        Level::Error,
                        &format!(
                            "SESSION: received not allowed opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::DropUnhandled => self.log.out(
                        Level::Debug,
                        &format!(
                            "SESSION: received not handled opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::DropWrongStatus => self.log.out(
                        Level::Error,
                        &format!(
                            "SESSION: received wrong-status-req opcode {} (0x{opcode:04X})",
                            meta.name
                        ),
                    ),
                    Dispatch::FloodBreak => {
                        let flood = entry.core.flood_counters();
                        self.log.out(
                            Level::Error,
                            &format!(
                                "AntiFlood: account {} on {} sent {} packets ({} slow, {} very slow) in one update ({})",
                                entry.account_id,
                                entry.remote_ip,
                                flood.total,
                                flood.slow,
                                flood.very_slow,
                                meta.name
                            ),
                        );
                        if self.config.antiflood_sanction & SANCTION_KICK != 0 {
                            kicked.push(*id);
                        }
                        break;
                    }
                }
            }
        }
        for id in kicked {
            self.disconnect(id);
        }
        for account_id in enum_requests {
            self.start_char_enum(account_id);
        }
        for (id, packet) in login_requests {
            self.on_player_login_opcode(id, &packet);
        }
        for (id, packet) in query_packets {
            self.on_query_packet(id, &packet);
        }
        for (id, packet) in social_packets {
            self.on_social_packet(id, &packet);
        }
        for (id, packet) in who_packets {
            self.on_who_packet(id, &packet);
        }
        for (id, packet) in chat_packets {
            self.on_message_chat(id, &packet);
        }
        for (id, packet) in channel_packets {
            let opcode = packet.opcode();
            self.on_channel_packet(id, opcode, &packet);
        }
        for (id, packet) in group_packets {
            self.on_group_packet(id, &packet);
        }
        for (id, packet) in guild_packets {
            self.on_guild_packet(id, &packet);
        }
        for (id, packet) in mail_packets {
            self.on_mail_packet(id, &packet);
        }
        for (id, packet) in trade_packets {
            self.on_trade_packet(id, &packet);
        }
        for (id, packet) in auction_packets {
            self.on_auction_packet(id, &packet);
        }
        for (id, packet) in account_data_packets {
            self.on_request_account_data(id, &packet);
        }
        for (id, packet) in movement_packets {
            self.on_movement_packet(id, &packet);
        }
        for (id, packet, kind) in ack_packets {
            self.on_movement_ack(id, &packet, kind);
        }
        for (id, packet) in cast_packets {
            self.on_cast_spell(id, &packet);
        }
        for (id, packet) in cancel_cast_packets {
            self.on_cancel_cast(id, &packet);
        }
        for (id, packet) in cancel_aura_packets {
            self.on_cancel_aura(id, &packet);
        }
        for (id, packet) in attack_swing_packets {
            self.on_attack_swing(id, &packet);
        }
        for id in attack_stop_packets {
            self.on_attack_stop(id);
        }
        for (id, packet) in set_selection_packets {
            self.on_set_selection(id, &packet);
        }
        for (id, packet) in action_button_packets {
            self.on_set_action_button(id, &packet);
        }
        for (id, packet) in emote_packets {
            self.on_emote(id, &packet);
        }
        for (id, packet) in text_emote_packets {
            self.on_text_emote(id, &packet);
        }
        for (id, packet) in areatrigger_packets {
            self.on_area_trigger(id, &packet);
        }
        for id in worldport_packets {
            self.finish_teleport(id);
        }
        for (id, packet) in gameobj_use_packets {
            self.on_gameobj_use(id, &packet);
        }
        for id in logout_requests {
            self.on_logout_request(id);
        }
    }
}

/// Wall-clock unix seconds (`time(nullptr)`).
/// Formation world queries over a map (navmesh
/// walk-hit approximated by passthrough until the
/// map exposes `GetWalkHitPosition`).
struct ActorFormationHost<'a> {
    map: &'a Map,
}

impl mangos_bots::FormationHost for ActorFormationHost<'_> {
    fn walk_hit_position(
        &self,
        _from: mangos_movement::Vec3,
        to: mangos_movement::Vec3,
    ) -> Option<mangos_movement::Vec3> {
        Some(to)
    }

    fn hostiles_near(&self, _bot: ObjectGuid, at: mangos_movement::Vec3, range: f32) -> bool {
        self.map.bot_hostiles_near_point(at.x, at.y, at.z, range)
    }
}

/// `[BotAI]` debug-line collector (logged after
/// the tick, fork-style "Check Server.log").
struct VecDebug<'a>(&'a mut Vec<String>);

impl mangos_bots::BehaviorDebug for VecDebug<'_> {
    fn line(&mut self, text: String) {
        self.0.push(text);
    }
}

/// `mangosd` `BotHost` over the world actor (the §9
/// host boundary: maps + actor group/chat state;
/// no `&mut Map` crosses into `mangos-bots`).
struct ActorBotHost<'a> {
    world: &'a mut World,
    bot_low: u32,
}

impl ActorBotHost<'_> {
    fn map_key(&self) -> Option<MapKey> {
        self.world.player_map.get(&self.bot_low).copied()
    }

    fn unit_view(
        unit: &mangos_entities::unit::UnitBase,
        guid: ObjectGuid,
        is_player: bool,
        is_casting: bool,
    ) -> mangos_bots::UnitView {
        let pos = unit.world.position();
        let health_pct = if unit.max_health() == 0 {
            100.0
        } else {
            (f64::from(unit.health()) * 100.0 / f64::from(unit.max_health())) as f32
        };
        let target = unit
            .world
            .object
            .values()
            .get_guid(mangos_protocol::update_field_table::offsets::UNIT_FIELD_TARGET);
        mangos_bots::UnitView {
            guid,
            is_alive: unit.is_alive(),
            health_pct,
            health: unit.health(),
            max_health: unit.max_health(),
            position: (pos.x, pos.y, pos.z),
            orientation: pos.o,
            in_combat: unit.melee_victim.is_some(),
            is_player,
            class_id: unit.class(),
            victim: unit.melee_victim,
            target: if target.is_empty() {
                None
            } else {
                Some(target)
            },
            is_casting,
            power_type: unit.power_type(),
            power_pct: {
                let idx = usize::from(unit.power_type().min(4));
                let max = unit.max_power(idx);
                if max == 0 {
                    100.0
                } else {
                    (f64::from(unit.power(idx)) * 100.0 / f64::from(max)) as f32
                }
            },
        }
    }
}

impl mangos_bots::BotHost for ActorBotHost<'_> {
    fn me(&self) -> ObjectGuid {
        player_guid(self.bot_low)
    }

    fn unit(&self, guid: ObjectGuid) -> Option<mangos_bots::UnitView> {
        let map = self.map_key().and_then(|id| self.world.maps.get(&id))?;
        let casting = map.bot_casting_spell(guid).is_some();
        if let Some(player) = map.player(guid) {
            return Some(Self::unit_view(&player.unit, guid, true, casting));
        }
        map.creature(guid)
            .map(|creature| Self::unit_view(&creature.unit, guid, false, casting))
    }

    fn party_members(&self) -> Vec<ObjectGuid> {
        match self
            .world
            .player_group
            .get(&self.bot_low)
            .and_then(|key| self.world.groups.get(key))
        {
            Some(group) => group
                .members()
                .iter()
                .map(|slot| player_guid(slot.guid_low))
                .collect(),
            None => vec![self.me()],
        }
    }

    fn can_try_cast(&self, target: Option<ObjectGuid>, spell_id: u32) -> bool {
        // Slice D approximation of `CanTryToCastSpell`
        // (full range/LoS/power/cooldown checks join
        // the behaviours that need them): known
        // spell, alive self, alive target.
        let Some(map) = self.map_key().and_then(|id| self.world.maps.get(&id)) else {
            return false;
        };
        if !map.bot_knows_spell(self.me(), spell_id) {
            return false;
        }
        if !map.player(self.me()).is_some_and(|p| p.unit.is_alive()) {
            return false;
        }
        match target {
            Some(t) => self.unit(t).is_some_and(|v| v.is_alive),
            None => true,
        }
    }

    fn cast_spell(
        &mut self,
        target: Option<ObjectGuid>,
        spell_id: u32,
    ) -> mangos_bots::CastOutcome {
        let me = self.me();
        let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) else {
            return mangos_bots::CastOutcome::Failed(0);
        };
        let target = match target {
            Some(t) => mangos_map::spells::CastTarget::Unit(t),
            None => mangos_map::spells::CastTarget::SelfCast,
        };
        let outcome = map.cast_spell(
            me,
            spell_id,
            target,
            mangos_map::spells::CastOptions::default(),
        );
        self.world.log.out(
            Level::Debug,
            &format!(
                "[BotCast] bot={} spell={spell_id} outcome={outcome:?}",
                self.bot_low
            ),
        );
        match outcome {
            mangos_map::spells::CastOutcome::Started => mangos_bots::CastOutcome::Ok,
            mangos_map::spells::CastOutcome::Failed(reason) => {
                mangos_bots::CastOutcome::Failed(reason)
            }
            mangos_map::spells::CastOutcome::UnknownSpell => mangos_bots::CastOutcome::UnknownSpell,
        }
    }

    fn move_chase(&mut self, target: ObjectGuid, distance: f32, angle: f32) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.creature_move_chase(me, target, distance, angle);
        }
    }

    fn move_follow(&mut self, target: ObjectGuid, distance: f32, angle: f32) {
        self.move_chase(target, distance, angle);
    }

    fn move_point(&mut self, x: f32, y: f32, z: f32) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            let dest = mangos_movement::movement_info::Position { x, y, z, o: 0.0 };
            map.creature_move_point(me, 0, dest, 0, 0.0, -10.0);
        }
    }

    fn stop_moving(&mut self) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.stop_moving(me);
        }
    }

    fn set_target(&mut self, target: Option<ObjectGuid>) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.bot_set_target(me, target.unwrap_or(ObjectGuid::EMPTY));
        }
    }

    fn say(&mut self, kind: mangos_bots::BotChatKind, text: &str) {
        self.world.bot_say(self.bot_low, kind, text);
    }

    fn whisper(&mut self, target: ObjectGuid, text: &str) {
        self.world.bot_whisper(self.bot_low, target.counter(), text);
    }

    fn attackers_of(&self, guid: ObjectGuid) -> Vec<ObjectGuid> {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .map_or_else(Vec::new, |map| map.bot_attackers_of(guid))
    }

    fn has_aura(&self, guid: ObjectGuid, spell_id: u32) -> bool {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_has_aura(guid, spell_id))
    }

    fn has_periodic_heal_aura(&self, guid: ObjectGuid) -> bool {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_has_periodic_heal(guid))
    }

    fn knows_spell(&self, spell_id: u32) -> bool {
        let me = self.me();
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_knows_spell(me, spell_id))
    }

    fn hostile_count_around(&self, center: ObjectGuid, radius: f32) -> u8 {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .map_or(0, |map| map.bot_hostile_count_around(center, radius))
    }

    fn incoming_damage(&self, guid: ObjectGuid) -> i32 {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .map_or(0, |map| map.bot_incoming_damage(guid))
    }

    fn party_tank(&self) -> Option<ObjectGuid> {
        self.world.find_party_tank(self.bot_low).map(player_guid)
    }

    fn party_leader(&self) -> Option<ObjectGuid> {
        if let Some(group) = self
            .world
            .player_group
            .get(&self.bot_low)
            .and_then(|key| self.world.groups.get(key))
        {
            return Some(player_guid(group.leader_low()));
        }
        self.world
            .bots
            .get(&self.bot_low)
            .and_then(|bot| bot.leader_low)
            .map(player_guid)
    }

    fn threat_of(&self, enemy: ObjectGuid, who: ObjectGuid) -> f32 {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .map_or(0.0, |map| map.threat_on(enemy, who))
    }

    fn motion_kind(&self) -> mangos_bots::MotionKind {
        let me = self.me();
        let motion_type = self
            .map_key()
            .and_then(|id| self.world.maps.get(&id))
            .map_or(0, |map| map.motion_type_of(me));
        match motion_type {
            x if x == mangos_map::motion::IDLE_MOTION_TYPE => mangos_bots::MotionKind::Idle,
            x if x == mangos_map::motion::CHASE_MOTION_TYPE => mangos_bots::MotionKind::Chase,
            x if x == mangos_map::motion::POINT_MOTION_TYPE => mangos_bots::MotionKind::Point,
            _ => mangos_bots::MotionKind::Other,
        }
    }

    fn attack_stop(&mut self) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.bot_attack_stop(me);
        }
    }

    fn can_reach_melee(&self, target: ObjectGuid) -> bool {
        let me = self.me();
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_can_reach_melee(me, target))
    }

    fn dispel_target(&self, spell_id: u32) -> Option<ObjectGuid> {
        self.world.bot_dispel_target(self.bot_low, spell_id)
    }

    fn cc_marks(&self) -> Vec<u8> {
        self.world
            .bots
            .get(&self.bot_low)
            .map_or_else(Vec::new, |bot| bot.cc_marks.clone())
    }

    fn marked_target(&self, icon: u8) -> Option<ObjectGuid> {
        let group = self
            .world
            .player_group
            .get(&self.bot_low)
            .and_then(|key| self.world.groups.get(key))?;
        let raw = group.target_with_icon(usize::from(icon));
        if raw == 0 {
            None
        } else {
            Some(ObjectGuid::from_raw(raw))
        }
    }

    fn is_marked(&self, guid: ObjectGuid) -> bool {
        let Some(group) = self
            .world
            .player_group
            .get(&self.bot_low)
            .and_then(|key| self.world.groups.get(key))
        else {
            return false;
        };
        (0..crate::group::TARGET_ICON_COUNT).any(|icon| group.target_with_icon(icon) == guid.raw())
    }

    fn auto_mark_cc(&mut self, target: ObjectGuid) -> bool {
        // Moon > Square > Diamond > Triangle.
        const CC_ICONS: [u8; 4] = [4, 5, 2, 3];
        let Some(key) = self.world.player_group.get(&self.bot_low).copied() else {
            return false;
        };
        for icon in CC_ICONS {
            let free = self
                .world
                .groups
                .get(&key)
                .is_some_and(|group| group.target_with_icon(usize::from(icon)) == 0);
            if !free {
                continue;
            }
            if let Some(group) = self.world.groups.get_mut(&key) {
                let fx = group.set_target_icon(usize::from(icon), target.raw());
                self.world.apply_group_fx(fx);
            }
            if let Some(bot) = self.world.bots.get_mut(&self.bot_low) {
                bot.cc_marks.push(icon);
            }
            return true;
        }
        false
    }

    fn is_ccd(&self, guid: ObjectGuid) -> bool {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_is_ccd(guid))
    }

    fn clear_target(&mut self) {
        self.set_target(None);
    }

    fn has_aura_type(&self, guid: ObjectGuid, aura_type: u32) -> bool {
        self.map_key()
            .and_then(|id| self.world.maps.get(&id))
            .is_some_and(|map| map.bot_has_aura_type(guid, aura_type))
    }

    fn cancel_aura(&mut self, spell_id: u32) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.cancel_aura(me, spell_id);
        }
    }

    fn remove_shapeshift(&mut self) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.bot_remove_shapeshift(me);
        }
    }

    fn pet(&self) -> Option<ObjectGuid> {
        // No pet system host-side yet (Phase 6 pet
        // persistence); pet behaviours stay inert.
        None
    }

    fn pet_attack(&mut self, _target: ObjectGuid) {}

    fn summon_pet_if_needed(&mut self) {}

    fn cast_weapon_buff(&mut self, _spell_id: u32, _offhand: bool) -> bool {
        false
    }

    fn totem_present(&self, slot: u32) -> bool {
        self.world.bots.get(&self.bot_low).is_some_and(|bot| {
            bot.totem_cast_at
                .get(usize::try_from(slot).unwrap_or(usize::MAX))
                .copied()
                .flatten()
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(60))
        })
    }

    fn note_totem_cast(&mut self, slot: u32) {
        if let Some(bot) = self.world.bots.get_mut(&self.bot_low) {
            if let Ok(idx) = usize::try_from(slot) {
                if let Some(entry) = bot.totem_cast_at.get_mut(idx) {
                    *entry = Some(Instant::now());
                }
            }
        }
    }

    fn face(&mut self, target: ObjectGuid) {
        let me = self.me();
        if let Some(map) = self.map_key().and_then(|id| self.world.maps.get_mut(&id)) {
            map.bot_face(me, target);
        }
    }

    fn pick_index(&mut self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let mut rng = rand::rng();
        rand::Rng::random_range(&mut rng, 0..len)
    }
}

/// `unix_now` as a mail-time i64 (C++ `time_t`).
#[allow(clippy::cast_possible_wrap)]
fn unix_now_i64() -> i64 {
    unix_now() as i64
}

/// u32 money into the mail rows' i32 columns (C++
/// stores both as unsigned; the row type is signed).
#[allow(clippy::cast_possible_wrap)]
fn money_as_i32(value: u32) -> i32 {
    value as i32
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Phase 3 class stub: the opcode passed the full C++
/// admission chain but no handler exists yet. Char enum is
/// real; everything else still lands here.
fn execute_stub(log: &Log, entry: &SessionEntry, packet: &WorldPacket) {
    let opcode = packet.opcode();
    let name = opcode_meta(opcode).map_or("<none>", |m| m.name);
    log.out(
        Level::Debug,
        &format!(
            "SESSION: opcode {name} (0x{opcode:04X}) from account {} reached Phase 3 stub",
            entry.account_id
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_stamp_advances() {
        let health = WorldHealth::new();
        assert_eq!(health.tick_count(), 0);
        health.stamp_tick();
        health.stamp_tick();
        assert_eq!(health.tick_count(), 2);
        assert!(health.tick_age() < Duration::from_secs(1));
    }
}
