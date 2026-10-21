//! Rust `mangosd` world server (migration document section 12:
//! Tokio runtime + world actor + per-map executors).
//!
//! Phase 3 scope: the boot/session skeleton — listener, socket-layer
//! auth and ping, a single world actor at the C++ 50 ms tick, and
//! class-stubbed opcode dispatch over the generated 828-slot table.
//! World simulation arrives in later phases; nothing here invents
//! behaviour the C++ does not have.

pub mod account_data;
pub mod addon_info;
pub mod anticheat;
pub mod auction;
pub mod auth;
pub mod channels;
pub mod char_enum;
pub mod char_queries;
pub mod character_save;
pub mod chat;
pub mod commands;
pub mod group;
pub mod guild;
pub mod mail;
pub mod petition;
pub mod ping;
pub mod player_login;
pub mod queries;
pub mod session_core;
pub mod social;
pub mod socket;
pub mod trade;
pub mod who;
pub mod world_actor;

use std::net::{IpAddr, Ipv4Addr};

use mangos_config::Config;

/// `SHUTDOWN_EXIT_CODE` (`World.h:77`): clean shutdown.
pub const SHUTDOWN_EXIT_CODE: u8 = 0;
/// `ERROR_EXIT_CODE` (`World.h:78`).
pub const ERROR_EXIT_CODE: u8 = 1;
/// `RESTART_EXIT_CODE` (`World.h:79`): what SIGINT maps to
/// (`Master.cpp:522-523`). Load-bearing for the supervisor.
pub const RESTART_EXIT_CODE: u8 = 2;

/// `EXPECTED_MANGOSD_CLIENT_BUILD` for the 1.12.1 progression gate
/// (`cmake/generators/Progression.h.in:40`).
pub const ACCEPTED_CLIENT_BUILDS: [u32; 3] = [5875, 6005, 6141];

/// `IsAcceptableClientBuild` (`DBCStores.cpp:117-125`).
#[must_use]
pub fn is_acceptable_client_build(build: u32) -> bool {
    ACCEPTED_CLIENT_BUILDS.contains(&build)
}

/// Security levels from `enum AccountTypes` (`Common.h:136-145`),
/// kept as the raw u8 the DB stores.
pub mod security {
    pub const PLAYER: u8 = 0;
    /// `SEC_MODERATOR` (`Common.h:139`).
    pub const MODERATOR: u8 = 1;
    /// `SEC_TICKETMASTER` (`Common.h:140`).
    pub const TICKETMASTER: u8 = 2;
    /// `SEC_GAMEMASTER` (`Common.h:141`).
    pub const GAMEMASTER: u8 = 3;
    /// `SEC_BASIC_ADMIN` (`Common.h:142`).
    pub const BASIC_ADMIN: u8 = 4;
    /// `SEC_DEVELOPER` (`Common.h:143`).
    pub const DEVELOPER: u8 = 5;
    pub const ADMINISTRATOR: u8 = 6;
    /// `SEC_CONSOLE` (`Common.h:145`).
    pub const CONSOLE: u8 = 7;
}

