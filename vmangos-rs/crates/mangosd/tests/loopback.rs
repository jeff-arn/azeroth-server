//! Loopback proof of the Phase 3 socket/world scaffold: a
//! scripted 1.12.1-style client performs the challenge/auth
//! handshake against `run_connection` + the world actor with an
//! in-memory fake backend. The test client implements the client
//! side of `AuthCrypt` by hand (6-byte send / 4-byte receive), so
//! a green run also cross-validates the server crypt direction.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration, timeout};

use mangos_db::DbError;
use mangos_log::Log;
use mangos_protocol::world::{
    auth_response, expected_auth_session_digest, opcode, session_key_from_hex,
};

use mangos_db::character_load::{CharacterLoad, CharacterRow, SocialRow, SpellRow};
use mangos_db::characters::{AccountDataEntry, CharEnumRow, NameQueryRow};
use mangos_world_data::enum_stores::{EnumStores, ItemDisplay};

use mangosd::ServerConfig;
use mangosd::auth::{AuthBackend, WorldAuthAccount};
use mangosd::char_queries::CharQueries;
use mangosd::group::GroupWrite;
use mangosd::guild::GuildWrite;
use mangosd::social::SocialWrite;
use mangosd::socket::{ConnectionDeps, run_connection};
use mangosd::world_actor::{
    AuctionStore, GroupStore, GuildStore, MailStore, PetitionStore, PresenceWriter, RealmPublisher,
    SessionId, SocialStore, TradeStore, WorldDeps, spawn_world,
};

const SESSIONKEY_HEX: &str = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";
const CLIENT_SEED: u32 = 0x1122_3344;

#[derive(Default)]
struct FakeBackend {
    accounts: HashMap<String, WorldAuthAccount>,
    last_ip_writes: Mutex<Vec<(String, String)>>,
}

impl FakeBackend {
    fn with_test_user() -> Arc<Self> {
        let mut accounts = HashMap::new();
        accounts.insert(
            "TESTUSER".to_string(),
            WorldAuthAccount {
                id: 42,
                security: None,
                sessionkey_hex: SESSIONKEY_HEX.to_string(),
                last_ip: "127.0.0.1".to_string(),
                mutetime: 0,
                locale: 0,
                os: "Win".to_string(),
                platform: "x86".to_string(),
                flags: 0,
                email: String::new(),
                email_verif: false,
                banned: false,
            },
        );
        Arc::new(FakeBackend {
            accounts,
            last_ip_writes: Mutex::new(Vec::new()),
        })
    }

    fn writes(&self) -> Vec<(String, String)> {
        self.last_ip_writes.lock().expect("mutex").clone()
    }
}

impl AuthBackend for FakeBackend {
    fn lookup_world_account<'a>(
        &'a self,
        _realm_id: u32,
        account: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<WorldAuthAccount>, DbError>> + Send + 'a>> {
        let row = self.accounts.get(account).cloned();
        Box::pin(async move { Ok(row) })
    }

    fn update_last_ip<'a>(
        &'a self,
        account: &'a str,
        ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DbError>> + Send + 'a>> {
        self.last_ip_writes
            .lock()
            .expect("mutex")
            .push((account.to_string(), ip.to_string()));
        Box::pin(async { Ok(()) })
    }

    fn is_ip_banned<'a>(
        &'a self,
        _ip: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

struct FakeCharQueries;

impl CharQueries for FakeCharQueries {
    fn char_enum(&self, account_id: u32) -> Result<Vec<CharEnumRow>, DbError> {
        assert_eq!(account_id, 42);
        Ok(vec![CharEnumRow {
            guid: 7,
            name: "Enumtest".to_string(),
            race: 1,
            class: 1,
            gender: 0,
            skin: 1,
            face: 2,
            hair_style: 3,
            hair_color: 4,
            facial_hair: 5,
            level: 12,
            zone: 12,
            map: 0,
            position_x: -8949.95,
            position_y: -132.493,
            position_z: 83.5312,
            guild_id: None,
            character_flags: 0,
            played_time_total: 60,
            pet_entry: None,
            pet_display_id: None,
            pet_level: None,
            equipment_cache: Some("25 0".to_string()),
        }])
    }

    fn load_character(&self, guid_low: u32) -> Result<Option<CharacterLoad>, DbError> {
        assert_eq!(guid_low, 7);
        Ok(Some(CharacterLoad {
            row: CharacterRow {
                guid: 7,
                account: 42,
                name: "Enumtest".into(),
                race: 1,
                class: 1,
                gender: 0,
                level: 12,
                xp: 0,
                money: 0,
                skin: 1,
                face: 2,
                hair_style: 3,
                hair_color: 4,
                facial_hair: 5,
                bank_bag_slots: 0,
                character_flags: 0,
                position_x: -8949.95,
                position_y: -132.493,
                position_z: 83.5312,
                map: 0,
                orientation: 1.5,
                known_taxi_mask: String::new(),
                played_time_total: 60,
                played_time_level: 60,
                rest_bonus: 0.0,
                logout_time: 0,
                reset_talents_multiplier: 0,
                reset_talents_time: 0,
                transport_guid: 0,
                transport_x: 0.0,
                transport_y: 0.0,
                transport_z: 0.0,
                transport_o: 0.0,
                extra_flags: 0,
                stable_slots: 0,
                death_expire_time: 0,
                current_taxi_path: String::new(),
                honor_rank_points: 0.0,
                honor_highest_rank: 0,
                honor_standing: 0,
                honor_last_week_hk: 0,
                honor_last_week_cp: 0.0,
                honor_stored_hk: 0,
                honor_stored_dk: 0,
                watched_faction: -1,
                drunk: 0,
                health: 100,
                power1: 0,
                power2: 0,
                power3: 0,
                power4: 0,
                power5: 0,
                explored_zones: String::new(),
                ammo_id: 0,
                action_bars: 0,
                world_phase_mask: 0,
                create_time: 0,
                instance: 0,
                zone: 0,
            },
            group_id: None,
            bound_instances: vec![],
            auras: vec![],
            // Language: Common (668), as creation would grant.
            spells: vec![SpellRow {
                spell: 668,
                active: 1,
                disabled: 0,
            }],
            quest_status: vec![],
            honor_cp: vec![],
            reputation: vec![],
            inventory: vec![],
            item_loot: vec![],
            actions: vec![],
            social: vec![SocialRow {
                friend: 9,
                flags: 0x01,
            }],
            homebind: None,
            spell_cooldowns: vec![],
            guild: Some(mangos_db::character_load::GuildRow {
                guild_id: 1,
                rank: 0,
            }),
            bg_data: None,
            account_data: vec![],
            skills: vec![],
            mails: loopback_mail_seed(),
            mailed_items: vec![],
            forgotten_skills: vec![],
            pets: Vec::new(),
        }))
    }

    fn name_query(&self, guid_low: u32) -> Result<Option<NameQueryRow>, DbError> {
        Ok((guid_low == 9).then(|| NameQueryRow {
            name: "Offlinefriend".to_string(),
            race: 4,
            gender: 1,
            class: 5,
        }))
    }

    fn global_account_data(&self, _account_id: u32) -> Result<Vec<AccountDataEntry>, DbError> {
        Ok(Vec::new())
    }

    fn tutorials(&self, _account_id: u32) -> Result<Option<[u32; 8]>, DbError> {
        Ok(Some([0x0000_0002, 0, 0, 0, 0, 0, 0, 0x8000_0000]))
    }
}

struct FakePlayerbotStore;

impl mangosd::world_actor::PlayerbotStore for FakePlayerbotStore {
    fn update_chances(&self, _chances: &[(u32, u32)]) -> Result<(), mangos_db::DbError> {
        Ok(())
    }
}

struct FakeInstanceStore;

