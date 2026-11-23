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

use mangos_db::character_load::{
    CharacterLoad, CharacterRow, InventoryRow, ItemInstanceRow, SocialRow, SpellRow,
};
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

/// Stateful in-memory characters table: the seeded
/// Enumtest row plus whatever the create/delete
/// lifecycle adds and removes.
struct FakeCharQueries {
    chars: Mutex<Vec<(u32, CharEnumRow)>>,
    /// Tier 4 step 4h: serve the Enumtest load as a
    /// hunter with current + stabled pets.
    hunter_mode: bool,
    /// Tier 4 B0-7: serve the Enumtest load with
    /// seeded honor scalars + CP rows.
    honor_mode: bool,
    /// Saved-ghost login probe: Enumtest load with
    /// the ghost character flag + saved ghost aura,
    /// plus a persisted corpse at the login spot. The
    /// value is the corpse age in seconds; the expired
    /// variant crosses the three-day resurrectable expiry.
    ghost_corpse_age_secs: Option<u64>,
    /// Wrapped-gift probe: append the 9003 (gift row
    /// present) + 9004 (no gift row) wrapped rows to
    /// the hunter inventory.
    gift_mode: bool,
}

fn seed_inventory() -> Vec<InventoryRow> {
    let row = |item_guid: u32, item_id: u32, count: u32, durability: u16, slot: u8| InventoryRow {
        item: ItemInstanceRow {
            creator_guid: 0,
            gift_creator_guid: 0,
            count,
            duration: 0,
            charges: "0 0 0 0 0 ".to_string(),
            flags: 0,
            enchantments: "0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0".to_string(),
            random_property_id: 0,
            durability,
            text: 0,
            item_guid,
            item_id,
            generated_loot: 0,
        },
        bag: 0,
        slot,
    };
    // Item 25 worn main-hand (slot 15) + a 3-stack
    // of linen (2589) in the backpack (slot 23).
    vec![row(9001, 25, 1, 11, 15), row(9002, 2589, 3, 0, 23)]
}

/// Wrapped-gift fixture rows (backpack slots 24-25):
/// linen wrappers with the gift bit set. 9003 has a
/// fake gift row (restores to a plain sword);
/// 9004 has none (fork miss arm: destroy).
fn gift_rows() -> Vec<InventoryRow> {
    let row = |item_guid: u32, slot: u8| InventoryRow {
        item: ItemInstanceRow {
            creator_guid: 0,
            gift_creator_guid: 9,
            count: 1,
            duration: 0,
            charges: "0 0 0 0 0 ".to_string(),
            flags: 0x8,
            enchantments: "0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0".to_string(),
            random_property_id: 0,
            durability: 0,
            text: 0,
            item_guid,
            item_id: 2589,
            generated_loot: 0,
        },
        bag: 0,
        slot,
    };
    vec![row(9003, 24), row(9004, 25)]
}

impl FakeCharQueries {
    fn new() -> Self {
        Self {
            chars: Mutex::new(vec![(
                42,
                CharEnumRow {
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
                    // CHARACTER_FLAG_RENAME: forced
                    // rename pending (rename probe).
                    character_flags: 0x0000_4000,
                    played_time_total: 60,
                    pet_entry: None,
                    pet_display_id: None,
                    pet_level: None,
                    equipment_cache: Some("25 0".to_string()),
                },
            )]),
            hunter_mode: false,
            honor_mode: false,
            ghost_corpse_age_secs: None,
            gift_mode: false,
        }
    }

    fn new_ghost() -> Self {
        let mut q = Self::new();
        q.ghost_corpse_age_secs = Some(3_600);
        q
    }

    fn new_ghost_expired_corpse() -> Self {
        let mut q = Self::new();
        q.ghost_corpse_age_secs = Some(3 * 86_400 + 60);
        q
    }

    /// Enumtest as a character who logged out dead:
    /// `CHARACTER_FLAG_GHOST`, 1 hp (fork repop
    /// health), and the saved Ghost aura (8326)
    /// the fork's `_SaveAuras` persists.
    fn ghost_load() -> CharacterLoad {
        use mangos_db::character_load::AuraRow;
        let mut load = Self::enumtest_load();
        load.row.character_flags = 0x0000_2000;
        load.row.health = 1;
        load.auras = vec![AuraRow {
            caster_guid: mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, 7)
                .raw(),
            item_guid: 0,
            spell: 8326,
            stacks: 1,
            charges: 0,
            base_points: [0.0; 3],
            periodic_time: [0; 3],
            max_duration: -1,
            duration: -1,
            effect_index_mask: 1,
        }];
        load
    }

    fn new_with_honor() -> Self {
        let mut q = Self::new();
        q.honor_mode = true;
        q
    }

    /// Enumtest with honor history: rank points in
    /// the rank-7 band, stored lifetime 100 HK /
    /// 2 DK, and CP rows spread over today (3 HK,
    /// 1 DK, 1 bonus), yesterday (2 HK), two days
    /// ago (1 HK), and ten days ago (5 HK, outside
    /// the test week window of today-3).
    fn honor_load() -> CharacterLoad {
        use mangos_db::character_load::HonorCpRow;
        let today = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            / 86_400) as u32;
        let hk = |date: u32, cp: f32| HonorCpRow {
            victim_type: 4,
            victim_id: 55,
            cp,
            date,
            kind: 1,
        };
        let mut load = Self::enumtest_load();
        load.row.honor_rank_points = 7500.0;
        load.row.honor_highest_rank = 0;
        load.row.honor_standing = 42;
        load.row.honor_last_week_hk = 11;
        load.row.honor_last_week_cp = 1234.0;
        load.row.honor_stored_hk = 100;
        load.row.honor_stored_dk = 2;
        load.honor_cp = vec![
            hk(today, 100.0),
            hk(today, 100.0),
            hk(today, 100.0),
            HonorCpRow {
                victim_type: 3,
                victim_id: 9209,
                cp: 100.0,
                date: today,
                kind: 2,
            },
            HonorCpRow {
                victim_type: 0,
                victim_id: 7,
                cp: 50.0,
                date: today,
                kind: 3,
            },
            hk(today - 1, 90.0),
            hk(today - 1, 90.0),
            hk(today - 2, 80.0),
            hk(today - 10, 70.0),
            hk(today - 10, 70.0),
            hk(today - 10, 70.0),
            hk(today - 10, 70.0),
            hk(today - 10, 70.0),
        ];
        load
    }

    fn new_hunter() -> Self {
        let mut q = Self::new();
        q.hunter_mode = true;
        if let Some((_, row)) = q.chars.lock().expect("mutex").first_mut() {
            row.class = 3;
        }
        q
    }

    fn new_gift_hunter() -> Self {
        let mut q = Self::new_hunter();
        q.gift_mode = true;
        q
    }

    /// Enumtest rebadged as a level-12 hunter with a
    /// current pet (id 501, entry 9101) and a stabled
    /// pet (id 502, entry 9102, stable slot 1); one
    /// stable slot purchased, deep pockets.
    fn hunter_load() -> CharacterLoad {
        let mut load = Self::enumtest_load();
        load.row.class = 3;
        load.row.money = 100_000;
        load.row.stable_slots = 1;
        let pet_row = |id: u32, entry: u32, slot: u32, name: &str| mangos_db::pet::PetRow {
            id,
            entry,
            owner_guid: 7,
            display_id: 49,
            level: 12,
            xp: 0,
            react_state: 1,
            loyalty_points: 0,
            loyalty: 1,
            training_points: 0,
            slot,
            name: name.to_string(),
            renamed: 1,
            current_health: 100,
            current_mana: 0,
            current_happiness: 100_000,
            action_bar: [(0, 0); 10],
            teach_spells: Vec::new(),
            save_time: 0,
            reset_talents_cost: 0,
            reset_talents_time: 0,
            created_by_spell: 0,
            pet_type: 1, // PET_TYPE_HUNTER
            spells: Vec::new(),
        };
        load.pets = vec![pet_row(501, 9101, 0, "Fang"), pet_row(502, 9102, 1, "Claw")];
        // Tier 4 B0-5 summon-object fixture spells.
        load.spells.push(SpellRow {
            spell: 9200,
            active: 1,
            disabled: 0,
        });
        load.spells.push(SpellRow {
            spell: 9205,
            active: 1,
            disabled: 0,
        });
        // Tier 4 B0-10 loot fixture: one-shot kill
        // spell for the lootable critter.
        load.spells.push(SpellRow {
            spell: 9210,
            active: 1,
            disabled: 0,
        });
        // Slice 1 skinning fixture: the skinning cast
        // plus a mid-rank skinning skill (60/75: orange
        // for the level-14 beast, so the gain roll
        // always lands when the cast goes through).
        load.spells.push(SpellRow {
            spell: 9211,
            active: 1,
            disabled: 0,
        });
        // Slice 2 pickpocket fixture: the pickpocket
        // cast for the humanoid victims.
        load.spells.push(SpellRow {
            spell: 9212,
            active: 1,
            disabled: 0,
        });
        load.skills.push(mangos_db::character_load::SkillRow {
            skill: 393,
            value: 60,
            max: 75,
        });
        load
    }

    fn enumtest_load() -> CharacterLoad {
        CharacterLoad {
            row: CharacterRow {
                guid: 7,
                account: 42,
                name: "Enumtest".into(),
                race: 1,
                class: 1,
                gender: 0,
                level: 12,
                xp: 0,
                money: 100,
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
                // Fork `WORLD_DEFAULT_CHAR` (0x01);
                // 0 is WORLD_ALONE (sees nothing).
                world_phase_mask: 1,
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
            inventory: seed_inventory(),
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
            // Axes skill: sword prototype subclass 0
            // proficiency for the equip probe.
            skills: vec![mangos_db::character_load::SkillRow {
                skill: 44,
                value: 1,
                max: 1,
            }],
            mails: loopback_mail_seed(),
            mailed_items: vec![],
            forgotten_skills: vec![],
            pets: Vec::new(),
        }
    }
}

impl CharQueries for FakeCharQueries {
    fn char_enum(&self, account_id: u32) -> Result<Vec<CharEnumRow>, DbError> {
        assert_eq!(account_id, 42);
        Ok(self
            .chars
            .lock()
            .expect("mutex")
            .iter()
            .filter(|(account, _)| *account == account_id)
            .map(|(_, row)| row.clone())
            .collect())
    }