/// The `mangosd.conf` keys the Phase 3 skeleton consumes. Defaults
/// mirror the C++ `setConfig` calls and `GetIntDefault` fallbacks,
/// not the `.dist` file's explicit values, so an absent key behaves
/// the same as in C++.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct ServerConfig {
    /// `RealmID`; 0/missing fails boot (`Master.cpp:461-466`).
    pub realm_id: u32,
    /// `WorldServerPort` (default 8085).
    pub port: u16,
    /// `BindIP` (default `0.0.0.0`).
    pub bind_ip: IpAddr,
    /// `MaxOverspeedPings` (default 2; 0 disables; 1 clamps to 2,
    /// `World.cpp:703-708`).
    pub max_overspeed_pings: u32,
    /// `Network.KickOnBadPacket` (default false, `World.cpp:1055`).
    pub kick_on_bad_packet: bool,
    /// `Network.TimeoutSecsIfNoAuth` (default 10, 0 disables;
    /// `WorldSocket.cpp:594`).
    pub no_auth_timeout_secs: u32,
    /// `CharacterScreenMaxIdleTime` seconds (C++ code default 0 =
    /// disabled, `World.cpp:555`; the `.dist` file sets 900).
    pub character_screen_max_idle_secs: u32,
    /// Security level floor for auth, derived from `PlayerLimit`
    /// exactly like `World::GetPlayerSecurityLimit` (`World.h:705`):
    /// a non-positive limit `-n` demands security `n`; any positive
    /// limit is a plain player count and imposes no floor.
    /// `PlayerLimit` defaults to 100 (`SystemConfig.h:92`,
    /// `World.cpp:467`).
    pub player_security_limit: u8,
    /// `Antiflood.Sanction` raw bitmask (default
    /// `CHEAT_ACTION_KICK = 0x08`, `World.cpp:804`,
    /// `Anticheat.h:34-41`). Phase 3 honors the kick bit and logs
    /// the rest; the full anticheat action lands later.
    pub antiflood_sanction: u32,
    /// Raw `PlayerLimit` (default 100): positive = admission
    /// cap (login queue beyond it), non-positive encodes the
    /// security floor in `player_security_limit`.
    pub player_limit: i32,
    /// `LoginQueue.GracePeriodSecs` (default 0, `World.cpp:554`).
    pub login_queue_grace_secs: u32,
    /// `RestrictUnverifiedAccounts` (default false,
    /// `World.cpp:614`); drives the trial billing flags.
    pub restrict_unverified_accounts: bool,
    /// `WorldAvailable` (default true, `World.cpp:609`); when
    /// false, plain players get `SMSG_CHARACTER_LOGIN_FAILED`.
    pub world_available: bool,
    /// `WowPatch` (default 0 = patch 1.2, clamped to 10 at the
    /// 1.12 gate, `World.cpp:461-465`).
    pub wow_patch: u32,
    /// `UpdateUptimeInterval` in minutes (default 10,
    /// `World.cpp:679`). Zero disables the periodic refresh
    /// (the C++ timer would fire every tick at 0; recorded
    /// deviation, dist default is 10).
    pub uptime_update_mins: u32,
    /// `Motd` base text; the patch banner is appended at load
    /// (`World.cpp:467`).
    pub motd: String,
    /// `GameType` (`RealmType`, default 0 = NORMAL,
    /// `World.cpp:582-583`). `FFA_PVP` (16) is published as `PVP` (1).
    pub game_type: u32,
    /// `RealmZone` (default 1 = DEVELOPMENT, `World.cpp:585-586`).
    pub realm_zone: u32,
    /// `AllowTwoSide.AddFriend` (default false,
    /// `World.cpp:599`).
    pub allow_two_side_add_friend: bool,
    /// `AllowTwoSide.WhoList` (default false, `World.cpp:598`).
    pub allow_two_side_who_list: bool,
    /// `AllowTwoSide.Interaction.Chat` (default false,
    /// `World.cpp:591`).
    pub allow_two_side_interaction_chat: bool,
    /// `AllowTwoSide.Interaction.Channel` (default false,
    /// `World.cpp:592`).
    pub allow_two_side_interaction_channel: bool,
    /// `AllowTwoSide.Interaction.Group` (default false,
    /// `World.cpp:593`).
    pub allow_two_side_interaction_group: bool,
    /// `AllowTwoSide.Interaction.Guild` (default false,
    /// `World.cpp:594`).
    pub allow_two_side_interaction_guild: bool,
    /// `Guild.EventLogRecordsCount` (default 100, min
    /// 100, `World.cpp:793`).
    pub guild_event_log_count: u32,
    /// `MinPetitionSigns` (default 9, clamped 0..=9,
    /// `World.cpp:647`): signatures that complete a
    /// guild charter petition.
    pub min_petition_signs: u32,
    /// `MinCharterName` (default 2, clamped 2..=24,
    /// `World.cpp:606`).
    pub min_charter_name: u32,
    /// `StrictCharterNames` (default 0, `World.cpp:602`).
    pub strict_charter_names: u32,
    /// `AllowTwoSide.Interaction.Mail` (default false,
    /// `World.cpp:597`).
    pub allow_two_side_interaction_mail: bool,
    /// `AllowTwoSide.Interaction.Trade` (default false,
    /// `World.cpp:595`).
    pub allow_two_side_interaction_trade: bool,
    /// `MailDeliveryDelay` seconds (default 3600,
    /// `World.cpp:672`).
    pub mail_delivery_delay_secs: u32,
    /// `GM.AllowTrades` (default true, `World.cpp:661`).
    pub gm_allow_trades: bool,
    /// `MailSpam.Level` (default 1, `World.cpp:1049`).
    pub mailspam_level: u32,
    /// `MailSpam.Money` (default 0, `World.cpp:1050`).
    pub mailspam_money: u32,
    /// `MailSpam.Item` (default false, `World.cpp:1051`).
    pub mailspam_item: bool,
    /// `MailSpam.ExpireSecs` (default 0, `World.cpp:1047`).
    pub mailspam_expire_secs: u32,
    /// `MailSpam.MaxMails` (default 2, `World.cpp:1048`).
    pub mailspam_max_mails: u32,
    /// `Mails.COD.ForceTag.MaxLevel` (default 0,
    /// `World.cpp:1052`).
    pub cod_force_tag_max_level: u32,
    /// `AllowTwoSide.Interaction.Auction` (default
    /// false, `World.cpp:596`).
    pub allow_two_side_interaction_auction: bool,
    /// `Progression.UnlinkedAuctionHouses` (default
    /// true, `World.cpp:1084`): separate house per
    /// DBC entry before patch 1.9.
    pub unlinked_auction_houses: bool,
    /// `Rate.Auction.Time` (default 1.0, `World.cpp:516`).
    pub rate_auction_time: f32,
    /// `Rate.Auction.Deposit` (default 1.0, `World.cpp:517`).
    pub rate_auction_deposit: f32,
    /// `Rate.Auction.Cut` (default 1.0, `World.cpp:518`).
    pub rate_auction_cut: f32,
    /// `Instance.IgnoreLevel`.
    pub instance_ignore_level: bool,
    /// `Rate.InstanceResetTime`.
    pub rate_instance_reset_time: f32,
    /// `Rate.XP.Kill` (default 1.0, `World.cpp:506`;
    /// production conf runs 1.4).
    pub rate_xp_kill: f32,
    /// `Rate.XP.Kill.Elite` (default 1.0, `World.cpp:507`).
    pub rate_xp_kill_elite: f32,
    /// `Rate.XP.Quest` (default 1.0, `World.cpp:508`).
    pub rate_xp_quest: f32,
    /// `Rate.XP.Explore` (default 1.0, `World.cpp:509`).
    pub rate_xp_explore: f32,
    /// `Spell.ProcDelay` batching window in ms (fork
    /// code default 400; example conf ships 800).
    pub spell_proc_delay_ms: u32,
    /// `Anticheat.*` movement-anticheat family.
    pub anticheat: crate::anticheat::AnticheatConfig,
    /// `Auction.Deposit.Min` (default 0, `World.cpp:519`).
    pub auction_deposit_min: u32,
    /// `Auction.AccountConcurrentLimit` (default 0 =
    /// unlimited, `World.cpp:520`).
    pub auction_account_concurrent_limit: u32,
    /// `PlayerCommands` (default true,
    /// `World.cpp:789`): whether player sessions
    /// may use dot commands at all.
    pub player_commands: bool,
    /// `GM.JoinOppositeFactionChannels` (default false,
    /// forced false when two-side chat is on,
    /// `World.cpp:663-665`).
    pub gm_join_opposite_faction_channels: bool,
    /// `Channel.SilentlyGMJoin` (default false,
    /// `World.cpp:734`).
    pub channel_silently_gm_join: bool,
    /// `ChannelInvite.MinLevel` (default 10,
    /// `World.cpp:1016`).
    pub channel_invite_min_level: u32,
    /// `WorldChan.MinLevel` (default 0, `World.cpp:1018`).
    pub world_chan_min_level: u32,
    /// `ListenRange.Say` (default 25, `World.cpp:538`).
    pub listen_range_say: f32,
    /// `ListenRange.Yell` (default 300, `World.cpp:539`).
    pub listen_range_yell: f32,
    /// `ListenRange.TextEmote` (default 25, `World.cpp:540`).
    pub listen_range_text_emote: f32,
    /// `YellRange.LinearScale.MaxLevel` (default 0 = off).
    pub yell_range_linear_scale_max_level: u32,
    /// `YellRange.QuadraticScale.MaxLevel` (default 0 = off).
    pub yell_range_quadratic_scale_max_level: u32,
    /// `YellRange.Min` (default 0).
    pub yell_range_min: u32,
    /// `GM.InWhoList.Level` (default `SEC_ADMINISTRATOR`,
    /// `World.cpp:656`): accounts above this security are
    /// hidden from player who lists.
    pub gm_level_in_who_list: u8,
    /// `Item.PreventDataMining` (default true).
    pub prevent_item_datamining: bool,
    /// `ActivateWeather` (default true).
    pub activate_weather: bool,
    /// `ChangeWeatherInterval` in milliseconds (default
    /// 600000 = 10 minutes).
    pub change_weather_interval_ms: u32,
    /// `BotChat.*` (fork `BotChatManager::Load`).
    pub bot_chat: mangos_bots::chat::ChatConfig,
    /// `BotService.Enable` (default true).
    pub bot_service_enable: bool,
    /// `BotService.Buy.BudgetPerDay` gold (default 200).
    pub bot_service_buy_budget_gold: u32,
    /// `RandomBot.*` / `PlayerBot.*` population config.
    pub bot_population: mangos_bots::population::PopulationConfig,
    /// `BotDirector.Enable` (default true).
    pub bot_director_enable: bool,
    /// `BotDirector.UpdateInterval` ms (default 120000).
    pub bot_director_interval_ms: u32,
    /// `GuildBot.*` config.
    pub guild_bot: mangos_bots::guild_bots::GuildBotConfig,
    /// `AHBot.*` config.
    pub ah_bot: mangos_bots::ah_bot::AhBotConfig,
    /// `PlayerSave.Interval` milliseconds (default
    /// 15 minutes, `World.cpp:565`).
    pub player_save_interval_ms: u32,
    /// `PlayerSave.Stats.MinLevel` (default 0,
    /// clamped from the C++ config range).
    pub player_save_stats_min_level: u32,
    /// `PlayerSave.Stats.SaveOnlyOnLogout`
    /// (default true, `World.cpp:567`).
    pub player_save_stats_only_on_logout: bool,
    /// `PlayerBot.AllowSaving` (default false,
    /// `PlayerBotMgr.cpp:56`).
    pub player_bot_allow_saving: bool,
}