impl mangosd::world_actor::InstanceStore for FakeInstanceStore {
    fn insert_instance(
        &self,
        _instance_id: u32,
        _map_id: u32,
        _reset_time: i64,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn insert_character_bind(
        &self,
        _guid_low: u32,
        _instance_id: u32,
        _permanent: bool,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn update_character_bind(
        &self,
        _guid_low: u32,
        _old_instance_id: u32,
        _new_instance_id: u32,
        _permanent: bool,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn delete_character_bind(&self, _guid_low: u32, _instance_id: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn insert_group_bind(
        &self,
        _leader_low: u32,
        _instance_id: u32,
        _permanent: bool,
    ) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeAhBotStore;

impl mangosd::world_actor::AhBotStore for FakeAhBotStore {
    fn load_ah_bot(
        &self,
        _seller_account_id: u32,
        _seller_count: u32,
    ) -> Result<mangosd::world_actor::InitialAhBot, DbError> {
        Ok(mangosd::world_actor::InitialAhBot::default())
    }

    fn save_refresh_time(&self, _now: i64) -> Result<(), DbError> {
        Ok(())
    }

    fn create_ahbot_item(&self, _item_id: u32, _count: u32) -> Result<u32, DbError> {
        Ok(0)
    }

    fn delete_item_instance(&self, _item_guid: u32) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeGuildBotStore;

impl mangosd::world_actor::GuildBotStore for FakeGuildBotStore {
    fn character_level(&self, _guid_low: u32) -> Result<Option<u32>, mangos_db::DbError> {
        Ok(None)
    }

    fn set_character_level(&self, _guid_low: u32, _level: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }
}

struct FakeSocialStore;

impl SocialStore for FakeSocialStore {
    fn player_data_by_name(
        &self,
        name: &str,
    ) -> Result<Option<mangos_db::social::NamedPlayerData>, DbError> {
        Ok(match name {
            "Enumtest" => Some(mangos_db::social::NamedPlayerData {
                guid_low: 7,
                race: 1,
            }),
            "Offlinefriend" => Some(mangos_db::social::NamedPlayerData {
                guid_low: 9,
                race: 4,
            }),
            "Orcfriend" => Some(mangos_db::social::NamedPlayerData {
                guid_low: 10,
                race: 2,
            }),
            _ => None,
        })
    }

    fn apply_social_write(
        &self,
        _guid_low: u32,
        _friend_low: u32,
        _write: SocialWrite,
    ) -> Result<(), DbError> {
        Ok(())
    }
}

/// One boot-loaded guild led by Enumtest (the fork
/// creates guilds via charter petitions; the seed
/// stands in for a turned-in charter).
fn loopback_guild_seed() -> mangos_db::guild::LoadedGuilds {
    use mangos_db::guild::{
        LoadedGuild, LoadedGuildMember, LoadedGuilds, StoredGuild, StoredGuildRank,
    };
    LoadedGuilds {
        guilds: vec![LoadedGuild {
            guild: StoredGuild {
                guild_id: 1,
                name: "Loopback Guild".into(),
                leader_low: 7,
                emblem_style: -1,
                emblem_color: -1,
                border_style: -1,
                border_color: -1,
                background_color: -1,
                info: String::new(),
                motd: "No message set.".into(),
                create_date: 0,
            },
            ranks: [
                ("Guild Master", 0x000F_F1FFu32),
                ("Officer", 0x000F_F1FF),
                ("Veteran", 0x43),
                ("Member", 0x43),
                ("Initiate", 0x43),
            ]
            .iter()
            .enumerate()
            .map(|(id, (name, rights))| StoredGuildRank {
                rank_id: id as u32,
                name: (*name).into(),
                rights: *rights,
            })
            .collect(),
            members: vec![LoadedGuildMember {
                guid_low: 7,
                rank_id: 0,
                pnote: String::new(),
                offnote: String::new(),
                name: "Enumtest".into(),
                level: 12,
                class: 1,
                zone_id: 0,
                logout_time: 0,
                account_id: 42,
            }],
            events: Vec::new(),
        }],
        next_guild_id: 2,
    }
}

struct FakeGuildStore;

impl GuildStore for FakeGuildStore {
    fn apply_guild_write(&self, _write: GuildWrite) -> Result<(), DbError> {
        Ok(())
    }

    fn guild_candidate(
        &self,
        _guid_low: u32,
    ) -> Result<Option<mangos_db::guild::GuildCandidate>, DbError> {
        Ok(None)
    }
}

/// One boot-loaded petition owned by Enumtest with a
/// signature from Offlinefriend; charter item 777 is
/// "carried" per the fake store below.
fn loopback_petition_seed() -> mangos_db::petition::LoadedPetitions {
    use mangos_db::petition::{LoadedPetitions, StoredPetition, StoredPetitionSign};
    LoadedPetitions {
        petitions: vec![StoredPetition {
            petition_id: 5,
            owner_low: 7,
            charter_low: 777,
            name: "Charter Pals".into(),
            owner_race: 1,
        }],
        signs: vec![StoredPetitionSign {
            petition_id: 5,
            player_low: 9,
            account_id: 99,
        }],
        next_petition_id: 6,
    }
}

struct FakePetStore;

impl mangosd::world_actor::PetStore for FakePetStore {
    fn save_pet(&self, _row: mangos_db::pet::PetRow, _mode: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }

    fn delete_pet(&self, _pet_id: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }

    fn promote_pet(&self, _owner_low: u32, _pet_id: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }

    fn rename_pet(
        &self,
        _owner_low: u32,
        _pet_id: u32,
        _name: String,
    ) -> Result<(), mangos_db::DbError> {
        Ok(())
    }
}

struct FakeQuestRewardStore;

impl mangosd::world_actor::QuestRewardStore for FakeQuestRewardStore {
    fn create_reward_item(
        &self,
        _owner_low: u32,
        _item_id: u32,
        _count: u32,
        _bag: u32,
        _slot: u32,
        _creator_low: u32,
    ) -> Result<u32, mangos_db::DbError> {
        Ok(990_001)
    }

    fn set_item_count(&self, _item_guid: u32, _count: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }

    fn set_item_charges(
        &self,
        _item_guid: u32,
        _charges: String,
    ) -> Result<(), mangos_db::DbError> {
        Ok(())
    }

    fn set_item_flags(&self, _item_guid: u32, _flags: u32) -> Result<(), mangos_db::DbError> {
        Ok(())
    }
}

struct FakePetitionStore;

impl PetitionStore for FakePetitionStore {
    fn apply_petition_write(
        &self,
        _write: mangosd::petition::PetitionWrite,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn create_charter_item(
        &self,
        _owner_low: u32,
        _petition_id: u32,
    ) -> Result<Option<u32>, DbError> {
        Ok(Some(778))
    }

    fn player_has_item(&self, owner_low: u32, item_low: u32) -> Result<bool, DbError> {
        Ok(owner_low == 7 && item_low == 777)
    }

    fn delete_charter_item(&self, _owner_low: u32, _item_low: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn is_reserved_name(&self, name: &str) -> Result<bool, DbError> {
        Ok(name == "Reservedname")
    }
}

/// Seeded mailbox for Enumtest: one delivered unread
/// mail with a body, one still in transit.
fn loopback_mail_seed() -> Vec<mangos_db::character_load::MailRow> {
    vec![
        // Delivered, unread, with body + money.
        mangos_db::character_load::MailRow {
            id: 41,
            message_type: 0,
            sender_guid: 9,
            receiver_guid: 7,
            subject: "care package".into(),
            item_text_id: 77,
            expire_time: 4_000_000_000,
            deliver_time: 1,
            money: 500,
            cod: 0,
            checked: 0x10,
            stationery: 41,
            mail_template_id: 0,
            has_items: 0,
        },
        // Still in transit.
        mangos_db::character_load::MailRow {
            id: 42,
            message_type: 0,
            sender_guid: 9,
            receiver_guid: 7,
            subject: "later".into(),
            item_text_id: 0,
            expire_time: 4_000_000_000,
            deliver_time: 3_999_000_000,
            money: 0,
            cod: 0,
            checked: 0x04,
            stationery: 41,
            mail_template_id: 0,
            has_items: 0,
        },
    ]
}

struct FakeAuctionStore;

impl AuctionStore for FakeAuctionStore {
    fn insert_auction(&self, _auction: mangos_db::auction::StoredAuction) -> Result<(), DbError> {
        Ok(())
    }

    fn delete_auction(&self, _auction_id: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn update_bid(
        &self,
        _auction_id: u32,
        _buyer_guid: u32,
        _last_bid: u32,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn detach_inventory_item(&self, _owner_low: u32, _item_low: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn set_item_owner(&self, _item_low: u32, _new_owner_low: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn delete_item_instance(&self, _item_low: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn account_id_by_guid(&self, _guid_low: u32) -> Result<Option<u32>, DbError> {
        Ok(None)
    }
}

struct FakeTradeStore;

impl TradeStore for FakeTradeStore {
    fn item_at_pos(
        &self,
        _owner_low: u32,
        _bag: u32,
        _slot: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        Ok(None)
    }

    fn inventory_item(
        &self,
        _owner_low: u32,
        _item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        Ok(None)
    }

    fn free_backpack_slots(&self, _owner_low: u32) -> Result<u32, DbError> {
        Ok(16)
    }

    fn first_free_backpack_slot(&self, _owner_low: u32) -> Result<Option<u32>, DbError> {
        Ok(Some(23))
    }

    fn move_item(
        &self,
        _from_low: u32,
        _to_low: u32,
        _item_low: u32,
        _item_id: u32,
        _to_slot: u32,
        _gift_creator_low: Option<u32>,
    ) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeMailStore;

impl MailStore for FakeMailStore {
    fn apply_mail_write(&self, _write: mangosd::mail::MailWrite) -> Result<(), DbError> {
        Ok(())
    }

    fn insert_mail(&self, _mail: mangos_db::mail::NewMail) -> Result<u32, DbError> {
        Ok(500)
    }

    fn create_item_text(&self, _text: &str) -> Result<u32, DbError> {
        Ok(78)
    }

    fn item_text(&self, item_text_id: u32) -> Result<String, DbError> {
        Ok(if item_text_id == 77 {
            "For the road.".to_string()
        } else {
            String::new()
        })
    }

    fn receiver_by_name(
        &self,
        _name: &str,
    ) -> Result<Option<mangos_db::mail::MailReceiverInfo>, DbError> {
        Ok(None)
    }

    fn mail_count(&self, _receiver_low: u32) -> Result<u32, DbError> {
        Ok(0)
    }

    fn account_id_by_guid(&self, _guid_low: u32) -> Result<Option<u32>, DbError> {
        Ok(None)
    }

    fn account_email_verified(&self, _account_id: u32) -> Result<Option<bool>, DbError> {
        Ok(Some(true))
    }

    fn inventory_item(
        &self,
        _owner_low: u32,
        _item_low: u32,
    ) -> Result<Option<mangos_db::mail::StoredInventoryItem>, DbError> {
        Ok(None)
    }

    fn store_mailed_item(
        &self,
        _owner_low: u32,
        _item_low: u32,
        _item_id: u32,
    ) -> Result<bool, DbError> {
        Ok(false)
    }

    fn create_text_item(
        &self,
        _owner_low: u32,
        _creator_low: u32,
        _item_text_id: u32,
    ) -> Result<bool, DbError> {
        Ok(false)
    }
}

struct FakeGroupStore;

impl GroupStore for FakeGroupStore {
    fn apply_group_write(&self, _write: GroupWrite) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeCharacterSaver;

impl mangosd::world_actor::CharacterSaver for FakeCharacterSaver {
    fn save_character(
        &self,
        _bundle: mangos_db::character_save::CharacterSaveBundle,
        _stats: Option<mangos_db::character_save::CharacterStatsRow>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn save_actions(
        &self,
        _guid_low: u32,
        _inserts: Vec<mangos_db::character_load::ActionRow>,
        _updates: Vec<mangos_db::character_load::ActionRow>,
        _deletes: Vec<u8>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn insert_homebind(
        &self,
        _guid_low: u32,
        _row: mangos_db::character_load::HomebindRow,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn update_homebind(
        &self,
        _guid_low: u32,
        _row: mangos_db::character_load::HomebindRow,
    ) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakePresence;

impl PresenceWriter for FakePresence {
    fn set_character_online(&self, _guid_low: u32, _online: bool) -> Result<(), DbError> {
        Ok(())
    }

    fn clear_creature_respawn(&self, _guid_low: u32, _instance_id: u32) -> Result<(), DbError> {
        Ok(())
    }
    fn store_creature_respawn(
        &self,
        _guid_low: u32,
        _respawn_time: u64,
        _map_id: u32,
        _instance_id: u32,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn clear_gameobject_respawn(&self, _guid_low: u32, _instance_id: u32) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeRealmPublisher;

impl RealmPublisher for FakeRealmPublisher {
    fn set_realm_population(&self, _realm_id: u32, _population: f32) -> Result<(), DbError> {
        Ok(())
    }
}

fn fake_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::chat_channels::{ChatChannelDbc, ChatChannelEntry};
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };
    let mut stores = mangos_map::WorldStores::empty();
    stores.chat_channels = ChatChannelDbc {
        entries: vec![ChatChannelEntry {
            channel_id: 1,
            flags: 0x3,
            patterns: vec!["General - %s".into()],
        }],
    };
    stores.player_infos = PlayerInfos::from_maps_for_tests(
        60,
        HashMap::from([(
            (1, 1),
            PlayerCreateInfo {
                map_id: 0,
                area_id: 12,
                position_x: -8949.95,
                position_y: -132.493,
                position_z: 83.5312,
                orientation: 1.5,
                display_id_m: 49,
                display_id_f: 50,
            },
        )]),
        HashMap::from([(
            1,
            vec![
                PlayerClassLevelInfo {
                    basehealth: 20,
                    basemana: 0
                };
                60
            ],
        )]),
        HashMap::from([((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60])]),
        vec![400; 61],
    );
    // Linen cloth for the trade gate checks.
    let linen = mangos_world_data::item_prototype::ItemPrototype {
        item_id: 2589,
        class: 7,
        display_id: 100,
        ..Default::default()
    };
    stores.item_prototypes =
        mangos_world_data::item_prototype::ItemPrototypes::from_entries_for_tests(HashMap::from([
            (2589, linen),
        ]));
    stores
}

fn fake_race_class_dbc() -> mangos_world_data::player_info::RaceClassDbc {
    use mangos_world_data::player_info::{RaceClassDbc, RaceDbc};
    let mut dbc = RaceClassDbc::default();
    dbc.races.insert(
        1,
        RaceDbc {
            faction_id: 1,
            model_m: 49,
            model_f: 50,
            team_id: 7,
        },
    );
    // Night elf (Offlinefriend): same Alliance team.
    dbc.races.insert(
        4,
        RaceDbc {
            faction_id: 4,
            model_m: 55,
            model_f: 56,
            team_id: 7,
        },
    );
    // Orc (Orcfriend): Horde team for the enemy check.
    dbc.races.insert(
        2,
        RaceDbc {
            faction_id: 2,
            model_m: 51,
            model_f: 52,
            team_id: 1,
        },
    );
    dbc.classes.insert(1);
    dbc.class_power.insert(1, 1);
    dbc
}

fn fake_enum_stores() -> EnumStores {
    EnumStores::from_parts(
        HashMap::from([(
            25,
            ItemDisplay {
                display_id: 1542,
                inventory_type: 21,
            },
        )]),
        HashMap::new(),
        HashSet::from([(1u8, 1u8)]),
    )
}

fn test_config() -> ServerConfig {
    ServerConfig::from_config(&mangos_config::Config::parse(
        b"RealmID = 1\nWorldServerPort = 0\n",
    ))
}

async fn start_server(backend: Arc<FakeBackend>) -> u16 {
    let log = Arc::new(
        Log::open(
            std::path::Path::new(""),
            "",
            false,
            mangos_log::Level::Debug,
        )
        .expect("disabled log"),
    );
    let char_queries: Arc<dyn CharQueries> = Arc::new(FakeCharQueries);
    let (world_tx, _world, _health) = spawn_world(
        test_config(),
        Arc::clone(&log),
        mangos_world_data::strings::MangosStrings::default(),
        WorldDeps {
            enum_stores: Arc::new(fake_enum_stores()),
            char_queries: Arc::clone(&char_queries),
            realm_publisher: Arc::new(FakeRealmPublisher),
            maps_dir: None,
            mmaps_dir: None,
            world_stores: Arc::new(fake_world_stores()),
            race_class_dbc: Arc::new(fake_race_class_dbc()),
            respawn_times: mangos_world_data::spawn_support::RespawnTimes::default(),
            presence: Arc::new(FakePresence),
            character_saver: Arc::new(FakeCharacterSaver),
            social: Arc::new(FakeSocialStore),
            group: Arc::new(FakeGroupStore),
            initial_groups: mangos_db::group::LoadedGroups::default(),
            guild: Arc::new(FakeGuildStore),
            initial_guilds: loopback_guild_seed(),
            petition: Arc::new(FakePetitionStore),
            initial_petitions: loopback_petition_seed(),
            mail: Arc::new(FakeMailStore),
            pets: Arc::new(FakePetStore),
            quest_reward: Arc::new(FakeQuestRewardStore),
            trade: Arc::new(FakeTradeStore),
            auction: Arc::new(FakeAuctionStore),
            initial_auctions: mangos_db::auction::LoadedAuctions::default(),
            initial_commands: mangosd::commands::CommandTable::new(),
            playerbot: Arc::new(FakePlayerbotStore),
            initial_playerbots: Vec::new(),
            guild_bot: Arc::new(FakeGuildBotStore),
            ah_bot: Arc::new(FakeAhBotStore),
            initial_ah_bot: mangosd::world_actor::InitialAhBot::default(),
            instance: Arc::new(FakeInstanceStore),
            initial_instances: mangos_db::instance::LoadedInstances::default(),
            initial_guild_bots: mangos_db::guild_bots::LoadedGuildBots::default(),
        },
    );
    let deps = Arc::new(ConnectionDeps {
        config: Arc::new(test_config()),
        backend,
        char_queries,
        world: world_tx,
        log,
        server_addresses: Arc::new(HashSet::from(["127.0.0.1".to_string()])),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let mut next_id = 1u64;
        while let Ok((stream, _)) = listener.accept().await {
            let deps = Arc::clone(&deps);
            let id = SessionId(next_id);
            next_id += 1;
            tokio::spawn(run_connection(stream, id, deps));
        }
    });
    port
}

/// Client-side `AuthCrypt` (inverse orientation of the server's):
/// send encrypts 6 header bytes, receive decrypts 4.
struct ClientCrypt {
    key: Vec<u8>,
    send_i: usize,
    send_j: u8,
    recv_i: usize,
    recv_j: u8,
    active: bool,
}

impl ClientCrypt {
    fn new() -> Self {
        ClientCrypt {
            key: Vec::new(),
            send_i: 0,
            send_j: 0,
            recv_i: 0,
            recv_j: 0,
            active: false,
        }
    }

    fn init(&mut self, sessionkey_hex: &str) {
        let key = session_key_from_hex(sessionkey_hex).expect("key");
        self.key = key.to_bytes_le();
        self.active = true;
    }

    fn encrypt_send(&mut self, data: &mut [u8]) {
        if !self.active {
            return;
        }
        for byte in data.iter_mut() {
            self.send_i %= self.key.len();
            let k = self.key[self.send_i];
            self.send_i += 1;
            let cipher = (*byte ^ k).wrapping_add(self.send_j);
            *byte = cipher;
            self.send_j = cipher;
        }
    }

    fn decrypt_recv(&mut self, data: &mut [u8]) {
        if !self.active {
            return;
        }
        for byte in data.iter_mut() {
            self.recv_i %= self.key.len();
            let k = self.key[self.recv_i];
            self.recv_i += 1;
            let cipher = *byte;
            *byte = cipher.wrapping_sub(self.recv_j) ^ k;
            self.recv_j = cipher;
        }
    }
}

struct Client {
    stream: TcpStream,
    crypt: ClientCrypt,
}

impl Client {
    async fn connect(port: u16) -> (Self, u32) {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let mut client = Client {
            stream,
            crypt: ClientCrypt::new(),
        };
        let (op, body) = client.read_packet().await.expect("challenge");
        assert_eq!(op, opcode::SMSG_AUTH_CHALLENGE);
        assert_eq!(body.len(), 4);
        let seed = u32::from_le_bytes(body[..4].try_into().expect("seed"));
        (client, seed)
    }

    async fn read_packet(&mut self) -> Option<(u16, Vec<u8>)> {
        let mut header = [0u8; 4];
        timeout(Duration::from_secs(2), self.stream.read_exact(&mut header))
            .await
            .ok()?
            .ok()?;
        self.crypt.decrypt_recv(&mut header);
        let size = usize::from(u16::from_be_bytes([header[0], header[1]]));
        let op = u16::from_le_bytes([header[2], header[3]]);
        assert!(size >= 2, "server size field");
        let mut body = vec![0u8; size - 2];
        timeout(Duration::from_secs(2), self.stream.read_exact(&mut body))
            .await
            .ok()?
            .ok()?;
        Some((op, body))
    }

    async fn read_until_opcode(&mut self, want: u16) -> Vec<u8> {
        loop {
            let (op, body) = self
                .read_packet()
                .await
                .unwrap_or_else(|| panic!("timed out waiting for opcode {want}"));
            if op == want {
                return body;
            }
            assert!(
                matches!(op, 169 | 502),
                "unexpected opcode {op} while waiting for {want}"
            );
        }
    }

    async fn send_packet(&mut self, op: u16, body: &[u8]) {
        let size = u16::try_from(body.len() + 4).expect("size");
        let mut header = [0u8; 6];
        header[..2].copy_from_slice(&size.to_be_bytes());
        header[2..].copy_from_slice(&u32::from(op).to_le_bytes());
        self.crypt.encrypt_send(&mut header);
        self.stream.write_all(&header).await.expect("write header");
        self.stream.write_all(body).await.expect("write body");
    }

    async fn send_auth(&mut self, server_seed: u32, account: &str, digest: [u8; 20]) {
        let mut body = Vec::new();
        body.extend_from_slice(&5875u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(account.as_bytes());
        body.push(0);
        body.extend_from_slice(&CLIENT_SEED.to_le_bytes());
        body.extend_from_slice(&digest);
        let _ = server_seed;
        self.send_packet(opcode::CMSG_AUTH_SESSION, &body).await;
    }

    async fn expect_closed(&mut self) {
        let mut buf = [0u8; 16];
        let read = timeout(Duration::from_secs(2), self.stream.read(&mut buf))
            .await
            .expect("close within timeout");
        match read {
            Ok(0) | Err(_) => {} // EOF or reset: closed either way
            Ok(n) => panic!("expected EOF, read {n} bytes"),
        }
    }
}

fn good_digest(server_seed: u32) -> [u8; 20] {
    let key = session_key_from_hex(SESSIONKEY_HEX).expect("key");
    expected_auth_session_digest("TESTUSER", CLIENT_SEED, server_seed, &key)
}

/// Phase 5 Slice B (who): the fake world holds only the
/// logged-in Enumtest, so an unrestricted who lists exactly
/// that row; a non-matching name filter lists nobody.
async fn assert_who_replies(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    let mut body = Vec::new();
    body.extend_from_slice(&0u32.to_le_bytes()); // level min
    body.extend_from_slice(&100u32.to_le_bytes()); // level max
    body.extend_from_slice(b"\0\0"); // player + guild name filters
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // race mask
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // class mask
    body.extend_from_slice(&0u32.to_le_bytes()); // zones count
    body.extend_from_slice(&0u32.to_le_bytes()); // strings count

    client.send_packet(opcode_id::CMSG_WHO, &body).await;
    let mut expected = 1u32.to_le_bytes().to_vec();
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.extend_from_slice(b"Enumtest\0Loopback Guild\0");
    expected.extend_from_slice(&12u32.to_le_bytes()); // level
    expected.extend_from_slice(&1u32.to_le_bytes()); // warrior
    expected.extend_from_slice(&1u32.to_le_bytes()); // human
    expected.extend_from_slice(&0u32.to_le_bytes()); // fake zone
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_WHO).await,
        expected
    );

    // Name filter "zzz" matches nobody.
    let mut filtered = Vec::new();
    filtered.extend_from_slice(&0u32.to_le_bytes());
    filtered.extend_from_slice(&100u32.to_le_bytes());
    filtered.extend_from_slice(b"zzz\0\0");
    filtered.extend_from_slice(&u32::MAX.to_le_bytes());
    filtered.extend_from_slice(&u32::MAX.to_le_bytes());
    filtered.extend_from_slice(&0u32.to_le_bytes());
    filtered.extend_from_slice(&0u32.to_le_bytes());
    client.send_packet(opcode_id::CMSG_WHO, &filtered).await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_WHO).await,
        [0u8; 8].as_slice()
    );
}

/// Phase 5 Slice A: friend/ignore mutation against the
/// fake social store. The fake load starts with guid 9
/// ("Offlinefriend") already a friend.
async fn assert_social_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let status = |result: u8, guid_low: u32| {
        let mut body = vec![result];
        body.extend_from_slice(
            &ObjectGuid::new(HighGuid::PLAYER, guid_low)
                .raw()
                .to_le_bytes(),
        );
        body
    };
    let guid9 = ObjectGuid::new(HighGuid::PLAYER, 9).raw().to_le_bytes();

    // Already a friend from the fake character load.
    client
        .send_packet(opcode_id::CMSG_ADD_FRIEND, b"Offlinefriend\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x08, 9)
    );

    client.send_packet(opcode_id::CMSG_DEL_FRIEND, &guid9).await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x05, 9)
    );

    // Lower-case input normalizes; guid 9 is offline.
    client
        .send_packet(opcode_id::CMSG_ADD_FRIEND, b"offlinefriend\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x07, 9)
    );

    client
        .send_packet(opcode_id::CMSG_ADD_IGNORE, b"Offlinefriend\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x0F, 9)
    );

    // Cross-faction friend add is refused for a plain player.
    client
        .send_packet(opcode_id::CMSG_ADD_FRIEND, b"Orcfriend\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x0A, 10)
    );

    client
        .send_packet(opcode_id::CMSG_ADD_FRIEND, b"Enumtest\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x09, 7)
    );

    client.send_packet(opcode_id::CMSG_DEL_IGNORE, &guid9).await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_FRIEND_STATUS)
            .await,
        status(0x10, 9)
    );
}

/// Phase 5 Slice B.2a (chat core): say self-echo with
/// the learned-language gate, whisper to self with
/// inform + AFK auto-reply, and the not-found notice.
async fn assert_chat_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let guid = ObjectGuid::new(HighGuid::PLAYER, 7).raw().to_le_bytes();
    let chat = |msg_type: u8, lang: u32, sender_count: usize, text: &str, tag: u8| {
        let mut body = vec![msg_type];
        body.extend_from_slice(&lang.to_le_bytes());
        for _ in 0..sender_count {
            body.extend_from_slice(&guid);
        }
        body.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
        body.extend_from_slice(text.as_bytes());
        body.push(0);
        body.push(tag);
        body
    };
    let send_chat = |msg_type: u32, lang: u32, target: Option<&str>, text: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(&msg_type.to_le_bytes());
        body.extend_from_slice(&lang.to_le_bytes());
        if let Some(target) = target {
            body.extend_from_slice(target.as_bytes());
            body.push(0);
        }
        body.extend_from_slice(text.as_bytes());
        body.push(0);
        body
    };

    // Orcish is unlearned and drops silently; the Common
    // say after it is the first chat packet back (ordered
    // stream proves the drop). SAY repeats the sender guid.
    let orcish = send_chat(0x00, 1, None, "zug zug");
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &orcish)
        .await;
    let say = send_chat(0x00, 7, None, "hello loopback");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &say).await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x00, 7, 2, "hello loopback", 0)
    );

    // Whisper to an offline name: not-found notice carries
    // the normalized name.
    let missing = send_chat(0x06, 0, Some("nobodyhere"), "psst");
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &missing)
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHAT_PLAYER_NOT_FOUND)
            .await,
        b"Nobodyhere\0".as_slice()
    );

    // Set AFK with a message; the next say carries tag 1.
    let afk = send_chat(0x14, 0, None, "brb");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &afk).await;
    let say = send_chat(0x00, 7, None, "still here");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &say).await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x00, 7, 2, "still here", 1)
    );

    // Whisper self (lower-case target normalizes): whisper
    // delivery, inform echo, then the AFK auto-reply — all
    // forced to universal language.
    let whisper = send_chat(0x06, 7, Some("enumtest"), "note to self");
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &whisper)
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x06, 0, 1, "note to self", 1)
    );
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x07, 0, 1, "note to self", 1)
    );
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x14, 0, 1, "brb", 0)
    );

    // Empty AFK toggles off; say tag returns to 0.
    let unafk = send_chat(0x14, 0, None, "");
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &unafk)
        .await;
    let say = send_chat(0x00, 7, None, "back");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &say).await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        chat(0x00, 7, 2, "back", 0)
    );
}

