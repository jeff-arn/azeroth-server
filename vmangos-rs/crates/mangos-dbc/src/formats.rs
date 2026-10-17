//! The format-string table from `vmangos/src/game/Database/DBCfmt.h`
//! (1.12.1 variants where the header is conditionally compiled) and
//! the store→file map from `DBCStores.cpp` `LoadDBCStores`.
//!
//! Each constant keeps the C++ name minus the `fmt` suffix so a diff
//! against the header stays mechanical.

/// One loaded store: DBC file name + its format string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreDef {
    /// File name inside the `dbc/` data directory.
    pub file: &'static str,
    /// Format string from `DBCfmt.h`.
    pub fmt: &'static str,
}

// Exemplar entries (driver-transcribed; the full table follows the
// same one-line-per-store shape).
pub const AUCTION_HOUSE: StoreDef = StoreDef {
    file: "AuctionHouse.dbc",
    fmt: "niiixxxxxxxxx",
};
pub const BANK_BAG_SLOT_PRICES: StoreDef = StoreDef {
    file: "BankBagSlotPrices.dbc",
    fmt: "ni",
};
pub const CREATURE_FAMILY: StoreDef = StoreDef {
    file: "CreatureFamily.dbc",
    fmt: "nfifiiiissssssssxx",
};
pub const CHAT_CHANNELS: StoreDef = StoreDef {
    file: "ChatChannels.dbc",
    fmt: "nixssssssssxxxxxxxxxx",
};
pub const CHARACTER_FACIAL_HAIR_STYLES: StoreDef = StoreDef {
    file: "CharacterFacialHairStyles.dbc",
    fmt: "iiixxxxxx",
};
pub const CHAR_SECTIONS: StoreDef = StoreDef {
    file: "CharSections.dbc",
    fmt: "diiiiixxxi",
};
pub const CHR_CLASSES: StoreDef = StoreDef {
    file: "ChrClasses.dbc",
    fmt: "nxxixssssssssxxix",
};
pub const CHR_RACES: StoreDef = StoreDef {
    file: "ChrRaces.dbc",
    fmt: "niixiixxiiiiixixissssssssxxxx",
};
pub const CINEMATIC_SEQUENCES: StoreDef = StoreDef {
    file: "CinematicSequences.dbc",
    fmt: "nxxxxxxxxx",
};
pub const CREATURE_DISPLAY_INFO: StoreDef = StoreDef {
    file: "CreatureDisplayInfo.dbc",
    fmt: "nixifxxxxxxx",
};
pub const CREATURE_DISPLAY_INFO_EXTRA: StoreDef = StoreDef {
    file: "CreatureDisplayInfoExtra.dbc",
    fmt: "nixxxxxxxxxxxxxxxxx",
};
pub const CREATURE_MODEL_DATA: StoreDef = StoreDef {
    file: "CreatureModelData.dbc",
    fmt: "nisxfxxxxxxxxxxf",
};
pub const CREATURE_TYPE: StoreDef = StoreDef {
    file: "CreatureType.dbc",
    fmt: "nxxxxxxxxxx",
};
pub const DURABILITY_COSTS: StoreDef = StoreDef {
    file: "DurabilityCosts.dbc",
    fmt: "niiiiiiiiiiiiiiiiiiiiiiiiiiiii",
};
pub const DURABILITY_QUALITY: StoreDef = StoreDef {
    file: "DurabilityQuality.dbc",
    fmt: "nf",
};
pub const EMOTES: StoreDef = StoreDef {
    file: "Emotes.dbc",
    fmt: "nsxiiix",
};
pub const EMOTES_TEXT: StoreDef = StoreDef {
    file: "EmotesText.dbc",
    fmt: "nxixxxxxxxxxxxxxxxx",
};
pub const GAME_OBJECT_DISPLAY_INFO: StoreDef = StoreDef {
    file: "GameObjectDisplayInfo.dbc",
    fmt: "nsxxxxxxxxxx",
};
pub const ITEM_BAG_FAMILY: StoreDef = StoreDef {
    file: "ItemBagFamily.dbc",
    fmt: "nxxxxxxxxx",
};
pub const ITEM_RANDOM_PROPERTIES: StoreDef = StoreDef {
    file: "ItemRandomProperties.dbc",
    fmt: "nsiiixxssssssssx",
};
pub const ITEM_SET: StoreDef = StoreDef {
    file: "ItemSet.dbc",
    fmt: "dssssssssxxxxxxxxxxxxxxxxxxiiiiiiiiiiiiiiiiii",
};
pub const LOCK: StoreDef = StoreDef {
    file: "Lock.dbc",
    fmt: "niiiiiiiiiiiiiiiiiiiiiiiixxxxxxxx",
};
pub const NAMES_PROFANITY: StoreDef = StoreDef {
    file: "NamesProfanity.dbc",
    fmt: "ds",
};
pub const NAMES_RESERVED: StoreDef = StoreDef {
    file: "NamesReserved.dbc",
    fmt: "ds",
};
pub const QUEST_SORT: StoreDef = StoreDef {
    file: "QuestSort.dbc",
    fmt: "nxxxxxxxxx",
};
pub const SKILL_LINE: StoreDef = StoreDef {
    file: "SkillLine.dbc",
    fmt: "nixssssssssxxxxxxxxxxi",
};
pub const SKILL_RACE_CLASS_INFO: StoreDef = StoreDef {
    file: "SkillRaceClassInfo.dbc",
    fmt: "diiiiiix",
};
pub const SKILL_TIERS: StoreDef = StoreDef {
    file: "SkillTiers.dbc",
    fmt: "niiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii",
};
pub const SPELL_CATEGORY: StoreDef = StoreDef {
    file: "SpellCategory.dbc",
    fmt: "ni",
};
pub const SPELL_CAST_TIMES: StoreDef = StoreDef {
    file: "SpellCastTimes.dbc",
    fmt: "niii",
};
pub const SPELL_DURATION: StoreDef = StoreDef {
    file: "SpellDuration.dbc",
    fmt: "niii",
};
pub const SPELL_FOCUS_OBJECT: StoreDef = StoreDef {
    file: "SpellFocusObject.dbc",
    fmt: "nxxxxxxxxx",
};
pub const SPELL_ITEM_ENCHANTMENT: StoreDef = StoreDef {
    file: "SpellItemEnchantment.dbc",
    fmt: "niiiiiixxxiiissssssssxii",
};
pub const SPELL_RADIUS: StoreDef = StoreDef {
    file: "SpellRadius.dbc",
    fmt: "nfxx",
};
pub const SPELL_RANGE: StoreDef = StoreDef {
    file: "SpellRange.dbc",
    fmt: "nffxxxxxxxxxxxxxxxxxxx",
};
pub const SPELL_SHAPESHIFT_FORM: StoreDef = StoreDef {
    file: "SpellShapeshiftForm.dbc",
    fmt: "nxssssssssxiix",
};
pub const SPELL_VISUAL: StoreDef = StoreDef {
    file: "SpellVisual.dbc",
    fmt: "niiiiiiiiiiiiiii",
};
pub const STABLE_SLOT_PRICES: StoreDef = StoreDef {
    file: "StableSlotPrices.dbc",
    fmt: "ni",
};
pub const TALENT: StoreDef = StoreDef {
    file: "Talent.dbc",
    fmt: "niiiiiiiixxxxixxixxxi",
};
pub const TALENT_TAB: StoreDef = StoreDef {
    file: "TalentTab.dbc",
    fmt: "nxxxxxxxxxxxiix",
};
pub const TAXI_PATH: StoreDef = StoreDef {
    file: "TaxiPath.dbc",
    fmt: "niii",
};
pub const TAXI_PATH_NODE: StoreDef = StoreDef {
    file: "TaxiPathNode.dbc",
    fmt: "diiifffii",
};
pub const TRANSPORT_ANIMATION: StoreDef = StoreDef {
    file: "TransportAnimation.dbc",
    fmt: "diifffx",
};
pub const WORLD_MAP_AREA: StoreDef = StoreDef {
    file: "WorldMapArea.dbc",
    fmt: "xinxffff",
};
pub const WMO_AREA_TABLE: StoreDef = StoreDef {
    file: "WMOAreaTable.dbc",
    fmt: "niiixxxxxiixxxxxxxxx",
};
pub const WORLD_SAFE_LOCS: StoreDef = StoreDef {
    file: "WorldSafeLocs.dbc",
    fmt: "nifffxxxxxxxxx",
};

