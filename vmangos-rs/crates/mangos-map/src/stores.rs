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
    pub areas: AreaStore,
    pub weather_chances: WeatherChanceStore,
    pub waypoints: WaypointStore,
    pub spells: SpellStore,
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
    /// Skill DBC tables (`_LoadSkills` validation/fixups).
    pub skills: SkillDbc,
    /// Built-in chat channel table (`ChatChannels.dbc`).
    pub chat_channels: ChatChannelDbc,
    /// Auction house table (`AuctionHouse.dbc`, Phase 5 B.7).
    pub auction_houses: mangos_world_data::auction_house::AuctionHouseDbc,
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
            areas: AreaStore::default(),
            weather_chances: WeatherChanceStore::default(),
            waypoints: WaypointStore::default(),
            spells: SpellStore::default(),
            creature_spells: CreatureSpellsMap::new(),
            scripts: mangos_scripts_engine::ScriptStores::default(),
            display_scales: HashMap::new(),
            model_data: HashMap::new(),
            skills: SkillDbc::default(),
            chat_channels: ChatChannelDbc::default(),
            auction_houses: mangos_world_data::auction_house::AuctionHouseDbc::default(),
        }
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