/// Phase 5 Slice B.2b (channels): built-in zone channel
/// join/say/list/leave and a custom channel's owner
/// assignment, exact `SMSG_CHANNEL_NOTIFY` bytes.
async fn assert_channel_flow(client: &mut Client) {
    assert_builtin_channel_flow(client).await;
    assert_custom_channel_flow(client).await;
}

async fn assert_builtin_channel_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    const GENERAL: &str = "General - Elwynn Forest";
    let guid = ObjectGuid::new(HighGuid::PLAYER, 7).raw().to_le_bytes();
    let notify = |ty: u8, name: &str, tail: &[u8]| {
        let mut body = vec![ty];
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(tail);
        body
    };
    let join_body = |name: &str, pass: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(pass.as_bytes());
        body.push(0);
        body
    };

    client
        .send_packet(opcode_id::CMSG_JOIN_CHANNEL, &join_body(GENERAL, ""))
        .await;
    // YouJoined: flags GENERAL|NOT_LFG (0x18), instance 0.
    let mut tail = 0x18u32.to_le_bytes().to_vec();
    tail.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x02, GENERAL, &tail)
    );

    // Channel say echoes to self as CHAT_MSG_CHANNEL.
    let mut say = Vec::new();
    say.extend_from_slice(&0x0Eu32.to_le_bytes());
    say.extend_from_slice(&7u32.to_le_bytes());
    say.extend_from_slice(GENERAL.as_bytes());
    say.push(0);
    say.extend_from_slice(b"hello general\0");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &say).await;
    let mut expected = vec![0x0Eu8];
    expected.extend_from_slice(&7u32.to_le_bytes());
    expected.extend_from_slice(GENERAL.as_bytes());
    expected.push(0);
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend_from_slice(&guid);
    expected.extend_from_slice(&14u32.to_le_bytes());
    expected.extend_from_slice(b"hello general\0");
    expected.push(0);
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        expected
    );

    client
        .send_packet(opcode_id::CMSG_CHANNEL_LIST, &join_body(GENERAL, ""))
        .await;
    let mut expected = Vec::new();
    expected.extend_from_slice(GENERAL.as_bytes());
    expected.push(0);
    expected.push(0x18);
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.extend_from_slice(&guid);
    expected.push(0);
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_CHANNEL_LIST).await,
        expected
    );

    client
        .send_packet(opcode_id::CMSG_LEAVE_CHANNEL, &join_body(GENERAL, ""))
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x03, GENERAL, &[])
    );
}