fn bot_chat_config(cfg: &Config) -> mangos_bots::chat::ChatConfig {
    mangos_bots::chat::ChatConfig {
        enabled: cfg.bool_default("BotChat.Enable", true),
        trade_interval_min_s: to_u32(cfg.int_default("BotChat.TradeChat.IntervalMin", 120)),
        trade_interval_max_s: to_u32(cfg.int_default("BotChat.TradeChat.IntervalMax", 300)),
        guild_interval_min_s: to_u32(cfg.int_default("BotChat.GuildChat.IntervalMin", 300)),
        guild_interval_max_s: to_u32(cfg.int_default("BotChat.GuildChat.IntervalMax", 900)),
        cooldown_trade_s: to_u32(cfg.int_default("BotChat.Cooldown.TradeChat", 300)),
        cooldown_guild_s: to_u32(cfg.int_default("BotChat.Cooldown.GuildChat", 180)),
        cooldown_party_s: to_u32(cfg.int_default("BotChat.Cooldown.PartyChat", 30)),
        cooldown_say_s: to_u32(cfg.int_default("BotChat.Cooldown.SayChat", 120)),
        cooldown_general_s: to_u32(cfg.int_default("BotChat.Cooldown.GeneralChat", 120)),
        cooldown_scale_threshold: to_u32(cfg.int_default("BotChat.Cooldown.ScaleThreshold", 30)),
    }
}

