//! Read-only world stores shared by every map (the
//! `ObjectMgr` products), plus the DBC-derived display
//! tables the aggregates consume through resolvers.

use std::collections::HashMap;

use mangos_entities::unit::ModelData;
use mangos_spells::SpellStore;
use mangos_world_data::areas::AreaStore;
use mangos_world_data::chat_channels::ChatChannelDbc;
use mangos_world_data::creature_spells::CreatureSpellsMap;
use mangos_world_data::creature_template::CreatureTemplates;
use mangos_world_data::gameobject_template::GameObjectTemplates;
use mangos_world_data::item_prototype::{ItemPrototypes, ItemPrototypesLite};
use mangos_world_data::player_info::PlayerInfos;
use mangos_world_data::reputations::FactionStore;
use mangos_world_data::skill_info::SkillDbc;
use mangos_world_data::spawn_support::{
    ClassLevelStats, CreatureAddons, DisplayInfoAddons, EquipmentTemplates,
};
use mangos_world_data::spawns::SpawnData;
use mangos_world_data::waypoints::WaypointStore;
use mangos_world_data::weather::WeatherChanceStore;

/// Static stores, built once at boot by the binary and lent
/// to maps by reference.
pub struct WorldStores {
    pub creature_templates: CreatureTemplates,
    pub gameobject_templates: GameObjectTemplates,
    pub spawns: SpawnData,
    pub creature_addons: CreatureAddons,
    pub equipment: EquipmentTemplates,
    pub cls: ClassLevelStats,
    pub display_addons: DisplayInfoAddons,
    pub player_infos: PlayerInfos,
    pub items: ItemPrototypesLite,
    pub item_prototypes: ItemPrototypes,
    pub factions: FactionStore,
    /// The `creature_onkill_reputation`,
    /// `reputation_reward_rate`, and
    /// `reputation_spillover_template` products
    /// (reputation-on-kill slice; empty until the world
    /// DB loads them).
    pub reputation_kills: mangos_world_data::reputations::ReputationKillStores,
    pub areas: AreaStore,
    pub weather_chances: WeatherChanceStore,
    pub waypoints: WaypointStore,
    pub spells: SpellStore,
    /// `spell_target_position` destinations
    /// (Tier 1 Slice 5).
    pub spell_target_positions: mangos_world_data::spell_target_position::SpellTargetPositions,
    /// `creature_spells` lists by `spell_list_id`
    /// (Slice 6 creature casting).
    pub creature_spells: CreatureSpellsMap,
    /// DB script engine stores (Slice 6b).
    pub scripts: mangos_scripts_engine::ScriptStores,
    /// `GetScaleForDisplayId` results by display id
    /// (missing -> `DEFAULT_OBJECT_SCALE`).
    pub display_scales: HashMap<u32, f32>,
    /// `UpdateModelData` inputs by display id.
    pub model_data: HashMap<u32, ModelData>,
    /// `SpellShapeshiftForm.dbc` form → `flags1`
    /// (`sSpellShapeshiftFormStore` subset).
    pub shapeshift_forms: HashMap<u32, u32>,
    /// Skill DBC tables (`_LoadSkills` validation/fixups).
    pub skills: SkillDbc,
    /// Built-in chat channel table (`ChatChannels.dbc`).
    pub chat_channels: ChatChannelDbc,
    /// Auction house table (`AuctionHouse.dbc`, Phase 5 B.7).
    pub auction_houses: mangos_world_data::auction_house::AuctionHouseDbc,
    /// `map_template` rows (`sMapStorage`, Phase 6 Slice 0).
    pub maps: mangos_world_data::map_template::MapStore,
    /// `WorldSafeLocs.dbc` + DB facings (Tier 3 Slice 3).
    pub safe_locs: mangos_world_data::battlegrounds::SafeLocStore,
    /// `battleground_template` rows (Tier 3 Slice 3).
    pub battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates,
    /// `battlemaster_entry` rows (Tier 3 Slice 3).
    pub battlemasters: mangos_world_data::battlegrounds::BattleMasters,
    /// `*_battleground` spawn event indexes (Tier 3 Slice 3).
    pub battle_event_indexes: mangos_world_data::battlegrounds::BattleEventIndexes,
    /// `game_graveyard_zone` links (Tier 3 Slice 3).
    pub graveyard_links: mangos_world_data::battlegrounds::GraveyardLinks,
    /// `areatrigger_bg_entrance` rows (Tier 3 Slice 3).
    pub bg_entrances: mangos_world_data::battlegrounds::BattlegroundEntrances,
    /// Area triggers + teleports (Phase 6 Slice 0).
    pub area_triggers: mangos_world_data::area_trigger::AreaTriggerStore,
    /// Quest template subset for load/save parity
    /// (Phase 6 Slice 1b).
    pub quests: mangos_world_data::quest_template::QuestTemplatesLite,
    /// Full `quest_template` rows for the questgiver
    /// runtime (Tier 1 Slice 1a).
    pub quest_templates: mangos_world_data::quest_template::QuestTemplates,
    /// Questgiver start/end relations (Tier 1 Slice 1a).
    pub quest_relations: mangos_world_data::quest_template::QuestRelations,
    /// Default gossip menus/options/npc texts
    /// (`LoadGossipMenu` family, Tier 4 gnome probe).
    pub gossip: mangos_world_data::gossip_menus::GossipStores,
    /// `areatrigger_tavern` ids (Phase 6 Slice 1b rest).
    pub tavern_triggers: std::collections::HashSet<u32>,
    /// `exploration_basexp` (depth Slice 1b).
    pub exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp,
    /// Pet level stats / create spells / name parts
    /// (Tier 1 Slice 2a).
    pub pets: mangos_world_data::pet_data::PetStores,
    /// `ObjectMgr::m_NextPetNumber` (Tier 1 Slice 2a):
    /// seeded past `MAX(character_pet.id)` at boot,
    /// bumped past any loaded row id.
    pub next_pet_number: std::sync::atomic::AtomicU32,
    /// `ObjectMgr::GenerateItemGuid` low counter
    /// (Tier 4 step 4b2): seeded past
    /// `MAX(item_instance.guid)` at boot; the
    /// DB-immediate grant path bumps it past any
    /// guid it allocates (`observe_item_guid`).
    pub next_item_guid: std::sync::atomic::AtomicU32,
    /// Vendor stock lists (Tier 4 step 4c).
    pub vendor_items: mangos_world_data::vendor_items::VendorItemStores,
    /// Durability repair cost tables (Tier 4 step 4c).
    pub durability: mangos_world_data::durability::DurabilityTables,
    /// `BankBagSlotPrices.dbc` by slot id (Tier 4
    /// step 4d); index 0 unused.
    pub bank_bag_slot_prices: Vec<u32>,
    /// `StableSlotPrices.dbc` indexed by slot number
    /// (fork `sStableSlotPricesStore`; Tier 4 step
    /// 4h).
    pub stable_slot_prices: Vec<u32>,
    /// Trainer spell lists and greetings (Tier 4
    /// step 4e).
    pub trainer_spells: mangos_world_data::trainer_spells::TrainerStores,
    /// Taxi flight graph (Tier 4 step 4f).
    pub taxi: mangos_world_data::taxi::TaxiStore,
    /// Escort script waypoints (Tier 4 B0-6).
    pub script_waypoints: mangos_world_data::script_waypoints::ScriptWaypointStore,
    /// Game-event definitions and spawn lists
    /// (Tier 4 B0-8).
    pub game_events: mangos_world_data::game_events::GameEvents,
    /// Loot templates (Tier 4 B0-10).
    pub loot: mangos_world_data::loot::LootStores,
    /// `Lock.dbc` rows (`sLockStore`, chest/mining
    /// lock gates; DBC-only, no DB/save change).
    pub locks: mangos_world_data::locks::LockStore,
    /// `skill_fishing_base_level` rows
    /// (`ObjectMgr::m_FishingBaseSkillMap`; DB
    /// read-only, no save change).
    pub fishing_base_skills: mangos_world_data::fishing::FishingBaseSkills,
}