async fn assert_custom_channel_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let guid = ObjectGuid::new(HighGuid::PLAYER, 7).raw().to_le_bytes();
    let notify = |ty: u8, name: &str, tail: &[u8]| {
        let mut body = vec![ty];
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(tail);
        body
    };
    let join_body = |name: &str, pass: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(pass.as_bytes());
        body.push(0);
        body
    };

    // Custom channel: first joiner becomes owner
    // (mode-change 0 -> OWNER|MODERATOR-set-after = 0x01
    // in the packet, moderator applied silently after).
    client
        .send_packet(opcode_id::CMSG_JOIN_CHANNEL, &join_body("mychan", ""))
        .await;
    let mut tail = 0x01u32.to_le_bytes().to_vec();
    tail.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x02, "Mychan", &tail)
    );
    let mut tail = guid.to_vec();
    tail.extend_from_slice(&[0x00, 0x01]);
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x0C, "Mychan", &tail)
    );

    client
        .send_packet(opcode_id::CMSG_LEAVE_CHANNEL, &join_body("Mychan", ""))
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x03, "Mychan", &[])
    );

    // The emptied custom channel is erased; listing it
    // answers not-member with the client-sent name.
    client
        .send_packet(opcode_id::CMSG_CHANNEL_LIST, &join_body("mychan", ""))
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_CHANNEL_NOTIFY)
            .await,
        notify(0x05, "mychan", &[])
    );
}

