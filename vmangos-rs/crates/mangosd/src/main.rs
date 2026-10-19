//! `mangosd` entry point: arguments and banner like the C++
//! `src/mangosd/Main.cpp`, exit codes 0/1/2 (`World.h:77-79`)
//! with SIGINT → restart (2) and SIGTERM → shutdown (0)
//! (`Master.cpp:522-529`). No SIGSEGV anticrash handler is
//! inherited; the supervisor decision is recorded in
//! `docs/rust-migration-evidence/PHASE3-PROGRESS.md`.
//!
//! Boot sequence mirrors `Master::_StartDB` (`Master.cpp:458-482`):
//! `RealmID` required, then World/Character/Login/Logs each connect
//! and pass the migration gate, then `clearOnlineAccounts`. Static
//! world-data loading lands in its own slice.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mysql::prelude::Queryable;
use tokio::net::TcpListener;

use mangos_config::Config;
use mangos_db::actor::DbActor;
use mangos_db::characters::{AccountDataEntry, CharEnumRow, NameQueryRow};
use mangos_db::migrations::{MigrationDb, check_required_migrations};
use mangos_db::{CharacterDb, ConnectionInfo, DbError, WorldAuthDb};
use mangos_log::{Level, Log};
use mangos_world_data::DataDir;
use mangos_world_data::assets_check::check_start_area_assets;
use mangos_world_data::strings::{MangosStrings, load_mangos_strings};

use mangos_map::WorldStores;
use mangos_world_data::player_info::RaceClassDbc;
use mangos_world_data::spawn_support::RespawnTimes;
use mangosd::auth::{AuthBackend, WorldAuthAccount};
use mangosd::char_queries::CharQueries;
use mangosd::group::GroupWrite;
use mangosd::guild::GuildWrite;
use mangosd::mail::MailWrite;
use mangosd::petition::PetitionWrite;
use mangosd::social::SocialWrite;
use mangosd::socket::{ConnectionDeps, default_server_addresses, run_connection};
use mangosd::world_actor::{
    AhBotStore, AuctionStore, GroupStore, GuildBotStore, GuildStore, InitialAhBot, MailStore,
    PetitionStore, PlayerbotStore, PresenceWriter, RealmPublisher, SessionId, SocialStore,
    TradeStore, WorldDeps, spawn_world,
};
use mangosd::{ERROR_EXIT_CODE, RESTART_EXIT_CODE, SHUTDOWN_EXIT_CODE, ServerConfig};

const DEFAULT_CONFIG_PATH: &str = "mangosd.conf";

enum ArgOutcome {
    Run {
        config_path: String,
        stop_daemon: bool,
    },
    Exit(ExitCode),
}

fn print_usage(exe: &str) {
    println!(
        "Usage: {exe} [<options>]\n    -v, --version               print version and exist\n    -c, --config <config_file>  use config_file as configuration file\n  Running as daemon functions:\n    -s run                      run as daemon\n    -s stop                     stop daemon"
    );
}

/// `ParseServerStartupArguments` (POSIX flavour), mirroring the
/// Rust realmd port of the same C++ code.
fn parse_args() -> ArgOutcome {
    let mut args = std::env::args();
    let exe = args.next().unwrap_or_else(|| "mangosd".to_string());
    let mut config_path = String::new();
    let mut stop_daemon = false;
    while let Some(part) = args.next() {
        match part.as_str() {
            "-h" | "--help" => {
                print_usage(&exe);
                return ArgOutcome::Exit(ExitCode::SUCCESS);
            }
            "-v" | "--version" => {
                println!("Core revision: {}", mangos_build_info::full_version());
                return ArgOutcome::Exit(ExitCode::SUCCESS);
            }
            "-c" | "--config" => {
                let Some(next) = args.next() else {
                    println!("Error: Config filename required");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                };
                config_path = next;
            }
            "-s" => match args.next().as_deref() {
                Some("run") => {}
                Some("stop") => stop_daemon = true,
                Some(_) => {
                    println!("Error: Unknown service mode");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                }
                None => {
                    println!("Error: Config filename required");
                    print_usage(&exe);
                    return ArgOutcome::Exit(ExitCode::FAILURE);
                }
            },
            other => {
                println!("Error: Unknown argument provided '{other}'");
                print_usage(&exe);
                return ArgOutcome::Exit(ExitCode::FAILURE);
            }
        }
    }
    if config_path.is_empty() {
        config_path = DEFAULT_CONFIG_PATH.to_string();
    }
    ArgOutcome::Run {
        config_path,
        stop_daemon,
    }
}

/// `-s stop`: signal the PID recorded in the configured `PidFile`.
fn stop_via_pidfile(config: &Config) -> ExitCode {
    let pidfile = config.string_default("PidFile", "");
    if pidfile.is_empty() {
        return ExitCode::FAILURE;
    }
    let Ok(contents) = std::fs::read_to_string(&pidfile) else {
        return ExitCode::FAILURE;
    };
    let Some(pid) = contents.trim().parse::<i32>().ok() else {
        return ExitCode::FAILURE;
    };
    // SIGINT is the C++ daemon-stop signal (restart code path is
    // what the run script treats as a managed stop).
    let status = std::process::Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn level_from_config(value: i32) -> Level {
    match value {
        i32::MIN..=0 => Level::Error,
        1 => Level::Minimal,
        2 => Level::Basic,
        3 => Level::Detail,
        _ => Level::Debug,
    }
}

/// Tokio adapter over the blocking `mangos-db` world-auth actor:
/// every call hops through `spawn_blocking` so the sync-channel
/// wait never parks a runtime worker.
struct MysqlAuthBackend {
    db: Arc<WorldAuthDb>,
}

impl AuthBackend for MysqlAuthBackend {
    fn lookup_world_account<'a>(
        &'a self,
        realm_id: u32,
        account: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<WorldAuthAccount>, DbError>> + Send + 'a>> {
        let db = Arc::clone(&self.db);
        let account = account.to_string();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || db.lookup_world_account(realm_id, account))
                .await
                .map_err(blocking_join_failed)?
        })
    }

    fn update_last_ip<'a>(
        &'a self,
        account: &'a str,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>> {
        let db = Arc::clone(&self.db);
        let account = account.to_string();
        let ip = ip.to_string();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || db.update_last_ip(account, ip))
                .await
                .map_err(blocking_join_failed)?
        })
    }

    fn is_ip_banned<'a>(
        &'a self,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'a>> {
        let db = Arc::clone(&self.db);
        let ip = ip.to_string();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || db.is_ip_banned(ip))
                .await
                .map_err(blocking_join_failed)?
        })
    }
}

/// Direct `CharacterDb` forwarding (already the blocking shape
/// the world actor's `spawn_blocking` jobs expect).
struct MysqlCharQueries {
    db: Arc<CharacterDb>,
}

impl CharQueries for MysqlCharQueries {
    fn char_enum(&self, account_id: u32) -> Result<Vec<CharEnumRow>, DbError> {
        self.db.char_enum(account_id)
    }

    fn load_character(
        &self,
        guid_low: u32,
    ) -> Result<Option<mangos_db::character_load::CharacterLoad>, DbError> {
        self.db.load_character(guid_low)
    }

    fn name_query(&self, guid_low: u32) -> Result<Option<NameQueryRow>, DbError> {
        self.db.name_query(guid_low)
    }

    fn global_account_data(&self, account_id: u32) -> Result<Vec<AccountDataEntry>, DbError> {
        self.db.load_global_account_data(account_id)
    }

    fn tutorials(&self, account_id: u32) -> Result<Option<[u32; 8]>, DbError> {
        self.db.load_tutorials(account_id)
    }
}

struct MysqlSocialStore {
    db: Arc<CharacterDb>,
}

impl SocialStore for MysqlSocialStore {
    fn player_data_by_name(
        &self,
        name: &str,
    ) -> Result<Option<mangos_db::social::NamedPlayerData>, DbError> {
        let name = name.to_owned();
        self.db
            .actor()
            .call(move |conn| mangos_db::social::load_player_data_by_name(conn, &name))
    }

    fn apply_social_write(
        &self,
        guid_low: u32,
        friend_low: u32,
        write: SocialWrite,
    ) -> Result<(), DbError> {
        self.db.actor().call(move |conn| match write {
            SocialWrite::Insert(flag) => {
                mangos_db::social::insert_social(conn, guid_low, friend_low, u32::from(flag))
            }
            SocialWrite::AddFlag(flag) => {
                mangos_db::social::add_social_flag(conn, guid_low, friend_low, u32::from(flag))
            }
            SocialWrite::ClearFlag(flag) => {
                mangos_db::social::clear_social_flag(conn, guid_low, friend_low, u32::from(flag))
            }
            SocialWrite::DeleteRow => mangos_db::social::delete_social(conn, guid_low, friend_low),
        })
    }
}

struct MysqlGroupStore {
    db: Arc<CharacterDb>,
}

impl GroupStore for MysqlGroupStore {
    fn apply_group_write(&self, write: GroupWrite) -> Result<(), DbError> {
        self.db.actor().call(move |conn| match write {
            GroupWrite::Create(stored) => mangos_db::group::create_group(conn, &stored),
            GroupWrite::DeleteGroup { group_id } => mangos_db::group::delete_group(conn, group_id),
            GroupWrite::InsertMember {
                group_id,
                member_low,
                assistant,
                subgroup,
            } => mangos_db::group::insert_group_member(
                conn, group_id, member_low, assistant, subgroup,
            ),
            GroupWrite::DeleteMember { member_low } => {
                mangos_db::group::delete_group_member(conn, member_low)
            }
            GroupWrite::SetLeader {
                group_id,
                leader_low,
            } => mangos_db::group::update_group_leader(conn, group_id, leader_low),
            GroupWrite::SetRaid { group_id } => mangos_db::group::update_group_raid(conn, group_id),
            GroupWrite::SetSubgroup {
                member_low,
                subgroup,
            } => mangos_db::group::update_member_subgroup(conn, member_low, subgroup),
            GroupWrite::SetAssistant {
                member_low,
                assistant,
            } => mangos_db::group::update_member_assistant(conn, member_low, assistant),
        })
    }
}