impl WorldStores {
    /// Empty stores (test worlds with no content).
    #[must_use]
    pub fn empty() -> WorldStores {
        WorldStores {
            creature_templates: CreatureTemplates::default(),
            gameobject_templates: GameObjectTemplates::default(),
            spawns: SpawnData::new(),
            creature_addons: CreatureAddons::default(),
            equipment: EquipmentTemplates::default(),
            cls: ClassLevelStats::default(),
            display_addons: DisplayInfoAddons::default(),
            player_infos: PlayerInfos::default(),
            items: ItemPrototypesLite::default(),
            item_prototypes: ItemPrototypes::default(),
            factions: FactionStore::default(),
            reputation_kills: mangos_world_data::reputations::ReputationKillStores::default(),
            areas: AreaStore::default(),
            weather_chances: WeatherChanceStore::default(),
            waypoints: WaypointStore::default(),
            spells: SpellStore::default(),
            spell_target_positions:
                mangos_world_data::spell_target_position::SpellTargetPositions::default(),
            creature_spells: CreatureSpellsMap::new(),
            scripts: mangos_scripts_engine::ScriptStores::default(),
            display_scales: HashMap::new(),
            model_data: HashMap::new(),
            shapeshift_forms: HashMap::new(),
            skills: SkillDbc::default(),
            maps: mangos_world_data::map_template::MapStore::default(),
            safe_locs: mangos_world_data::battlegrounds::SafeLocStore::default(),
            battlegrounds: mangos_world_data::battlegrounds::BattlegroundTemplates::default(),
            battlemasters: mangos_world_data::battlegrounds::BattleMasters::default(),
            battle_event_indexes: mangos_world_data::battlegrounds::BattleEventIndexes::default(),
            graveyard_links: mangos_world_data::battlegrounds::GraveyardLinks::default(),
            bg_entrances: mangos_world_data::battlegrounds::BattlegroundEntrances::default(),
            area_triggers: mangos_world_data::area_trigger::AreaTriggerStore::default(),
            quests: mangos_world_data::quest_template::QuestTemplatesLite::default(),
            quest_templates: mangos_world_data::quest_template::QuestTemplates::default(),
            quest_relations: mangos_world_data::quest_template::QuestRelations::default(),
            gossip: mangos_world_data::gossip_menus::GossipStores::default(),
            tavern_triggers: std::collections::HashSet::new(),
            exploration_base_xp: mangos_world_data::exploration::ExplorationBaseXp::default(),
            chat_channels: ChatChannelDbc::default(),
            auction_houses: mangos_world_data::auction_house::AuctionHouseDbc::default(),
            pets: mangos_world_data::pet_data::PetStores::default(),
            next_pet_number: std::sync::atomic::AtomicU32::new(1),
            next_item_guid: std::sync::atomic::AtomicU32::new(1),
            vendor_items: mangos_world_data::vendor_items::VendorItemStores::default(),
            durability: mangos_world_data::durability::DurabilityTables::default(),
            bank_bag_slot_prices: Vec::new(),
            stable_slot_prices: Vec::new(),
            trainer_spells: mangos_world_data::trainer_spells::TrainerStores::default(),
            taxi: mangos_world_data::taxi::TaxiStore::default(),
            script_waypoints: mangos_world_data::script_waypoints::ScriptWaypointStore::default(),
            game_events: mangos_world_data::game_events::GameEvents::default(),
            loot: mangos_world_data::loot::LootStores::default(),
            locks: mangos_world_data::locks::LockStore::default(),
            fishing_base_skills: mangos_world_data::fishing::FishingBaseSkills::default(),
        }
    }