/// Phase 5 Slice B.3 (group/party) single-client edges:
/// the self-invite shell tears itself down silently
/// (C++ `AddInvite` refuses the already-invited leader),
/// unknown names get `ERR_BAD_PLAYER_NAME_S`, accept /
/// disband with no group are silent, solo random roll
/// echoes to self, and party-stats for a non-member
/// answers the offline form. The two-member happy path
/// is covered by unit tests on the state machine and
/// the two-client bench smoke.
async fn assert_group_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let guid = ObjectGuid::new(HighGuid::PLAYER, 7).raw().to_le_bytes();
    let result = |op: u32, member: &str, code: u32| {
        let mut body = Vec::new();
        body.extend_from_slice(&op.to_le_bytes());
        body.extend_from_slice(member.as_bytes());
        body.push(0);
        body.extend_from_slice(&code.to_le_bytes());
        body
    };

    // Invite a name that is not online.
    client
        .send_packet(opcode_id::CMSG_GROUP_INVITE, b"nobodyhere\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PARTY_COMMAND_RESULT)
            .await,
        result(0, "Nobodyhere", 1)
    );

    // Self-invite plus accept/disband without a group are
    // all silent in C++; the next result packet proves no
    // invite/group state stuck.
    client
        .send_packet(opcode_id::CMSG_GROUP_INVITE, b"Enumtest\0")
        .await;
    client.send_packet(opcode_id::CMSG_GROUP_ACCEPT, &[]).await;
    client.send_packet(opcode_id::CMSG_GROUP_DISBAND, &[]).await;
    client
        .send_packet(opcode_id::CMSG_GROUP_INVITE, b"nobodyhere\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PARTY_COMMAND_RESULT)
            .await,
        result(0, "Nobodyhere", 1)
    );

    // Solo random roll: min == max pins the roll; the
    // packet echoes to self with the sender guid.
    let mut roll = Vec::new();
    roll.extend_from_slice(&5u32.to_le_bytes());
    roll.extend_from_slice(&5u32.to_le_bytes());
    client.send_packet(opcode_id::MSG_RANDOM_ROLL, &roll).await;
    let mut expected = Vec::new();
    expected.extend_from_slice(&5u32.to_le_bytes());
    expected.extend_from_slice(&5u32.to_le_bytes());
    expected.extend_from_slice(&5u32.to_le_bytes());
    expected.extend_from_slice(&guid);
    assert_eq!(
        client.read_until_opcode(opcode_id::MSG_RANDOM_ROLL).await,
        expected
    );

    // Party stats for self while group-less: packed guid,
    // STATUS-only mask, offline.
    client
        .send_packet(opcode_id::CMSG_REQUEST_PARTY_MEMBER_STATS, &guid)
        .await;
    let mut packed = Vec::new();
    ObjectGuid::new(HighGuid::PLAYER, 7).write_packed(&mut packed);
    let mut expected = packed;
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.push(0);
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PARTY_MEMBER_STATS_FULL)
            .await,
        expected
    );
}

