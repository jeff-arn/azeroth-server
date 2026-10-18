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

use mangos_db::character_load::{CharacterLoad, CharacterRow, SocialRow};
use mangos_db::characters::{AccountDataEntry, CharEnumRow, NameQueryRow};
use mangos_world_data::enum_stores::{EnumStores, ItemDisplay};

use mangosd::ServerConfig;
use mangosd::auth::{AuthBackend, WorldAuthAccount};
use mangosd::char_queries::CharQueries;
use mangosd::socket::{ConnectionDeps, run_connection};
use mangosd::world_actor::{PresenceWriter, RealmPublisher, SessionId, WorldDeps, spawn_world};

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
            },
            group_id: None,
            bound_instances: vec![],
            auras: vec![],
            spells: vec![],
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
            guild: None,
            bg_data: None,
            account_data: vec![],
            skills: vec![],
            mails: vec![],
            mailed_items: vec![],
            forgotten_skills: vec![],
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

struct FakePresence;

impl PresenceWriter for FakePresence {
    fn set_character_online(&self, _guid_low: u32, _online: bool) -> Result<(), DbError> {
        Ok(())
    }

    fn clear_creature_respawn(&self, _guid_low: u32) -> Result<(), DbError> {
        Ok(())
    }
    fn store_creature_respawn(
        &self,
        _guid_low: u32,
        _respawn_time: u64,
        _map_id: u32,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn clear_gameobject_respawn(&self, _guid_low: u32) -> Result<(), DbError> {
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
    use mangos_world_data::player_info::{
        PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
    };
    let mut stores = mangos_map::WorldStores::empty();
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
    let body = loop {
        let (op, body) = client.read_packet().await.expect("pong or update");
        match op {
            opcode::SMSG_PONG => break body,
            169 | 502 => saw_update = true, // UPDATE_OBJECT / COMPRESSED
            706 => saw_world_states = true, // INIT_WORLD_STATES
            756 => saw_weather = true,      // WEATHER
            other => panic!("unexpected opcode {other} before pong"),
        }
    };
    assert!(saw_update, "init-self object update after login");
    assert!(saw_world_states, "initial world states after login");
    assert!(saw_weather, "initial weather after login");
    assert_eq!(body, 0xDEAD_BEEFu32.to_le_bytes());

    client.send_packet(opcode::CMSG_PING, &ping_body).await;
    let (op, _) = client.read_packet().await.expect("second pong");
    assert_eq!(op, opcode::SMSG_PONG);

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