    fn load_character(&self, guid_low: u32) -> Result<Option<CharacterLoad>, DbError> {
        if guid_low != 7 {
            // A lifecycle-created character: the Enumtest
            // template rebadged as a never-played level-1
            // with an empty inventory (first-login seed).
            let created = self
                .chars
                .lock()
                .expect("mutex")
                .iter()
                .find(|(_, row)| row.guid == guid_low)
                .map(|(_, row)| row.clone());
            let Some(row) = created else {
                return Ok(None);
            };
            let mut load = Self::enumtest_load();
            load.row.guid = row.guid;
            load.row.name = row.name;
            load.row.level = row.level;
            load.row.played_time_total = 0;
            load.row.played_time_level = 0;
            load.guild = None;
            load.social = vec![];
            load.mails = vec![];
            return Ok(Some(load));
        }
        Ok(Some(if self.ghost_corpse_age_secs.is_some() {
            Self::ghost_load()
        } else if self.honor_mode {
            Self::honor_load()
        } else if self.hunter_mode {
            let mut load = Self::hunter_load();
            if self.gift_mode {
                load.inventory.extend(gift_rows());
            }
            load
        } else {
            Self::enumtest_load()
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

    fn name_in_use(&self, name: &str) -> Result<bool, DbError> {
        Ok(self
            .chars
            .lock()
            .expect("mutex")
            .iter()
            .any(|(_, row)| row.name == name))
    }

    fn character_count(&self, account_id: u32) -> Result<u32, DbError> {
        Ok(self
            .chars
            .lock()
            .expect("mutex")
            .iter()
            .filter(|(account, _)| *account == account_id)
            .count() as u32)
    }

    fn character_owner(
        &self,
        guid_low: u32,
    ) -> Result<Option<mangos_db::characters::CharacterOwner>, DbError> {
        Ok(self
            .chars
            .lock()
            .expect("mutex")
            .iter()
            .find(|(_, row)| row.guid == guid_low)
            .map(|(account, row)| mangos_db::characters::CharacterOwner {
                account_id: *account,
                name: row.name.clone(),
                level: u32::from(row.level),
            }))
    }

    fn create_character(&self, new: &mangos_db::characters::NewCharacter) -> Result<u32, DbError> {
        let mut chars = self.chars.lock().expect("mutex");
        let guid = chars.iter().map(|(_, row)| row.guid).max().unwrap_or(0) + 1;
        chars.push((
            new.account_id,
            CharEnumRow {
                guid,
                name: new.name.clone(),
                race: new.race,
                class: new.class,
                gender: new.gender,
                skin: new.skin,
                face: new.face,
                hair_style: new.hair_style,
                hair_color: new.hair_color,
                facial_hair: new.facial_hair,
                level: new.level as u8,
                zone: 0,
                map: new.map_id,
                position_x: new.position_x,
                position_y: new.position_y,
                position_z: new.position_z,
                guild_id: None,
                character_flags: 0,
                played_time_total: 0,
                pet_entry: None,
                pet_display_id: None,
                pet_level: None,
                equipment_cache: Some(new.equipment_cache.clone()),
            },
        ));
        Ok(guid)
    }

    fn delete_character(
        &self,
        guid_low: u32,
        _plan: &mangos_db::characters::DeletePlan,
    ) -> Result<(), DbError> {
        self.chars
            .lock()
            .expect("mutex")
            .retain(|(_, row)| row.guid != guid_low);
        Ok(())
    }

    fn rename_character(
        &self,
        guid_low: u32,
        account_id: u32,
        new_name: &str,
    ) -> Result<Option<String>, DbError> {
        const RENAME: u32 = 0x0000_4000;
        const NEEDS_GM_REVIEW: u32 = 0x0000_8000;
        let mut chars = self.chars.lock().expect("mutex");
        if chars.iter().any(|(_, row)| row.name == new_name) {
            return Ok(None);
        }
        let Some((account, row)) = chars.iter_mut().find(|(_, row)| row.guid == guid_low) else {
            return Ok(None);
        };
        if *account != account_id || row.character_flags & RENAME == 0 {
            return Ok(None);
        }
        let old = std::mem::replace(&mut row.name, new_name.to_owned());
        row.character_flags = (row.character_flags & !RENAME) | NEEDS_GM_REVIEW;
        Ok(Some(old))
    }

    fn old_deleted_characters(
        &self,
        _keep_days: u32,
        _now: u64,
    ) -> Result<Vec<(u32, u32)>, DbError> {
        Ok(Vec::new())
    }

    fn sync_realm_char_count(&self, _account_id: u32, _count: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn save_corpse(&self, _row: &mangos_db::corpse::CorpseRow) -> Result<(), DbError> {
        Ok(())
    }

    fn delete_corpse(&self, _player_low: u32) -> Result<(), DbError> {
        Ok(())
    }

    fn load_corpses(&self) -> Result<Vec<mangos_db::corpse::LoadedCorpse>, DbError> {
        let Some(corpse_age_secs) = self.ghost_corpse_age_secs else {
            return Ok(Vec::new());
        };
        // Enumtest's resurrectable corpse at the
        // login spot. The fresh variant is an hour old
        // (past the 30 s reclaim delay); the expired
        // variant crosses the 3-day expiry to exercise
        // seeded bones.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Ok(vec![mangos_db::corpse::LoadedCorpse {
            row: mangos_db::corpse::CorpseRow {
                guid: 42,
                player_guid: 7,
                position_x: -8949.95,
                position_y: -132.493,
                position_z: 83.5312,
                orientation: 1.5,
                map: 0,
                time: now - corpse_age_secs,
                corpse_type: 1,
                instance: 0,
            },
            gender: 0,
            race: 1,
            class: 1,
            skin: 1,
            face: 2,
            hair_style: 3,
            hair_color: 4,
            facial_hair: 5,
            equipment_cache: None,
            guild_id: 0,
            character_flags: 0,
        }])
    }

    fn max_corpse_guid(&self) -> Result<u32, DbError> {
        Ok(0)
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

struct FakeQuestRewardStore {
    next_item_guid: std::sync::atomic::AtomicU32,
    /// In-memory `character_gifts` stand-in: the
    /// wrapped-gift fixture rows (`item_guid` ->
    /// `(item_id, flags)`). `gift_unwrap` removes on
    /// hit, mirroring the fork's unwrap-time delete.
    gifts: Mutex<HashMap<u32, (u32, u32)>>,
}

impl FakeQuestRewardStore {
    fn new() -> Self {
        Self {
            next_item_guid: std::sync::atomic::AtomicU32::new(990_001),
            // Wrapped-gift fixture: 9003 restores to a
            // plain sword (25, flags 0); 9004 has no
            // gift row (fork miss arm: destroy).
            gifts: Mutex::new(HashMap::from([(9003, (25, 0))])),
        }
    }
}

impl mangosd::world_actor::QuestRewardStore for FakeQuestRewardStore {
    fn create_reward_item(
        &self,
        _owner_low: u32,
        _item_id: u32,
        _count: u32,
        _bag: u32,
        _slot: u32,
        _creator_low: u32,
        _durability: u32,
    ) -> Result<u32, mangos_db::DbError> {
        Ok(self
            .next_item_guid
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst))
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

    fn gift_unwrap(&self, item_guid: u32) -> Result<Option<(u32, u32)>, mangos_db::DbError> {
        Ok(self.gifts.lock().expect("gifts mutex").remove(&item_guid))
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

struct FakeHonorMarker;

impl mangosd::world_actor::HonorMarkerStore for FakeHonorMarker {
    fn set_honor_maintenance_marker(&self, _marker: bool) -> Result<(), DbError> {
        Ok(())
    }
}

struct FakeGameEventStatus;

impl mangosd::world_actor::GameEventStatusStore for FakeGameEventStatus {
    fn game_event_started(&self, _event: u16) -> Result<(), DbError> {
        Ok(())
    }

    fn game_event_ended(&self, _event: u16) -> Result<(), DbError> {
        Ok(())
    }
}

/// Capturing saver: records every bundle so tests
/// can observe the persisted shape (append-only
/// observation, no save-shape change).
#[derive(Default)]
struct FakeCharacterSaver {
    saved: Mutex<
        Vec<(
            mangos_db::character_save::CharacterSaveBundle,
            Option<mangos_db::character_save::CharacterStatsRow>,
        )>,
    >,
}

impl mangosd::world_actor::CharacterSaver for FakeCharacterSaver {
    fn save_character(
        &self,
        bundle: mangos_db::character_save::CharacterSaveBundle,
        stats: Option<mangos_db::character_save::CharacterStatsRow>,
    ) -> Result<(), DbError> {
        self.saved
            .lock()
            .expect("saver mutex")
            .push((bundle, stats));
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
    // Starter loadout for the create/delete lifecycle
    // test: a main-hand weapon (equip path) plus linen
    // (backpack stack path).
    stores
        .player_infos
        .set_create_items_for_tests(HashMap::from([(
            (1, 1),
            vec![
                mangos_world_data::player_info::PlayerCreateItem {
                    item_id: 25,
                    amount: 1,
                },
                mangos_world_data::player_info::PlayerCreateItem {
                    item_id: 2589,
                    amount: 3,
                },
            ],
        )]));
    // Linen cloth for the trade gate checks.
    let linen = mangos_world_data::item_prototype::ItemPrototype {
        item_id: 2589,
        class: 7,
        display_id: 100,
        stackable: 20,
        buy_count: 1,
        buy_price: 40,
        sell_price: 10,
        // DB rows carry -1 (unrestricted).
        allowable_class: u32::MAX,
        allowable_race: u32::MAX,
        ..Default::default()
    };
    let sword = mangos_world_data::item_prototype::ItemPrototype {
        item_id: 25,
        class: 2,
        display_id: 1542,
        inventory_type: 21,
        stackable: 1,
        max_durability: 20,
        buy_count: 1,
        buy_price: 200,
        sell_price: 50,
        item_level: 1,
        quality: 1,
        allowable_class: u32::MAX,
        allowable_race: u32::MAX,
        ..Default::default()
    };
    stores.item_prototypes =
        mangos_world_data::item_prototype::ItemPrototypes::from_entries_for_tests(HashMap::from([
            (2589, linen),
            (25, sword),
        ]));
    stores
}

/// Vendor fixture: one repair-capable vendor next to
/// the seed player, limited linen + unlimited sword
/// stock, and level-1 durability tables (sword repair
/// = lost 9 * multiplier 2 * quality-mod 1.0 = 18).
fn fake_vendor_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::durability::{DurabilityCostEntry, DurabilityTables};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};
    use mangos_world_data::vendor_items::{VendorItem, VendorItemStores};

    let mut stores = fake_world_stores();
    let info = CreatureInfo {
        entry: 66,
        name: "Vendortest".into(),
        faction: 35,
        npc_flags: 0x1104, // VENDOR | BANKER | REPAIR
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![info]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let mut spawn = CreatureSpawn::default();
    spawn.creature_id[0] = 66;
    spawn.position = WorldLocation {
        map_id: 0,
        x: -8948.95,
        y: -132.493,
        z: 83.5312,
        o: 0.0,
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4242, spawn)]);
    stores.vendor_items = VendorItemStores::from_maps_for_tests(
        HashMap::from([(
            66,
            vec![
                VendorItem {
                    item_id: 2589,
                    max_count: 5,
                    incr_time: 3600,
                    item_flags: 0,
                    condition_id: 0,
                },
                VendorItem {
                    item_id: 25,
                    max_count: 0,
                    incr_time: 0,
                    item_flags: 0,
                    condition_id: 0,
                },
            ],
        )]),
        HashMap::new(),
    );
    let mut multipliers = [1u32; 29];
    multipliers[0] = 2;
    stores.bank_bag_slot_prices = vec![0, 10];
    stores.durability = DurabilityTables::from_entries(
        vec![DurabilityCostEntry {
            item_level: 1,
            multipliers,
        }],
        vec![(4, 1.0)],
    );
    stores
}

/// Trainer fixture (4e): a warrior class trainer
/// (entry 67) and a mage class trainer (entry 68)
/// beside the seed player (human warrior, level 12,
/// 100 copper, knows spell 668). Both teach:
/// wrapper 90001 -> 90010 (level 10, cost 50: GREEN),
/// wrapper 90002 -> 90011 (level 20: RED), and
/// wrapper 90003 -> 668 (already known: GRAY).
#[allow(clippy::too_many_lines)] // one fixture assembly
fn fake_trainer_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};
    use mangos_world_data::trainer_spells::{TrainerSpell, TrainerSpellData, TrainerStores};

    let mut stores = fake_world_stores();
    let trainer = |entry: u32, class: u32| CreatureInfo {
        entry,
        name: "Trainertest".into(),
        faction: 35,
        npc_flags: 0x10, // TRAINER
        civilian: true,
        unit_class: 1,
        trainer_type: 0, // TRAINER_TYPE_CLASS
        trainer_class: class,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates =
        CreatureTemplates::from_entries_for_tests(vec![trainer(67, 1), trainer(68, 8)]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let spawn_for = |entry: u32| {
        let mut spawn = CreatureSpawn::default();
        spawn.creature_id[0] = entry;
        spawn.position = WorldLocation {
            map_id: 0,
            x: -8948.95,
            y: -132.493,
            z: 83.5312,
            o: 0.0,
        };
        spawn
    };
    stores.spawns =
        SpawnData::from_creatures_for_tests(vec![(4243, spawn_for(67)), (4244, spawn_for(68))]);

    let wrapper = |id: u32, taught: u32| SpellEntry {
        id,
        effects: [
            SpellEffectData {
                effect: 36, // SPELL_EFFECT_LEARN_SPELL
                trigger_spell: taught,
                ..Default::default()
            },
            SpellEffectData::default(),
            SpellEffectData::default(),
        ],
        ..Default::default()
    };
    let taught = |id: u32, spell_level: u32| SpellEntry {
        id,
        spell_level,
        ..Default::default()
    };
    for entry in [
        wrapper(90001, 90010),
        wrapper(90002, 90011),
        wrapper(90003, 668),
        taught(90010, 10),
        taught(90011, 20),
        taught(668, 0),
    ] {
        stores.spells.spells.insert(entry.id, entry);
    }
    let list = TrainerSpellData {
        spells: vec![
            TrainerSpell {
                spell: 90001,
                spell_cost: 50,
                req_skill: 0,
                req_skill_value: 0,
                req_level: 10,
            },
            TrainerSpell {
                spell: 90002,
                spell_cost: 10,
                req_skill: 0,
                req_skill_value: 0,
                req_level: 20,
            },
            TrainerSpell {
                spell: 90003,
                spell_cost: 10,
                req_skill: 0,
                req_skill_value: 0,
                req_level: 0,
            },
        ],
        trainer_type: 0,
    };
    stores.trainer_spells = TrainerStores::from_maps_for_tests(
        HashMap::from([(67, list.clone()), (68, list)]),
        HashMap::new(),
        HashMap::new(),
    );
    stores
}

/// Dynamic-GO fixture (Tier 4 B0-5): the stable
/// fixture (hunter login) plus a critter beside the
/// player, a static environmental trap GO one yard
/// away (spell 9203, ~5 damage), and summon-object
/// spells: 9200 slot-1 -> trap 9202 (charges 1,
/// spell 9206 ~7 damage), 9205 wild -> generic 9201.
#[allow(clippy::too_many_lines)] // one fixture assembly
fn fake_go_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::gameobject_template::{GameObjectInfo, GameObjectTemplates};
    use mangos_world_data::spawns::{CreatureSpawn, GameObjectSpawn, SpawnData, WorldLocation};

    let mut stores = fake_stable_world_stores();
    let critter = CreatureInfo {
        entry: 9207,
        name: "Crittertest".into(),
        faction: 35,
        creature_type: 8, // CRITTER (NullAI)
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    // Keep the stable master + pet beasts, add the
    // critter (stable fixture templates are replaced
    // wholesale, so rebuild the list from its shape).
    let master = CreatureInfo {
        entry: 69,
        name: "Stabletest".into(),
        faction: 35,
        npc_flags: 0x2000,
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    let beast = |entry: u32| CreatureInfo {
        entry,
        name: "Beasttest".into(),
        faction: 35,
        creature_type: 1,
        pet_family: 1,
        static_flags1: 0x10,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates =
        CreatureTemplates::from_entries_for_tests(vec![master, beast(9101), beast(9102), critter]);

    let trap_info = |id: u32, spell: i32, charges: i32| {
        let mut data = [0i32; 24];
        data[2] = 3; // radius
        data[3] = spell;
        data[4] = charges;
        data[5] = 4; // cooldown secs
        GameObjectInfo {
            id,
            go_type: 6,
            display_id: 49,
            name: "Traptest".into(),
            faction: 35,
            size: 1.0,
            data,
            ..Default::default()
        }
    };
    let generic = GameObjectInfo {
        id: 9201,
        go_type: 5,
        display_id: 49,
        name: "Wildtest".into(),
        faction: 35,
        size: 1.0,
        ..Default::default()
    };
    stores.gameobject_templates = GameObjectTemplates::from_entries_for_tests(vec![
        generic,
        trap_info(9202, 9206, 1),
        trap_info(9204, 9203, 0),
    ]);

    let mut critter_spawn = CreatureSpawn::default();
    critter_spawn.creature_id[0] = 9207;
    critter_spawn.position = WorldLocation {
        map_id: 0,
        x: -8949.5,
        y: -131.0,
        z: 83.5312,
        o: 0.0,
    };
    let mut spawns = SpawnData::from_creatures_for_tests(vec![(4247, critter_spawn)]);
    let trap_spawn = GameObjectSpawn {
        id: 9204,
        position: WorldLocation {
            map_id: 0,
            x: -8948.95,
            y: -132.493,
            z: 83.5312,
            o: 0.0,
        },
        spawntimesecsmin: 25,
        spawntimesecsmax: 25,
        animprogress: 100,
        ..Default::default()
    };
    spawns.insert_gameobject_for_tests(4246, trap_spawn);
    stores.spawns = spawns;

    stores.spells.tables.ranges.insert(
        9001,
        mangos_spells::entry::RangeEntry {
            min: 0.0,
            max: 100.0,
        },
    );
    let damage = |id: u32, base_points: i32| SpellEntry {
        id,
        range_index: 9001,
        effects: [
            SpellEffectData {
                effect: 2, // SCHOOL_DAMAGE
                base_points,
                die_sides: 1,
                base_dice: 1,
                implicit_target_a: 6, // UNIT_ENEMY
                ..Default::default()
            },
            SpellEffectData::default(),
            SpellEffectData::default(),
        ],
        ..Default::default()
    };
    let summon = |id: u32, effect: u32, misc: i32| SpellEntry {
        id,
        effects: [
            SpellEffectData {
                effect,
                misc_value: misc,
                implicit_target_a: 1, // UNIT_CASTER
                ..Default::default()
            },
            SpellEffectData::default(),
            SpellEffectData::default(),
        ],
        ..Default::default()
    };
    for entry in [
        summon(9200, 104, 9202),
        summon(9205, 76, 9201),
        damage(9203, 4),
        damage(9206, 6),
    ] {
        stores.spells.spells.insert(entry.id, entry);
    }
    stores
}

/// Stable fixture (4h): stable master 69 one yard
/// from the seed player; tameable beast templates
/// 9101/9102 for the hunter's pets.
fn fake_stable_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};

    let mut stores = fake_world_stores();
    let master = CreatureInfo {
        entry: 69,
        name: "Stabletest".into(),
        faction: 35,
        npc_flags: 0x2000, // STABLEMASTER
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    let beast = |entry: u32| CreatureInfo {
        entry,
        name: "Beasttest".into(),
        faction: 35,
        creature_type: 1,    // BEAST
        pet_family: 1,       // Wolf
        static_flags1: 0x10, // TAMEABLE
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates =
        CreatureTemplates::from_entries_for_tests(vec![master, beast(9101), beast(9102)]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let mut spawn = CreatureSpawn::default();
    spawn.creature_id[0] = 69;
    spawn.position = WorldLocation {
        map_id: 0,
        x: -8948.95,
        y: -132.493,
        z: 83.5312,
        o: 0.0,
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4245, spawn)]);
    stores.stable_slot_prices = vec![0, 10, 50];
    // Hunter (race 1, class 3) create/level info,
    // mirroring the warrior seed in fake_world_stores
    // (login materialization requires it).
    {
        use mangos_world_data::player_info::{
            PlayerClassLevelInfo, PlayerCreateInfo, PlayerInfos, PlayerLevelInfo,
        };
        let create = PlayerCreateInfo {
            map_id: 0,
            area_id: 12,
            position_x: -8949.95,
            position_y: -132.493,
            position_z: 83.5312,
            orientation: 1.5,
            display_id_m: 49,
            display_id_f: 50,
        };
        stores.player_infos = PlayerInfos::from_maps_for_tests(
            60,
            HashMap::from([((1, 1), create), ((1, 3), create)]),
            HashMap::from([
                (
                    1,
                    vec![
                        PlayerClassLevelInfo {
                            basehealth: 20,
                            basemana: 0
                        };
                        60
                    ],
                ),
                (
                    3,
                    vec![
                        PlayerClassLevelInfo {
                            basehealth: 20,
                            basemana: 20
                        };
                        60
                    ],
                ),
            ]),
            HashMap::from([
                ((1, 1), vec![PlayerLevelInfo { stats: [20; 5] }; 60]),
                ((1, 3), vec![PlayerLevelInfo { stats: [20; 5] }; 60]),
            ]),
            vec![400; 61],
        );
    }
    stores
}

/// Taxi fixture (4f): two flight masters 1 and 3
/// yards from the seed player, at taxi nodes 1 and
/// 2 respectively; path 1 flies 1 -> 2 for 10
/// copper over three vertices. The mount creature
/// is template 70 (display 49, never spawned).
fn fake_taxi_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};
    use mangos_world_data::taxi::{TaxiNode, TaxiPath, TaxiPathNode, TaxiStore};

    let mut stores = fake_world_stores();
    let info = |entry: u32, flags: u32| CreatureInfo {
        entry,
        name: "Flighttest".into(),
        faction: 35,
        npc_flags: flags,
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates =
        CreatureTemplates::from_entries_for_tests(vec![info(69, 0x8), info(70, 0), info(71, 0x8)]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let spawn_at = |entry: u32, x: f32| {
        let mut spawn = CreatureSpawn::default();
        spawn.creature_id[0] = entry;
        spawn.position = WorldLocation {
            map_id: 0,
            x,
            y: -132.493,
            z: 83.5312,
            o: 0.0,
        };
        spawn
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![
        (4245, spawn_at(69, -8948.95)),
        (4246, spawn_at(71, -8946.95)),
    ]);
    let node = |id: u32, x: f32| TaxiNode {
        id,
        map_id: 0,
        x,
        y: -132.493,
        z: 83.5312,
        mount_creature_id: [70, 70],
    };
    let vertex = |index: u32, x: f32, z: f32| TaxiPathNode {
        path: 1,
        index,
        map_id: 0,
        x,
        y: -132.493,
        z,
        action_flag: 0,
        delay: 0,
    };
    stores.taxi = TaxiStore::from_parts_for_tests(
        vec![node(1, -8948.95), node(2, -8946.95)],
        vec![TaxiPath {
            id: 1,
            from: 1,
            to: 2,
            price: 10,
        }],
        vec![
            vertex(0, -8948.95, 83.5312),
            vertex(1, -8947.95, 93.5312),
            vertex(2, -8946.95, 83.5312),
        ],
    );
    stores
}

/// Talent fixture (4g): warrior tab 1 with talent
/// 500 (row 0, ranks 9101/9102) and talent 501
/// (row 1, rank 9103, locked until 5 points spent
/// in the tab). The seed player is a level-12
/// warrior: 3 free talent points, 100 copper.
fn fake_talent_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEntry, TalentEntry};

    // Trainer fixture base: creature 67 (warrior
    // trainer at spawn 4243) serves the wipe gate.
    let mut stores = fake_trainer_world_stores();
    for id in [9101u32, 9102, 9103] {
        stores.spells.spells.insert(
            id,
            SpellEntry {
                id,
                ..Default::default()
            },
        );
    }
    let talent = |id: u32, row: u32, ranks: [u32; 5]| TalentEntry {
        id,
        tab: 1,
        row,
        col: 0,
        rank_spells: ranks,
        depends_on: 0,
        depends_on_rank: 0,
        depends_on_spell: 0,
    };
    stores.spells.talents = HashMap::from([
        (500, talent(500, 0, [9101, 9102, 0, 0, 0])),
        (501, talent(501, 1, [9103, 0, 0, 0, 0])),
    ]);
    stores.spells.talent_tabs = HashMap::from([(1, 0x1)]);
    stores.spells.talent_spell_pos =
        mangos_spells::SpellStore::build_talent_spell_pos(&stores.spells.talents);
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
            res_sickness_spell_id: 15007,
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
            res_sickness_spell_id: 15007,
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
            res_sickness_spell_id: 15007,
            team_id: 1,
        },
    );
    dbc.classes.insert(1);
    dbc.class_power.insert(1, 1);
    // Hunter (Tier 4 step 4h stable fixture): mana.
    dbc.classes.insert(3);
    dbc.class_power.insert(3, 0);
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
    start_server_with_stores(backend, fake_world_stores()).await
}

async fn start_server_with_stores(
    backend: Arc<FakeBackend>,
    world_stores: mangos_map::WorldStores,
) -> u16 {
    start_server_with_queries(backend, world_stores, Arc::new(FakeCharQueries::new())).await
}

async fn start_server_with_queries(
    backend: Arc<FakeBackend>,
    world_stores: mangos_map::WorldStores,
    queries: Arc<FakeCharQueries>,
) -> u16 {
    Box::pin(start_server_with_queries_cfg(
        backend,
        world_stores,
        queries,
        test_config(),
        0,
    ))
    .await
}

async fn start_server_with_capture(
    backend: Arc<FakeBackend>,
    world_stores: mangos_map::WorldStores,
    queries: Arc<FakeCharQueries>,
) -> (u16, Arc<FakeCharacterSaver>) {
    let saver = Arc::new(FakeCharacterSaver::default());
    let port = Box::pin(start_server_with_queries_cfg_with_saver(
        backend,
        world_stores,
        queries,
        test_config(),
        0,
        Arc::clone(&saver),
    ))
    .await;
    (port, saver)
}

async fn start_server_with_queries_cfg(
    backend: Arc<FakeBackend>,
    world_stores: mangos_map::WorldStores,
    queries: Arc<FakeCharQueries>,
    config: ServerConfig,
    honor_week_begin: u32,
) -> u16 {
    Box::pin(start_server_with_queries_cfg_with_saver(
        backend,
        world_stores,
        queries,
        config,
        honor_week_begin,
        Arc::new(FakeCharacterSaver::default()),
    ))
    .await
}

async fn start_server_with_queries_cfg_with_saver(
    backend: Arc<FakeBackend>,
    world_stores: mangos_map::WorldStores,
    queries: Arc<FakeCharQueries>,
    config: ServerConfig,
    honor_week_begin: u32,
    saver: Arc<FakeCharacterSaver>,
) -> u16 {
    let log = Arc::new(
        Log::open(
            std::path::Path::new(""),
            "",
            false,
            mangos_log::Level::Debug,
        )
        .expect("disabled log"),
    );
    let char_queries: Arc<dyn CharQueries> = queries;
    let (world_tx, _world, _health) = spawn_world(
        config.clone(),
        Arc::clone(&log),
        mangos_world_data::strings::MangosStrings::default(),
        WorldDeps {
            enum_stores: Arc::new(fake_enum_stores()),
            char_queries: Arc::clone(&char_queries),
            realm_publisher: Arc::new(FakeRealmPublisher),
            maps_dir: None,
            mmaps_dir: None,
            vmaps_dir: None,
            world_stores: Arc::new(world_stores),
            race_class_dbc: Arc::new(fake_race_class_dbc()),
            respawn_times: mangos_world_data::spawn_support::RespawnTimes::default(),
            presence: Arc::new(FakePresence),
            character_saver: saver,
            social: Arc::new(FakeSocialStore),
            group: Arc::new(FakeGroupStore),
            initial_groups: mangos_db::group::LoadedGroups::default(),
            guild: Arc::new(FakeGuildStore),
            initial_guilds: loopback_guild_seed(),
            petition: Arc::new(FakePetitionStore),
            initial_petitions: loopback_petition_seed(),
            mail: Arc::new(FakeMailStore),
            pets: Arc::new(FakePetStore),
            quest_reward: Arc::new(FakeQuestRewardStore::new()),
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
            initial_honor_week_begin: honor_week_begin,
            initial_honor_next_day: u32::MAX,
            honor_marker: Arc::new(FakeHonorMarker),
            initial_shutdown_events: Vec::new(),
            game_event_status: Arc::new(FakeGameEventStatus),
            initial_guild_bots: mangos_db::guild_bots::LoadedGuildBots::default(),
        },
    );
    let deps = Arc::new(ConnectionDeps {
        config: Arc::new(config),
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
        .send_packet(
            mangos_protocol::opcode_table::opcode_id::CMSG_NAME_QUERY,
            &7u64.to_le_bytes(),
        )
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
    let mut update_bodies: Vec<Vec<u8>> = Vec::new();
    let body = loop {
        let (op, body) = client.read_packet().await.expect("pong or update");
        match op {
            opcode::SMSG_PONG => break body,
            169 | 502 => {
                saw_update = true; // UPDATE_OBJECT / COMPRESSED
                update_bodies.push(if op == 502 {
                    use std::io::Read as _;
                    let mut out = Vec::new();
                    flate2::read::ZlibDecoder::new(&body[4..])
                        .read_to_end(&mut out)
                        .expect("zlib update body");
                    out
                } else {
                    body
                });
            }
            706 => saw_world_states = true, // INIT_WORLD_STATES
            756 => saw_weather = true,      // WEATHER
            146 => saw_guild_event = true,  // GUILD_EVENT (login MOTD/signed-on)
            other => panic!("unexpected opcode {other} before pong"),
        }
    };
    assert!(saw_update, "init-self object update after login");
    // Item creates ride the update stream: both seed
    // item GUIDs appear as OBJECT_FIELD_GUID words.
    for item_low in [9001u32, 9002] {
        let raw = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::ITEM, 0, item_low)
            .raw()
            .to_le_bytes();
        assert!(
            update_bodies.iter().any(|b| b.windows(8).any(|w| w == raw)),
            "item {item_low} create block after login"
        );
    }
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
async fn character_create_and_delete_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let port = start_server(backend).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);

    // The real client always enums first; the fork's
    // session character count starts at the realm limit
    // until the enum lands.
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_ENUM).await;
    assert_eq!(body[0], 1, "seeded character enumerated");