struct MysqlGuildStore {
    db: Arc<CharacterDb>,
}

impl GuildStore for MysqlGuildStore {
    fn apply_guild_write(&self, write: GuildWrite) -> Result<(), DbError> {
        self.db.actor().call(move |conn| match write {
            GuildWrite::CreateGuild(stored) => mangos_db::guild::create_guild(conn, &stored),
            GuildWrite::DeleteGuild { guild_id } => mangos_db::guild::delete_guild(conn, guild_id),
            GuildWrite::RenameGuild { guild_id, name } => {
                mangos_db::guild::rename_guild(conn, guild_id, &name)
            }
            GuildWrite::SetLeader {
                guild_id,
                leader_low,
            } => mangos_db::guild::update_guild_leader(conn, guild_id, leader_low),
            GuildWrite::SetMotd { guild_id, motd } => {
                mangos_db::guild::update_guild_motd(conn, guild_id, &motd)
            }
            GuildWrite::SetInfo { guild_id, info } => {
                mangos_db::guild::update_guild_info(conn, guild_id, &info)
            }
            GuildWrite::SetEmblem { guild_id, emblem } => {
                mangos_db::guild::update_guild_emblem(conn, guild_id, emblem)
            }
            GuildWrite::InsertMember {
                guild_id,
                member_low,
                rank_id,
            } => mangos_db::guild::insert_guild_member(conn, guild_id, member_low, rank_id),
            GuildWrite::DeleteMember { member_low } => {
                mangos_db::guild::delete_guild_member(conn, member_low)
            }
            GuildWrite::SetMemberRank {
                member_low,
                rank_id,
            } => mangos_db::guild::update_member_rank(conn, member_low, rank_id),
            GuildWrite::SetMemberPnote { member_low, note } => {
                mangos_db::guild::update_member_pnote(conn, member_low, &note)
            }
            GuildWrite::SetMemberOffnote { member_low, note } => {
                mangos_db::guild::update_member_offnote(conn, member_low, &note)
            }
            GuildWrite::InsertRank {
                guild_id,
                rank_id,
                name,
                rights,
            } => mangos_db::guild::insert_rank(conn, guild_id, rank_id, &name, rights),
            GuildWrite::DeleteAllRanks { guild_id } => {
                mangos_db::guild::delete_all_ranks(conn, guild_id)
            }
            GuildWrite::DeleteRanksFrom { guild_id, rank_id } => {
                mangos_db::guild::delete_ranks_from(conn, guild_id, rank_id)
            }
            GuildWrite::SetRankName {
                guild_id,
                rank_id,
                name,
            } => mangos_db::guild::update_rank_name(conn, guild_id, rank_id, &name),
            GuildWrite::SetRankRights {
                guild_id,
                rank_id,
                rights,
            } => mangos_db::guild::update_rank_rights(conn, guild_id, rank_id, rights),
            GuildWrite::ReplaceEventLog { guild_id, event } => {
                mangos_db::guild::replace_event_log(conn, guild_id, event)
            }
        })
    }

    fn guild_candidate(
        &self,
        guid_low: u32,
    ) -> Result<Option<mangos_db::guild::GuildCandidate>, DbError> {
        self.db
            .actor()
            .call(move |conn| mangos_db::guild::load_guild_candidate(conn, guid_low))
    }
}

struct MysqlPetitionStore {
    characters: Arc<CharacterDb>,
    world: Arc<StartedDb>,
}

impl PetitionStore for MysqlPetitionStore {
    fn apply_petition_write(&self, write: PetitionWrite) -> Result<(), DbError> {
        self.characters.actor().call(move |conn| match write {
            PetitionWrite::InsertPetition {
                petition_id,
                owner_low,
                charter_low,
                name,
            } => mangos_db::petition::insert_petition(
                conn,
                petition_id,
                owner_low,
                charter_low,
                &name,
            ),
            PetitionWrite::InsertSign {
                owner_low,
                petition_id,
                player_low,
                account_id,
            } => mangos_db::petition::insert_petition_sign(
                conn,
                owner_low,
                petition_id,
                player_low,
                account_id,
            ),
            PetitionWrite::Rename { petition_id, name } => {
                mangos_db::petition::rename_petition(conn, petition_id, &name)
            }
            PetitionWrite::Delete { petition_id } => {
                mangos_db::petition::delete_petition(conn, petition_id)
            }
            PetitionWrite::PurgePlayer { player_low } => {
                mangos_db::petition::purge_player_petitions(conn, player_low)
            }
        })
    }

    fn create_charter_item(
        &self,
        owner_low: u32,
        petition_id: u32,
    ) -> Result<Option<u32>, DbError> {
        self.characters.actor().call(move |conn| {
            mangos_db::petition::create_charter_item(conn, owner_low, petition_id)
        })
    }

    fn player_has_item(&self, owner_low: u32, item_low: u32) -> Result<bool, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::petition::player_has_item(conn, owner_low, item_low))
    }

    fn delete_charter_item(&self, owner_low: u32, item_low: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::petition::delete_charter_item(conn, owner_low, item_low))
    }

    fn is_reserved_name(&self, name: &str) -> Result<bool, DbError> {
        let name = name.to_owned();
        self.world
            .actor
            .call(move |conn| mangos_db::petition::is_reserved_name(conn, &name))
    }
}

struct MysqlMailStore {
    characters: Arc<CharacterDb>,
    login: Arc<WorldAuthDb>,
}

impl MailStore for MysqlMailStore {
    fn apply_mail_write(&self, write: MailWrite) -> Result<(), DbError> {
        self.characters.actor().call(move |conn| match write {
            MailWrite::Update(update) => mangos_db::mail::update_mail(conn, &update),
            MailWrite::DeleteFull {
                mail_id,
                item_lows,
                item_text_id,
                stationery,
            } => mangos_db::mail::delete_mail(conn, mail_id, &item_lows, item_text_id, stationery),
            MailWrite::DeleteRows { mail_id } => mangos_db::mail::delete_mail_rows(conn, mail_id),
            MailWrite::DeleteMailItem { item_guid } => {
                mangos_db::mail::delete_mail_item_row(conn, item_guid)
            }
            MailWrite::MoveItemToMail {
                owner_low,
                item_low,
                new_owner_low,
            } => mangos_db::mail::move_item_to_mail(conn, owner_low, item_low, new_owner_low),
            MailWrite::SetItemOwner {
                item_low,
                new_owner_low,
            } => mangos_db::mail::set_item_owner(conn, item_low, new_owner_low),
            MailWrite::DeleteItemInstance { item_low } => {
                mangos_db::mail::delete_item_instance(conn, item_low)
            }
        })
    }

    fn insert_mail(&self, mail: mangos_db::mail::NewMail) -> Result<u32, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::insert_mail(conn, &mail))
    }

    fn create_item_text(&self, text: &str) -> Result<u32, DbError> {
        let text = text.to_owned();
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::create_item_text(conn, &text))
    }

    fn item_text(&self, item_text_id: u32) -> Result<String, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::item_text(conn, item_text_id))
    }

    fn receiver_by_name(
        &self,
        name: &str,
    ) -> Result<Option<mangos_db::mail::MailReceiverInfo>, DbError> {
        let name = name.to_owned();
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::receiver_by_name(conn, &name))
    }

    fn mail_count(&self, receiver_low: u32) -> Result<u32, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::mail_count(conn, receiver_low))
    }

    fn account_id_by_guid(&self, guid_low: u32) -> Result<Option<u32>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::account_id_by_guid(conn, guid_low))
    }

    fn account_email_verified(&self, account_id: u32) -> Result<Option<bool>, DbError> {
        self.login.account_email_verified(account_id)
    }

    fn inventory_item(
        &self,
        owner_low: u32,
        item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::inventory_item(conn, owner_low, item_low))
    }

    fn store_mailed_item(
        &self,
        owner_low: u32,
        item_low: u32,
        item_id: u32,
    ) -> Result<bool, DbError> {
        self.characters.actor().call(move |conn| {
            mangos_db::mail::store_mailed_item(conn, owner_low, item_low, item_id)
        })
    }

    fn create_text_item(
        &self,
        owner_low: u32,
        creator_low: u32,
        item_text_id: u32,
    ) -> Result<bool, DbError> {
        self.characters.actor().call(move |conn| {
            mangos_db::mail::create_text_item(conn, owner_low, creator_low, item_text_id)
        })
    }
}

struct MysqlTradeStore {
    characters: Arc<CharacterDb>,
}

impl TradeStore for MysqlTradeStore {
    fn item_at_pos(
        &self,
        owner_low: u32,
        bag: u32,
        slot: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::trade::item_at_pos(conn, owner_low, bag, slot))
    }

    fn inventory_item(
        &self,
        owner_low: u32,
        item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::inventory_item(conn, owner_low, item_low))
    }

    fn free_backpack_slots(&self, owner_low: u32) -> Result<u32, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::trade::free_backpack_slots(conn, owner_low))
    }

    fn first_free_backpack_slot(&self, owner_low: u32) -> Result<Option<u32>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::petition::first_free_backpack_slot(conn, owner_low))
    }

    fn move_item(
        &self,
        from_low: u32,
        to_low: u32,
        item_low: u32,
        item_id: u32,
        to_slot: u32,
        gift_creator_low: Option<u32>,
    ) -> Result<(), DbError> {
        self.characters.actor().call(move |conn| {
            mangos_db::trade::move_item(
                conn,
                from_low,
                to_low,
                item_low,
                item_id,
                to_slot,
                gift_creator_low,
            )
        })
    }
}

struct MysqlAuctionStore {
    characters: Arc<CharacterDb>,
}