/// Every loaded store, in `LoadDBCStores` call order.
pub const ALL_STORES: &[StoreDef] = &[
    AUCTION_HOUSE,
    BANK_BAG_SLOT_PRICES,
    CHAT_CHANNELS,
    CHARACTER_FACIAL_HAIR_STYLES,
    CHAR_SECTIONS,
    CHR_CLASSES,
    CHR_RACES,
    CINEMATIC_SEQUENCES,
    CREATURE_DISPLAY_INFO,
    CREATURE_DISPLAY_INFO_EXTRA,
    CREATURE_MODEL_DATA,
    CREATURE_FAMILY,
    CREATURE_TYPE,
    DURABILITY_COSTS,
    DURABILITY_QUALITY,
    EMOTES,
    EMOTES_TEXT,
    GAME_OBJECT_DISPLAY_INFO,
    ITEM_BAG_FAMILY,
    ITEM_RANDOM_PROPERTIES,
    ITEM_SET,
    LOCK,
    NAMES_PROFANITY,
    NAMES_RESERVED,
    QUEST_SORT,
    SKILL_LINE,
    SKILL_RACE_CLASS_INFO,
    SKILL_TIERS,
    SPELL_CATEGORY,
    SPELL_CAST_TIMES,
    SPELL_DURATION,
    SPELL_FOCUS_OBJECT,
    SPELL_ITEM_ENCHANTMENT,
    SPELL_RADIUS,
    SPELL_RANGE,
    SPELL_SHAPESHIFT_FORM,
    SPELL_VISUAL,
    STABLE_SLOT_PRICES,
    TALENT,
    TALENT_TAB,
    TAXI_PATH,
    TAXI_PATH_NODE,
    TRANSPORT_ANIMATION,
    WORLD_MAP_AREA,
    WMO_AREA_TABLE,
    WORLD_SAFE_LOCS,
];