    let mut create = b"Createdtest\0".to_vec();
    create.extend_from_slice(&[1, 1, 0, 1, 2, 3, 4, 5, 0]);
    client
        .send_packet(opcode_id::CMSG_CHAR_CREATE, &create)
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_CHAR_CREATE).await,
        [46] // CHAR_CREATE_SUCCESS
    );

    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_ENUM).await;
    assert_eq!(body[0], 2, "created character is enumerated");
    assert!(
        body.windows(b"Createdtest\0".len())
            .any(|w| w == b"Createdtest\0"),
        "created character name in enum"
    );
    let created_guid = {
        let mut pos = 1usize;
        let mut found = None;
        for _ in 0..body[0] {
            let guid = u64::from_le_bytes(body[pos..pos + 8].try_into().expect("guid"));
            pos += 8;
            let name_start = pos;
            while body[pos] != 0 {
                pos += 1;
            }
            let name = &body[name_start..pos];
            pos += 1;
            if name == b"Createdtest" {
                found = Some(guid);
            }
            pos += 9 + 8 + 12 + 4 + 4 + 1 + 12 + 20 * 5;
        }
        found.expect("created guid")
    };

    // First login of the created character runs the
    // starting-item grant and enters the world.
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &created_guid.to_le_bytes())
        .await;
    let _verify = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    client.send_packet(opcode::CMSG_LOGOUT_REQUEST, &[]).await;
    // Drain the login packet train + map traffic until
    // the logout lands (read_until_opcode only tolerates
    // auth-phase noise).
    let mut logged_out = false;
    while let Some((op, _)) = client.read_packet().await {
        if op == opcode_id::SMSG_LOGOUT_COMPLETE {
            logged_out = true;
            break;
        }
    }
    assert!(logged_out, "logout complete received");

    client
        .send_packet(opcode_id::CMSG_CHAR_DELETE, &created_guid.to_le_bytes())
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_CHAR_DELETE).await,
        [57] // CHAR_DELETE_SUCCESS
    );

    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_ENUM).await;
    assert_eq!(body[0], 1, "deleted character leaves the enum");
}

#[tokio::test]
async fn character_rename_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let port = start_server(backend).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);

    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_ENUM).await;
    assert_eq!(body[0], 1, "seeded character enumerated");

    // Seed guid 7 carries CHARACTER_FLAG_RENAME.
    let player_guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::PLAYER, 0, 7);
    let mut rename = player_guid.raw().to_le_bytes().to_vec();
    rename.extend_from_slice(b"Renamedtest\0");
    client
        .send_packet(opcode_id::CMSG_CHAR_RENAME, &rename)
        .await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_RENAME).await;
    assert_eq!(body[0], 0, "RESPONSE_SUCCESS");
    assert_eq!(
        u64::from_le_bytes(body[1..9].try_into().expect("guid")),
        player_guid.raw(),
        "renamed guid echoed"
    );
    assert_eq!(&body[9..], b"Renamedtest\0", "new name echoed");
    // Invalidate broadcast follows on the same socket.
    let body = client
        .read_until_opcode(opcode_id::SMSG_INVALIDATE_PLAYER)
        .await;
    assert_eq!(
        u64::from_le_bytes(body[..8].try_into().expect("guid")),
        player_guid.raw(),
        "invalidate carries the guid"
    );

    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let body = client.read_until_opcode(opcode_id::SMSG_CHAR_ENUM).await;
    assert!(
        body.windows(b"Renamedtest\0".len())
            .any(|w| w == b"Renamedtest\0"),
        "new name in enum"
    );

    // Flag consumed: a second rename hits the gate.
    let mut rename = player_guid.raw().to_le_bytes().to_vec();
    rename.extend_from_slice(b"Secondname\0");
    client
        .send_packet(opcode_id::CMSG_CHAR_RENAME, &rename)
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_CHAR_RENAME).await,
        [47], // CHAR_CREATE_ERROR on gate miss
        "rename without pending flag rejected"
    );

    // Invalid characters rejected before the DB gate.
    let mut rename = player_guid.raw().to_le_bytes().to_vec();
    rename.extend_from_slice(b"Ab1\0");
    client
        .send_packet(opcode_id::CMSG_CHAR_RENAME, &rename)
        .await;
    assert_eq!(
        client.read_until_opcode(opcode_id::SMSG_CHAR_RENAME).await,
        [73], // CHAR_NAME_MIXED_LANGUAGES
        "digit in name rejected by CheckPlayerName"
    );
}

/// Collect packets until the PONG answering our
/// flush ping (update bodies pre-decompressed).
async fn flush_packets(client: &mut Client) -> Vec<(u16, Vec<u8>)> {
    use std::io::Read as _;
    // Barrier: CMSG_NAME_QUERY travels the same
    // socket -> actor FIFO as the item packets, so
    // its response proves every earlier packet was
    // processed. (CMSG_PING cannot be the barrier:
    // it is socket-answered, and the fork kicks
    // overspeed pingers inside a 27s interval.)
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    client
        .send_packet(
            mangos_protocol::opcode_table::opcode_id::CMSG_NAME_QUERY,
            &7u64.to_le_bytes(),
        )
        .await;
    let mut out = Vec::new();
    loop {
        let (op, body) = client.read_packet().await.unwrap_or_else(|| {
            panic!(
                "packet before barrier; collected so far: {:?}",
                out.iter().map(|(o, _)| *o).collect::<Vec<_>>()
            )
        });
        if op == mangos_protocol::opcode_table::opcode_id::SMSG_NAME_QUERY_RESPONSE {
            return out;
        }
        if op == 502 {
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(&body[4..])
                .read_to_end(&mut raw)
                .expect("zlib update body");
            out.push((169, raw));
        } else {
            out.push((op, body));
        }
    }
}

/// Any item GUID (high `0x4000`) in the bodies that
/// is not one of the seeded instances. Checks both
/// the full-word form (values) and the packed form
/// (block headers: mask `0x81`, low byte, `0x40`).
fn new_item_guid(bodies: &[(u16, Vec<u8>)]) -> Option<u64> {
    // SMSG_UPDATE_OBJECT: u32 block count + u8
    // has-transport, then blocks of u8 update type
    // (0 = values, 2/3 = create) + packed GUID
    // (mask byte, then present bytes LE).
    fn packed_guid(body: &[u8], pos: &mut usize) -> Option<u64> {
        let mask = *body.get(*pos)?;
        *pos += 1;
        let mut raw = 0u64;
        for byte in 0..8 {
            if mask & (1 << byte) != 0 {
                raw |= u64::from(*body.get(*pos)?) << (8 * byte);
                *pos += 1;
            }
        }
        Some(raw)
    }
    for (_, body) in bodies {
        if body.len() < 7 {
            continue;
        }
        let count = u32::from_le_bytes(body[0..4].try_into().ok()?);
        let mut pos = 5usize;
        for _ in 0..count {
            let block_type = *body.get(pos)?;
            pos += 1;
            let raw = packed_guid(body, &mut pos)?;
            let low = raw & 0xffff_ffff;
            if (block_type == 2 || block_type == 3)
                && raw >> 48 == 0x4000
                && low != 9001
                && low != 9002
                && low != 0
            {
                return Some(raw);
            }
            if block_type == 0 {
                // Values: u8 mask-word count, mask
                // words, then one u32 per set bit.
                let words = usize::from(*body.get(pos)?);
                pos += 1;
                let mut set_bits = 0usize;
                for w in 0..words {
                    let word = u32::from_le_bytes(
                        body.get(pos + 4 * w..pos + 4 * w + 4)?.try_into().ok()?,
                    );
                    set_bits += word.count_ones() as usize;
                }
                pos += 4 * words + 4 * set_bits;
            } else {
                break;
            }
        }
    }
    None
}

#[tokio::test]
async fn inventory_slot_ops_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let port = start_server(backend).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    // Verify-world proves the async load finished
    // and the session entered the world (item
    // opcodes are LoggedIn-gated).
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    // Move: linen slot 23 -> 24. No failure packet.
    client
        .send_packet(opcode_id::CMSG_SWAP_INV_ITEM, &[23, 24])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter().all(|(op, _)| *op != 274),
        "plain move sends no inventory failure"
    );

    // Split 1 linen off slot 24 -> 25: a fresh item
    // GUID appears in the update stream.
    client
        .send_packet(opcode_id::CMSG_SPLIT_ITEM, &[255, 24, 255, 25, 1])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(got.iter().all(|(op, _)| *op != 274), "split succeeds");
    let clone_guid = new_item_guid(&got).unwrap_or_else(|| {
        panic!(
            "split clone create block; packets: {:?}",
            got.iter()
                .map(|(op, b)| (*op, b.iter().take(14).copied().collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        )
    });

    // Destroy the split stack: destroy-object for
    // the clone GUID.
    client
        .send_packet(opcode_id::CMSG_DESTROYITEM, &[255, 25, 1, 0, 0, 0])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, body)| *op == 170 && body.as_slice() == clone_guid.to_le_bytes()),
        "destroy-object for the split clone {clone_guid:#x}; got {:?}",
        got.iter().map(|(op, b)| (*op, b.len())).collect::<Vec<_>>()
    );

    // Sword (slot 15) into head slot 0: no equipment
    // slot available.
    client
        .send_packet(opcode_id::CMSG_SWAP_INV_ITEM, &[15, 0])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, body)| *op == 274 && body.first() == Some(&20)),
        "sword to head slot rejected with ITEM_CANT_BE_EQUIPPED (20); got {:?}",
        got.iter()
            .map(|(op, b)| (*op, b.first().copied()))
            .collect::<Vec<_>>()
    );

    // Unequip sword to backpack slot 26, then
    // auto-equip it back; both silent.
    client
        .send_packet(opcode_id::CMSG_SWAP_INV_ITEM, &[15, 26])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(got.iter().all(|(op, _)| *op != 274), "unequip succeeds");
    client
        .send_packet(opcode_id::CMSG_AUTOEQUIP_ITEM, &[255, 26])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(got.iter().all(|(op, _)| *op != 274), "auto-equip succeeds");

    // Proof the sword is equipped again: the head
    // swap errors (a missing source would silently
    // no-op in the fork instead).
    client
        .send_packet(opcode_id::CMSG_SWAP_INV_ITEM, &[15, 0])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, body)| *op == 274 && body.first() == Some(&20)),
        "sword present at slot 15 after auto-equip"
    );
}

/// Reads one field value for `guid` out of the
/// values blocks in flushed update bodies.
fn values_field(bodies: &[(u16, Vec<u8>)], guid: u64, field: u16) -> Option<u32> {
    fn packed_guid(body: &[u8], pos: &mut usize) -> Option<u64> {
        let mask = *body.get(*pos)?;
        *pos += 1;
        let mut raw = 0u64;
        for byte in 0..8 {
            if mask & (1 << byte) != 0 {
                raw |= u64::from(*body.get(*pos)?) << (8 * byte);
                *pos += 1;
            }
        }
        Some(raw)
    }
    // Per-body parse: a non-update packet body that
    // fails to parse is skipped, never fatal to the
    // search (the `?`s below bail only this body).
    fn scan_body(body: &[u8], guid: u64, field: u16) -> Option<u32> {
        if body.len() < 7 {
            return None;
        }
        let count = u32::from_le_bytes(body[0..4].try_into().ok()?);
        let mut pos = 5usize;
        for _ in 0..count {
            let block_type = *body.get(pos)?;
            pos += 1;
            let raw = packed_guid(body, &mut pos)?;
            if block_type != 0 {
                break;
            }
            let words = usize::from(*body.get(pos)?);
            pos += 1;
            let mut bits: Vec<u32> = Vec::new();
            for w in 0..words {
                let word =
                    u32::from_le_bytes(body.get(pos + 4 * w..pos + 4 * w + 4)?.try_into().ok()?);
                bits.push(word);
            }
            pos += 4 * words;
            let mut index = 0u16;
            for word in bits {
                for bit in 0..32 {
                    if word & (1 << bit) != 0 {
                        let value = u32::from_le_bytes(body.get(pos..pos + 4)?.try_into().ok()?);
                        if raw == guid && index == field {
                            return Some(value);
                        }
                        pos += 4;
                    }
                    index += 1;
                }
            }
        }
        None
    }
    bodies
        .iter()
        .find_map(|(_, body)| scan_body(body, guid, field))
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn vendor_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::{ITEM_FIELD_DURABILITY, PLAYER_BYTES_2};

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_stores(backend, fake_vendor_world_stores()).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let vendor = ObjectGuid::with_entry(HighGuid::UNIT, 66, 4242);
    let vendor_raw = vendor.raw().to_le_bytes();
    let sword_guid = ObjectGuid::with_entry(HighGuid::ITEM, 0, 9001).raw();
    let linen_guid = ObjectGuid::with_entry(HighGuid::ITEM, 0, 9002).raw();

    // List: guid, count 2, then per item
    // slot/item/display/stock/price/maxdur/buycount.
    client
        .send_packet(opcode_id::CMSG_LIST_INVENTORY, &vendor_raw)
        .await;
    let got = flush_packets(&mut client).await;
    let (_, list) = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_LIST_INVENTORY)
        .unwrap_or_else(|| {
            panic!(
                "list inventory; got {:?}",
                got.iter().map(|(op, b)| (*op, b.len())).collect::<Vec<_>>()
            )
        });
    assert_eq!(list.len(), 8 + 1 + 2 * 28, "two vendor rows");
    assert_eq!(list[8], 2, "item count byte");
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    assert_eq!(u32_at(list, 9), 1, "first vendor slot");
    assert_eq!(u32_at(list, 13), 2589, "first item");
    assert_eq!(u32_at(list, 21), 5, "limited stock current");
    assert_eq!(u32_at(list, 25), 40, "linen price");
    assert_eq!(u32_at(list, 37), 2, "second vendor slot");
    assert_eq!(u32_at(list, 41), 25, "second item");
    assert_eq!(u32_at(list, 49), 0xFFFF_FFFF, "unlimited stock marker");

    // Buy 1 linen bundle (40c): SMSG_BUY_ITEM slot 1,
    // remaining stock 4, count 1.
    let mut buy = vendor_raw.to_vec();
    buy.extend_from_slice(&2589u32.to_le_bytes());
    buy.extend_from_slice(&[1, 1]);
    client.send_packet(opcode_id::CMSG_BUY_ITEM, &buy).await;
    let got = flush_packets(&mut client).await;
    let (_, bought) = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_BUY_ITEM)
        .expect("buy item packet");
    assert_eq!(u32_at(bought, 8), 1, "buy slot");
    assert_eq!(u32_at(bought, 12), 4, "remaining stock");
    assert_eq!(u32_at(bought, 16), 1, "buy count");

    // Buy 5 more exceeds stock 4: ALREADY_SOLD (1).
    let mut buy5 = vendor_raw.to_vec();
    buy5.extend_from_slice(&2589u32.to_le_bytes());
    buy5.extend_from_slice(&[5, 1]);
    client.send_packet(opcode_id::CMSG_BUY_ITEM, &buy5).await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_BUY_FAILED && b.last() == Some(&1)),
        "over-stock buy rejected; got {:?}",
        got.iter()
            .map(|(op, b)| (*op, b.last().copied()))
            .collect::<Vec<_>>()
    );

    // Sell 2 linen from the seed stack (now 4 after
    // the buy merged): moves a clone to buyback, no
    // sell-error packet (fork sends none on success).
    let mut sell = vendor_raw.to_vec();
    sell.extend_from_slice(&linen_guid.to_le_bytes());
    sell.push(2);
    client.send_packet(opcode_id::CMSG_SELL_ITEM, &sell).await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter().all(|(op, _)| *op != opcode_id::SMSG_SELL_ITEM),
        "sell success sends no SMSG_SELL_ITEM"
    );

    // Buy the stack back from slot 69 (price 20c).
    let mut buyback = vendor_raw.to_vec();
    buyback.extend_from_slice(&69u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_BUYBACK_ITEM, &buyback)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter().all(|(op, _)| *op != opcode_id::SMSG_BUY_FAILED),
        "buyback succeeds; got {:?}",
        got.iter()
            .map(|(op, b)| (*op, b.last().copied()))
            .collect::<Vec<_>>()
    );

    // Repair the damaged sword (11/20): durability
    // value returns to 20 in the item values update.
    let mut repair = vendor_raw.to_vec();
    repair.extend_from_slice(&sword_guid.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_REPAIR_ITEM, &repair)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(
        values_field(&got, sword_guid, ITEM_FIELD_DURABILITY),
        Some(20),
        "sword durability restored; ops {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // Bank (4d): activate shows the bank, buying a
    // slot sets PLAYER_BYTES_2 byte 2, and a backpack
    // -> bank move then passes the bank reachability
    // gate (no inventory failure).
    client
        .send_packet(opcode_id::CMSG_BANKER_ACTIVATE, &vendor_raw)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_SHOW_BANK && b.as_slice() == vendor_raw),
        "show bank packet; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    client
        .send_packet(opcode_id::CMSG_BUY_BANK_SLOT, &vendor_raw)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .all(|(op, _)| *op != opcode_id::SMSG_BUY_BANK_SLOT_RESULT),
        "buy bank slot success is silent"
    );
    let bytes2 = values_field(&got, 7, PLAYER_BYTES_2).expect("bytes2 update after slot buy");
    assert_eq!((bytes2 >> 16) & 0xff, 1, "bank bag slot count = 1");
    client
        .send_packet(opcode_id::CMSG_SWAP_INV_ITEM, &[23, 39])
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter().all(|(op, _)| *op != 274),
        "backpack -> bank move succeeds at banker"
    );
}