fn guild_command_result_body(op: u32, text: &str, code: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&op.to_le_bytes());
    body.extend_from_slice(text.as_bytes());
    body.push(0);
    body.extend_from_slice(&code.to_le_bytes());
    body
}

/// Phase 5 Slice B.4 (guild) single-client flow over
/// the boot-seeded guild: exact query/roster bytes,
/// MOTD event, guild-chat self echo, offline invite
/// result, and the sole-leader leave disbanding.
async fn assert_guild_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let guid = ObjectGuid::new(HighGuid::PLAYER, 7).raw().to_le_bytes();
    let result = guild_command_result_body;

    // Query id 1: name, five default ranks + five empty,
    // emblem -1 across.
    client
        .send_packet(opcode_id::CMSG_GUILD_QUERY, &1u32.to_le_bytes())
        .await;
    let mut expect = Vec::new();
    expect.extend_from_slice(&1u32.to_le_bytes());
    expect.extend_from_slice(b"Loopback Guild\0");
    for rank in ["Guild Master", "Officer", "Veteran", "Member", "Initiate"] {
        expect.extend_from_slice(rank.as_bytes());
        expect.push(0);
    }
    expect.extend_from_slice(&[0; 5]);
    for _ in 0..5 {
        expect.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    }
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_GUILD_QUERY_RESPONSE)
            .await,
        expect
    );

    // Roster: one online member, default MOTD, rank
    // rights ALL/ALL/0x43 x3.
    client.send_packet(opcode_id::CMSG_GUILD_ROSTER, &[]).await;
    let mut expect = Vec::new();
    expect.extend_from_slice(&1u32.to_le_bytes());
    expect.extend_from_slice(b"No message set.\0\0");
    expect.extend_from_slice(&5u32.to_le_bytes());
    for rights in [0x000F_F1FFu32, 0x000F_F1FF, 0x43, 0x43, 0x43] {
        expect.extend_from_slice(&rights.to_le_bytes());
    }
    expect.extend_from_slice(&guid);
    expect.push(0x01); // ONLINE
    expect.extend_from_slice(b"Enumtest\0");
    expect.extend_from_slice(&0u32.to_le_bytes()); // rank
    expect.push(12); // level (fake character row)
    expect.push(1); // class warrior
    expect.extend_from_slice(&0u32.to_le_bytes()); // zone (no terrain)
    expect.extend_from_slice(b"\0\0"); // notes
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_GUILD_ROSTER).await,
        expect
    );

    // MOTD set broadcasts the event to self.
    client
        .send_packet(opcode_id::CMSG_GUILD_MOTD, b"raid friday\0")
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_GUILD_EVENT).await,
        [0x02, 0x01]
            .into_iter()
            .chain(*b"raid friday\0")
            .collect::<Vec<_>>()
    );

    // Guild chat echoes to the (only) member as
    // universal language, single-guid form.
    let mut chat = Vec::new();
    chat.extend_from_slice(&0x03u32.to_le_bytes());
    chat.extend_from_slice(&0u32.to_le_bytes());
    chat.extend_from_slice(b"guild hello\0");
    client.send_packet(opcode_id::CMSG_MESSAGECHAT, &chat).await;
    let mut expect = vec![0x03u8];
    expect.extend_from_slice(&0u32.to_le_bytes());
    expect.extend_from_slice(&guid);
    expect.extend_from_slice(&12u32.to_le_bytes());
    expect.extend_from_slice(b"guild hello\0");
    expect.push(0);
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_MESSAGECHAT).await,
        expect
    );

    // Invite an offline name.
    client
        .send_packet(opcode_id::CMSG_GUILD_INVITE, b"nobodyhere\0")
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_GUILD_COMMAND_RESULT)
            .await,
        result(1, "nobodyhere", 0x0B)
    );

    // Sole leader leaving disbands; the query after it
    // reports not-in-guild.
    client.send_packet(opcode_id::CMSG_GUILD_LEAVE, &[]).await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_GUILD_EVENT).await,
        [0x08, 0x00]
    );
    client
        .send_packet(opcode_id::CMSG_GUILD_QUERY, &1u32.to_le_bytes())
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_GUILD_COMMAND_RESULT)
            .await,
        result(0, "", 0x09)
    );
}

async fn assert_petition_flow(client: &mut Client) {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    fn packed(guid: ObjectGuid) -> Vec<u8> {
        // Plain ObjectGuid streams as a full u64.
        guid.raw().to_le_bytes().to_vec()
    }
    let item = ObjectGuid::new(HighGuid::ITEM, 777);
    let player = ObjectGuid::new(HighGuid::PLAYER, 7);

    // Query: id, owner, name, the fixed C++ tail.
    let mut body = 5u32.to_le_bytes().to_vec();
    body.extend_from_slice(&packed(item));
    client
        .send_packet(opcode_id::CMSG_PETITION_QUERY, &body)
        .await;
    let mut expect = 5u32.to_le_bytes().to_vec();
    expect.extend_from_slice(&packed(player));
    expect.extend_from_slice(b"Charter Pals\0");
    expect.push(0);
    expect.extend_from_slice(&1u32.to_le_bytes());
    expect.extend_from_slice(&9u32.to_le_bytes());
    expect.extend_from_slice(&9u32.to_le_bytes());
    for _ in 0..5 {
        expect.extend_from_slice(&0u32.to_le_bytes());
    }
    expect.extend_from_slice(&0u16.to_le_bytes());
    for _ in 0..4 {
        expect.extend_from_slice(&0u32.to_le_bytes());
    }
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PETITION_QUERY_RESPONSE)
            .await,
        expect
    );

    // Show signatures (the guild flow disbanded
    // Enumtest's guild, so the owner-side form opens):
    // one signature from Offlinefriend (guid 9).
    client
        .send_packet(opcode_id::CMSG_PETITION_SHOW_SIGNATURES, &packed(item))
        .await;
    let mut expect = packed(item);
    expect.extend_from_slice(&packed(player));
    expect.extend_from_slice(&5u32.to_le_bytes());
    expect.push(1);
    expect.extend_from_slice(&packed(ObjectGuid::new(HighGuid::PLAYER, 9)));
    expect.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PETITION_SHOW_SIGNATURES)
            .await,
        expect
    );

    // Signing your own charter reports CANT_SIGN_OWN.
    let mut body = packed(item);
    body.push(0);
    client
        .send_packet(opcode_id::CMSG_PETITION_SIGN, &body)
        .await;
    let mut expect = packed(item);
    expect.extend_from_slice(&packed(player));
    expect.extend_from_slice(&3u32.to_le_bytes());
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_PETITION_SIGN_RESULTS)
            .await,
        expect
    );

    // Rename to a reserved name is refused.
    let mut body = packed(item);
    body.extend_from_slice(b"Reservedname\0");
    client
        .send_packet(opcode_id::MSG_PETITION_RENAME, &body)
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_GUILD_COMMAND_RESULT)
            .await,
        guild_command_result_body(0, "Reservedname", 0x06)
    );

    // Rename to a free name echoes back.
    let mut body = packed(item);
    body.extend_from_slice(b"Charter Pals II\0");
    client
        .send_packet(opcode_id::MSG_PETITION_RENAME, &body)
        .await;
    let mut expect = packed(item);
    expect.extend_from_slice(b"Charter Pals II\0");
    assert_eq!(
        client
            .read_until_opcode(opcode_id::MSG_PETITION_RENAME)
            .await,
        expect
    );

    // Turn-in with one of nine signatures reports
    // PETITION_SIGN_NEED_MORE.
    client
        .send_packet(opcode_id::CMSG_TURN_IN_PETITION, &packed(item))
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_TURN_IN_PETITION_RESULTS)
            .await,
        4u32.to_le_bytes()
    );
}