impl ServerConfig {
    #[must_use]
    // Flat key-by-key config mirror; length is data, not logic.
    #[allow(clippy::too_many_lines)]
    pub fn from_config(cfg: &Config) -> Self {
        let mut max_overspeed_pings = to_u32(cfg.int_default("MaxOverspeedPings", 2));
        if max_overspeed_pings == 1 {
            max_overspeed_pings = 2;
        }
        let bind_ip: IpAddr = cfg
            .string_default("BindIP", "0.0.0.0")
            .parse()
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let player_limit = cfg.int_default("PlayerLimit", 100);
        let player_security_limit = if player_limit <= 0 {
            i64::from(-player_limit).min(i64::from(security::ADMINISTRATOR)) as u8
        } else {
            security::PLAYER
        };
        ServerConfig {
            realm_id: to_u32(cfg.int_default("RealmID", 0)),
            port: to_u32(cfg.int_default("WorldServerPort", 8085)) as u16,
            bind_ip,
            max_overspeed_pings,
            kick_on_bad_packet: cfg.bool_default("Network.KickOnBadPacket", false),
            no_auth_timeout_secs: to_u32(cfg.int_default("Network.TimeoutSecsIfNoAuth", 10)),
            character_screen_max_idle_secs: to_u32(
                cfg.int_default("CharacterScreenMaxIdleTime", 0),
            ),
            player_security_limit,
            antiflood_sanction: to_u32(cfg.int_default("Antiflood.Sanction", 8)),
            player_limit,
            login_queue_grace_secs: to_u32(cfg.int_default("LoginQueue.GracePeriodSecs", 0)),
            restrict_unverified_accounts: cfg.bool_default("RestrictUnverifiedAccounts", false),
            world_available: cfg.bool_default("WorldAvailable", true),
            wow_patch: to_u32(cfg.int_default("WowPatch", 0)).min(10),
            uptime_update_mins: to_u32(cfg.int_default("UpdateUptimeInterval", 10)),
            motd: cfg.string_default("Motd", "Welcome to the Massive Network Game Object Server."),
            game_type: to_u32(cfg.int_default("GameType", 0)),
            realm_zone: to_u32(cfg.int_default("RealmZone", 1)),
            allow_two_side_add_friend: cfg.bool_default("AllowTwoSide.AddFriend", false),
            allow_two_side_who_list: cfg.bool_default("AllowTwoSide.WhoList", false),
            allow_two_side_interaction_chat: cfg
                .bool_default("AllowTwoSide.Interaction.Chat", false),
            allow_two_side_interaction_channel: cfg
                .bool_default("AllowTwoSide.Interaction.Channel", false),
            allow_two_side_interaction_group: cfg
                .bool_default("AllowTwoSide.Interaction.Group", false),
            allow_two_side_interaction_guild: cfg
                .bool_default("AllowTwoSide.Interaction.Guild", false),
            guild_event_log_count: u32::try_from(
                cfg.int_default("Guild.EventLogRecordsCount", 100).max(100),
            )
            .unwrap_or(100),
            min_petition_signs: to_u32(cfg.int_default("MinPetitionSigns", 9)).min(9),
            min_charter_name: to_u32(cfg.int_default("MinCharterName", 2)).clamp(2, 24),
            strict_charter_names: to_u32(cfg.int_default("StrictCharterNames", 0)),
            allow_two_side_interaction_mail: cfg
                .bool_default("AllowTwoSide.Interaction.Mail", false),
            allow_two_side_interaction_trade: cfg
                .bool_default("AllowTwoSide.Interaction.Trade", false),
            mail_delivery_delay_secs: to_u32(cfg.int_default("MailDeliveryDelay", 3600)),
            gm_allow_trades: cfg.bool_default("GM.AllowTrades", true),
            mailspam_level: to_u32(cfg.int_default("MailSpam.Level", 1)),
            mailspam_money: to_u32(cfg.int_default("MailSpam.Money", 0)),
            mailspam_item: cfg.bool_default("MailSpam.Item", false),
            mailspam_expire_secs: to_u32(cfg.int_default("MailSpam.ExpireSecs", 0)),
            mailspam_max_mails: to_u32(cfg.int_default("MailSpam.MaxMails", 2)),
            cod_force_tag_max_level: to_u32(cfg.int_default("Mails.COD.ForceTag.MaxLevel", 0)),
            allow_two_side_interaction_auction: cfg
                .bool_default("AllowTwoSide.Interaction.Auction", false),
            unlinked_auction_houses: cfg.bool_default("Progression.UnlinkedAuctionHouses", true),
            rate_auction_time: cfg.float_default("Rate.Auction.Time", 1.0),
            rate_auction_deposit: cfg.float_default("Rate.Auction.Deposit", 1.0),
            rate_auction_cut: cfg.float_default("Rate.Auction.Cut", 1.0),
            rate_instance_reset_time: cfg.float_default("Rate.InstanceResetTime", 1.0),
            rate_xp_kill: cfg.float_default("Rate.XP.Kill", 1.0),
            rate_xp_kill_elite: cfg.float_default("Rate.XP.Kill.Elite", 1.0),
            rate_xp_quest: cfg.float_default("Rate.XP.Quest", 1.0),
            rate_xp_explore: cfg.float_default("Rate.XP.Explore", 1.0),
            spell_proc_delay_ms: to_u32(cfg.int_default("Spell.ProcDelay", 400)),
            anticheat: crate::anticheat::AnticheatConfig::from_config(cfg),
            instance_ignore_level: cfg.bool_default("Instance.IgnoreLevel", false),
            auction_deposit_min: to_u32(cfg.int_default("Auction.Deposit.Min", 0)),
            auction_account_concurrent_limit: to_u32(
                cfg.int_default("Auction.AccountConcurrentLimit", 0),
            ),
            player_commands: cfg.bool_default("PlayerCommands", true),
            gm_join_opposite_faction_channels: cfg
                .bool_default("GM.JoinOppositeFactionChannels", false)
                && !cfg.bool_default("AllowTwoSide.Interaction.Chat", false),
            channel_silently_gm_join: cfg.bool_default("Channel.SilentlyGMJoin", false),
            channel_invite_min_level: to_u32(cfg.int_default("ChannelInvite.MinLevel", 10)),
            world_chan_min_level: to_u32(cfg.int_default("WorldChan.MinLevel", 0)),
            listen_range_say: cfg.float_default("ListenRange.Say", 25.0),
            listen_range_yell: cfg.float_default("ListenRange.Yell", 300.0),
            listen_range_text_emote: cfg.float_default("ListenRange.TextEmote", 25.0),
            yell_range_linear_scale_max_level: to_u32(
                cfg.int_default("YellRange.LinearScale.MaxLevel", 0),
            ),
            yell_range_quadratic_scale_max_level: to_u32(
                cfg.int_default("YellRange.QuadraticScale.MaxLevel", 0),
            ),
            yell_range_min: to_u32(cfg.int_default("YellRange.Min", 0)),
            gm_level_in_who_list: to_u32(
                cfg.int_default("GM.InWhoList.Level", i32::from(security::ADMINISTRATOR)),
            ) as u8,
            prevent_item_datamining: cfg.bool_default("Item.PreventDataMining", true),
            activate_weather: cfg.bool_default("ActivateWeather", true),
            change_weather_interval_ms: to_u32(cfg.int_default("ChangeWeatherInterval", 600_000)),
            bot_chat: bot_chat_config(cfg),
            bot_population: mangos_bots::population::PopulationConfig {
                enable_random_bots: cfg.bool_default("RandomBot.Enable", false),
                min_bots: to_u32(cfg.int_default("RandomBot.MinBots", 3)),
                max_bots: to_u32(cfg.int_default("RandomBot.MaxBots", 10)),
                refresh_ms: u64::from(to_u32(cfg.int_default("RandomBot.Refresh", 60_000))),
                update_ms: u64::from(to_u32(cfg.int_default("PlayerBot.UpdateMs", 10_000))),
                debug: cfg.bool_default("PlayerBot.Debug", false),
                peak_hour: to_u32(cfg.int_default("RandomBot.PeakHour", 20)),
                off_peak_percent: to_u32(cfg.int_default("RandomBot.OffPeakPercent", 50)),
                timezone_offset: cfg.int_default("RandomBot.TimezoneOffset", -4),
            },
            ah_bot: mangos_bots::ah_bot::AhBotConfig {
                enable: cfg.bool_default("AHBot.Enable", false),
                ah_fid: to_u32(cfg.int_default("AHBot.ah.fid", 120)),
                item_count: to_u32(cfg.int_default("AHBot.itemcount", 200)),
                refresh_interval_secs: u64::from(to_u32(
                    cfg.int_default("AHBot.RefreshInterval", 259_200),
                )),
                seller_account_id: to_u32(cfg.int_default("AHBot.SellerAccountId", 99)),
                seller_count: to_u32(cfg.int_default("AHBot.SellerCount", 12)),
                quota_min: [
                    to_u32(cfg.int_default("AHBot.Quota.TradeGoods.Min", 35)),
                    to_u32(cfg.int_default("AHBot.Quota.Crafting.Min", 15)),
                    to_u32(cfg.int_default("AHBot.Quota.Consumables.Min", 20)),
                    to_u32(cfg.int_default("AHBot.Quota.Recipes.Min", 3)),
                    to_u32(cfg.int_default("AHBot.Quota.Equipment.Min", 5)),
                    to_u32(cfg.int_default("AHBot.Quota.Leveling.Min", 10)),
                ],
                quota_max: [
                    to_u32(cfg.int_default("AHBot.Quota.TradeGoods.Max", 40)),
                    to_u32(cfg.int_default("AHBot.Quota.Crafting.Max", 20)),
                    to_u32(cfg.int_default("AHBot.Quota.Consumables.Max", 25)),
                    to_u32(cfg.int_default("AHBot.Quota.Recipes.Max", 5)),
                    to_u32(cfg.int_default("AHBot.Quota.Equipment.Max", 10)),
                    to_u32(cfg.int_default("AHBot.Quota.Leveling.Max", 15)),
                ],
                buyer_interval_secs: u64::from(to_u32(
                    cfg.int_default("AHBot.Buyer.IntervalSecs", 1800),
                )),
                buyer_budget_per_cycle: to_u32(
                    cfg.int_default("AHBot.Buyer.BudgetPerCycle", 500_000),
                ),
            },
            guild_bot: mangos_bots::guild_bots::GuildBotConfig {
                enabled: cfg.bool_default("GuildBot.Enable", true),
                account_id: to_u32(cfg.int_default("GuildBot.AccountId", 98)),
                presence_enabled: cfg.bool_default("GuildBot.Presence.Enable", true),
                presence_check_interval_ms: u64::from(to_u32(
                    cfg.int_default("GuildBot.Presence.CheckInterval", 300),
                )) * 1000,
                presence_peak_hour: to_u32(cfg.int_default("GuildBot.Presence.PeakHour", 20)),
                presence_peak_count: to_u32(cfg.int_default("GuildBot.Presence.PeakCount", 14)),
                presence_off_peak_count: to_u32(
                    cfg.int_default("GuildBot.Presence.OffPeakCount", 3),
                ),
                presence_timezone_offset: cfg.int_default("GuildBot.Presence.TimezoneOffset", 0),
            },
            bot_director_enable: cfg.bool_default("BotDirector.Enable", true),
            bot_director_interval_ms: to_u32(
                cfg.int_default("BotDirector.UpdateInterval", 120_000),
            ),
            bot_service_enable: cfg.bool_default("BotService.Enable", true),
            bot_service_buy_budget_gold: to_u32(
                cfg.int_default("BotService.Buy.BudgetPerDay", 200),
            ),
            player_save_interval_ms: to_u32(cfg.int_default("PlayerSave.Interval", 900_000)),
            player_save_stats_min_level: to_u32(cfg.int_default("PlayerSave.Stats.MinLevel", 0)),
            player_save_stats_only_on_logout: cfg
                .bool_default("PlayerSave.Stats.SaveOnlyOnLogout", true),
            player_bot_allow_saving: cfg.bool_default("PlayerBot.AllowSaving", false),
        }
    }
}