impl AuctionStore for MysqlAuctionStore {
    fn insert_auction(&self, auction: mangos_db::auction::StoredAuction) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::auction::insert_auction(conn, &auction))
    }

    fn delete_auction(&self, auction_id: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::auction::delete_auction(conn, auction_id))
    }

    fn update_bid(&self, auction_id: u32, buyer_guid: u32, last_bid: u32) -> Result<(), DbError> {
        self.characters.actor().call(move |conn| {
            mangos_db::auction::update_bid(conn, auction_id, buyer_guid, last_bid)
        })
    }

    fn detach_inventory_item(&self, owner_low: u32, item_low: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::auction::detach_inventory_item(conn, owner_low, item_low))
    }

    fn set_item_owner(&self, item_low: u32, new_owner_low: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::set_item_owner(conn, item_low, new_owner_low))
    }

    fn delete_item_instance(&self, item_low: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::delete_item_instance(conn, item_low))
    }

    fn account_id_by_guid(&self, guid_low: u32) -> Result<Option<u32>, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::mail::account_id_by_guid(conn, guid_low))
    }
}

struct MysqlPresenceWriter {
    db: Arc<CharacterDb>,
}

impl PresenceWriter for MysqlPresenceWriter {
    fn set_character_online(&self, guid_low: u32, online: bool) -> Result<(), DbError> {
        self.db.actor().call(move |conn| {
            conn.exec_drop(
                "UPDATE `characters` SET `online` = ? WHERE `guid` = ?",
                (u8::from(online), guid_low),
            )
            .map_err(DbError::Driver)?;
            Ok(())
        })
    }

    fn clear_creature_respawn(&self, guid_low: u32) -> Result<(), DbError> {
        self.db.actor().call(move |conn| {
            conn.exec_drop(
                "DELETE FROM `creature_respawn` WHERE `guid` = ? AND `instance` = 0",
                (guid_low,),
            )
            .map_err(DbError::Driver)?;
            Ok(())
        })
    }

    fn store_creature_respawn(
        &self,
        guid_low: u32,
        respawn_time: u64,
        map_id: u32,
    ) -> Result<(), DbError> {
        self.db.actor().call(move |conn| {
            conn.exec_drop(
                "REPLACE INTO `creature_respawn` (`guid`, `respawn_time`, `instance`, `map`) VALUES (?, ?, 0, ?)",
                (guid_low, respawn_time, map_id),
            )
            .map_err(DbError::Driver)?;
            Ok(())
        })
    }

    fn clear_gameobject_respawn(&self, guid_low: u32) -> Result<(), DbError> {
        self.db.actor().call(move |conn| {
            conn.exec_drop(
                "DELETE FROM `gameobject_respawn` WHERE `guid` = ? AND `instance` = 0",
                (guid_low,),
            )
            .map_err(DbError::Driver)?;
            Ok(())
        })
    }
}

struct MysqlRealmPublisher {
    db: Arc<WorldAuthDb>,
}

impl RealmPublisher for MysqlRealmPublisher {
    fn set_realm_population(&self, realm_id: u32, population: f32) -> Result<(), DbError> {
        self.db.set_realm_population(realm_id, population)
    }
}

/// A panicked blocking DB job reads as "no row" to callers,
/// matching how a dead C++ connection surfaces in auth (the
/// socket layer logs it). Kept narrow on purpose.
fn blocking_join_failed(_: tokio::task::JoinError) -> DbError {
    DbError::NoRow
}

fn main() -> ExitCode {
    let (config_path, stop_daemon) = match parse_args() {
        ArgOutcome::Run {
            config_path,
            stop_daemon,
        } => (config_path, stop_daemon),
        ArgOutcome::Exit(code) => return code,
    };

    let Ok(config) = Config::load_from_file(Path::new(&config_path)) else {
        println!("Could not find or parse configuration file {config_path}");
        return ExitCode::FAILURE;
    };
    if stop_daemon {
        return stop_via_pidfile(&config);
    }

    let logs_dir = config.string_default("LogsDir", "");
    let log = match Log::open(
        Path::new(&logs_dir),
        &config.string_default("LogFile.Basic", "Server.log"),
        config.bool_default("LogFile.TimeStamp", false),
        level_from_config(config.int_default("LogLevel.File", 2)),
    ) {
        Ok(log) => log,
        Err(err) => {
            println!("Could not open log file: {err}");
            return ExitCode::FAILURE;
        }
    };
    let console_level = level_from_config(config.int_default("LogLevel.Console", 2));
    let banner = |msg: &str| {
        log.out(Level::Minimal, msg);
        if Level::Minimal <= console_level {
            println!("{msg}");
        }
    };
    banner(&format!(
        "Core revision: {} [world-daemon]",
        mangos_build_info::full_version()
    ));
    banner("<Ctrl-C> to stop.\n");
    banner(&format!("Using configuration file {config_path}."));

    let log = Arc::new(log);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            log.out(Level::Error, &format!("Failed to build runtime: {err}"));
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };
    runtime.block_on(run_server(config, log))
}

/// One started database: its actor plus the log-facing name.
struct StartedDb {
    actor: DbActor,
}