async fn assert_mail_flow(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    // One delivered unread mail is seeded, so the
    // next-mail-time answer is "unread waiting".
    client
        .send_packet(opcode_id::MSG_QUERY_NEXT_MAIL_TIME, &[])
        .await;
    assert_eq!(
        client
            .read_until_opcode(opcode_id::MSG_QUERY_NEXT_MAIL_TIME)
            .await,
        0.0f32.to_le_bytes()
    );

    // The seeded mail's body text resolves by id.
    let mut body = Vec::new();
    body.extend_from_slice(&77u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_ITEM_TEXT_QUERY, &body)
        .await;
    let mut expect = 77u32.to_le_bytes().to_vec();
    expect.extend_from_slice(b"For the road.\0");
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_ITEM_TEXT_QUERY_RESPONSE)
            .await,
        expect
    );
}

/// Trade initiate gates reachable with one live
/// player (the two-player exchange is bench-probe
/// covered; the fake backend has a single account).
async fn assert_trade_flow(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    // Another player who is not in the world.
    client
        .send_packet(opcode_id::CMSG_INITIATE_TRADE, &9u64.to_le_bytes())
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_TRADE_STATUS).await,
        6u32.to_le_bytes(), // TRADE_STATUS_NO_TARGET
    );

    // Trading with yourself reads busy.
    client
        .send_packet(opcode_id::CMSG_INITIATE_TRADE, &7u64.to_le_bytes())
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_TRADE_STATUS).await,
        0u32.to_le_bytes(), // TRADE_STATUS_BUSY
    );
}

/// Auction gates reachable without a live
/// auctioneer (the fake world has no creature
/// spawns): sell and bid fail at the auctioneer
/// check with the fork's error codes. The full
/// exchange is bench-probe covered.
async fn assert_auction_flow(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    // Sell against a nonexistent auctioneer:
    // DATABASE error (auction id 0, action STARTED).
    let mut sell = Vec::new();
    sell.extend_from_slice(&0xF130_0000_0000_0001u64.to_le_bytes());
    sell.extend_from_slice(&4242u64.to_le_bytes());
    sell.extend_from_slice(&100u32.to_le_bytes()); // bid
    sell.extend_from_slice(&0u32.to_le_bytes()); // buyout
    sell.extend_from_slice(&120u32.to_le_bytes()); // etime minutes
    client
        .send_packet(opcode_id::CMSG_AUCTION_SELL_ITEM, &sell)
        .await;
    let mut expected = Vec::new();
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend_from_slice(&0u32.to_le_bytes()); // AUCTION_STARTED
    expected.extend_from_slice(&2u32.to_le_bytes()); // AUCTION_ERR_DATABASE
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_AUCTION_COMMAND_RESULT)
            .await,
        expected
    );

    // Bid against the same ghost auctioneer:
    // ITEM_NOT_FOUND on BID_PLACED.
    let mut bid = Vec::new();
    bid.extend_from_slice(&0xF130_0000_0000_0001u64.to_le_bytes());
    bid.extend_from_slice(&7u32.to_le_bytes()); // auction id
    bid.extend_from_slice(&100u32.to_le_bytes()); // price
    client
        .send_packet(opcode_id::CMSG_AUCTION_PLACE_BID, &bid)
        .await;
    let mut expected = Vec::new();
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend_from_slice(&2u32.to_le_bytes()); // AUCTION_BID_PLACED
    expected.extend_from_slice(&4u32.to_le_bytes()); // AUCTION_ERR_ITEM_NOT_FOUND
    assert_eq!(
        client
            .read_until_opcode(opcode_id::SMSG_AUCTION_COMMAND_RESULT)
            .await,
        expected
    );
}

/// Dot-command consumption at player security: the
/// available-commands list, hidden GM commands, and
/// the double-dot escape (Slice C contract flow).
async fn assert_command_flow(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    let say = |text: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // CHAT_MSG_SAY
        body.extend_from_slice(&7u32.to_le_bytes()); // LANG_COMMON
        body.extend_from_slice(text.as_bytes());
        body.push(0);
        body
    };
    // Read chat packets until the marker say echoes,
    // collecting system lines seen on the way.
    #[allow(clippy::items_after_statements)]
    async fn collect_until_say(client: &mut Client, marker: &str) -> Vec<String> {
        use mangos_protocol::opcode_table::opcode_id;
        let mut system_lines = Vec::new();
        loop {
            let (op, body) = client.read_packet().await.expect("chat packet");
            if op != opcode_id::SMSG_MESSAGECHAT {
                continue;
            }
            // System bodies carry one guid (text at
            // 17); SAY echoes carry two (text at 25).
            let offset = if body[0] == 0x0A { 17 } else { 25 };
            let text = String::from_utf8_lossy(&body[offset..body.len().saturating_sub(1)])
                .trim_end_matches('\0')
                .to_string();
            if body[0] == 0x00 && text == marker {
                return system_lines;
            }
            assert_ne!(
                text, ".commands",
                "dot command must be consumed, not broadcast"
            );
            if body[0] == 0x0A {
                system_lines.push(text);
            }
        }
    }

    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say(".commands"))
        .await;
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say("aftercmd"))
        .await;
    let lines = collect_until_say(client, "aftercmd").await;
    assert!(
        lines.iter().any(|line| line.trim() == "help"),
        "available list must include help: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("partybot")),
        "SEC_ADMINISTRATOR commands stay hidden: {lines:?}"
    );

    // A known GM command is consumed (unavailable at
    // player security), never broadcast.
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say(".gm on"))
        .await;
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say("after2"))
        .await;
    let lines = collect_until_say(client, "after2").await;
    assert!(
        lines.is_empty(),
        "no system text in fixture strings: {lines:?}"
    );

    // Double dot is not a command: it says normally.
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say("..dots"))
        .await;
    client
        .send_packet(opcode_id::CMSG_MESSAGECHAT, &say("after3"))
        .await;
    collect_until_say(client, "..dots").await;
    collect_until_say(client, "after3").await;
}