/// C++ int configs are reinterpreted as unsigned at use sites;
/// negative values wrap. Centralised so the cast is reviewed once.
const fn to_u32(value: i32) -> u32 {
    value as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(text: &str) -> Config {
        Config::parse(text.as_bytes())
    }

    #[test]
    fn defaults_match_cpp_code() {
        let s = ServerConfig::from_config(&cfg("RealmID = 1\n"));
        assert_eq!(s.realm_id, 1);
        assert_eq!(s.port, 8085);
        assert_eq!(s.max_overspeed_pings, 2);
        assert!(!s.kick_on_bad_packet);
        assert_eq!(s.no_auth_timeout_secs, 10);
        assert_eq!(s.character_screen_max_idle_secs, 0);
        assert_eq!(s.player_security_limit, security::PLAYER);
    }

    #[test]
    fn overspeed_clamp_and_security_floor() {
        let s = ServerConfig::from_config(&cfg("MaxOverspeedPings = 1\nPlayerLimit = -3\n"));
        assert_eq!(s.max_overspeed_pings, 2);
        assert_eq!(s.player_security_limit, 3);
        let s = ServerConfig::from_config(&cfg("MaxOverspeedPings = 0\nPlayerLimit = 50\n"));
        assert_eq!(s.max_overspeed_pings, 0);
        assert_eq!(s.player_security_limit, security::PLAYER);
    }

    #[test]
    fn accepted_builds() {
        assert!(is_acceptable_client_build(5875));
        assert!(is_acceptable_client_build(6141));
        assert!(!is_acceptable_client_build(5464));
        assert!(!is_acceptable_client_build(0));
    }
}