/// Trainer round trip (Tier 4 step 4e): list states
/// (GREEN learnable, RED level-gated, GRAY known),
/// wrong-class gossip rejection, level-gated buy
/// failure, and a successful buy that learns the
/// spell and charges exactly the listed cost.
#[tokio::test]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn trainer_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::PLAYER_FIELD_COINAGE;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_stores(backend, fake_trainer_world_stores()).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    let trainer = ObjectGuid::with_entry(HighGuid::UNIT, 67, 4243);
    let trainer_raw = trainer.raw().to_le_bytes();

    // List: guid + type 0 + count 3 + 3 rows of 38
    // bytes + title. States: 90001 GREEN (0),
    // 90002 RED (1), 90003 GRAY (2).
    client
        .send_packet(opcode_id::CMSG_TRAINER_LIST, &trainer_raw)
        .await;
    let got = flush_packets(&mut client).await;
    let (_, list) = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_TRAINER_LIST)
        .unwrap_or_else(|| {
            panic!(
                "trainer list; got {:?}",
                got.iter().map(|(op, b)| (*op, b.len())).collect::<Vec<_>>()
            )
        });
    assert_eq!(u32_at(list, 8), 0, "trainer type (class)");
    assert_eq!(u32_at(list, 12), 3, "trainer spell count");
    let row = |i: usize| 16 + i * 38;
    assert_eq!(u32_at(list, row(0)), 90001, "first wrapper");
    assert_eq!(list[row(0) + 4], 0, "90001 GREEN");
    assert_eq!(u32_at(list, row(0) + 5), 50, "90001 cost");
    assert_eq!(list[row(0) + 17], 10, "90001 req level");
    assert_eq!(u32_at(list, row(1)), 90002, "second wrapper");
    assert_eq!(list[row(1) + 4], 1, "90002 RED (level)");
    assert_eq!(u32_at(list, row(2)), 90003, "third wrapper");
    assert_eq!(list[row(2) + 4], 2, "90003 GRAY (known)");

    // Wrong-class trainer (mage): gossip rejection
    // menu 328, no trainer list.
    let mage_trainer = ObjectGuid::with_entry(HighGuid::UNIT, 68, 4244);
    client
        .send_packet(
            opcode_id::CMSG_TRAINER_LIST,
            &mage_trainer.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_GOSSIP_MESSAGE && u32_at(b, 8) == 328),
        "mage rejection gossip; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert!(
        got.iter()
            .all(|(op, _)| *op != opcode_id::SMSG_TRAINER_LIST),
        "no trainer list from wrong-class trainer"
    );

    // Buying the level-gated spell fails with
    // TRAIN_FAIL_NOT_ENOUGH_SKILL (2).
    let mut buy = trainer_raw.to_vec();
    buy.extend_from_slice(&90002u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_TRAINER_BUY_SPELL, &buy)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_TRAINER_BUY_FAILED
                && u32_at(b, 8) == 90002
                && u32_at(b, 12) == 2),
        "level-gated buy fails; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // Buying the GREEN spell succeeds: learned
    // packet for 90010, buy-succeeded for 90001,
    // and the coinage drops 100 -> 50.
    let mut buy = trainer_raw.to_vec();
    buy.extend_from_slice(&90001u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_TRAINER_BUY_SPELL, &buy)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_TRAINER_BUY_SUCCEEDED && u32_at(b, 8) == 90001),
        "buy succeeded; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LEARNED_SPELL && u32_at(b, 0) == 90010),
        "learned spell packet; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    // The coinage values update rides a later map
    // tick than the buy reply; one more barrier round
    // collects it.
    let mut got = got;
    got.extend(flush_packets(&mut client).await);

    assert_eq!(
        values_field(&got, 7, PLAYER_FIELD_COINAGE),
        Some(50),
        "coinage after buy; ops {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // The just-learned spell re-lists as GRAY and a
    // repeat buy fails with NOT_ENOUGH_SKILL.
    client
        .send_packet(opcode_id::CMSG_TRAINER_LIST, &trainer_raw)
        .await;
    let got = flush_packets(&mut client).await;
    let (_, list) = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_TRAINER_LIST)
        .expect("trainer list after buy");
    assert_eq!(list[row(0) + 4], 2, "90001 GRAY after learn");
}

/// Taxi round trip (Tier 4 step 4f): learn both
/// nodes from their flight masters, open the menu,
/// activate the flight (reply OK, mount spline,
/// fare charged), and land (mount display clears).
#[tokio::test]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn taxi_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::{
        PLAYER_FIELD_COINAGE, UNIT_FIELD_MOUNTDISPLAYID,
    };

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_stores(backend, fake_taxi_world_stores()).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    let fm_a = ObjectGuid::with_entry(HighGuid::UNIT, 69, 4245);
    let fm_b = ObjectGuid::with_entry(HighGuid::UNIT, 71, 4246);

    // Status query before learning: known = 0.
    client
        .send_packet(
            opcode_id::CMSG_TAXINODE_STATUS_QUERY,
            &fm_a.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_TAXINODE_STATUS && b[8] == 0),
        "node 1 unknown at first; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // Query available nodes at FM-A: learns node 1
    // (NEW_TAXI_PATH + status 1), no menu yet.
    client
        .send_packet(
            opcode_id::CMSG_TAXIQUERYAVAILABLENODES,
            &fm_a.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, _)| *op == opcode_id::SMSG_NEW_TAXI_PATH)
            && got
                .iter()
                .any(|(op, b)| *op == opcode_id::SMSG_TAXINODE_STATUS && b[8] == 1)
            && got
                .iter()
                .all(|(op, _)| *op != opcode_id::SMSG_SHOWTAXINODES),
        "learn node 1; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // Same at FM-B learns node 2.
    client
        .send_packet(
            opcode_id::CMSG_TAXIQUERYAVAILABLENODES,
            &fm_b.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, _)| *op == opcode_id::SMSG_NEW_TAXI_PATH),
        "learn node 2; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // Query FM-A again: the menu opens at node 1
    // with both bits set in the mask.
    client
        .send_packet(
            opcode_id::CMSG_TAXIQUERYAVAILABLENODES,
            &fm_a.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    let (_, menu) = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_SHOWTAXINODES)
        .expect("taxi menu");
    assert_eq!(u32_at(menu, 0), 1, "menu window flag");
    assert_eq!(u32_at(menu, 12), 1, "menu current node");
    assert_eq!(u32_at(menu, 16) & 0b11, 0b11, "known mask bits 1+2");

    // Fly 1 -> 2: reply OK, spline move for self,
    // fare 10 charged (100 -> 90).
    let mut activate = fm_a.raw().to_le_bytes().to_vec();
    activate.extend_from_slice(&1u32.to_le_bytes());
    activate.extend_from_slice(&2u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_ACTIVATETAXI, &activate)
        .await;
    let mut got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_ACTIVATETAXIREPLY && u32_at(b, 0) == 0),
        "activate reply OK; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    got.extend(flush_packets(&mut client).await);
    assert!(
        got.iter()
            .any(|(op, _)| *op == opcode_id::SMSG_MONSTER_MOVE),
        "flight spline started; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert_eq!(
        values_field(&got, 7, PLAYER_FIELD_COINAGE),
        Some(90),
        "fare charged"
    );
    assert!(
        values_field(&got, 7, UNIT_FIELD_MOUNTDISPLAYID).is_some_and(|v| v != 0),
        "mounted during flight"
    );

    // Land: within a few barrier rounds the mount
    // display clears (flight finalized).
    let mut landed = false;
    for _ in 0..10 {
        let got = flush_packets(&mut client).await;
        if values_field(&got, 7, UNIT_FIELD_MOUNTDISPLAYID) == Some(0) {
            landed = true;
            break;
        }
    }
    assert!(landed, "flight lands and dismounts");
}

/// Talent round trip (Tier 4 step 4g): learn rank 1
/// (point spent), locked row-1 talent refused, rank
/// up replaces rank 1 (removed packet, used = 2),
/// and a talent wipe fails on insufficient money
/// (first respec costs 1 gold, seed has 100 copper).
#[tokio::test]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn talent_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::PLAYER_CHARACTER_POINTS1;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_stores(backend, fake_talent_world_stores()).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    let learn = |talent: u32, rank: u32| {
        let mut body = talent.to_le_bytes().to_vec();
        body.extend_from_slice(&rank.to_le_bytes());
        body
    };

    // Learn rank 1 of talent 500: learned packet,
    // free points 3 -> 2.
    client
        .send_packet(opcode_id::CMSG_LEARN_TALENT, &learn(500, 0))
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LEARNED_SPELL && u32_at(b, 0) == 9101),
        "rank 1 learned; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert_eq!(
        values_field(&got, 7, PLAYER_CHARACTER_POINTS1),
        Some(2),
        "free talent points after rank 1"
    );

    // Row-1 talent 501 needs 5 spent points: refused.
    client
        .send_packet(opcode_id::CMSG_LEARN_TALENT, &learn(501, 0))
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .all(|(op, b)| *op != opcode_id::SMSG_LEARNED_SPELL || u32_at(b, 0) != 9103),
        "row-locked talent refused"
    );

    // Rank up to 2: rank 1 removed, rank 2 learned,
    // free points 2 -> 1 (used = 2).
    client
        .send_packet(opcode_id::CMSG_LEARN_TALENT, &learn(500, 1))
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LEARNED_SPELL && u32_at(b, 0) == 9102),
        "rank 2 learned; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_REMOVED_SPELL
                && u16::from_le_bytes(b[0..2].try_into().expect("u16")) == 9101),
        "rank 1 removed on rank up"
    );
    assert_eq!(
        values_field(&got, 7, PLAYER_CHARACTER_POINTS1),
        Some(1),
        "free talent points after rank 2"
    );

    // Wipe confirm at the trainer: the seed has 100
    // copper, first respec is 1 gold -> buy error,
    // talents kept (no removed-spell packet).
    let trainer = ObjectGuid::with_entry(HighGuid::UNIT, 67, 4243);
    client
        .send_packet(
            opcode_id::MSG_TALENT_WIPE_CONFIRM,
            &trainer.raw().to_le_bytes(),
        )
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_BUY_FAILED && b[12] == 1),
        "wipe fails on money; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );
    assert!(
        got.iter()
            .all(|(op, _)| *op != opcode_id::SMSG_REMOVED_SPELL),
        "talents kept after failed wipe"
    );
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

#[allow(clippy::too_many_lines)]
/// Tier 4 step 4h: pet stable master round trip.
/// The hunter logs in with a current pet (501) and a
/// stabled pet (502): the stable list shows both;
/// buying the second slot charges 50 copper; swap
/// exchanges the pets; stable stores the new current
/// pet in the free slot; unstable brings 501 back;
/// the revive handler stays silent (fork no-op).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stable_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::PLAYER_FIELD_COINAGE;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_stable_world_stores(),
        Arc::new(FakeCharQueries::new_hunter()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let login = flush_packets(&mut client).await;
    assert!(
        login
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_PET_SPELLS),
        "current pet materialized at login; got {:?}",
        login.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    let master = ObjectGuid::with_entry(HighGuid::UNIT, 69, 4245)
        .raw()
        .to_le_bytes();
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    let stable_result = |got: &[(u16, Vec<u8>)]| {
        got.iter()
            .find(|(op, _)| *op == opcode_id::SMSG_STABLE_RESULT)
            .map(|(_, b)| b[0])
    };
    // Parse one list packet body into
    // (slots, [(id, entry, slot byte)]).
    let parse_list = |b: &[u8]| -> (u8, Vec<(u32, u32, u8)>) {
        let num = b[8];
        let slots = b[9];
        let mut at = 10usize;
        let mut rows = Vec::new();
        for _ in 0..num {
            let id = u32_at(b, at);
            let entry = u32_at(b, at + 4);
            at += 12; // id, entry, level
            while b[at] != 0 {
                at += 1;
            }
            at += 1; // name NUL
            at += 4; // loyalty
            let slot = b[at];
            at += 1;
            rows.push((id, entry, slot));
        }
        (slots, rows)
    };
    let list = |got: &[(u16, Vec<u8>)]| {
        got.iter()
            .rev()
            .find(|(op, _)| *op == opcode_id::MSG_LIST_STABLED_PETS)
            .map(|(_, b)| parse_list(b))
    };

    // List: current pet + one stabled pet, 1 slot.
    client
        .send_packet(opcode_id::MSG_LIST_STABLED_PETS, &master)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(
        list(&got),
        Some((1, vec![(501, 9101, 1), (502, 9102, 2)])),
        "initial stable list"
    );

    // Buy the second slot for 50 copper.
    client
        .send_packet(opcode_id::CMSG_BUY_STABLE_SLOT, &master)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(stable_result(&got), Some(0x0A), "buy slot success");
    assert_eq!(
        values_field(&got, 7, PLAYER_FIELD_COINAGE),
        Some(99_950),
        "stable slot charged"
    );

    // Swap: 501 goes to the stable, 502 comes out.
    let mut body = master.to_vec();
    body.extend_from_slice(&502u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_STABLE_SWAP_PET, &body)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(stable_result(&got), Some(0x09), "swap success");
    client
        .send_packet(opcode_id::MSG_LIST_STABLED_PETS, &master)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(
        list(&got),
        Some((2, vec![(502, 9102, 1), (501, 9101, 2)])),
        "list after swap"
    );

    // Stable the current pet into the free slot 2.
    client
        .send_packet(opcode_id::CMSG_STABLE_PET, &master)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(stable_result(&got), Some(0x08), "stable success");

    // Unstable 501 back out.
    let mut body = master.to_vec();
    body.extend_from_slice(&501u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_UNSTABLE_PET, &body)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(stable_result(&got), Some(0x09), "unstable success");

    // Revive is the fork's empty handler: no result.
    client
        .send_packet(opcode_id::CMSG_STABLE_REVIVE_PET, &[])
        .await;
    client
        .send_packet(opcode_id::CMSG_NAME_QUERY, &9u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_NAME_QUERY_RESPONSE)
        .await;
    let got = flush_packets(&mut client).await;
    assert_eq!(stable_result(&got), None, "revive is silent");
}

/// All gameobject GUIDs (`0xF110` high) in create
/// blocks of the update bodies.
fn created_go_guids(bodies: &[(u16, Vec<u8>)]) -> Vec<u64> {
    fn packed_guid(body: &[u8], pos: &mut usize) -> Option<u64> {
        let mask = *body.get(*pos)?;
        *pos += 1;
        let mut raw = 0u64;
        for byte in 0..8 {
            if mask & (1 << byte) != 0 {
                raw |= u64::from(*body.get(*pos)?) << (8 * byte);
                *pos += 1;
            }
        }
        Some(raw)
    }
    let mut out = Vec::new();
    for (op, body) in bodies {
        if *op != 169 || body.len() < 7 {
            continue;
        }
        let count = u32::from_le_bytes(body[0..4].try_into().expect("count"));
        let mut pos = 5usize;
        for _ in 0..count {
            let Some(block_type) = body.get(pos).copied() else {
                break;
            };
            pos += 1;
            let Some(raw) = packed_guid(body, &mut pos) else {
                break;
            };
            if (block_type == 2 || block_type == 3) && raw >> 48 == 0xF110 {
                out.push(raw);
            }
            // Create blocks carry movement + values we
            // do not walk; stop at the first block we
            // cannot skip safely.
            if block_type != 0 {
                break;
            }
            let Some(words) = body.get(pos).copied().map(usize::from) else {
                break;
            };
            pos += 1;
            let mut set_bits = 0usize;
            for w in 0..words {
                let Some(bytes) = body.get(pos + 4 * w..pos + 4 * w + 4) else {
                    break;
                };
                set_bits +=
                    u32::from_le_bytes(bytes.try_into().expect("word")).count_ones() as usize;
            }
            pos += 4 * words + 4 * set_bits;
        }
    }
    out
}