    /// `ObjectMgr::GeneratePetNumber` (the fork walks
    /// its pet cache for the next free id from the
    /// counter; seeded past the DB max here, so a
    /// plain fetch-add allocates the same ids).
    pub fn generate_pet_number(&self) -> u32 {
        self.next_pet_number
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Allocate the next item instance guid.
    pub fn generate_item_guid(&self) -> u32 {
        self.next_item_guid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Keep the item counter past an externally
    /// allocated guid (DB-immediate grants).
    pub fn observe_item_guid(&self, item_guid: u32) {
        self.next_item_guid
            .fetch_max(item_guid + 1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Keep the counter past a loaded row id (fork
    /// cache coherence on `GeneratePetNumber`).
    pub fn observe_pet_number(&self, pet_id: u32) {
        self.next_pet_number
            .fetch_max(pet_id + 1, std::sync::atomic::Ordering::Relaxed);
    }

    #[must_use]
    pub fn scale_for_display(&self, display_id: u32) -> f32 {
        self.display_scales
            .get(&display_id)
            .copied()
            .unwrap_or(mangos_entities::unit::DEFAULT_OBJECT_SCALE)
    }

    #[must_use]
    pub fn model_for_display(&self, display_id: u32) -> Option<ModelData> {
        self.model_data.get(&display_id).copied()
    }
}