async fn assert_query_replies(client: &mut Client) {
    use mangos_protocol::opcode_table::opcode_id;

    // Cache queries answer with the C++ not-found marker for a
    // template entry absent from the fake stores.
    let missing_creature = 999_999u32;
    let mut creature_query = Vec::new();
    creature_query.extend_from_slice(&missing_creature.to_le_bytes());
    creature_query.extend_from_slice(&7u64.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_CREATURE_QUERY, &creature_query)
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_CREATURE_QUERY_RESPONSE)
        .await;
    assert_eq!(body, (missing_creature | 0x8000_0000).to_le_bytes());

    let missing_gameobject = 888_888u32;
    let mut gameobject_query = Vec::new();
    gameobject_query.extend_from_slice(&missing_gameobject.to_le_bytes());
    gameobject_query.extend_from_slice(&7u64.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_GAMEOBJECT_QUERY, &gameobject_query)
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_GAMEOBJECT_QUERY_RESPONSE)
        .await;
    assert_eq!(body, (missing_gameobject | 0x8000_0000).to_le_bytes());

    // A name query for the entered player answers from live
    // session state in the 1.12.1 shape.
    client
        .send_packet(opcode_id::CMSG_NAME_QUERY, &7u64.to_le_bytes())
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_NAME_QUERY_RESPONSE)
        .await;
    let mut expected = Vec::new();
    expected.extend_from_slice(&7u64.to_le_bytes());
    expected.extend_from_slice(b"Enumtest\0\0");
    expected.extend_from_slice(&1u32.to_le_bytes()); // race
    expected.extend_from_slice(&0u32.to_le_bytes()); // gender
    expected.extend_from_slice(&1u32.to_le_bytes()); // class
    assert_eq!(body, expected);

    // An offline name query takes the database fallback.
    client
        .send_packet(opcode_id::CMSG_NAME_QUERY, &9u64.to_le_bytes())
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_NAME_QUERY_RESPONSE)
        .await;
    let mut expected = Vec::new();
    expected.extend_from_slice(&9u64.to_le_bytes());
    expected.extend_from_slice(b"Offlinefriend\0\0");
    expected.extend_from_slice(&4u32.to_le_bytes()); // race
    expected.extend_from_slice(&1u32.to_le_bytes()); // gender
    expected.extend_from_slice(&5u32.to_le_bytes()); // class
    assert_eq!(body, expected);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn auth_then_ping_pong_and_enum() {
    let backend = FakeBackend::with_test_user();
    let port = start_server(Arc::clone(&backend)).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);

    // Admission answers with the long AUTH_OK form
    // (code, billing time u32, billing flags u8, rested u32).
    let (op, body) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    assert_eq!(body, [auth_response::OK, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    // CHAR_ENUM round-trips through the world actor's async
    // query: one character, slot 0 shows item 25's display.
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let (op, body) = client.read_packet().await.expect("char enum");
    assert_eq!(op, opcode::SMSG_CHAR_ENUM);
    assert_eq!(body[0], 1, "one character");
    assert_eq!(&body[1..9], &7u64.to_le_bytes(), "player guid");
    assert_eq!(&body[9..18], b"Enumtest\0");
    // Equipment visuals start after: guid(8) name(9) race..level(9)
    // zone/map(8) pos(12) guild(4) flags(4) firstlogin(1) pet(12)
    let visual = 1 + 8 + 9 + 9 + 8 + 12 + 4 + 4 + 1 + 12;
    assert_eq!(&body[visual..visual + 4], &1542u32.to_le_bytes());
    assert_eq!(body[visual + 4], 21);
    assert_eq!(body.len(), visual + 20 * 5);

    // PLAYER_LOGIN runs the async load and answers with the
    // empty-world packet train in C++ order.
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let (op, body) = client.read_packet().await.expect("verify world");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_LOGIN_VERIFY_WORLD
    );
    let mut expected = Vec::new();
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend_from_slice(&(-8949.95f32).to_le_bytes());
    expected.extend_from_slice(&(-132.493f32).to_le_bytes());
    expected.extend_from_slice(&83.5312f32.to_le_bytes());
    expected.extend_from_slice(&1.5f32.to_le_bytes());
    assert_eq!(body, expected);

    let (op, body) = client.read_packet().await.expect("account data md5");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_ACCOUNT_DATA_MD5
    );
    assert_eq!(body.len(), 8 * 16);

    let (op, body) = client.read_packet().await.expect("friend list");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_FRIEND_LIST
    );
    assert_eq!(body[0], 1);
    assert_eq!(&body[1..9], &9u64.to_le_bytes());
    assert_eq!(body[9], 0, "offline in the empty world");

    let (op, body) = client.read_packet().await.expect("ignore list");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_IGNORE_LIST
    );
    assert_eq!(body, [0]);

    // MOTD: base line + patch banner (default test config has
    // no WowPatch key -> patch 0 name).
    let (op, body) = client.read_packet().await.expect("motd line 1");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_MESSAGECHAT
    );
    assert!(body.ends_with(b"\x00"));
    let (op, body) = client.read_packet().await.expect("motd line 2");
    assert_eq!(
        op,
        mangos_protocol::opcode_table::opcode_id::SMSG_MESSAGECHAT
    );
    assert!(
        String::from_utf8_lossy(&body).contains("Patch 1.2: Mysteries of Maraudon is now live!"),
        "patch banner: {body:?}"
    );

    // SendInitialPacketsBeforeAddToMap subset, in C++ order:
    // rest start, bindpoint, tutorial flags, initial spells,
    // action buttons twice, factions, time speed.
    for want in [542u16, 341, 253, 298, 297, 297, 290, 66] {
        let (op, body) = client.read_packet().await.expect("login train tail");
        assert_eq!(op, want);
        match want {
            253 => assert_eq!((body.len(), &body[..4]), (32, &[2, 0, 0, 0][..])),
            297 => assert_eq!(body.len(), 120 * 4),
            290 => assert_eq!(body.len(), 4 + 64 * 5),
            _ => {}
        }
    }

    // Ping proves the session exists and the crypt directions agree.
    // Phase 4: the map also sends init-self/visibility object
    // updates after the login train; they must arrive, and the
    // pong still answers after them.
    let mut ping_body = Vec::new();
    ping_body.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    ping_body.extend_from_slice(&123u32.to_le_bytes());
    client.send_packet(opcode::CMSG_PING, &ping_body).await;
    let mut saw_update = false;
    let mut saw_world_states = false;
    let mut saw_weather = false;
    let mut saw_guild_event = false;
    let body = loop {
        let (op, body) = client.read_packet().await.expect("pong or update");
        match op {
            opcode::SMSG_PONG => break body,
            169 | 502 => saw_update = true, // UPDATE_OBJECT / COMPRESSED
            706 => saw_world_states = true, // INIT_WORLD_STATES
            756 => saw_weather = true,      // WEATHER
            146 => saw_guild_event = true,  // GUILD_EVENT (login MOTD/signed-on)
            other => panic!("unexpected opcode {other} before pong"),
        }
    };
    assert!(saw_update, "init-self object update after login");
    assert!(saw_world_states, "initial world states after login");
    assert!(saw_weather, "initial weather after login");
    assert!(saw_guild_event, "guild login events after login");
    assert_eq!(body, 0xDEAD_BEEFu32.to_le_bytes());

    client.send_packet(opcode::CMSG_PING, &ping_body).await;
    let (op, _) = client.read_packet().await.expect("second pong");
    assert_eq!(op, opcode::SMSG_PONG);

    assert_social_flow(&mut client).await;
    assert_who_replies(&mut client).await;
    assert_chat_flow(&mut client).await;
    assert_channel_flow(&mut client).await;
    assert_group_flow(&mut client).await;
    assert_guild_flow(&mut client).await;
    assert_petition_flow(&mut client).await;
    assert_mail_flow(&mut client).await;
    assert_trade_flow(&mut client).await;
    assert_auction_flow(&mut client).await;
    assert_command_flow(&mut client).await;
    assert_query_replies(&mut client).await;

    assert_eq!(
        backend.writes(),
        vec![("TESTUSER".to_string(), "127.0.0.1".to_string())]
    );
}

#[tokio::test]
async fn wrong_digest_rejects_failed_without_last_ip_write() {
    let backend = FakeBackend::with_test_user();
    let port = start_server(Arc::clone(&backend)).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", [0xAB; 20]).await;
    let (op, body) = client.read_packet().await.expect("auth response");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    assert_eq!(body, [auth_response::FAILED]);
    client.expect_closed().await;
    assert!(backend.writes().is_empty());
}

#[tokio::test]
async fn unknown_account_and_pre_auth_rules() {
    let backend = FakeBackend::with_test_user();
    let port = start_server(backend).await;

    // Unknown account -> AUTH_UNKNOWN_ACCOUNT, then close.
    let (mut client, seed) = Client::connect(port).await;
    client.send_auth(seed, "NOSUCH", [0; 20]).await;
    let (op, body) = client.read_packet().await.expect("auth response");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    assert_eq!(body, [auth_response::UNKNOWN_ACCOUNT]);
    client.expect_closed().await;

    // Pre-auth ping -> close with no packet.
    let (mut client, _seed) = Client::connect(port).await;
    let mut ping_body = Vec::new();
    ping_body.extend_from_slice(&7u32.to_le_bytes());
    ping_body.extend_from_slice(&7u32.to_le_bytes());
    client.send_packet(opcode::CMSG_PING, &ping_body).await;
    client.expect_closed().await;

    // Pre-auth non-auth opcode -> close with no packet.
    let (mut client, _seed) = Client::connect(port).await;
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    client.expect_closed().await;
}

#[tokio::test]
async fn malformed_header_closes() {
    let backend = FakeBackend::with_test_user();
    let port = start_server(backend).await;
    let (mut client, _seed) = Client::connect(port).await;
    // size = 3 (< 4), cmd = CMSG_PING, plaintext (crypt inactive).
    let mut header = [0u8; 6];
    header[..2].copy_from_slice(&3u16.to_be_bytes());
    header[2..].copy_from_slice(&u32::from(opcode::CMSG_PING).to_le_bytes());
    client.stream.write_all(&header).await.expect("write");
    client.expect_closed().await;
}