/// Tier 4 B0-5: dynamic gameobjects round trip. The
/// static environmental trap beside the login point
/// casts its damage spell on the player (GO-cast
/// path); the slot-1 summon creates a hunter trap
/// (spawn-anim + guid), which fires on the critter
/// (owner-cast path) and is spent; re-casting the
/// slot spell replaces the object; the wild summon
/// creates a visible generic GO.
#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summon_object_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::UNIT_FIELD_HEALTH;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_go_world_stores(),
        Arc::new(FakeCharQueries::new_hunter()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);

    // Environmental trap (GO caster): damage log for
    // spell 9203 or a health drop on the player.
    let env_hit = got
        .iter()
        .any(|(op, b)| *op == opcode_id::SMSG_SPELLNONMELEEDAMAGELOG && b.len() > 16)
        || values_field(&got, 7, UNIT_FIELD_HEALTH).is_some_and(|hp| hp < 100);
    assert!(
        env_hit,
        "environmental trap cast lands; got {:?} hp {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
        values_field(&got, 7, UNIT_FIELD_HEALTH)
    );

    let cast_self = |spell: u32| {
        let mut body = spell.to_le_bytes().to_vec();
        body.extend_from_slice(&0u16.to_le_bytes());
        body
    };
    // Slot summon: trap GO created, spawn anim sent.
    client
        .send_packet(opcode_id::CMSG_CAST_SPELL, &cast_self(9200))
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    let anim_go = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_GAMEOBJECT_SPAWN_ANIM)
        .map(|(_, b)| u64::from_le_bytes(b[0..8].try_into().expect("guid")));
    let trap_go = anim_go.expect("slot summon spawn anim");
    assert_eq!(trap_go >> 48, 0xF110, "summoned guid is a gameobject");

    // Hunter trap fires on the critter (owner cast,
    // charges 1): damage log or critter health drop
    // (the hit may land in the spawn-anim batches).
    let critter =
        mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9207, 4247).raw();
    let saw_hit = |batch: &[(u16, Vec<u8>)]| {
        batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_SPELLNONMELEEDAMAGELOG)
            || values_field(batch, critter, UNIT_FIELD_HEALTH).is_some_and(|hp| hp < 100)
    };
    let mut trap_hit = saw_hit(&got);
    for _ in 0..6 {
        if trap_hit {
            break;
        }
        let batch = flush_packets(&mut client).await;
        trap_hit = saw_hit(&batch);
    }
    assert!(trap_hit, "hunter trap fires on the critter");

    // Recast replaces the slot object: a new spawn
    // anim guid, different from the first.
    client
        .send_packet(opcode_id::CMSG_CAST_SPELL, &cast_self(9200))
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    let anim_go2 = got
        .iter()
        .find(|(op, _)| *op == opcode_id::SMSG_GAMEOBJECT_SPAWN_ANIM)
        .map(|(_, b)| u64::from_le_bytes(b[0..8].try_into().expect("guid")));
    assert!(
        anim_go2.is_some_and(|g| g != trap_go),
        "slot replacement summons a fresh object: {anim_go2:?} vs {trap_go:x}"
    );

    // Wild summon: a new gameobject create block.
    client
        .send_packet(opcode_id::CMSG_CAST_SPELL, &cast_self(9205))
        .await;
    let mut got = flush_packets(&mut client).await;
    for _ in 0..6 {
        if !created_go_guids(&got).is_empty() {
            break;
        }
        got.extend(flush_packets(&mut client).await);
    }
    let created = created_go_guids(&got);
    assert!(
        !created.is_empty(),
        "wild summon creates a visible GO; got {:?}",
        got.iter()
            .map(|(op, b)| (*op, b.iter().take(24).copied().collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

/// Escort fixture (Tier 4 B0-6): Daphne Stilwell
/// script NPC (entry 9208, spawn guid 4248) one
/// yard from the seed player, quest 1651 starting
/// at her, and a two-point `script_waypoint` path
/// running 120 yards east — past the 100-yard
/// escort leash, so the range check must fail the
/// quest while the player stands still.
fn fake_escort_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::quest_template::{QuestRelations, QuestTemplate, QuestTemplates};
    use mangos_world_data::script_waypoints::{ScriptWaypoint, ScriptWaypointStore};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};

    let mut stores = fake_world_stores();
    let daphne = CreatureInfo {
        entry: 9208,
        name: "Escorttest".into(),
        faction: 35,
        npc_flags: 0x2, // QUESTGIVER
        civilian: true,
        unit_class: 1,
        script_name: "npc_daphne_stilwell".into(),
        speed_walk: 4.0,
        speed_run: 4.0,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![daphne]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let mut spawn = CreatureSpawn::default();
    spawn.creature_id[0] = 9208;
    spawn.position = WorldLocation {
        map_id: 0,
        x: -8948.95,
        y: -132.493,
        z: 83.5312,
        o: 0.0,
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4248, spawn)]);
    stores.quest_templates = QuestTemplates::from_entries([QuestTemplate {
        entry: 1651,
        // Method 0 is the fork's auto-complete shape
        // in `can_complete_quest`; real escort
        // quests use the normal method.
        method: 2,
        quest_level: 12,
        // An unmet kill objective keeps the quest
        // INCOMPLETE (an objective-less quest
        // auto-completes at accept and can never
        // fail).
        req_creature_or_go_ids: [9207, 0, 0, 0],
        req_creature_or_go_counts: [1, 0, 0, 0],
        ..Default::default()
    }]);
    stores.quest_relations =
        QuestRelations::from_creature_starts_for_tests(vec![(9208, vec![1651])]);
    stores.script_waypoints = ScriptWaypointStore::from_entries_for_tests(vec![(
        9208,
        vec![
            ScriptWaypoint {
                point_id: 1,
                x: -8930.0,
                y: -132.493,
                z: 83.5312,
                wait_time_ms: 0,
            },
            ScriptWaypoint {
                point_id: 2,
                x: -8830.0,
                y: -132.493,
                z: 83.5312,
                wait_time_ms: 0,
            },
        ],
    )]);
    stores
}

/// B0-8: an always-active event's positive spawn
/// is visible at login while its negative mirror
/// never materializes.
fn fake_event_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::game_events::{GameEventData, GameEvents};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};

    let mut stores = fake_go_world_stores();
    let mut positive = CreatureSpawn::default();
    positive.creature_id[0] = 9207;
    positive.position = WorldLocation {
        map_id: 0,
        x: -8950.5,
        y: -131.0,
        z: 83.5312,
        o: 0.0,
    };
    positive.game_event = 1;
    let mut negative = positive.clone();
    negative.position.x = -8948.5;
    negative.game_event = -1;
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4301, positive), (4302, negative)]);
    stores.game_events = GameEvents::from_fixtures_for_tests(
        vec![(
            1,
            GameEventData {
                start: 1,
                end: u64::MAX,
                occurence_min: 1,
                length_min: 1,
                description: "Test Event".into(),
                ..GameEventData::default()
            },
        )],
        vec![(1, vec![4301]), (-1, vec![4302])],
        vec![],
    );
    stores
}

#[tokio::test]
async fn game_event_spawns_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_event_world_stores(),
        Arc::new(FakeCharQueries::new()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;

    let positive = ObjectGuid::with_entry(HighGuid::UNIT, 9207, 4301).raw();
    let negative = ObjectGuid::with_entry(HighGuid::UNIT, 9207, 4302).raw();
    let mut all: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut tries = 0;
    while created_field(&all, positive, offsets::UNIT_FIELD_DISPLAYID).is_none() && tries < 40 {
        all.extend(flush_packets(&mut client).await);
        tries += 1;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    all.extend(flush_packets(&mut client).await);
    assert_eq!(
        created_field(&all, positive, offsets::UNIT_FIELD_DISPLAYID),
        Some(49),
        "positive event spawn visible"
    );
    assert_eq!(
        created_field(&all, negative, offsets::UNIT_FIELD_DISPLAYID),
        None,
        "negative event spawn must not materialize while the event runs"
    );
}

/// Tier 4 B0-6: escort round trip. Accepting quest
/// 1651 at Daphne starts the escort (`npc_escortAI`):
/// she spline-moves along the script waypoints,
/// and once she is more than 100 yards from the
/// stationary player the fork range check fails
/// the quest (`SMSG_QUESTUPDATE_FAILED`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escort_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_escort_world_stores(),
        Arc::new(FakeCharQueries::new()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    // Accept the escort quest.
    let npc = ObjectGuid::with_entry(HighGuid::UNIT, 9208, 4248);
    let mut body = npc.raw().to_le_bytes().to_vec();
    body.extend_from_slice(&1651u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_QUESTGIVER_ACCEPT_QUEST, &body)
        .await;
    let got = flush_packets(&mut client).await;
    assert!(
        !got.iter()
            .any(|(op, _)| *op == opcode_id::SMSG_QUESTGIVER_QUEST_INVALID),
        "escort quest accepted; got {:?}",
        got.iter().map(|(op, _)| *op).collect::<Vec<_>>()
    );

    // She runs east along the waypoints; past the
    // 100-yard leash the quest must fail.
    let mut saw_move = false;
    let mut failed_quest: Option<u32> = None;
    for _round in 0..160 {
        let batch = flush_packets(&mut client).await;
        for (op, b) in &batch {
            if *op == opcode_id::SMSG_MONSTER_MOVE {
                saw_move = true;
            }
            if *op == opcode_id::SMSG_QUESTUPDATE_FAILED && b.len() >= 4 {
                failed_quest = Some(u32::from_le_bytes(b[0..4].try_into().expect("quest id")));
            }
        }
        if failed_quest.is_some() {
            break;
        }
    }
    assert!(saw_move, "escort NPC spline-moves after quest accept");
    assert_eq!(
        failed_quest,
        Some(1651),
        "out-of-range escort fails quest 1651"
    );
}

/// Skip a create block's object-type byte +
/// movement section. False = bail (truncated or
/// spline-carrying).
fn skip_movement_section(body: &[u8], pos: &mut usize) -> bool {
    fn packed_guid(body: &[u8], pos: &mut usize) -> Option<u64> {
        let mask = *body.get(*pos)?;
        *pos += 1;
        let mut raw = 0u64;
        for byte in 0..8 {
            if mask & (1 << byte) != 0 {
                raw |= u64::from(*body.get(*pos)?) << (8 * byte);
                *pos += 1;
            }
        }
        Some(raw)
    }
    fn skip(body: &[u8], pos: &mut usize, n: usize) -> Option<()> {
        *pos += n;
        (*pos <= body.len()).then_some(())
    }
    fn read_u32(body: &[u8], pos: &mut usize) -> Option<u32> {
        let v = u32::from_le_bytes(body.get(*pos..*pos + 4)?.try_into().ok()?);
        *pos += 4;
        Some(v)
    }
    // Object type + update flags.
    if skip(body, pos, 1).is_none() {
        return false;
    }
    let Some(&flags) = body.get(*pos) else {
        return false;
    };
    *pos += 1;
    if flags & 0x20 != 0 {
        // LIVING movement section.
        let Some(move_flags) = read_u32(body, pos) else {
            return false;
        };
        if skip(body, pos, 4 + 16).is_none() {
            return false;
        }
        if move_flags & 0x0200_0000 != 0 && skip(body, pos, 24).is_none() {
            return false;
        }
        if move_flags & 0x0020_0000 != 0 && skip(body, pos, 4).is_none() {
            return false;
        }
        if skip(body, pos, 4).is_none() {
            return false;
        }
        if move_flags & 0x0000_2000 != 0 && skip(body, pos, 16).is_none() {
            return false;
        }
        if move_flags & 0x0400_0000 != 0 && skip(body, pos, 4).is_none() {
            return false;
        }
        if move_flags & 0x0040_0000 != 0 {
            return false;
        }
        if skip(body, pos, 24).is_none() {
            return false;
        }
    } else if flags & 0x40 != 0 && skip(body, pos, 16).is_none() {
        return false;
    }
    if flags & 0x08 != 0 && skip(body, pos, 4).is_none() {
        return false;
    }
    if flags & 0x10 != 0 && skip(body, pos, 4).is_none() {
        return false;
    }
    if flags & 0x04 != 0 && packed_guid(body, pos).is_none() {
        return false;
    }
    if flags & 0x02 != 0 && skip(body, pos, 4).is_none() {
        return false;
    }
    true
}

/// Last value of `field` for `guid` across create
/// and values blocks in update bodies (Tier 4
/// B0-7). Movement-section skipping mirrors
/// `BuildMovementUpdate` for a stationary login
/// player (no spline/transport blocks expected;
/// bails on those).
fn created_field(bodies: &[(u16, Vec<u8>)], guid: u64, field: u16) -> Option<u32> {
    fn packed_guid(body: &[u8], pos: &mut usize) -> Option<u64> {
        let mask = *body.get(*pos)?;
        *pos += 1;
        let mut raw = 0u64;
        for byte in 0..8 {
            if mask & (1 << byte) != 0 {
                raw |= u64::from(*body.get(*pos)?) << (8 * byte);
                *pos += 1;
            }
        }
        Some(raw)
    }
    let mut found = None;
    for (op, body) in bodies {
        if *op != 169 || body.len() < 6 {
            continue;
        }
        let count = u32::from_le_bytes(body[0..4].try_into().expect("count"));
        let mut pos = 5usize;
        for _ in 0..count {
            let Some(&block_type) = body.get(pos) else {
                break;
            };
            pos += 1;
            if block_type == 1 {
                break;
            }
            let Some(raw) = packed_guid(body, &mut pos) else {
                break;
            };
            if (block_type == 2 || block_type == 3) && !skip_movement_section(body, &mut pos) {
                break;
            }
            // Values mask + words.
            let Some(&words) = body.get(pos) else { break };
            pos += 1;
            let words = usize::from(words);
            let mut value_index = 0usize;
            let mut ok = true;
            for w in 0..words {
                let Some(word) = body
                    .get(pos + 4 * w..pos + 4 * w + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().expect("word")))
                else {
                    ok = false;
                    break;
                };
                for bit in 0..32 {
                    if word & (1u32 << bit) != 0 {
                        let idx = (w * 32 + bit) as u16;
                        let vpos = pos + 4 * words + 4 * value_index;
                        if raw == guid && idx == field {
                            if let Some(b) = body.get(vpos..vpos + 4) {
                                found = Some(u32::from_le_bytes(b.try_into().expect("value")));
                            }
                        }
                        value_index += 1;
                    }
                }
            }
            if !ok {
                break;
            }
            pos += 4 * words + 4 * value_index;
        }
    }
    found
}

/// Tier 4 B0-7: `HonorMgr::Update` at login. The
/// seeded character's CP rows + stored scalars
/// must land in the honor client fields of the
/// init-self create block exactly as the fork
/// computes them (week window starts today-3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn honor_login_fields_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets;

    let today = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        / 86_400) as u32;
    let config = ServerConfig::from_config(&mangos_config::Config::parse(
        b"RealmID = 1\nWorldServerPort = 0\nWowPatch = 10\n",
    ));
    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries_cfg(
        backend,
        fake_world_stores(),
        Arc::new(FakeCharQueries::new_with_honor()),
        config,
        today - 3,
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);

    let f = |field: u16| created_field(&got, 7, field);
    // Lifetime: stored 100/2 + in-week HK 6 + all DK 1.
    assert_eq!(f(offsets::PLAYER_FIELD_LIFETIME_HONORBALE_KILLS), Some(106));
    assert_eq!(
        f(offsets::PLAYER_FIELD_LIFETIME_DISHONORBALE_KILLS),
        Some(3)
    );
    // Today: 3 HK (low u16), 1 DK (high u16).
    assert_eq!(f(offsets::PLAYER_FIELD_SESSION_KILLS), Some(3 | (1 << 16)));
    // Yesterday: 2 kills, 180 CP.
    assert_eq!(f(offsets::PLAYER_FIELD_YESTERDAY_KILLS), Some(2));
    assert_eq!(f(offsets::PLAYER_FIELD_YESTERDAY_CONTRIBUTION), Some(180));
    // This week: 6 kills; CP 300+50+180+80 = 610
    // (dishonorable CP excluded).
    assert_eq!(f(offsets::PLAYER_FIELD_THIS_WEEK_KILLS), Some(6));
    assert_eq!(f(offsets::PLAYER_FIELD_THIS_WEEK_CONTRIBUTION), Some(610));
    // Last week passthrough from stored scalars.
    assert_eq!(f(offsets::PLAYER_FIELD_LAST_WEEK_KILLS), Some(11));
    assert_eq!(f(offsets::PLAYER_FIELD_LAST_WEEK_CONTRIBUTION), Some(1234));
    assert_eq!(f(offsets::PLAYER_FIELD_LAST_WEEK_RANK), Some(42));
    // Rank 7 at 7500 RP; highest promoted 0 -> 7.
    let bytes3 = f(offsets::PLAYER_BYTES_3).expect("bytes3");
    assert_eq!((bytes3 >> 24) & 0xff, 7, "rank byte");
    let field_bytes = f(offsets::PLAYER_FIELD_BYTES).expect("field bytes");
    assert_eq!((field_bytes >> 24) & 0xff, 7, "highest rank byte");
    // Honor bar: (7500-5000)/(10000-5000)*255 = 127.
    let bytes2 = f(offsets::PLAYER_FIELD_BYTES2).expect("bytes2");
    assert_eq!(bytes2 & 0xff, 127, "honor bar byte");
}

/// Ghost fixture: base stores plus spell 8326
/// (Ghost) with its fork effect shape (effect 1 =
/// `APPLY_AURA` / `SPELL_AURA_GHOST` 95).
fn fake_ghost_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    let mut stores = fake_world_stores();
    stores.spells.spells.insert(
        8326,
        SpellEntry {
            id: 8326,
            effects: [
                SpellEffectData {
                    effect: 6,            // APPLY_AURA
                    apply_aura_name: 95,  // SPELL_AURA_GHOST
                    implicit_target_a: 1, // UNIT_CASTER
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores
}

fn fake_ghost_spirit_healer_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::gossip_menus::{GossipMenuOption, GossipMenuText, GossipStores};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};

    let mut stores = fake_ghost_world_stores();
    stores.spells.spells.insert(
        17251,
        SpellEntry {
            id: 17251,
            effects: [
                SpellEffectData {
                    effect: 3,            // DUMMY
                    implicit_target_a: 1, // UNIT_CASTER
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![CreatureInfo {
        entry: 6491,
        name: "Spirit Healer".into(),
        faction: 35,
        npc_flags: 0x21, // GOSSIP | SPIRITHEALER
        gossip_menu_id: 83,
        civilian: true,
        unit_class: 1,
        static_flags1: 0x0020_0000, // CREATURE_STATIC_FLAG_VISIBLE_TO_GHOSTS
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    }]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let mut spawn = CreatureSpawn::default();
    spawn.creature_id[0] = 6491;
    spawn.position = WorldLocation {
        map_id: 0,
        x: -8949.95,
        y: -132.493,
        z: 83.5312,
        o: 0.0,
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4350, spawn)]);
    stores.gossip = GossipStores::from_maps_for_tests(
        HashMap::from([(
            83,
            vec![GossipMenuText {
                text_id: 91083,
                script_id: 0,
                condition_id: 0,
            }],
        )]),
        HashMap::from([(
            83,
            vec![GossipMenuOption {
                id: 0,
                option_icon: 4,
                option_text: "Return me to life.".into(),
                option_broadcast_text: 0,
                option_id: 6, // GOSSIP_OPTION_SPIRITHEALER
                npc_option_npcflag: 0x20,
                action_menu_id: -1,
                action_poi_id: 0,
                action_script_id: 0,
                box_coded: false,
                box_money: 0,
                box_text: String::new(),
                box_broadcast_text: 0,
                condition_id: 0,
            }],
        )]),
    );
    stores
}

fn fake_gossip_world_stores() -> mangos_map::WorldStores {
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::gossip_menus::{GossipMenuOption, GossipMenuText, GossipStores};
    use mangos_world_data::spawn_support::{CreatureDisplayInfoAddon, DisplayInfoAddons};
    use mangos_world_data::spawns::{CreatureSpawn, SpawnData, WorldLocation};

    let mut stores = fake_world_stores();
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![CreatureInfo {
        entry: 6500,
        name: "Gossiptest".into(),
        faction: 35,
        npc_flags: 0x1, // UNIT_NPC_FLAG_GOSSIP
        gossip_menu_id: 90,
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    }]);
    stores.display_addons =
        DisplayInfoAddons::from_creature_addons_for_tests(vec![CreatureDisplayInfoAddon {
            display_id: 49,
            bounding_radius: 0.3,
            combat_reach: 1.5,
            speed_walk: 2.5,
            speed_run: 8.0,
            gender: 0,
            display_id_other_gender: 0,
        }]);
    let mut spawn = CreatureSpawn::default();
    spawn.creature_id[0] = 6500;
    spawn.position = WorldLocation {
        map_id: 0,
        x: -8948.95,
        y: -132.493,
        z: 83.5312,
        o: 0.0,
    };
    stores.spawns = SpawnData::from_creatures_for_tests(vec![(4351, spawn)]);
    let option = |text: &str, action_menu_id: i32| GossipMenuOption {
        id: 0,
        option_icon: 0,
        option_text: text.into(),
        option_broadcast_text: 0,
        option_id: 1, // GOSSIP_OPTION_GOSSIP
        npc_option_npcflag: 0x1,
        action_menu_id,
        action_poi_id: 0,
        action_script_id: 0,
        box_coded: false,
        box_money: 0,
        box_text: String::new(),
        box_broadcast_text: 0,
        condition_id: 0,
    };
    stores.gossip = GossipStores::from_maps_for_tests(
        HashMap::from([
            (
                90,
                vec![GossipMenuText {
                    text_id: 91090,
                    script_id: 0,
                    condition_id: 0,
                }],
            ),
            (
                91,
                vec![GossipMenuText {
                    text_id: 91091,
                    script_id: 0,
                    condition_id: 0,
                }],
            ),
        ]),
        HashMap::from([
            (90, vec![option("Tell me more.", 91)]),
            (91, vec![option("Farewell.", -1)]),
        ]),
    );
    stores
}

/// Default gossip hello/select round trip: hello opens
/// menu 90, selecting its gossip option follows to
/// menu 91, and selecting the terminal option closes.
#[tokio::test]
async fn gossip_hello_select_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_stores(backend, fake_gossip_world_stores()).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let npc = ObjectGuid::with_entry(HighGuid::UNIT, 6500, 4351);
    let npc_raw = npc.raw();
    client
        .send_packet(opcode_id::CMSG_GOSSIP_HELLO, &npc_raw.to_le_bytes())
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_GOSSIP_MESSAGE)
        .await;
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));
    assert_eq!(u32_at(&body, 8), 91090, "first gossip text");
    assert_eq!(u32_at(&body, 12), 1, "first gossip option count");
    assert!(
        body.windows(b"Tell me more.".len())
            .any(|w| w == b"Tell me more."),
        "first gossip option text"
    );

    let mut select = npc_raw.to_le_bytes().to_vec();
    select.extend_from_slice(&0u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_GOSSIP_SELECT_OPTION, &select)
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_GOSSIP_MESSAGE)
        .await;
    assert_eq!(u32_at(&body, 8), 91091, "follow-up gossip text");
    assert_eq!(u32_at(&body, 12), 1, "follow-up gossip option count");
    assert!(
        body.windows(b"Farewell.".len()).any(|w| w == b"Farewell."),
        "follow-up gossip option text"
    );

    client
        .send_packet(opcode_id::CMSG_GOSSIP_SELECT_OPTION, &select)
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_GOSSIP_COMPLETE)
        .await;
}