/// `Master::StartDB` for one database: parse the connection
/// string, log it masked, connect, run the migration gate.
/// Missing migrations refuse boot; extras warn
/// (`Database.cpp:606-623`).
fn start_db(
    config: &Config,
    log: &Log,
    name: &str,
    info_key: &str,
    db: MigrationDb,
) -> Result<StartedDb, ExitCode> {
    let dbstring = config.string_default(info_key, "");
    let Some(info) = ConnectionInfo::parse(&dbstring) else {
        log.out(
            Level::Error,
            &format!("Incorrectly formatted database connection string for database {name}"),
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    };
    log.out(
        Level::Minimal,
        &format!("{name} Database: {}", info.masked()),
    );
    let actor = match DbActor::spawn(&format!("{name}-db"), &info.mysql_url()) {
        Ok(actor) => actor,
        Err(e) => {
            log.out(
                Level::Error,
                &format!("Cannot connect to world database {name}: {e}"),
            );
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    report_gate(
        log,
        actor.call(move |conn| check_required_migrations(conn, db)),
    )?;
    Ok(StartedDb { actor })
}

/// Report a gate outcome in the C++ log shape; `Err` refuses boot.
fn report_gate(
    log: &Log,
    gate: Result<(String, mangos_db::migrations::GateOutcome), DbError>,
) -> Result<(), ExitCode> {
    match gate {
        Ok((db_name, outcome)) => {
            if !outcome.passes() {
                log.out(
                    Level::Minimal,
                    &format!("Database `{db_name}` is missing the following migrations:"),
                );
                for missing in &outcome.missing {
                    log.out(Level::Minimal, &format!("\t{missing}"));
                }
                return Err(ExitCode::from(ERROR_EXIT_CODE));
            }
            if !outcome.extra.is_empty() {
                log.out(
                    Level::Minimal,
                    &format!("WARNING! Database `{db_name}` has the following extra migrations:"),
                );
                for extra in &outcome.extra {
                    log.out(Level::Minimal, &format!("\t{extra}"));
                }
            }
            Ok(())
        }
        Err(e) => {
            log.out(Level::Error, &format!("Migration gate query failed: {e}"));
            Err(ExitCode::from(ERROR_EXIT_CODE))
        }
    }
}

/// `Master::StartDB` for the login database, returning the
/// world-auth handle so gate and auth share one actor.
fn start_login_db(config: &Config, log: &Log) -> Result<Arc<WorldAuthDb>, ExitCode> {
    let dbstring = config.string_default("LoginDatabaseInfo", "");
    let Some(info) = ConnectionInfo::parse(&dbstring) else {
        log.out(
            Level::Error,
            "Incorrectly formatted database connection string for database Login",
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    };
    log.out(
        Level::Minimal,
        &format!("Login Database: {}", info.masked()),
    );
    let db = match WorldAuthDb::spawn(&info.mysql_url()) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            log.out(
                Level::Error,
                &format!("Cannot connect to world database Login: {e}"),
            );
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    report_gate(
        log,
        db.actor()
            .call(|conn| check_required_migrations(conn, MigrationDb::Logon)),
    )?;
    Ok(db)
}

/// `Master::StartDB` for the character database, returning
/// the typed handle so gate and queries share one actor.
fn start_character_db(config: &Config, log: &Log) -> Result<Arc<CharacterDb>, ExitCode> {
    let dbstring = config.string_default("CharacterDatabaseInfo", "");
    let Some(info) = ConnectionInfo::parse(&dbstring) else {
        log.out(
            Level::Error,
            "Incorrectly formatted database connection string for database Character",
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    };
    log.out(
        Level::Minimal,
        &format!("Character Database: {}", info.masked()),
    );
    let db = match CharacterDb::spawn(&info.mysql_url()) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            log.out(
                Level::Error,
                &format!("Cannot connect to world database Character: {e}"),
            );
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    report_gate(
        log,
        db.actor()
            .call(|conn| check_required_migrations(conn, MigrationDb::Characters)),
    )?;
    Ok(db)
}

/// All four databases, held for the process lifetime.
struct Databases {
    world: Arc<StartedDb>,
    characters: Arc<CharacterDb>,
    login: Arc<WorldAuthDb>,
    _logs: StartedDb,
}

/// `Master::_StartDB` (`Master.cpp:458-482`) including the
/// recorded unavoidable boot writes in `clearOnlineAccounts`.
fn start_databases(config: &Config, log: &Log, realm_id: u32) -> Result<Databases, ExitCode> {
    if realm_id == 0 {
        log.out(Level::Error, "Realm ID not defined in configuration file");
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    let world = start_db(
        config,
        log,
        "World",
        "WorldDatabaseInfo",
        MigrationDb::World,
    )?;
    let characters = start_character_db(config, log)?;
    let login = start_login_db(config, log)?;
    let logs = start_db(config, log, "Logs", "LogsDatabaseInfo", MigrationDb::Logs)?;

    // clearOnlineAccounts (Master.cpp:485-493): boot writes on
    // record in PHASE3-PROGRESS.
    let cleared = login.actor().call(move |conn| {
        use mangos_db::DbError as E;
        conn.exec_drop(
            "UPDATE `account` SET `current_realm` = 0 WHERE `current_realm` = ?",
            (realm_id,),
        )
        .map_err(E::Driver)?;
        Ok(())
    });
    if let Err(e) = cleared {
        log.out(Level::Error, &format!("clearOnlineAccounts (login): {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    let cleared = characters.actor().call(|conn| {
        use mangos_db::DbError as E;
        conn.query_drop("UPDATE `characters` SET `online` = 0 WHERE `online`<>0")
            .map_err(E::Driver)?;
        conn.query_drop("UPDATE `character_battleground_data` SET `instance_id` = 0")
            .map_err(E::Driver)?;
        Ok(())
    });
    if let Err(e) = cleared {
        log.out(
            Level::Error,
            &format!("clearOnlineAccounts (characters): {e}"),
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }

    Ok(Databases {
        world: Arc::new(world),
        characters,
        login,
        _logs: logs,
    })
}

/// `World::SetInitialWorldSettings` — the Phase 3 subset, in
/// C++ order (`World.cpp:1283-1345`): start-area asset gate,
/// `mangos_string` load (empty aborts), `realmlist` publish, and
/// the corpse cleanup. The remaining loaders (spells, DBC
/// stores beyond this slice, ~90 `ObjectMgr` loads, scripts)
/// are later phases; that deferral is logged, never silent.
struct InitialWorldSettings {
    /// Extracted `.map` directory for ground heights.
    maps_dir: std::path::PathBuf,
    /// Extracted `mmaps` directory for navmesh paths.
    mmaps_dir: std::path::PathBuf,
    strings: MangosStrings,
    enum_stores: mangos_world_data::enum_stores::EnumStores,
    world_stores: Arc<WorldStores>,
    race_class_dbc: Arc<RaceClassDbc>,
    respawn_times: RespawnTimes,
    /// Boot-loaded groups (Phase 5 Slice B.3).
    initial_groups: mangos_db::group::LoadedGroups,
    /// Boot-loaded guilds (Phase 5 Slice B.4).
    initial_guilds: mangos_db::guild::LoadedGuilds,
    /// Boot-loaded petitions (Phase 5 Slice B.4b).
    initial_petitions: mangos_db::petition::LoadedPetitions,
    /// Boot-loaded auctions (Phase 5 Slice B.7).
    initial_auctions: mangos_db::auction::LoadedAuctions,
    /// Boot-built GM command table (Phase 5 Slice C).
    initial_commands: mangosd::commands::CommandTable,
    initial_playerbots: Vec<mangos_db::playerbot::PlayerbotRow>,
    initial_guild_bots: mangos_db::guild_bots::LoadedGuildBots,
    /// Boot-loaded AH bot rows (Slice F).
    initial_ah_bot: InitialAhBot,
}

fn load_initial_world_settings(
    config: &Config,
    server_config: &ServerConfig,
    databases: &Databases,
    log: &Log,
) -> Result<InitialWorldSettings, ExitCode> {
    let data_dir = DataDir::from_config(config);
    log.out(
        Level::Minimal,
        &format!("Using DataDir {}", data_dir.path().display()),
    );
    if let Err(failure) = check_start_area_assets(&data_dir) {
        log.out(Level::Error, &failure.to_string());
        log.out(
            Level::Error,
            &format!(
                "Correct *.map files not found in path '{}' or *.vmtree/*.vmtile files in '{}'. Please place *.map and vmap files in appropriate directories or correct the DataDir value in the mangosd.conf file.",
                data_dir.maps_dir().display(),
                data_dir.vmaps_dir().display()
            ),
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }

    log.out(Level::Minimal, "Loading MaNGOS strings...");
    let strings = match databases.world.actor.call(load_mangos_strings) {
        Ok(strings) => strings,
        Err(e) => {
            log.out(Level::Error, &format!("Loading MaNGOS strings failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    if strings.is_empty() {
        log.out(
            Level::Minimal,
            ">> Loaded 0 mangos strings. DB table `mangos_string` is empty. Cannot continue.",
        );
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} MaNGOS strings from table mangos_string",
            strings.len()
        ),
    );

    // Publish realm type/zone (World.cpp:1328-1330); FFA_PVP is
    // not sent to the realm list under its custom id.
    let server_type = if server_config.game_type == 16 {
        1
    } else {
        server_config.game_type
    };
    let realm_id = server_config.realm_id;
    let realm_zone = server_config.realm_zone;
    let published = databases.login.actor().call(move |conn| {
        conn.exec_drop(
            "UPDATE `realmlist` SET `icon` = ?, `timezone` = ? WHERE `id` = ?",
            (server_type, realm_zone, realm_id),
        )
        .map_err(DbError::Driver)?;
        Ok(())
    });
    if let Err(e) = published {
        log.out(Level::Error, &format!("realmlist publish failed: {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }

    // Remove bones and old corpses (World.cpp:1340; 3*DAY).
    let cleaned = databases.characters.actor().call(|conn| {
        conn.exec_drop(
            "DELETE FROM `corpse` WHERE `corpse_type` = '0' OR `time` < (UNIX_TIMESTAMP() - ?)",
            (3 * 86_400_u32,),
        )
        .map_err(DbError::Driver)?;
        Ok(())
    });
    if let Err(e) = cleaned {
        log.out(Level::Error, &format!("corpse cleanup failed: {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }

    let wow_patch = server_config.wow_patch;
    let enum_stores = match databases
        .world
        .actor
        .call(move |conn| mangos_world_data::enum_stores::load_enum_stores(conn, wow_patch))
    {
        Ok(stores) => stores,
        Err(e) => {
            log.out(Level::Error, &format!("enum store load failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} item definitions, {} creature templates (enum subset), {} player create infos",
            enum_stores.item_count(),
            enum_stores.creature_count(),
            enum_stores.player_info_count()
        ),
    );

    // Phase 4 static stores: DBC subsets, templates,
    // spawns/support stores, player infos, persisted
    // respawn times, and (Slice 5) the spell store.
    // Loader diagnostics are counted and sampled, never
    // silent; AI/script stores remain later slices and
    // are logged as deferred below.
    let phase4 = load_phase4_stores(server_config, databases, &data_dir, log)?;
    log.out(
        Level::Minimal,
        "Phase 4: AI/script stores are later slices; the simulated world runs without them by design",
    );
    sweep_old_mails(&databases.characters, log);
    Ok(InitialWorldSettings {
        maps_dir: data_dir.maps_dir(),
        mmaps_dir: data_dir.mmaps_dir(),
        strings,
        enum_stores,
        world_stores: Arc::clone(&phase4.0),
        race_class_dbc: phase4.1,
        respawn_times: phase4.2,
        initial_groups: load_initial_groups(&databases.characters, log),
        initial_guilds: load_initial_guilds(&databases.characters, config, log),
        initial_petitions: load_initial_petitions(&databases.characters, log),
        initial_auctions: load_initial_auctions(&databases.characters, &phase4.0, log),
        initial_commands: load_initial_commands(databases, server_config, log),
        initial_playerbots: load_initial_playerbots(&databases.characters, server_config, log),
        initial_guild_bots: load_initial_guild_bots(&databases.characters, server_config, log),
        initial_ah_bot: load_initial_ah_bot(databases, server_config, log),
    })
}

/// DBC display tables: display id -> (native scale,
/// collision height).
type DisplayModelMap = std::collections::HashMap<u32, (f32, Option<f32>)>;

/// The DBC half of the world stores.
struct DbcTables {
    race_class: RaceClassDbc,
    skills: mangos_world_data::skill_info::SkillDbc,
    chat_channels: mangos_world_data::chat_channels::ChatChannelDbc,
    auction_houses: mangos_world_data::auction_house::AuctionHouseDbc,
    display_scales: std::collections::HashMap<u32, f32>,
    display_model: DisplayModelMap,
}

fn read_dbc_tables(data_dir: &DataDir, log: &Log) -> Result<DbcTables, ExitCode> {
    use mangos_dbc::{DbcFile, FieldValue, formats};
    use mangos_entities::unit::check_valid_scale;
    use mangos_world_data::player_info::RaceDbc;
    use std::collections::HashMap;

    fn uint(row: &[FieldValue], idx: usize) -> u32 {
        match row.get(idx) {
            Some(FieldValue::UInt(v)) => *v,
            _ => 0,
        }
    }
    fn float(row: &[FieldValue], idx: usize) -> f32 {
        match row.get(idx) {
            Some(FieldValue::Float(v)) => *v,
            _ => 0.0,
        }
    }
    let dbc_dir = data_dir.dbc_dir().join("dbc");
    let read_store = |def: formats::StoreDef| -> Result<Vec<Vec<FieldValue>>, ExitCode> {
        let path = dbc_dir.join(def.file);
        let bytes = std::fs::read(&path).map_err(|e| {
            log.out(
                Level::Error,
                &format!("cannot read {}: {e}", path.display()),
            );
            ExitCode::from(ERROR_EXIT_CODE)
        })?;
        let store = DbcFile::parse(&bytes, def.fmt)
            .and_then(|f| f.produce(def.fmt))
            .map_err(|e| {
                log.out(Level::Error, &format!("{}: {e}", path.display()));
                ExitCode::from(ERROR_EXIT_CODE)
            })?;
        Ok(store.rows().to_vec())
    };

    let mut race_class = RaceClassDbc::default();
    for row in read_store(formats::CHR_RACES)? {
        race_class.races.insert(
            uint(&row, 0),
            RaceDbc {
                faction_id: uint(&row, 2),
                model_m: uint(&row, 3),
                model_f: uint(&row, 4),
                team_id: uint(&row, 5),
            },
        );
    }
    for row in read_store(formats::CHR_CLASSES)? {
        race_class.classes.insert(uint(&row, 0));
        race_class.class_power.insert(uint(&row, 0), uint(&row, 1));
    }
    let skill_dbc = read_skill_dbc(&read_store, uint)?;
    let chat_channels = read_chat_channel_dbc(&read_store, uint)?;
    let auction_houses = read_auction_house_dbc(&read_store, uint)?;
    let mut model_scale: HashMap<u32, (f32, Option<f32>)> = HashMap::new();
    for row in read_store(formats::CREATURE_MODEL_DATA)? {
        model_scale.insert(uint(&row, 0), (float(&row, 3), Some(float(&row, 4))));
    }
    let mut display_scales: HashMap<u32, f32> = HashMap::new();
    let mut display_model: HashMap<u32, (f32, Option<f32>)> = HashMap::new();
    for row in read_store(formats::CREATURE_DISPLAY_INFO)? {
        let (display_id, model_id, scale) = (uint(&row, 0), uint(&row, 1), float(&row, 3));
        match model_scale.get(&model_id) {
            Some((ms, collision)) => {
                display_scales.insert(display_id, check_valid_scale(ms * scale));
                let native = check_valid_scale(ms * scale);
                let collision = collision.map(|h| if *ms == 0.0 { h } else { h / ms });
                display_model.insert(display_id, (native, collision));
            }
            None => {
                display_scales.insert(display_id, check_valid_scale(scale));
            }
        }
    }
    Ok(DbcTables {
        race_class,
        skills: skill_dbc,
        chat_channels,
        auction_houses,
        display_scales,
        display_model,
    })
}

/// Skill tables for `_LoadSkills` (Phase 5 B.2a):
/// `SkillLine` categories, `SkillRaceClassInfo` rows
/// (produced fields 0-5 of fmt `diiiiiix`), and the
/// `SkillTiers` id set.
fn read_skill_dbc(
    read_store: &impl Fn(
        mangos_dbc::formats::StoreDef,
    ) -> Result<Vec<Vec<mangos_dbc::FieldValue>>, ExitCode>,
    uint: impl Fn(&[mangos_dbc::FieldValue], usize) -> u32,
) -> Result<mangos_world_data::skill_info::SkillDbc, ExitCode> {
    use mangos_dbc::formats;
    use mangos_world_data::skill_info::{RaceClassSkill, SkillDbc};
    let mut skill_dbc = SkillDbc::default();
    for row in read_store(formats::SKILL_LINE)? {
        #[allow(clippy::cast_possible_wrap)]
        let category = uint(&row, 1) as i32;
        skill_dbc.categories.insert(uint(&row, 0), category);
    }
    for row in read_store(formats::SKILL_RACE_CLASS_INFO)? {
        skill_dbc.race_class.push(RaceClassSkill {
            skill_id: uint(&row, 0),
            race_mask: uint(&row, 1),
            class_mask: uint(&row, 2),
            flags: uint(&row, 3),
            req_level: uint(&row, 4),
            skill_tier_id: uint(&row, 5),
        });
    }
    for row in read_store(formats::SKILL_TIERS)? {
        skill_dbc.tiers.insert(uint(&row, 0));
    }
    Ok(skill_dbc)
}

/// Built-in chat channels (Slice B.2b): produced fields
/// of fmt `nixssssssssxxxxxxxxxx` are the id, flags, then
/// the eight locale patterns.
fn read_chat_channel_dbc(
    read_store: &impl Fn(
        mangos_dbc::formats::StoreDef,
    ) -> Result<Vec<Vec<mangos_dbc::FieldValue>>, ExitCode>,
    uint: impl Fn(&[mangos_dbc::FieldValue], usize) -> u32,
) -> Result<mangos_world_data::chat_channels::ChatChannelDbc, ExitCode> {
    use mangos_dbc::{FieldValue, formats};
    let mut chat_channels = mangos_world_data::chat_channels::ChatChannelDbc::default();
    for row in read_store(formats::CHAT_CHANNELS)? {
        let patterns = (2..10)
            .map(|idx| match row.get(idx) {
                Some(FieldValue::Text(s)) => s.clone(),
                _ => String::new(),
            })
            .collect();
        chat_channels
            .entries
            .push(mangos_world_data::chat_channels::ChatChannelEntry {
                channel_id: uint(&row, 0),
                flags: uint(&row, 1),
                patterns,
            });
    }
    Ok(chat_channels)
}

/// Auction houses (Phase 5 B.7): produced fields of
/// fmt `niiixxxxxxxxx` are house id, faction, deposit
/// percent, cut percent.
fn read_auction_house_dbc(
    read_store: &impl Fn(
        mangos_dbc::formats::StoreDef,
    ) -> Result<Vec<Vec<mangos_dbc::FieldValue>>, ExitCode>,
    uint: impl Fn(&[mangos_dbc::FieldValue], usize) -> u32,
) -> Result<mangos_world_data::auction_house::AuctionHouseDbc, ExitCode> {
    use mangos_dbc::formats;
    let entries = read_store(formats::AUCTION_HOUSE)?
        .iter()
        .map(|row| mangos_world_data::auction_house::AuctionHouseEntry {
            house_id: uint(row, 0),
            faction: uint(row, 1),
            deposit_percent: uint(row, 2),
            cut_percent: uint(row, 3),
        })
        .collect();
    Ok(mangos_world_data::auction_house::AuctionHouseDbc::from_entries(entries))
}

/// The world-DB half of the Phase 4 stores (one actor call).
#[allow(clippy::type_complexity)]
fn load_world_db_stores(
    databases: &Databases,
    wow_patch: u32,
) -> Result<
    (
        mangos_world_data::creature_template::CreatureTemplates,
        mangos_world_data::gameobject_template::GameObjectTemplates,
        mangos_world_data::spawns::SpawnData,
        mangos_world_data::spawn_support::CreatureAddons,
        mangos_world_data::spawn_support::EquipmentTemplates,
        mangos_world_data::spawn_support::ClassLevelStats,
        mangos_world_data::spawn_support::DisplayInfoAddons,
        mangos_world_data::item_prototype::ItemPrototypesLite,
        mangos_world_data::reputations::FactionStore,
        mangos_world_data::areas::AreaStore,
        mangos_world_data::weather::WeatherChanceStore,
        mangos_world_data::waypoints::WaypointStore,
        std::collections::HashMap<u32, mangos_spells::SpellEntry>,
        mangos_world_data::creature_spells::CreatureSpellsMap,
        mangos_scripts_engine::ScriptStores,
        std::collections::HashSet<u32>,
        Vec<(&'static str, mangos_world_data::load_report::LoadReport)>,
    ),
    DbError,
> {
    use mangos_world_data::areas::load_areas;
    use mangos_world_data::creature_spells::load_creature_spells;
    use mangos_world_data::creature_template::{TemplateValidation, load_creature_templates};
    use mangos_world_data::gameobject_template::{GameObjectValidation, load_gameobject_templates};
    use mangos_world_data::item_prototype::load_item_prototypes_lite;
    use mangos_world_data::load_report::LoadReport;
    use mangos_world_data::reputations::load_factions;
    use mangos_world_data::script_engine::load_script_stores;
    use mangos_world_data::spawn_support::{
        load_class_level_stats, load_creature_addons, load_display_info_addons,
        load_equipment_templates,
    };
    use mangos_world_data::spawns::{SpawnData, load_map_ids};
    use mangos_world_data::spell_template::load_spell_templates;
    use mangos_world_data::waypoints::load_waypoints;
    use mangos_world_data::weather::load_weather_chances;

    databases.world.actor.call(move |conn| {
        let (creature_templates, r) =
            load_creature_templates(conn, wow_patch, &TemplateValidation::default())?;
        let reports = [("creature_template", r)];
        let (gameobject_templates, r2) =
            load_gameobject_templates(conn, wow_patch, &GameObjectValidation::default())?;
        let map_ids = load_map_ids(conn)?;
        let mut spawns = SpawnData::new();
        let r3 =
            spawns.load_creatures(conn, wow_patch, &creature_templates, Some(&map_ids), false)?;
        let r4 = spawns.load_gameobjects(
            conn,
            wow_patch,
            &gameobject_templates,
            Some(&map_ids),
            None,
            false,
        )?;
        let (creature_addons, r5) = load_creature_addons(conn, wow_patch, None, None, None)?;
        let items = load_item_prototypes_lite(conn, wow_patch)?;
        let factions = load_factions(conn)?;
        let areas = load_areas(conn, wow_patch)?;
        let weather_chances = load_weather_chances(conn)?;
        let inv_types = items.inventory_types();
        let (equipment, r6) = load_equipment_templates(conn, wow_patch, Some(&inv_types))?;
        let (cls, r7) = load_class_level_stats(conn)?;
        let (display_addons, r8) = load_display_info_addons(conn, 5875, None, None)?;
        let mut r9 = LoadReport::default();
        let waypoints = load_waypoints(conn, &mut r9)?;
        let mut r10 = LoadReport::default();
        let spell_entries = load_spell_templates(conn, 5875, &mut r10)?;
        let spell_id_set: std::collections::HashSet<u32> = spell_entries.keys().copied().collect();
        let mut r11 = LoadReport::default();
        let creature_spells = load_creature_spells(conn, &spell_id_set, &mut r11)?;
        let template_ids: std::collections::HashSet<u32> =
            creature_templates.iter().map(|(id, _)| *id).collect();
        let mut r12 = LoadReport::default();
        let script_stores = load_script_stores(conn, &template_ids, &mut r12)?;
        let mut reports_all: Vec<(&str, LoadReport)> = reports.into_iter().collect();
        reports_all.extend([
            ("gameobject_template", r2),
            ("creature spawns", r3),
            ("gameobject spawns", r4),
            ("creature_addon", r5),
            ("creature_equip_template", r6),
            ("creature_classlevelstats", r7),
            ("display_info_addon", r8),
            ("creature_movement", r9),
            ("spell_template", r10),
            ("creature_spells", r11),
            ("script engine", r12),
        ]);
        Ok((
            creature_templates,
            gameobject_templates,
            spawns,
            creature_addons,
            equipment,
            cls,
            display_addons,
            items,
            factions,
            areas,
            weather_chances,
            waypoints,
            spell_entries,
            creature_spells,
            script_stores,
            map_ids,
            reports_all,
        ))
    })
}

/// Expired-row cleanup (write register) + respawn load.
fn load_persisted_respawns(
    databases: &Databases,
    stores: &WorldStores,
    map_ids: std::collections::HashSet<u32>,
    log: &Log,
) -> Result<RespawnTimes, ExitCode> {
    use mangos_world_data::spawn_support::load_respawn_times;
    use std::collections::HashMap;

    // Expired respawn cleanup (write register) then load.
    let cleaned = databases.characters.actor().call(|conn| {
        conn.query_drop(
            "DELETE FROM `creature_respawn` WHERE `respawn_time` <= UNIX_TIMESTAMP(NOW())",
        )
        .map_err(DbError::Driver)?;
        conn.query_drop(
            "DELETE FROM `gameobject_respawn` WHERE `respawn_time` <= UNIX_TIMESTAMP(NOW())",
        )
        .map_err(DbError::Driver)?;
        Ok(())
    });
    if let Err(e) = cleaned {
        log.out(Level::Error, &format!("respawn cleanup failed: {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    let creature_maps: HashMap<u32, u32> = stores
        .spawns
        .creatures()
        .map(|(g, s)| (*g, u32::from(s.position.map_id)))
        .collect();
    let go_maps: HashMap<u32, u32> = stores
        .spawns
        .gameobjects()
        .map(|(g, s)| (*g, u32::from(s.position.map_id)))
        .collect();
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let respawn_times =
        match databases.characters.actor().call(move |conn| {
            load_respawn_times(conn, now_unix, &creature_maps, &go_maps, &map_ids)
        }) {
            Ok(times) => times,
            Err(e) => {
                log.out(Level::Error, &format!("respawn time load failed: {e}"));
                return Err(ExitCode::from(ERROR_EXIT_CODE));
            }
        };
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} creature and {} gameobject respawn times",
            respawn_times.creature_len(),
            respawn_times.gameobject_len()
        ),
    );
    Ok(respawn_times)
}
/// Phase 4 Slice 3 stores: the DBC subsets the aggregates
/// consume, the world-DB template/spawn/support loaders,
/// and persisted respawn times. The expired-respawn DELETEs
/// are on the owner-approved Phase 4 write register
/// (2026-10-19) and run before the load, exactly where C++
/// runs them.
fn log_loaded_counts(
    log: &Log,
    creature_templates: &mangos_world_data::creature_template::CreatureTemplates,
    gameobject_templates: &mangos_world_data::gameobject_template::GameObjectTemplates,
    spawns: &mangos_world_data::spawns::SpawnData,
    waypoints: &mangos_world_data::waypoints::WaypointStore,
) {
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} creature templates, {} GO templates, {} creature spawns, {} GO spawns, {} waypoint nodes",
            creature_templates.len(),
            gameobject_templates.len(),
            spawns.creature_count(),
            spawns.gameobject_count(),
            waypoints.total_nodes()
        ),
    );
}

/// Spell DBC aux tables (cast times/durations/ranges/radii).
fn load_spell_tables(
    data_dir: &DataDir,
    log: &Log,
) -> Result<mangos_spells::SpellDbcTables, ExitCode> {
    match mangos_spells::load_spell_dbc_tables(&data_dir.dbc_dir().join("dbc")) {
        Ok(t) => Ok(t),
        Err(e) => {
            log.out(Level::Error, &format!("spell DBC load failed: {e}"));
            Err(ExitCode::from(ERROR_EXIT_CODE))
        }
    }
}

/// Model data join: display-addon bounding/combat reach
/// with DBC native scale + collision height.
fn build_model_data(
    display_model: &std::collections::HashMap<u32, (f32, Option<f32>)>,
    display_addons: &mangos_world_data::spawn_support::DisplayInfoAddons,
) -> std::collections::HashMap<u32, mangos_entities::unit::ModelData> {
    use mangos_entities::unit::ModelData;
    let mut model_data = std::collections::HashMap::new();
    for (display_id, (native, collision)) in display_model {
        if let Some(addon) = display_addons.creature(*display_id) {
            model_data.insert(
                *display_id,
                ModelData {
                    native_scale: *native,
                    bounding_radius: addon.bounding_radius,
                    combat_reach: addon.combat_reach,
                    collision_height: *collision,
                },
            );
        }
    }
    model_data
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_lines)]
fn load_phase4_stores(
    server_config: &ServerConfig,
    databases: &Databases,
    data_dir: &DataDir,
    log: &Log,
) -> Result<(Arc<WorldStores>, Arc<RaceClassDbc>, RespawnTimes), ExitCode> {
    use mangos_world_data::load_report::LoadReport;
    use mangos_world_data::player_info::load_player_infos;
    use std::collections::HashSet;

    let DbcTables {
        race_class,
        skills: skill_dbc,
        chat_channels,
        auction_houses,
        display_scales,
        display_model,
    } = read_dbc_tables(data_dir, log)?;

    let wow_patch = server_config.wow_patch;
    let report = |r: &LoadReport, what: &str| {
        if !r.is_empty() {
            log.out(
                Level::Error,
                &format!(
                    "{what}: {} loader diagnostics (first: {})",
                    r.len(),
                    r.errors[0]
                ),
            );
        }
    };
    let (
        creature_templates,
        gameobject_templates,
        spawns,
        creature_addons,
        equipment,
        cls,
        display_addons,
        items,
        factions,
        areas,
        weather_chances,
        waypoints,
        spell_entries,
        creature_spells,
        script_stores,
        map_ids,
        reports,
    ) = match load_world_db_stores(databases, wow_patch) {
        Ok(v) => v,
        Err(e) => {
            log.out(Level::Error, &format!("world store load failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    for (what, r) in &reports {
        report(r, what);
    }
    // Player infos need the DBC tables (display ids).
    let race_class_owned = race_class.clone();
    let map_ids_owned: HashSet<u32> = map_ids.clone();
    let player_infos = match databases.world.actor.call(move |conn| {
        let (infos, r) = load_player_infos(
            conn,
            60,
            Some(&race_class_owned),
            Some(&map_ids_owned),
            None,
        )?;
        Ok((infos, r))
    }) {
        Ok((infos, r)) => {
            report(&r, "player info");
            infos
        }
        Err(e) => {
            log.out(Level::Error, &format!("player info load failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    let model_data = build_model_data(&display_model, &display_addons);
    log_loaded_counts(
        log,
        &creature_templates,
        &gameobject_templates,
        &spawns,
        &waypoints,
    );
    let spell_tables = load_spell_tables(data_dir, log)?;
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} spells, {} creature spell templates.",
            spell_entries.len(),
            creature_spells.len()
        ),
    );
    let obtained_items = match databases
        .characters
        .actor()
        .call(mangos_world_data::item_prototype::load_obtained_item_ids)
    {
        Ok(items) => items,
        Err(e) => {
            log.out(Level::Error, &format!("obtained item load failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    let prevent_item_datamining = server_config.prevent_item_datamining;
    let item_prototypes = match databases.world.actor.call(move |conn| {
        let mut discovered_items = obtained_items;
        discovered_items
            .extend(mangos_world_data::spell_template::load_spell_referenced_item_ids(conn, 5875)?);
        mangos_world_data::item_prototype::load_item_prototypes(
            conn,
            wow_patch,
            prevent_item_datamining,
            &discovered_items,
        )
    }) {
        Ok(prototypes) => prototypes,
        Err(e) => {
            log.out(Level::Error, &format!("item prototype load failed: {e}"));
            return Err(ExitCode::from(ERROR_EXIT_CODE));
        }
    };
    log.out(
        Level::Minimal,
        &format!(">> Loaded {} item prototypes.", item_prototypes.len()),
    );
    log.out(
        Level::Minimal,
        &format!(
            ">> Loaded {} EventAI events, {} creature AI script steps, \
             {} generic script steps, {} conditions, {} broadcast texts.",
            script_stores.event_count(),
            script_stores.creature_script_steps(),
            script_stores.generic_script_steps(),
            script_stores.conditions.len(),
            script_stores.broadcast_texts.len()
        ),
    );
    let stores = Arc::new(WorldStores {
        creature_templates,
        gameobject_templates,
        spawns,
        creature_addons,
        equipment,
        cls,
        display_addons,
        player_infos,
        items,
        item_prototypes,
        factions,
        areas,
        weather_chances,
        waypoints,
        spells: mangos_spells::SpellStore {
            spells: spell_entries,
            tables: spell_tables,
        },
        creature_spells,
        scripts: script_stores,
        display_scales,
        model_data,
        skills: skill_dbc,
        chat_channels,
        auction_houses,
    });

    let respawn_times = load_persisted_respawns(databases, &stores, map_ids, log)?;
    Ok((stores, Arc::new(race_class), respawn_times))
}

async fn accept_loop(
    listener: TcpListener,
    deps: Arc<ConnectionDeps>,
    session_counter: Arc<AtomicU64>,
    log: &Log,
) -> u8 {
    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _peer)) => {
                        let deps = Arc::clone(&deps);
                        let id = SessionId(session_counter.fetch_add(1, Ordering::Relaxed));
                        tokio::spawn(run_connection(stream, id, deps));
                    }
                    Err(err) => {
                        log.out(Level::Error, &format!("Accept failed: {err}"));
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                log.out(Level::Minimal, "Received SIGINT; exiting with restart code");
                return RESTART_EXIT_CODE;
            }
            () = sigterm() => {
                log.out(Level::Minimal, "Received SIGTERM; shutting down");
                return SHUTDOWN_EXIT_CODE;
            }
        }
    }
}

/// Freeze-detector replacement: a plain thread watching the
/// world actor's tick stamp. No signal handlers, no crash
/// masking — a stalled world is logged at >10 s and the
/// process exits non-zero at >30 s so the supervisor restarts
/// it (recorded in PHASE3-PROGRESS).
fn spawn_watchdog(log: Arc<Log>, health: Arc<mangosd::world_actor::WorldHealth>) {
    use mangosd::world_actor::{WATCHDOG_ERROR_AFTER, WATCHDOG_EXIT_AFTER};
    std::thread::Builder::new()
        .name("world-watchdog".to_string())
        .spawn(move || {
            let mut error_logged = false;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let age = health.tick_age();
                if age > WATCHDOG_EXIT_AFTER {
                    log.out(
                        Level::Error,
                        &format!(
                            "World tick stalled for {age:?} ({} ticks run); exiting for supervisor restart",
                            health.tick_count()
                        ),
                    );
                    std::process::exit(i32::from(ERROR_EXIT_CODE));
                }
                if age > WATCHDOG_ERROR_AFTER {
                    if !error_logged {
                        log.out(
                            Level::Error,
                            &format!(
                                "World tick stalled for {age:?} ({} ticks run)",
                                health.tick_count()
                            ),
                        );
                        error_logged = true;
                    }
                } else {
                    error_logged = false;
                }
            }
        })
        .expect("spawn watchdog");
}

/// Realm online publish + uptime row (`Master.cpp:202-208`,
/// `World.cpp:1756`): runs once the world actor exists, before
/// connections are accepted. Returns the start time the uptime
/// refresh keys on.
fn publish_realm_online(
    databases: &Databases,
    server_config: &ServerConfig,
    log: &Log,
) -> Result<u64, ExitCode> {
    let mut builds = String::new();
    for build in mangosd::ACCEPTED_CLIENT_BUILDS {
        use std::fmt::Write as _;
        let _ = write!(builds, "{build} ");
    }
    if let Err(e) = databases
        .login
        .set_realm_online(server_config.realm_id, builds)
    {
        log.out(Level::Error, &format!("realm online publish failed: {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let start_string = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    if let Err(e) = databases.login.insert_uptime(
        server_config.realm_id,
        start_time,
        start_string,
        mangos_build_info::REVISION_HASH.to_string(),
    ) {
        log.out(Level::Error, &format!("uptime insert failed: {e}"));
        return Err(ExitCode::from(ERROR_EXIT_CODE));
    }
    Ok(start_time)
}

/// Periodic uptime refresh (`WUPDATE_UPTIME`, `World.cpp:2017`).
fn spawn_uptime_refresh(
    databases_login: Arc<WorldAuthDb>,
    server_config: &ServerConfig,
    health: Arc<mangosd::world_actor::WorldHealth>,
    start_time: u64,
) {
    let interval_mins = server_config.uptime_update_mins;
    if interval_mins == 0 {
        return;
    }
    let realm_id = server_config.realm_id;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            u64::from(interval_mins) * 60,
        ));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // first interval elapses before updating
        loop {
            tick.tick().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let db = Arc::clone(&databases_login);
            let uptime = now.saturating_sub(start_time) as u32;
            let online = health.active_sessions();
            let max_online = health.max_active_sessions();
            let _ = tokio::task::spawn_blocking(move || {
                db.update_uptime(realm_id, start_time, uptime, online, max_online)
            })
            .await;
        }
    });
}

/// `ObjectMgr::LoadGroups` at boot (with its stale-row
/// cleanups); a failure starts with no groups, matching
/// the respawn-times boot posture.
/// Writes the `PidFile` when configured and returns its
/// path for shutdown cleanup; `None` mirrors the startup
/// error exit.
fn write_pid_file(config: &Config, log: &Log) -> Option<String> {
    let pidfile = config.string_default("PidFile", "");
    if pidfile.is_empty() {
        return Some(pidfile);
    }
    match std::fs::write(&pidfile, format!("{}\n", std::process::id())) {
        Ok(()) => Some(pidfile),
        Err(err) => {
            log.out(Level::Error, &format!("Could not create PID file: {err}"));
            None
        }
    }
}

/// `GuildMgr::LoadGuilds` at boot; a failure starts
/// with no guilds, matching the group boot posture.
fn load_initial_guilds(
    characters: &CharacterDb,
    config: &Config,
    log: &Log,
) -> mangos_db::guild::LoadedGuilds {
    let event_log_count = u32::try_from(
        config
            .int_default("Guild.EventLogRecordsCount", 100)
            .max(100),
    )
    .unwrap_or(100);
    match characters
        .actor()
        .call(move |conn| mangos_db::guild::load_guilds(conn, event_log_count))
    {
        Ok(loaded) => {
            log.out(
                Level::Minimal,
                &format!(">> Loaded {} guild definitions", loaded.guilds.len()),
            );
            loaded
        }
        Err(e) => {
            log.out(Level::Error, &format!("guild boot load failed: {e}"));
            mangos_db::guild::LoadedGuilds::default()
        }
    }
}

/// `ObjectMgr::ReturnOrDeleteOldMails(false)` at
/// boot (the daily re-run is not ported yet).
fn sweep_old_mails(characters: &CharacterDb, log: &Log) {
    #[allow(clippy::cast_possible_wrap)]
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs()) as i64;
    match characters
        .actor()
        .call(move |conn| mangos_db::mail::return_or_delete_old_mails(conn, now))
    {
        Ok(0) => {}
        Ok(count) => log.out(
            Level::Minimal,
            &format!(">> Returned or deleted {count} expired mails"),
        ),
        Err(e) => log.out(Level::Error, &format!("expired mail sweep failed: {e}")),
    }
}

/// `GuildMgr::LoadPetitions` at boot; a failure
/// starts with no petitions.
fn load_initial_petitions(
    characters: &CharacterDb,
    log: &Log,
) -> mangos_db::petition::LoadedPetitions {
    match characters.actor().call(mangos_db::petition::load_petitions) {
        Ok(loaded) => {
            log.out(
                Level::Minimal,
                &format!(">> Loaded {} petitions", loaded.petitions.len()),
            );
            loaded
        }
        Err(e) => {
            log.out(Level::Error, &format!("petition boot load failed: {e}"));
            mangos_db::petition::LoadedPetitions::default()
        }
    }
}

/// `AuctionHouseMgr::LoadAuctionItems` +
/// `LoadAuctions` at boot (with the fork's orphan
/// cleanup); a failure starts with empty houses.
fn load_initial_auctions(
    characters: &CharacterDb,
    world_stores: &Arc<WorldStores>,
    log: &Log,
) -> mangos_db::auction::LoadedAuctions {
    let valid_house_ids: std::collections::HashSet<u32> = world_stores
        .auction_houses
        .entries
        .iter()
        .map(|entry| entry.house_id)
        .collect();
    let stores = Arc::clone(world_stores);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    match characters.actor().call(move |conn| {
        mangos_db::auction::load_auctions(
            conn,
            &valid_house_ids,
            |id| stores.item_prototypes.get(id).is_some(),
            now,
        )
    }) {
        Ok(loaded) => {
            log.out(
                Level::Minimal,
                &format!(">> Loaded {} auctions", loaded.auctions.len()),
            );
            loaded
        }
        Err(e) => {
            log.out(Level::Error, &format!("auction boot load failed: {e}"));
            mangos_db::auction::LoadedAuctions::default()
        }
    }
}

/// The GM command table at boot (`getCommandTable`
/// DB overrides + `LoadRbacPermissions`); failures
/// keep the static defaults, like empty tables do
/// in the fork.
fn load_initial_commands(
    databases: &Databases,
    config: &ServerConfig,
    log: &Log,
) -> mangosd::commands::CommandTable {
    let mut table = mangosd::commands::CommandTable::new();
    match databases
        .world
        .actor
        .call(mangos_db::commands::load_command_overrides)
    {
        Ok(rows) => {
            for row in &rows {
                for line in table.apply_override(row) {
                    log.out(Level::Detail, &line);
                }
            }
        }
        Err(e) => log.out(Level::Error, &format!("command override load failed: {e}")),
    }
    match databases
        .login
        .actor()
        .call(mangos_db::commands::load_rbac_rows)
    {
        Ok(rows) => {
            for line in table.apply_rbac(&rows) {
                log.out(Level::Error, &line);
            }
        }
        Err(e) => log.out(Level::Error, &format!("rbac load failed: {e}")),
    }
    log.out(
        Level::Minimal,
        &format!(">> Loaded {} chat commands", table.count()),
    );
    log.out(
        Level::Minimal,
        &format!(
            "BotChat: Loaded. Trade chat interval: {}-{}s, Guild chat interval: {}-{}s.",
            config.bot_chat.trade_interval_min_s,
            config.bot_chat.trade_interval_max_s,
            config.bot_chat.guild_interval_min_s,
            config.bot_chat.guild_interval_max_s,
        ),
    );
    log.out(
        Level::Minimal,
        &format!(
            "BotServiceMgr: Loaded. Enabled: {}, Buy budget: {}g/day",
            if config.bot_service_enable {
                "yes"
            } else {
                "no"
            },
            config.bot_service_buy_budget_gold,
        ),
    );
    table
}

fn load_initial_groups(characters: &CharacterDb, log: &Log) -> mangos_db::group::LoadedGroups {
    match characters.actor().call(mangos_db::group::load_groups) {
        Ok(loaded) => {
            log.out(
                Level::Minimal,
                &format!(">> Loaded {} group definitions", loaded.groups.len()),
            );
            loaded
        }
        Err(e) => {
            log.out(Level::Error, &format!("group boot load failed: {e}"));
            mangos_db::group::LoadedGroups::default()
        }
    }
}

/// Wires the world-actor dependency set out of the
/// started databases and the boot-loaded settings.
fn build_world_deps(
    databases: &Databases,
    initial: &mut InitialWorldSettings,
    char_queries: &Arc<dyn CharQueries>,
) -> WorldDeps {
    WorldDeps {
        enum_stores: Arc::new(std::mem::take(&mut initial.enum_stores)),
        char_queries: Arc::clone(char_queries),
        realm_publisher: Arc::new(MysqlRealmPublisher {
            db: Arc::clone(&databases.login),
        }),
        world_stores: Arc::clone(&initial.world_stores),
        maps_dir: Some(initial.maps_dir.clone()),
        mmaps_dir: Some(initial.mmaps_dir.clone()),
        race_class_dbc: Arc::clone(&initial.race_class_dbc),
        respawn_times: std::mem::take(&mut initial.respawn_times),
        presence: Arc::new(MysqlPresenceWriter {
            db: Arc::clone(&databases.characters),
        }),
        social: Arc::new(MysqlSocialStore {
            db: Arc::clone(&databases.characters),
        }),
        group: Arc::new(MysqlGroupStore {
            db: Arc::clone(&databases.characters),
        }),
        initial_groups: std::mem::take(&mut initial.initial_groups),
        guild: Arc::new(MysqlGuildStore {
            db: Arc::clone(&databases.characters),
        }),
        initial_guilds: std::mem::take(&mut initial.initial_guilds),
        petition: Arc::new(MysqlPetitionStore {
            characters: Arc::clone(&databases.characters),
            world: Arc::clone(&databases.world),
        }),
        initial_petitions: std::mem::take(&mut initial.initial_petitions),
        mail: Arc::new(MysqlMailStore {
            characters: Arc::clone(&databases.characters),
            login: Arc::clone(&databases.login),
        }),
        trade: Arc::new(MysqlTradeStore {
            characters: Arc::clone(&databases.characters),
        }),
        auction: Arc::new(MysqlAuctionStore {
            characters: Arc::clone(&databases.characters),
        }),
        initial_auctions: std::mem::take(&mut initial.initial_auctions),
        initial_commands: std::mem::take(&mut initial.initial_commands),
        playerbot: Arc::new(MysqlPlayerbotStore {
            db: Arc::clone(&databases.characters),
        }),
        initial_playerbots: std::mem::take(&mut initial.initial_playerbots),
        guild_bot: Arc::new(MysqlGuildBotStore {
            db: Arc::clone(&databases.characters),
        }),
        initial_guild_bots: std::mem::take(&mut initial.initial_guild_bots),
        ah_bot: Arc::new(MysqlAhBotStore {
            world: Arc::clone(&databases.world),
            characters: Arc::clone(&databases.characters),
        }),
        initial_ah_bot: std::mem::take(&mut initial.initial_ah_bot),
    }
}

struct MysqlGuildBotStore {
    db: Arc<CharacterDb>,
}

impl GuildBotStore for MysqlGuildBotStore {
    fn character_level(&self, guid_low: u32) -> Result<Option<u32>, DbError> {
        self.db
            .actor()
            .call(move |conn| mangos_db::guild_bots::character_level(conn, guid_low))
    }

    fn set_character_level(&self, guid_low: u32, level: u32) -> Result<(), DbError> {
        self.db
            .actor()
            .call(move |conn| mangos_db::guild_bots::set_character_level(conn, guid_low, level))
    }
}

struct MysqlAhBotStore {
    world: Arc<StartedDb>,
    characters: Arc<CharacterDb>,
}

impl AhBotStore for MysqlAhBotStore {
    fn load_ah_bot(
        &self,
        seller_account_id: u32,
        seller_count: u32,
    ) -> Result<InitialAhBot, DbError> {
        load_ah_bot_data(
            &self.world,
            &self.characters,
            seller_account_id,
            seller_count,
        )
    }

    fn save_refresh_time(&self, now: i64) -> Result<(), DbError> {
        self.world
            .actor
            .call(move |conn| mangos_db::auction::save_ahbot_refresh_time(conn, now))
    }

    fn create_ahbot_item(&self, item_id: u32, count: u32) -> Result<u32, DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::auction::create_ahbot_item(conn, item_id, count))
    }

    fn delete_item_instance(&self, item_guid: u32) -> Result<(), DbError> {
        self.characters
            .actor()
            .call(move |conn| mangos_db::auction::delete_item_instance(conn, item_guid))
    }
}

/// `AuctionHouseBotMgr::Load` DB half (incl.
/// `CreateSellerIdentities`).
fn load_ah_bot_data(
    world: &StartedDb,
    characters: &CharacterDb,
    seller_account_id: u32,
    seller_count: u32,
) -> Result<InitialAhBot, DbError> {
    let items = world.actor.call(mangos_db::auction::load_ahbot_items)?;
    let last_refresh_s = world
        .actor
        .call(mangos_db::auction::load_ahbot_refresh_time)?;
    let mut sellers = characters
        .actor()
        .call(move |conn| mangos_db::auction::load_ahbot_sellers(conn, seller_account_id))?;
    let mut next_guid = 900_001u32;
    let owned_names: std::collections::BTreeSet<String> =
        sellers.iter().map(|(_, owned)| owned.clone()).collect();
    let mut names = mangos_bots::ah_bot::SELLER_NAMES
        .iter()
        .filter(|name| !owned_names.contains(**name));
    while u32::try_from(sellers.len()).unwrap_or(u32::MAX) < seller_count {
        let Some(name) = names.next() else { break };
        while next_guid <= 900_100 && sellers.iter().any(|(guid, _)| *guid == next_guid) {
            next_guid += 1;
        }
        if next_guid > 900_100 {
            break;
        }
        let guid = next_guid;
        let created = characters.actor().call(move |conn| {
            mangos_db::auction::create_ahbot_seller(conn, guid, seller_account_id, name)
        })?;
        if created {
            sellers.push((guid, (*name).to_owned()));
            next_guid += 1;
        }
    }
    Ok(InitialAhBot {
        items,
        sellers,
        last_refresh_s,
    })
}

/// AH bot boot read (rows load regardless of
/// `AHBot.Enable`; the update gate lives in the actor).
fn load_initial_ah_bot(databases: &Databases, config: &ServerConfig, log: &Log) -> InitialAhBot {
    match load_ah_bot_data(
        &databases.world,
        &databases.characters,
        config.ah_bot.seller_account_id,
        config.ah_bot.seller_count,
    ) {
        Ok(initial) => initial,
        Err(e) => {
            log.out(Level::Error, &format!("ahbot load failed: {e}"));
            InitialAhBot::default()
        }
    }
}

/// `GuildBotMgr::Load` boot read.
fn load_initial_guild_bots(
    characters: &CharacterDb,
    config: &ServerConfig,
    log: &Log,
) -> mangos_db::guild_bots::LoadedGuildBots {
    if !config.guild_bot.enabled {
        log.out(Level::Minimal, "[GuildBotMgr] Disabled.");
        return mangos_db::guild_bots::LoadedGuildBots::default();
    }
    match characters
        .actor()
        .call(mangos_db::guild_bots::load_guild_bots)
    {
        Ok(loaded) => loaded,
        Err(e) => {
            log.out(Level::Error, &format!("guild bot load failed: {e}"));
            mangos_db::guild_bots::LoadedGuildBots::default()
        }
    }
}

struct MysqlPlayerbotStore {
    db: Arc<CharacterDb>,
}

impl PlayerbotStore for MysqlPlayerbotStore {
    fn update_chances(&self, chances: &[(u32, u32)]) -> Result<(), DbError> {
        let chances = chances.to_vec();
        self.db
            .actor()
            .call(move |conn| mangos_db::playerbot::update_playerbot_chances(conn, &chances))
    }
}

/// `PlayerBotMgr::Load` + `BotDirector::Load` boot reads.
fn load_initial_playerbots(
    characters: &CharacterDb,
    config: &ServerConfig,
    log: &Log,
) -> Vec<mangos_db::playerbot::PlayerbotRow> {
    log.out(Level::Minimal, ">> [PlayerBotMgr] Loading Bots ...");
    if config.bot_director_enable {
        log.out(
            Level::Minimal,
            &format!(
                "[BotDirector] Loaded (interval: {} ms)",
                config.bot_director_interval_ms
            ),
        );
    } else {
        log.out(Level::Minimal, "[BotDirector] Disabled by config");
    }
    match characters
        .actor()
        .call(mangos_db::playerbot::load_playerbots)
    {
        Ok(rows) => rows,
        Err(e) => {
            log.out(Level::Error, &format!("playerbot load failed: {e}"));
            Vec::new()
        }
    }
}

async fn run_server(config: Config, log: Arc<Log>) -> ExitCode {
    let server_config = ServerConfig::from_config(&config);

    // Databases before the listener: no logins are accepted
    // until boot data paths are proven (architecture section 3.2).
    let databases = match start_databases(&config, &log, server_config.realm_id) {
        Ok(databases) => databases,
        Err(code) => return code,
    };
    let mut initial = match load_initial_world_settings(&config, &server_config, &databases, &log) {
        Ok(initial) => initial,
        Err(code) => return code,
    };
    let _characters_db = &databases.characters; // character slice consumes this
    let auth_backend: Arc<dyn AuthBackend> = Arc::new(MysqlAuthBackend {
        db: Arc::clone(&databases.login),
    });

    let bind = (server_config.bind_ip, server_config.port);
    let listener = match TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            log.out(
                Level::Error,
                &format!("Failed to bind {}:{}: {err}", bind.0, bind.1),
            );
            return ExitCode::from(ERROR_EXIT_CODE);
        }
    };
    log.out(
        Level::Minimal,
        &format!(
            "World server listening on {}:{} (realm {})",
            bind.0, bind.1, server_config.realm_id
        ),
    );

    let Some(pidfile) = write_pid_file(&config, &log) else {
        return ExitCode::from(ERROR_EXIT_CODE);
    };

    let char_queries: Arc<dyn CharQueries> = Arc::new(MysqlCharQueries {
        db: Arc::clone(&databases.characters),
    });
    let world_deps = build_world_deps(&databases, &mut initial, &char_queries);
    let (world_tx, _world_handle, world_health) = spawn_world(
        server_config.clone(),
        Arc::clone(&log),
        initial.strings,
        world_deps,
    );
    let realm_id_for_shutdown = server_config.realm_id;
    spawn_watchdog(Arc::clone(&log), Arc::clone(&world_health));
    let start_time = match publish_realm_online(&databases, &server_config, &log) {
        Ok(t) => t,
        Err(code) => return code,
    };
    spawn_uptime_refresh(
        Arc::clone(&databases.login),
        &server_config,
        world_health,
        start_time,
    );
    let deps = Arc::new(ConnectionDeps {
        config: Arc::new(server_config),
        backend: auth_backend,
        char_queries,
        world: world_tx,
        log: Arc::clone(&log),
        server_addresses: Arc::new(default_server_addresses()),
    });
    let session_counter = Arc::new(AtomicU64::new(1));

    let exit_code = accept_loop(listener, deps, session_counter, &log).await;

    // Set server offline in realmlist (Master.cpp:317).
    if let Err(e) = databases.login.set_realm_offline(realm_id_for_shutdown) {
        log.out(Level::Error, &format!("realm offline publish failed: {e}"));
    }
    if !pidfile.is_empty() {
        let _ = std::fs::remove_file(&pidfile);
    }
    ExitCode::from(exit_code)
}

#[cfg(unix)]
async fn sigterm() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut stream) => {
            stream.recv().await;
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

#[cfg(not(unix))]
async fn sigterm() {
    std::future::pending::<()>().await
}