async fn login_ghost_client(port: u16) -> (Client, Vec<(u16, Vec<u8>)>) {
    use mangos_protocol::opcode_table::opcode_id;

    let (mut client, seed) = Client::connect(port).await;
    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    (client, got)
}

/// Saved-ghost login (owner canary report,
/// 2026-11-12): a character who logged out dead
/// must re-enter as a ghost the real client can
/// play — ghost player flag + ghost vis byte from
/// the restored 8326 holder (`HandleAuraGhost`),
/// `MSG_CORPSE_QUERY` answered with the persisted
/// corpse (`HandleCorpseQueryOpcode`), and corpse
/// reclaim resurrecting with both ghost bits
/// cleared (`HandleReclaimCorpseOpcode`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ghost_login_corpse_query_and_reclaim_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_ghost_world_stores(),
        Arc::new(FakeCharQueries::new_ghost()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);

    let flags = created_field(&got, 7, offsets::PLAYER_FLAGS).expect("player flags");
    assert_ne!(flags & 0x0000_0010, 0, "PLAYER_FLAGS_GHOST at ghost login");
    let bytes1 = created_field(&got, 7, offsets::UNIT_FIELD_BYTES_1).expect("bytes1");
    assert_ne!(
        (bytes1 >> 24) & 0x01,
        0,
        "UNIT_VIS_FLAGS_GHOST vis byte at ghost login"
    );

    client.send_packet(opcode_id::MSG_CORPSE_QUERY, &[]).await;
    let body = client.read_until_opcode(opcode_id::MSG_CORPSE_QUERY).await;
    assert_eq!(body.len(), 21, "corpse query found shape");
    assert_eq!(body[0], 1, "corpse found");
    let f32_at = |i: usize| f32::from_le_bytes(body[i..i + 4].try_into().expect("f32"));
    assert_eq!(i32::from_le_bytes(body[1..5].try_into().expect("map")), 0);
    assert!((f32_at(5) - -8949.95).abs() < 0.01, "corpse x");
    assert!((f32_at(9) - -132.493).abs() < 0.01, "corpse y");
    assert!((f32_at(13) - 83.5312).abs() < 0.01, "corpse z");
    assert_eq!(
        u32::from_le_bytes(body[17..21].try_into().expect("map2")),
        0
    );

    // The fork handler reads and ignores the packet
    // guid (`Player::GetCorpse` resolves instead):
    // send a bogus corpse guid to lock that in. The
    // seeded resurrectable corpse is guid low 42.
    let bogus_raw =
        mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::CORPSE, 0, 9999).raw();
    client
        .send_packet(opcode_id::CMSG_RECLAIM_CORPSE, &bogus_raw.to_le_bytes())
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    let health = created_field(&got, 7, offsets::UNIT_FIELD_HEALTH).expect("health");
    assert!(
        health > 1,
        "reclaim resurrects above ghost health, got {health}"
    );
    let flags = created_field(&got, 7, offsets::PLAYER_FLAGS).expect("player flags");
    assert_eq!(flags & 0x0000_0010, 0, "ghost flag cleared by reclaim");
    let bytes1 = created_field(&got, 7, offsets::UNIT_FIELD_BYTES_1).expect("bytes1");
    assert_eq!(
        (bytes1 >> 24) & 0x01,
        0,
        "vis ghost byte cleared by reclaim"
    );
}

/// A saved ghost whose resurrectable corpse has
/// crossed the three-day expiry: the seeded bones
/// remain visible (fresh one-hour bones lifetime),
/// and login follows `Player::LoadCorpse` by
/// resurrecting at 50% instead of stranding the
/// player with no reclaimable corpse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ghost_login_without_resurrectable_corpse_resurrects() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::update_field_table::offsets;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_ghost_world_stores(),
        Arc::new(FakeCharQueries::new_ghost_expired_corpse()),
    )
    .await;
    let (_client, got) = login_ghost_client(port).await;

    let bones = ObjectGuid::with_entry(HighGuid::CORPSE, 0, 42).raw();
    assert_eq!(
        created_field(&got, bones, offsets::CORPSE_FIELD_OWNER),
        Some(7),
        "seeded bones are visible after their first expiry pass"
    );
    let health = created_field(&got, 7, offsets::UNIT_FIELD_HEALTH).expect("health");
    assert!(
        health > 1,
        "LoadCorpse resurrects a ghost with no resurrectable corpse, got {health}"
    );
    let flags = created_field(&got, 7, offsets::PLAYER_FLAGS).expect("player flags");
    assert_eq!(flags & 0x0000_0010, 0, "ghost flag cleared at login");
    let bytes1 = created_field(&got, 7, offsets::UNIT_FIELD_BYTES_1).expect("bytes1");
    assert_eq!((bytes1 >> 24) & 0x01, 0, "vis ghost byte cleared at login");
}

/// Real-client spirit-healer path for a saved ghost:
/// gossip hello must open the healer menu, its
/// Spirit Healer option must produce the client's
/// confirmation prompt, and accepting that prompt
/// resurrects the player. This is distinct from the
/// scripted driver's direct activate packet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ghost_spirit_healer_gossip_round_trip() {
    use mangos_guid::{HighGuid, ObjectGuid};
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_ghost_spirit_healer_world_stores(),
        Arc::new(FakeCharQueries::new_ghost()),
    )
    .await;
    let (mut client, got) = login_ghost_client(port).await;
    let healer = ObjectGuid::with_entry(HighGuid::UNIT, 6491, 4350);
    let healer_raw = healer.raw();
    assert_eq!(
        created_field(&got, healer_raw, offsets::UNIT_NPC_FLAGS),
        Some(0x21),
        "spirit healer is visible to the saved ghost"
    );

    client
        .send_packet(opcode_id::CMSG_GOSSIP_HELLO, &healer_raw.to_le_bytes())
        .await;
    let body = client
        .read_until_opcode(opcode_id::SMSG_GOSSIP_MESSAGE)
        .await;
    let u32_at = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().expect("u32"));
    assert_eq!(u32_at(8), 91_083, "spirit-healer gossip text");
    assert_eq!(u32_at(12), 1, "one spirit-healer option");
    assert_eq!(u32_at(16), 0, "spirit-healer option index");
    assert!(
        body.windows(b"Return me to life.".len())
            .any(|w| w == b"Return me to life."),
        "spirit-healer option text"
    );

    let mut select = healer_raw.to_le_bytes().to_vec();
    select.extend_from_slice(&0u32.to_le_bytes());
    client
        .send_packet(opcode_id::CMSG_GOSSIP_SELECT_OPTION, &select)
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    assert!(
        got.iter()
            .any(|(op, b)| *op == opcode_id::SMSG_SPIRIT_HEALER_CONFIRM
                && b.as_slice() == healer_raw.to_le_bytes()),
        "spirit-healer confirmation for the healer; got {:?}",
        got.iter().map(|(op, b)| (*op, b.len())).collect::<Vec<_>>()
    );

    client
        .send_packet(
            opcode_id::CMSG_SPIRIT_HEALER_ACTIVATE,
            &healer_raw.to_le_bytes(),
        )
        .await;
    let mut got = flush_packets(&mut client).await;
    got.extend(flush_packets(&mut client).await);
    let health = created_field(&got, 7, offsets::UNIT_FIELD_HEALTH).expect("health");
    assert!(health > 1, "spirit resurrect restores health, got {health}");
    let flags = created_field(&got, 7, offsets::PLAYER_FLAGS).expect("player flags");
    assert_eq!(
        flags & 0x0000_0010,
        0,
        "ghost flag cleared by spirit resurrect"
    );
}

/// Loot fixture (Tier 4 B0-10): the GO fixture's
/// critter 9207 (spawn 4247) with a fixed loot
/// template — 137 copper + 2x Linen Cloth (2589,
/// proto from the base fixture) at 100% — and a
/// one-shot kill spell (9210).
#[allow(clippy::too_many_lines)] // one fixture assembly
fn fake_loot_world_stores() -> mangos_map::WorldStores {
    use mangos_spells::entry::{SpellEffectData, SpellEntry};
    use mangos_world_data::creature_template::{CreatureInfo, CreatureTemplates};
    use mangos_world_data::loot::{LootStore, LootStoreItem};

    let mut stores = fake_go_world_stores();
    let mut critter = CreatureInfo {
        entry: 9207,
        name: "Crittertest".into(),
        faction: 35,
        creature_type: 8,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    critter.loot_id = 9301;
    critter.gold_min = 137;
    critter.gold_max = 137;
    let master = CreatureInfo {
        entry: 69,
        name: "Stabletest".into(),
        faction: 35,
        npc_flags: 0x2000,
        civilian: true,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    let beast = |entry: u32| CreatureInfo {
        entry,
        name: "Beasttest".into(),
        faction: 35,
        creature_type: 1,
        pet_family: 1,
        static_flags1: 0x10,
        unit_class: 1,
        display_id: [49, 0, 0, 0],
        display_probability: [100, 0, 0, 0],
        display_total_probability: 100,
        ..Default::default()
    };
    // Skinning fixture (Slice 1): a level-14 beast with
    // the shared body template plus a skinning template
    // (1x hide 2934 at 100%). Civilian so it never
    // fights back during the scripted kill.
    let mut skinbeast = beast(9103);
    skinbeast.name = "Skintest".into();
    skinbeast.pet_family = 0;
    skinbeast.static_flags1 = 0;
    skinbeast.civilian = true;
    skinbeast.level_min = 14;
    skinbeast.level_max = 14;
    skinbeast.loot_id = 9301;
    skinbeast.gold_min = 137;
    skinbeast.gold_max = 137;
    skinbeast.skinning_loot_id = 9302;
    // Pickpocket fixture (Slice 2): a level-14 humanoid
    // with the shared body template plus a pickpocket
    // template (1x linen 2589 at 100%), and a
    // template-less humanoid fellow. Civilian so neither
    // fights back during the scripted casts.
    let mut pickbeast = beast(9104);
    pickbeast.name = "Picktest".into();
    pickbeast.pet_family = 0;
    pickbeast.static_flags1 = 0;
    pickbeast.civilian = true;
    pickbeast.creature_type = 7;
    pickbeast.level_min = 14;
    pickbeast.level_max = 14;
    pickbeast.loot_id = 9301;
    pickbeast.gold_min = 137;
    pickbeast.gold_max = 137;
    pickbeast.pickpocket_loot_id = 9303;
    let mut pickfellow = beast(9105);
    pickfellow.name = "Pickfellow".into();
    pickfellow.pet_family = 0;
    pickfellow.static_flags1 = 0;
    pickfellow.civilian = true;
    pickfellow.creature_type = 7;
    pickfellow.level_min = 14;
    pickfellow.level_max = 14;
    stores.creature_templates = CreatureTemplates::from_entries_for_tests(vec![
        master,
        beast(9101),
        beast(9102),
        critter,
        skinbeast,
        pickbeast,
        pickfellow,
    ]);
    // Respawn in an hour: the GO fixture spawn has a
    // zero respawn time, so the corpse would
    // re-materialize before the loot window opens.
    {
        use mangos_world_data::spawns::{CreatureSpawn, GameObjectSpawn, SpawnData, WorldLocation};
        let mut critter_spawn = CreatureSpawn::default();
        critter_spawn.creature_id[0] = 9207;
        critter_spawn.spawntimesecsmin = 3600;
        critter_spawn.spawntimesecsmax = 3600;
        critter_spawn.position = WorldLocation {
            map_id: 0,
            x: -8949.5,
            y: -131.0,
            z: 83.5312,
            o: 0.0,
        };
        let mut beast_spawn = CreatureSpawn::default();
        beast_spawn.creature_id[0] = 9103;
        beast_spawn.spawntimesecsmin = 3600;
        beast_spawn.spawntimesecsmax = 3600;
        beast_spawn.position = WorldLocation {
            map_id: 0,
            x: -8947.5,
            y: -130.0,
            z: 83.5312,
            o: 0.0,
        };
        let mut pick_spawn = CreatureSpawn::default();
        pick_spawn.creature_id[0] = 9104;
        pick_spawn.spawntimesecsmin = 3600;
        pick_spawn.spawntimesecsmax = 3600;
        pick_spawn.position = WorldLocation {
            map_id: 0,
            x: -8948.0,
            y: -130.5,
            z: 83.5312,
            o: 0.0,
        };
        let mut fellow_spawn = CreatureSpawn::default();
        fellow_spawn.creature_id[0] = 9105;
        fellow_spawn.spawntimesecsmin = 3600;
        fellow_spawn.spawntimesecsmax = 3600;
        fellow_spawn.position = WorldLocation {
            map_id: 0,
            x: -8947.0,
            y: -131.5,
            z: 83.5312,
            o: 0.0,
        };
        let mut spawns = SpawnData::from_creatures_for_tests(vec![
            (4247, critter_spawn),
            (4248, beast_spawn),
            (4249, pick_spawn),
            (4250, fellow_spawn),
        ]);
        spawns.insert_gameobject_for_tests(
            4246,
            GameObjectSpawn {
                id: 9204,
                position: WorldLocation {
                    map_id: 0,
                    x: -8948.95,
                    y: -132.493,
                    z: 83.5312,
                    o: 0.0,
                },
                spawntimesecsmin: 25,
                spawntimesecsmax: 25,
                animprogress: 100,
                ..Default::default()
            },
        );
        stores.spawns = spawns;
    }
    stores.spells.spells.insert(
        9210,
        SpellEntry {
            id: 9210,
            range_index: 9001,
            effects: [
                SpellEffectData {
                    effect: 2, // SCHOOL_DAMAGE
                    base_points: 10_000,
                    die_sides: 1,
                    base_dice: 1,
                    implicit_target_a: 6, // UNIT_ENEMY
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    stores.loot.creature = LootStore::from_fixtures_for_tests(
        vec![(
            9301,
            LootStoreItem {
                itemid: 2589,
                chance: 100.0,
                mincount_or_ref: 2,
                group: 0,
                needs_quest: false,
                maxcount: 2,
                condition_id: 0,
            },
        )],
        true,
    );
    // Skinning store: hide 2934 (see the proto below in
    // the round-trip test).
    stores.loot.skinning = LootStore::from_fixtures_for_tests(
        vec![(
            9302,
            LootStoreItem {
                itemid: 2934,
                chance: 100.0,
                mincount_or_ref: 1,
                group: 0,
                needs_quest: false,
                maxcount: 1,
                condition_id: 0,
            },
        )],
        true,
    );
    // Skinning cast (Slice 1): effect 95 on the unit
    // target, spell range 100 like the kill spell,
    // always-hit for a deterministic scripted hit
    // (the orange-attempt roll stays live).
    stores.spells.spells.insert(
        9211,
        SpellEntry {
            id: 9211,
            range_index: 9001,
            attributes_ex3: 0x0004_0000, // SPELL_ATTR_EX3_ALWAYS_HIT
            dmg_class: 1,                // MAGIC
            effects: [
                SpellEffectData {
                    effect: 95,            // SPELL_EFFECT_SKINNING
                    implicit_target_a: 25, // TARGET_UNIT
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Pickpocket store (Slice 2): linen 2589 (proto from
    // the base fixture).
    stores.loot.pickpocketing = LootStore::from_fixtures_for_tests(
        vec![(
            9303,
            LootStoreItem {
                itemid: 2589,
                chance: 100.0,
                mincount_or_ref: 1,
                group: 0,
                needs_quest: false,
                maxcount: 1,
                condition_id: 0,
            },
        )],
        true,
    );
    // Pickpocket cast (Slice 2): effect 71 on the unit
    // target, spell range 100 like the kill spell,
    // always-hit for a deterministic scripted hit (the
    // miss-path stealth break stays uncovered by this
    // fixture).
    stores.spells.spells.insert(
        9212,
        SpellEntry {
            id: 9212,
            range_index: 9001,
            attributes_ex3: 0x0004_0000, // SPELL_ATTR_EX3_ALWAYS_HIT
            dmg_class: 1,                // MAGIC
            effects: [
                SpellEffectData {
                    effect: 71,            // SPELL_EFFECT_PICKPOCKET
                    implicit_target_a: 25, // TARGET_UNIT
                    ..Default::default()
                },
                SpellEffectData::default(),
                SpellEffectData::default(),
            ],
            ..Default::default()
        },
    );
    // Skill-line rows so the login projection emits the
    // skinning triplet (`write_skill_fields` needs the
    // category + race/class rows; Rank keeps row max).
    stores.skills.categories.insert(393, 11);
    stores
        .skills
        .race_class
        .push(mangos_world_data::skill_info::RaceClassSkill {
            skill_id: 393,
            race_mask: 0,
            class_mask: 0,
            flags: 0,
            req_level: 0,
            skill_tier_id: 7,
        });
    stores.skills.tiers.insert(7);
    stores
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn loot_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::PLAYER_FIELD_COINAGE;

    let backend = FakeBackend::with_test_user();
    let port = start_server_with_queries(
        backend,
        fake_loot_world_stores(),
        Arc::new(FakeCharQueries::new_hunter()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;
    let _ = flush_packets(&mut client).await;

    let critter_guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9207, 4247);
    let critter = critter_guid.raw();

    // One-shot the critter with 9210.
    let mut cast_body = 9210u32.to_le_bytes().to_vec();
    cast_body.extend_from_slice(&0x0002u16.to_le_bytes());
    let mut packed = Vec::new();
    critter_guid.write_packed(&mut packed);
    cast_body.extend_from_slice(&packed);
    client
        .send_packet(opcode_id::CMSG_CAST_SPELL, &cast_body)
        .await;
    // Death proof is the loot window itself (a live
    // creature answers with a DIDNT_KILL error body:
    // loot type 0). Retry the open until the kill
    // lands and the corpse view arrives.
    let mut view: Option<Vec<u8>> = None;
    let mut diag: Vec<(u16, Vec<u8>)> = Vec::new();
    for _ in 0..10 {
        let _ = flush_packets(&mut client).await;
        client
            .send_packet(opcode_id::CMSG_LOOT, &critter.to_le_bytes())
            .await;
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch
            .iter()
            .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&1))
        {
            view = Some(b.clone());
            break;
        }
        diag.extend(batch);
    }
    let view = view.unwrap_or_else(|| {
        panic!(
            "loot response; hp {:?} loot-ops {:?}",
            values_field(
                &diag,
                critter,
                mangos_protocol::update_field_table::offsets::UNIT_FIELD_HEALTH
            ),
            diag.iter()
                .filter(|(op, _)| matches!(
                    *op,
                    opcode_id::SMSG_LOOT_RESPONSE | opcode_id::SMSG_LOOT_RELEASE_RESPONSE
                ))
                .map(|(op, b)| (*op, b.clone()))
                .collect::<Vec<_>>()
        )
    });
    let rd32 = |at: usize| u32::from_le_bytes(view[at..at + 4].try_into().expect("u32"));
    assert_eq!(
        u64::from_le_bytes(view[0..8].try_into().expect("guid")),
        critter,
        "loot guid"
    );
    assert_eq!(view[8], 1, "LOOT_CORPSE");
    assert_eq!(rd32(9), 137, "gold");
    assert_eq!(view[13], 1, "one item shown");
    assert_eq!(view[14], 0, "slot 0");
    assert_eq!(rd32(15), 2589, "linen cloth");
    assert_eq!(rd32(19), 2, "stack of 2");
    assert_eq!(view[35], 0, "ALLOW_LOOT slot type");

    // Take the item.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    let mut pushed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        pushed |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_ITEM_PUSH_RESULT);
        if removed && pushed {
            break;
        }
    }
    assert!(removed, "loot removed notify");
    assert!(pushed, "item push result");

    // Take the money (hunter seed money is 100_000).
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared = false;
    let mut coinage: Option<u32> = None;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if let Some(v) = values_field(&batch, 7, PLAYER_FIELD_COINAGE) {
            coinage = Some(v);
        }
        if cleared && coinage == Some(100_137) {
            break;
        }
    }
    assert!(cleared, "loot clear money");
    assert_eq!(coinage, Some(100_137), "coinage after loot money");

    // Release the (now empty) corpse.
    client
        .send_packet(opcode_id::CMSG_LOOT_RELEASE, &critter.to_le_bytes())
        .await;
    let mut released = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        released |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE);
        if released {
            break;
        }
    }
    assert!(released, "loot release response");
}

/// Skinning round trip (Slice 1, fork
/// `Spell.cpp:6069-6094` + `SpellEffects.cpp:5265-5284`
/// + `Player.cpp:7985-8068`): the level-14 beast
/// (entry 9103, spawn 4248) carries body template 9301
/// and skinning template 9302 (1x hide 2934).
/// `UNSKINNABLE` on the live, unskinnable critter;
/// `NOT_LOOTED` while the body is full; the skinning
/// window (wire byte 2, fork `:8054-8056`) after full
/// loot + release; skill in [60, 61] (orange band, so
/// the gain roll always lands when the cast goes
/// through; the `TRY_AGAIN` roll itself stays live and
/// is retried); `UNSKINNABLE` on the second cast; the
/// shared slot shows the emptied skin window on
/// re-open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn skinning_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets;
    use mangos_world_data::item_prototype::ItemPrototype;

    async fn cast_at(client: &mut Client, spell: u32, target: u64) {
        let mut body = spell.to_le_bytes().to_vec();
        body.extend_from_slice(&0x0002u16.to_le_bytes());
        let mut packed = Vec::new();
        mangos_guid::ObjectGuid::from_raw(target).write_packed(&mut packed);
        body.extend_from_slice(&packed);
        client.send_packet(opcode_id::CMSG_CAST_SPELL, &body).await;
    }

    // Fail result for one cast (panics on a window or a
    // different reason).
    async fn expect_cast_fail(
        client: &mut Client,
        spell: u32,
        target: u64,
        reason: u8,
        what: &str,
    ) {
        cast_at(client, spell, target).await;
        for _ in 0..6 {
            let batch = flush_packets(client).await;
            if batch
                .iter()
                .any(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&2))
            {
                panic!("{what}: unexpected skinning window");
            }
            if let Some((_, b)) = batch.iter().find(|(op, b)| {
                *op == opcode_id::SMSG_CAST_RESULT
                    && b.len() >= 5
                    && u32::from_le_bytes(b[0..4].try_into().expect("spell")) == spell
            }) {
                assert_eq!(b[4], reason, "{what}: cast result");
                return;
            }
        }
        panic!("{what}: no cast result");
    }

    // The slot-id word only replicates at create (which
    // `values_field` cannot parse), so match the packed
    // value word directly: only changed words replicate
    // after login.
    fn skill_packed_in(bodies: &[(u16, Vec<u8>)], value: u32, max: u32) -> bool {
        let want = value | (max << 16);
        for slot in 0..127u16 {
            let packed_at = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3 + 1;
            if values_field(bodies, 7, packed_at) == Some(want) {
                return true;
            }
        }
        false
    }

    let backend = FakeBackend::with_test_user();
    let mut stores = fake_loot_world_stores();
    // Hide proto for the skinning template (mirrors
    // the linen fields the grant path needs).
    stores.item_prototypes.insert_for_tests(ItemPrototype {
        item_id: 2934,
        class: 7,
        display_id: 2112,
        stackable: 20,
        buy_count: 1,
        buy_price: 40,
        sell_price: 10,
        allowable_class: u32::MAX,
        allowable_race: u32::MAX,
        ..Default::default()
    });
    let port =
        start_server_with_queries(backend, stores, Arc::new(FakeCharQueries::new_hunter())).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let login_a = flush_packets(&mut client).await;
    let login_b = flush_packets(&mut client).await;

    let critter_guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9207, 4247);
    let critter = critter_guid.raw();
    let beast_guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9103, 4248);
    let beast = beast_guid.raw();

    // NOTE: the login create block is not parseable by
    // `values_field` (not even the pre-existing axes
    // triplet shows), so the pre-gain skill is not
    // asserted here; the post-gain values update below
    // proves the projection and the +1 together (the
    // fixture seeds the row at 60/75).
    let _ = (login_a, login_b);
    // Every batch from the skinning cast on: the skill
    // update replicates here.
    let mut post_cast: Vec<(u16, Vec<u8>)> = Vec::new();

    // The live, unskinnable critter fails with
    // UNSKINNABLE (116): no skinning template, so the
    // kill-time flag was never set.
    expect_cast_fail(&mut client, 9211, critter, 116, "live critter skin").await;

    // One-shot the beast.
    cast_at(&mut client, 9210, beast).await;
    // Death proof is the body window (mirror the loot
    // round trip).
    let mut body: Option<Vec<u8>> = None;
    for _ in 0..10 {
        let _ = flush_packets(&mut client).await;
        client
            .send_packet(opcode_id::CMSG_LOOT, &beast.to_le_bytes())
            .await;
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch
            .iter()
            .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&1))
        {
            body = Some(b.clone());
            break;
        }
    }
    let body = body.expect("beast body response");
    let rd32 = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().expect("u32"));
    assert_eq!(rd32(9), 137, "body gold");
    assert_eq!(body[13], 1, "one body item shown");
    assert_eq!(rd32(15), 2589, "body linen");

    // Skinning the full body fails with NOT_LOOTED
    // (112): non-critter bodies skin only once fully
    // looted.
    expect_cast_fail(&mut client, 9211, beast, 112, "full body skin").await;

    // Take the body item and money, then release.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        if removed {
            break;
        }
    }
    assert!(removed, "body loot removed");
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if cleared {
            break;
        }
    }
    assert!(cleared, "body money cleared");
    client
        .send_packet(opcode_id::CMSG_LOOT_RELEASE, &beast.to_le_bytes())
        .await;
    let mut released = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        released |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE);
        if released {
            break;
        }
    }
    assert!(released, "body release response");

    // Skin the emptied corpse: TRY_AGAIN (122) retries,
    // any other failure is fatal.
    let mut view: Option<Vec<u8>> = None;
    for _ in 0..10 {
        cast_at(&mut client, 9211, beast).await;
        for _ in 0..6 {
            let batch = flush_packets(&mut client).await;
            if let Some((_, b)) = batch
                .iter()
                .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&2))
            {
                view = Some(b.clone());
                post_cast.extend(batch);
                break;
            }
            if let Some((_, b)) = batch.iter().find(|(op, b)| {
                *op == opcode_id::SMSG_CAST_RESULT
                    && b.len() >= 5
                    && u32::from_le_bytes(b[0..4].try_into().expect("spell")) == 9211
            }) {
                // TRY_AGAIN is the only legal failure
                // here; the next cast retries it.
                assert_eq!(b[4], 122, "skinning retry reason");
                post_cast.extend(batch);
                break;
            }
            post_cast.extend(batch);
        }
        if view.is_some() {
            break;
        }
    }
    let view = view.expect("skinning window");
    let rd32 = |at: usize| u32::from_le_bytes(view[at..at + 4].try_into().expect("u32"));
    assert_eq!(
        u64::from_le_bytes(view[0..8].try_into().expect("guid")),
        beast,
        "skin loot guid"
    );
    assert_eq!(view[8], 2, "LOOT_PICKPOCKETING wire byte");
    assert_eq!(rd32(9), 0, "skin gold");
    assert_eq!(view[13], 1, "one skin item shown");
    assert_eq!(view[14], 0, "slot 0");
    assert_eq!(rd32(15), 2934, "hide");
    assert_eq!(rd32(19), 1, "stack of 1");
    assert_eq!(view[35], 0, "ALLOW_LOOT slot type");

    // Take the hide and release the skin window.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        post_cast.extend(batch);
        if removed {
            break;
        }
    }
    assert!(removed, "skin loot removed");
    // Orange band: the gain roll always lands, so the
    // replicated packed word moves 60 -> 61 at the same
    // trained max 75.
    assert!(skill_packed_in(&post_cast, 61, 75), "skinning skill after");
    client
        .send_packet(opcode_id::CMSG_LOOT_RELEASE, &beast.to_le_bytes())
        .await;
    let mut released = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        released |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE);
        if released {
            break;
        }
    }
    assert!(released, "skin release response");

    // The flag died with the effect: UNSKINNABLE (116).
    expect_cast_fail(&mut client, 9211, beast, 116, "reskin").await;

    // The shared slot serves the emptied skin window on
    // re-open (corpse wire byte, no resurrected body
    // loot).
    client
        .send_packet(opcode_id::CMSG_LOOT, &beast.to_le_bytes())
        .await;
    let mut reopened: Option<Vec<u8>> = None;
    for _ in 0..6 {
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch.iter().find(|(op, b)| {
            *op == opcode_id::SMSG_LOOT_RESPONSE
                && b.len() > 13
                && u64::from_le_bytes(b[0..8].try_into().expect("guid")) == beast
        }) {
            reopened = Some(b.clone());
            break;
        }
    }
    let reopened = reopened.expect("corpse re-open");
    assert_eq!(reopened[8], 1, "LOOT_CORPSE wire byte");
    assert_eq!(
        u32::from_le_bytes(reopened[9..13].try_into().expect("u32")),
        0,
        "no gold on re-open"
    );
    assert_eq!(reopened[13], 0, "no items on re-open");
}

/// Pickpocket round trip (Slice 2, fork
/// `Spell.cpp:6188-6198` + `SpellEffects.cpp:2580-2590`
/// + `Player.cpp:7872-7939`): the level-14 humanoid
/// (entry 9104, spawn 4249) carries pickpocket template
/// 9303 (1x linen 2589). `NO_POCKETS` on the critter;
/// the template-less fellow humanoid (9105) still opens
/// a money-only window; first open fills items + extra
/// money (wire byte 2); a same-looter re-cast re-serves
/// untouched; take + release then re-cast refills
/// quest-only (gold 0, no items); the kill wipes the
/// mark and the body loots; pickpocketing the corpse
/// fails at cast with `TARGETS_DEAD`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn pickpocket_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;

    async fn cast_at(client: &mut Client, spell: u32, target: u64) {
        let mut body = spell.to_le_bytes().to_vec();
        body.extend_from_slice(&0x0002u16.to_le_bytes());
        let mut packed = Vec::new();
        mangos_guid::ObjectGuid::from_raw(target).write_packed(&mut packed);
        body.extend_from_slice(&packed);
        client.send_packet(opcode_id::CMSG_CAST_SPELL, &body).await;
    }

    async fn expect_cast_fail(
        client: &mut Client,
        spell: u32,
        target: u64,
        reason: u8,
        what: &str,
    ) {
        cast_at(client, spell, target).await;
        for _ in 0..6 {
            let batch = flush_packets(client).await;
            if batch
                .iter()
                .any(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&2))
            {
                panic!("{what}: unexpected pickpocket window");
            }
            if let Some((_, b)) = batch.iter().find(|(op, b)| {
                *op == opcode_id::SMSG_CAST_RESULT
                    && b.len() >= 5
                    && u32::from_le_bytes(b[0..4].try_into().expect("spell")) == spell
            }) {
                assert_eq!(b[4], reason, "{what}: cast result");
                return;
            }
        }
        panic!("{what}: no cast result");
    }

    // Window bytes for one guid: the first
    // `SMSG_LOOT_RESPONSE` for it within the flushes.
    async fn open_window(client: &mut Client, spell: u32, target: u64) -> Vec<u8> {
        for _ in 0..10 {
            let _ = flush_packets(client).await;
            cast_at(client, spell, target).await;
            let batch = flush_packets(client).await;
            if let Some((_, b)) = batch.iter().find(|(op, b)| {
                *op == opcode_id::SMSG_LOOT_RESPONSE
                    && b.len() > 13
                    && u64::from_le_bytes(b[0..8].try_into().expect("guid")) == target
            }) {
                return b.clone();
            }
        }
        panic!("no loot window for {target:#x}");
    }

    async fn release_window(client: &mut Client, target: u64, what: &str) {
        client
            .send_packet(opcode_id::CMSG_LOOT_RELEASE, &target.to_le_bytes())
            .await;
        let mut released = false;
        for _ in 0..8 {
            let batch = flush_packets(client).await;
            released |= batch
                .iter()
                .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE);
            if released {
                break;
            }
        }
        assert!(released, "{what}: release response");
    }

    let backend = FakeBackend::with_test_user();
    let stores = fake_loot_world_stores();
    let port =
        start_server_with_queries(backend, stores, Arc::new(FakeCharQueries::new_hunter())).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;
    let _ = flush_packets(&mut client).await;

    let critter =
        mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9207, 4247).raw();
    let pick = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9104, 4249).raw();
    let fellow = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::UNIT, 9105, 4250).raw();
    let rd32 = |at: usize, b: &[u8]| u32::from_le_bytes(b[at..at + 4].try_into().expect("u32"));

    // No template, no humanoid/undead mask: the critter
    // has NO_POCKETS (114).
    expect_cast_fail(&mut client, 9212, critter, 114, "critter pickpocket").await;

    // The template-less fellow humanoid still opens a
    // money-only window (fork accepts humanoids without
    // a template id).
    let fellow_view = open_window(&mut client, 9212, fellow).await;
    assert_eq!(fellow_view[8], 2, "fellow wire byte");
    assert!(rd32(9, &fellow_view) <= 130, "fellow extra money bound");
    assert_eq!(fellow_view[13], 0, "fellow no items");
    release_window(&mut client, fellow, "fellow").await;

    // First open: items + extra money on wire byte 2.
    // Victim 14 (half 7), hunter 12 (half 6): gold in
    // [0, 10 * (7 + 6)] at rate 1.0.
    let first = open_window(&mut client, 9212, pick).await;
    assert_eq!(first[8], 2, "LOOT_PICKPOCKETING wire byte");
    let first_gold = rd32(9, &first);
    assert!(first_gold <= 130, "extra money bound, got {first_gold}");
    assert_eq!(first[13], 1, "one pickpocket item shown");
    assert_eq!(rd32(15, &first), 2589, "linen");
    release_window(&mut client, pick, "first pick").await;

    // Same-looter re-cast with a live window re-serves
    // it untouched (no refill: non-empty, original
    // looter).
    let second = open_window(&mut client, 9212, pick).await;
    assert_eq!(second[8], 2, "re-open wire byte");
    assert_eq!(rd32(9, &second), first_gold, "gold untouched");
    assert_eq!(second[13], 1, "item still shown");

    // Take the linen and the money, then release: the
    // emptied entry drops but the mark survives.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        if removed {
            break;
        }
    }
    assert!(removed, "pickpocket loot removed");
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if cleared {
            break;
        }
    }
    assert!(cleared, "pickpocket money cleared");
    release_window(&mut client, pick, "emptied pick").await;

    // Re-cast after full take + release refills
    // quest-only: no money, no items (no quest rows in
    // this fixture), still wire byte 2.
    let quest_only = open_window(&mut client, 9212, pick).await;
    assert_eq!(quest_only[8], 2, "quest-only wire byte");
    assert_eq!(rd32(9, &quest_only), 0, "no money on stale reopen");
    assert_eq!(quest_only[13], 0, "no items on stale reopen");
    release_window(&mut client, pick, "quest-only pick").await;

    // Killing the victim wipes the mark
    // (`GenerateLootForBody` parity) and generates the
    // body: the corpse window carries body gold.
    cast_at(&mut client, 9210, pick).await;
    let mut body: Option<Vec<u8>> = None;
    for _ in 0..10 {
        let _ = flush_packets(&mut client).await;
        client
            .send_packet(opcode_id::CMSG_LOOT, &pick.to_le_bytes())
            .await;
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch
            .iter()
            .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&1))
        {
            body = Some(b.clone());
            break;
        }
    }
    let body = body.expect("pick victim body response");
    assert_eq!(rd32(9, &body), 137, "body gold");
    release_window(&mut client, pick, "pick victim body").await;

    // The corpse is no longer pickpocketable: the
    // generic alive gate fails the cast with
    // TARGETS_DEAD (101).
    expect_cast_fail(&mut client, 9212, pick, 101, "corpse pickpocket").await;
}

/// GO chest loot (`CMSG_GAMEOBJ_USE` 177 +
/// `Player::SendLoot` GO arm, fork
/// `Player.cpp:7726-7793` + `DoLootRelease` GO arm,
/// `LootHandler.cpp:398-523`): the unlocked static
/// chest (entry 9300, spawn 4251) opens through
/// direct use with wire byte 2, fills 1x linen 2589
/// + 50 copper from the `gameobject` store, takes +
/// money + release deplete it, and a reopen while
/// the respawn delay runs serves an empty window
/// (no refill); the vein chest (9301, min2/max5)
/// re-fills after one full release (uses 1 < min).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn go_chest_loot_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_world_data::gameobject_template::{GameObjectInfo, GameObjectTemplates};
    use mangos_world_data::loot::{LootStore, LootStoreItem};
    use mangos_world_data::spawns::{GameObjectSpawn, SpawnData, WorldLocation};

    fn chest_info(id: u32, loot: u32, min: i32, max: i32) -> GameObjectInfo {
        let mut data = [0i32; 24];
        data[0] = 0; // lockId: unlocked
        #[allow(clippy::cast_possible_wrap)]
        let loot_word = loot as i32;
        data[1] = loot_word; // lootId
        data[4] = min; // minSuccessOpens
        data[5] = max; // maxSuccessOpens
        data[9] = 1; // level: under the opener gate
        GameObjectInfo {
            id,
            go_type: 3,
            display_id: 49,
            name: "Chesttest".into(),
            faction: 35,
            size: 1.0,
            data,
            min_money_loot: 50,
            max_money_loot: 50,
            ..Default::default()
        }
    }

    fn chest_spawn(id: u32, x: f32) -> GameObjectSpawn {
        GameObjectSpawn {
            id,
            position: WorldLocation {
                map_id: 0,
                x,
                y: -132.493,
                z: 83.5312,
                o: 0.0,
            },
            spawntimesecsmin: 25,
            spawntimesecsmax: 25,
            animprogress: 100,
            ..Default::default()
        }
    }

    async fn open_chest(client: &mut Client, guid: u64, what: &str) -> Vec<u8> {
        // Direct use (the OpenLock-spell indirection
        // lands in the same window).
        client
            .send_packet(opcode_id::CMSG_GAMEOBJ_USE, &guid.to_le_bytes())
            .await;
        for _ in 0..10 {
            let batch = flush_packets(client).await;
            if let Some((_, b)) = batch
                .iter()
                .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&2))
            {
                return b.clone();
            }
        }
        panic!("{what}: no chest window");
    }

    async fn open_loot(client: &mut Client, guid: u64, what: &str) -> Vec<u8> {
        for _ in 0..10 {
            client
                .send_packet(opcode_id::CMSG_LOOT, &guid.to_le_bytes())
                .await;
            let batch = flush_packets(client).await;
            if let Some((_, b)) = batch
                .iter()
                .find(|(op, b)| *op == opcode_id::SMSG_LOOT_RESPONSE && b.get(8) == Some(&2))
            {
                return b.clone();
            }
        }
        panic!("{what}: no chest window");
    }

    async fn release_window(client: &mut Client, guid: u64, what: &str) {
        client
            .send_packet(opcode_id::CMSG_LOOT_RELEASE, &guid.to_le_bytes())
            .await;
        for _ in 0..8 {
            let batch = flush_packets(client).await;
            if batch
                .iter()
                .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE)
            {
                return;
            }
        }
        panic!("{what}: no release response");
    }

    let mut stores = fake_go_world_stores();
    let mut templates: Vec<GameObjectInfo> = Vec::new();
    for info in stores
        .gameobject_templates
        .iter()
        .map(|(_, info)| info.clone())
    {
        templates.push(info);
    }
    templates.push(chest_info(9300, 9400, 0, 0));
    templates.push(chest_info(9301, 9401, 2, 5));
    stores.gameobject_templates = GameObjectTemplates::from_entries_for_tests(templates);
    let mut spawns = SpawnData::from_creatures_for_tests(Vec::new());
    spawns.insert_gameobject_for_tests(4251, chest_spawn(9300, -8948.95));
    spawns.insert_gameobject_for_tests(4252, chest_spawn(9301, -8947.95));
    stores.spawns = spawns;
    let linen = |loot_id: u32| {
        (
            loot_id,
            LootStoreItem {
                itemid: 2589,
                chance: 100.0,
                mincount_or_ref: 1,
                group: 0,
                needs_quest: false,
                maxcount: 1,
                condition_id: 0,
            },
        )
    };
    stores.loot.gameobject =
        LootStore::from_fixtures_for_tests(vec![linen(9400), linen(9401)], true);

    let backend = FakeBackend::with_test_user();
    let port =
        start_server_with_queries(backend, stores, Arc::new(FakeCharQueries::new_hunter())).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;

    let chest =
        mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::GAMEOBJECT, 9300, 4251).raw();
    let vein =
        mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::GAMEOBJECT, 9301, 4252).raw();
    let rd32 =
        |at: usize, body: &[u8]| u32::from_le_bytes(body[at..at + 4].try_into().expect("u32"));

    // A. Plain chest: use opens the window (wire
    // byte 2), 1x linen + 50 copper.
    let view = open_chest(&mut client, chest, "plain chest").await;
    assert_eq!(rd32(9, &view), 50, "chest gold");
    assert_eq!(view[13], 1, "one chest item shown");
    assert_eq!(rd32(15, &view), 2589, "chest linen");

    // Take + money, then release depletes it.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        if removed {
            break;
        }
    }
    assert!(removed, "chest loot removed");
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if cleared {
            break;
        }
    }
    assert!(cleared, "chest money cleared");
    release_window(&mut client, chest, "plain chest release").await;

    // Reopen while the 25s respawn delay runs: empty
    // window, no refill.
    let empty = open_loot(&mut client, chest, "depleted chest").await;
    assert_eq!(rd32(9, &empty), 0, "no gold while despawned");
    assert_eq!(empty[13], 0, "no items while despawned");
    release_window(&mut client, chest, "depleted release").await;

    // B. Vein: first full release leaves uses 1 <
    // min 2, so the next open re-fills.
    let first = open_chest(&mut client, vein, "vein first").await;
    assert_eq!(first[13], 1, "vein first fill");
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        if batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0))
        {
            break;
        }
    }
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        if batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY)
        {
            break;
        }
    }
    release_window(&mut client, vein, "vein first release").await;
    let second = open_loot(&mut client, vein, "vein second").await;
    assert_eq!(rd32(15, &second), 2589, "vein re-fills below min");
    assert_eq!(second[13], 1, "vein second fill shown");
    release_window(&mut client, vein, "vein second release").await;
}

/// Item container open (`CMSG_OPEN_ITEM` 172,
/// fork `HandleOpenItemOpcode` + `Player::SendLoot`
/// item arm): the seeded linen stack
/// (guid 9002, slot 23) carries an item-loot
/// template (1x sword 25) plus 137 copper. Open,
/// take, re-open mid-window (no regen), double-take
/// (rejected), money twice (idempotent), release
/// (an emptied non-bag container is destroyed), and
/// the logout save is observed through the
/// capturing saver.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn item_open_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;
    use mangos_protocol::update_field_table::offsets::PLAYER_FIELD_COINAGE;
    use mangos_world_data::loot::{LootStore, LootStoreItem};

    let backend = FakeBackend::with_test_user();
    let mut stores = fake_loot_world_stores();
    stores.loot.item = LootStore::from_fixtures_for_tests(
        vec![(
            2589,
            LootStoreItem {
                itemid: 25,
                chance: 100.0,
                mincount_or_ref: 1,
                group: 0,
                needs_quest: false,
                maxcount: 1,
                condition_id: 0,
            },
        )],
        true,
    );
    let mut linen = stores
        .item_prototypes
        .get(2589)
        .expect("linen proto")
        .clone();
    linen.min_money_loot = 137;
    linen.max_money_loot = 137;
    // Open-item always uses the item template in the
    // fork; a nonzero disenchant id must not reroute
    // generation to the disenchant template.
    linen.disenchant_id = 9999;
    stores.item_prototypes.insert_for_tests(linen);
    let (port, saver) =
        start_server_with_capture(backend, stores, Arc::new(FakeCharQueries::new_hunter())).await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;
    let _ = flush_packets(&mut client).await;

    let container_guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::ITEM, 0, 9002);
    let container = container_guid.raw();

    // Open the container (bag 255, slot 23).
    let mut view: Option<Vec<u8>> = None;
    for _ in 0..10 {
        let _ = flush_packets(&mut client).await;
        client
            .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 23])
            .await;
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch.iter().find(|(op, b)| {
            *op == opcode_id::SMSG_LOOT_RESPONSE
                && b.len() > 13
                && u64::from_le_bytes(b[0..8].try_into().expect("guid")) == container
                && b[8] == 1
        }) {
            view = Some(b.clone());
            break;
        }
    }
    let view = view.expect("item loot response");
    let rd32 = |at: usize| u32::from_le_bytes(view[at..at + 4].try_into().expect("u32"));
    assert_eq!(
        u64::from_le_bytes(view[0..8].try_into().expect("guid")),
        container,
        "loot guid"
    );
    assert_eq!(view[8], 1, "LOOT_CORPSE");
    assert_eq!(rd32(9), 137, "gold");
    assert_eq!(view[13], 1, "one item shown");
    assert_eq!(view[14], 0, "slot 0");
    assert_eq!(rd32(15), 25, "sword");
    assert_eq!(rd32(19), 1, "stack of 1");
    assert_eq!(rd32(23), 1542, "sword display");
    assert_eq!(view[35], 0, "ALLOW_LOOT slot type");

    // Take the item.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut removed = false;
    let mut pushed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        removed |= batch
            .iter()
            .any(|(op, b)| *op == opcode_id::SMSG_LOOT_REMOVED && b.first() == Some(&0));
        pushed |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_ITEM_PUSH_RESULT);
        if removed && pushed {
            break;
        }
    }
    assert!(removed, "loot removed notify");
    assert!(pushed, "item push result");

    // Re-open mid-window: the served state is the
    // stored one (gold kept, taken slot hidden),
    // never regenerated.
    client
        .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 23])
        .await;
    let mut reopened: Option<Vec<u8>> = None;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        if let Some((_, b)) = batch.iter().find(|(op, b)| {
            *op == opcode_id::SMSG_LOOT_RESPONSE
                && b.len() > 13
                && u64::from_le_bytes(b[0..8].try_into().expect("guid")) == container
        }) {
            reopened = Some(b.clone());
            break;
        }
    }
    let reopened = reopened.expect("reopen response");
    assert_eq!(
        u32::from_le_bytes(reopened[9..13].try_into().expect("u32")),
        137
    );
    assert_eq!(reopened[13], 0, "taken slot hidden");

    // Double-take the same slot: rejected, no
    // second push.
    client
        .send_packet(opcode_id::CMSG_AUTOSTORE_LOOT_ITEM, &[0])
        .await;
    let mut already_looted = false;
    let mut repushed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        already_looted |= batch.iter().any(|(op, b)| {
            *op == opcode_id::SMSG_INVENTORY_CHANGE_FAILURE && b.first() == Some(&49)
        });
        repushed |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_ITEM_PUSH_RESULT);
        if already_looted {
            break;
        }
    }
    assert!(already_looted, "double take rejected");
    assert!(!repushed, "no second push");

    // Take the money (hunter seed money is 100_000).
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared = false;
    let mut coinage: Option<u32> = None;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if let Some(v) = values_field(&batch, 7, PLAYER_FIELD_COINAGE) {
            coinage = Some(v);
        }
        if cleared && coinage == Some(100_137) {
            break;
        }
    }
    assert!(cleared, "loot clear money");
    assert_eq!(coinage, Some(100_137), "coinage after loot money");

    // Second money: idempotent, no extra credit.
    client.send_packet(opcode_id::CMSG_LOOT_MONEY, &[]).await;
    let mut cleared_twice = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        cleared_twice |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_CLEAR_MONEY);
        if let Some(v) = values_field(&batch, 7, PLAYER_FIELD_COINAGE) {
            coinage = Some(v);
        }
        if cleared_twice {
            break;
        }
    }
    assert_eq!(coinage, Some(100_137), "no extra credit");

    // Release the emptied container: a fully looted
    // non-bag is destroyed (fork `DoLootRelease`).
    client
        .send_packet(opcode_id::CMSG_LOOT_RELEASE, &container.to_le_bytes())
        .await;
    let mut released = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        released |= batch
            .iter()
            .any(|(op, _)| *op == opcode_id::SMSG_LOOT_RELEASE_RESPONSE);
        if released {
            break;
        }
    }
    assert!(released, "loot release response");

    // Re-open after destruction: the container is
    // gone (`EQUIP_ERR_ITEM_NOT_FOUND`).
    client
        .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 23])
        .await;
    let mut gone = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        gone |= batch.iter().any(|(op, b)| {
            *op == opcode_id::SMSG_INVENTORY_CHANGE_FAILURE && b.first() == Some(&23)
        });
        if gone {
            break;
        }
    }
    assert!(gone, "destroyed container not found");

    // Logout: the capturing saver observes the
    // granted sword, the coinage, and the removed
    // container (whose loot rows never reached the
    // DB, so no `item_loot` delete is armed).
    client
        .send_packet(opcode_id::CMSG_LOGOUT_REQUEST, &[])
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGOUT_COMPLETE)
        .await;
    let bundles = saver.saved.lock().expect("saver mutex");
    let (bundle, _) = bundles.last().expect("logout save");
    assert_eq!(bundle.row.money, 100_137);
    let granted = bundle
        .inventory
        .iter()
        .find(|i| i.row.item.item_id == 25 && i.row.item.item_guid != 9001)
        .expect("granted sword row");
    assert_eq!(granted.state, mangos_db::character_save::ItemSaveState::New);
    assert_eq!(granted.row.item.count, 1);
    let removed = bundle
        .inventory
        .iter()
        .find(|i| i.row.item.item_guid == 9002)
        .expect("removed container row");
    assert_eq!(
        removed.state,
        mangos_db::character_save::ItemSaveState::Removed
    );
    assert!(matches!(
        removed.loot,
        mangos_db::character_save::LootSave::Unchanged
    ));
    assert!(!removed.has_saved_loot);
}

/// Wrapped-gift unwrap (`CMSG_OPEN_ITEM` on an
/// `ITEM_DYNFLAG_WRAPPED` row, fork
/// `HandleOpenItemOpcode` wrapped arm,
/// `SpellHandler.cpp:221-246`): 9003 carries a gift
/// row and restores to its real entry/flags with the
/// gift row deleted (no loot window opens for an
/// unwrap); 9004 has no gift row and is destroyed
/// (fork miss arm). The logout save observes the
/// restored row as `Changed` and the destroyed
/// wrapper as `Removed` with its saved wrapped bit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // scripted packet sequence
async fn gift_unwrap_round_trip() {
    use mangos_protocol::opcode_table::opcode_id;

    let backend = FakeBackend::with_test_user();
    let stores = fake_loot_world_stores();
    let (port, saver) = start_server_with_capture(
        backend,
        stores,
        Arc::new(FakeCharQueries::new_gift_hunter()),
    )
    .await;
    let (mut client, seed) = Client::connect(port).await;

    client.send_auth(seed, "TESTUSER", good_digest(seed)).await;
    client.crypt.init(SESSIONKEY_HEX);
    let (op, _) = client.read_packet().await.expect("auth ok");
    assert_eq!(op, opcode::SMSG_AUTH_RESPONSE);
    client.send_packet(opcode::CMSG_CHAR_ENUM, &[]).await;
    let _ = client.read_until_opcode(opcode::SMSG_CHAR_ENUM).await;
    client
        .send_packet(opcode::CMSG_PLAYER_LOGIN, &7u64.to_le_bytes())
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGIN_VERIFY_WORLD)
        .await;
    let _ = flush_packets(&mut client).await;
    let _ = flush_packets(&mut client).await;

    let is_window_for = |batch: &Vec<(u16, Vec<u8>)>, low: u32| {
        let guid = mangos_guid::ObjectGuid::with_entry(mangos_guid::HighGuid::ITEM, 0, low).raw();
        batch.iter().any(|(op, b)| {
            *op == opcode_id::SMSG_LOOT_RESPONSE
                && b.len() > 8
                && u64::from_le_bytes(b[0..8].try_into().expect("guid")) == guid
        })
    };

    // Open the gifted wrapper (bag 255, slot 24): an
    // unwrap restores the row and opens no window.
    client
        .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 24])
        .await;
    let mut windowed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        windowed |= is_window_for(&batch, 9003);
    }
    assert!(!windowed, "unwrap opens no loot window");

    // Open the gift-less wrapper (slot 25): the fork
    // miss arm destroys it, so no window either, and
    // a re-open reports it missing.
    client
        .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 25])
        .await;
    let mut miss_windowed = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        miss_windowed |= is_window_for(&batch, 9004);
    }
    assert!(!miss_windowed, "miss opens no loot window");
    client
        .send_packet(opcode_id::CMSG_OPEN_ITEM, &[255, 25])
        .await;
    let mut gone = false;
    for _ in 0..8 {
        let batch = flush_packets(&mut client).await;
        gone |= batch.iter().any(|(op, b)| {
            *op == opcode_id::SMSG_INVENTORY_CHANGE_FAILURE && b.first() == Some(&23)
        });
        if gone {
            break;
        }
    }
    assert!(gone, "gift-less wrapper destroyed");

    // Logout: the capturing saver observes the
    // restored row (`Changed` with the real
    // entry/flags and a zeroed creator) and the
    // destroyed wrapper (`Removed` with its saved
    // wrapped bit, so the save deletes the gift row
    // exactly as forked).
    client
        .send_packet(opcode_id::CMSG_LOGOUT_REQUEST, &[])
        .await;
    let _ = client
        .read_until_opcode(opcode_id::SMSG_LOGOUT_COMPLETE)
        .await;
    let bundles = saver.saved.lock().expect("saver mutex");
    let (bundle, _) = bundles.last().expect("logout save");
    let restored = bundle
        .inventory
        .iter()
        .find(|i| i.row.item.item_guid == 9003)
        .expect("restored gift row");
    assert_eq!(
        restored.state,
        mangos_db::character_save::ItemSaveState::Changed
    );
    assert_eq!(restored.row.item.item_id, 25);
    assert_eq!(restored.row.item.flags, 0);
    assert_eq!(restored.row.item.gift_creator_guid, 0);
    assert!(!restored.wrapped);
    let removed = bundle
        .inventory
        .iter()
        .find(|i| i.row.item.item_guid == 9004)
        .expect("removed wrapper row");
    assert_eq!(
        removed.state,
        mangos_db::character_save::ItemSaveState::Removed
    );
    assert!(removed.wrapped);
}
