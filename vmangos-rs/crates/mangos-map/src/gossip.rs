//! Fork gossip/quest script host (`Script` fields
//! `pGossipHello` / `pGossipSelect` /
//! `pQuestAcceptNPC` / `pQuestRewardedNPC`,
//! Tier 3 Slice 2 step 5): dispatch by creature
//! `script_name` at the session seams. The menu
//! model mirrors fork `GossipMenu` items as sent
//! by `PlayerMenu::SendGossipMenu`
//! (`GossipDef.cpp:130`); item text ids resolve
//! through broadcast text like the int32
//! `AddMenuItem` overload (`mangos_string` ids < 0
//! are not modeled — empty text). Hello returns
//! `None` when the script does not handle the
//! interaction (fork `false`); select returns
//! `None` likewise.

use mangos_entities::unit::{Team, UnitBase};
use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;
use mangos_world_data::spawns::WorldLocation;

/// `GOSSIP_ICON_CHAT` (`GossipDef.h`).
const GOSSIP_ICON_CHAT: u8 = 0;
/// `GOSSIP_SENDER_MAIN` (`ScriptedGossip.h`).
const GOSSIP_SENDER_MAIN: u32 = 1;
/// `GOSSIP_ACTION_INFO_DEF` (`ScriptedGossip.h`).
const GOSSIP_ACTION_INFO_DEF: u32 = 1000;

use mangos_movement::movement_info::Position;

use crate::battlegrounds::team_index;
use crate::bg_av::{assault, challenge};
use crate::content::{
    BRD_NOT_STARTED, BRD_SPECIAL, BRD_TYPE_QUEST_JAIL_BREAK, BRD_TYPE_ROCKNOT,
    BWL_DATA_SCEPTER_CHAMPION, BWL_NOT_STARTED, BWL_SPECIAL, BWL_TYPE_RAZORGORE,
    BWL_TYPE_SCEPTER_RUN, DM_TYPE_GORDOK_TRIBUTE, DONE, GNO_FAIL, GNO_TYPE_GRUBBIS, IN_PROGRESS,
    ZF_EVENT_PYRAMID, ZF_PYRAMID_KILLED_ALL_TROLLS, ZF_PYRAMID_NOT_STARTED,
};
use crate::map::Map;
use crate::spells::CastTarget;

mod alterac_valley;
mod default_menu;
mod script_hooks;

/// One `GossipMenuItem` (fork send shape: index is
/// positional, icon + coded bytes, message).
#[derive(Clone, Debug)]
pub struct GossipItem {
    pub icon: u8,
    pub text: String,
    pub sender: u32,
    pub action: u32,
}

/// A script-built menu (`SendGossipMenu(textId)`).
#[derive(Clone, Debug)]
pub struct GossipMenuData {
    pub text_id: u32,
    pub items: Vec<GossipItem>,
}

/// Result of a handled gossip select.
#[derive(Clone, Debug)]
pub enum GossipSelectOutcome {
    /// `CLOSE_GOSSIP_MENU` (`SMSG_GOSSIP_COMPLETE`).
    Close,
    /// A follow-up `SendGossipMenu`.
    ShowMenu(GossipMenuData),
}

/// One default-menu `GossipMenuItem` plus its
/// `GossipMenuItemData` (`PrepareGossipMenu` output,
/// non-script arm).
#[derive(Clone, Debug)]
pub struct DefaultGossipItem {
    pub icon: u8,
    pub coded: bool,
    pub text: String,
    pub box_text: String,
    /// `m_gBoxMoney` (client confirmation popup amount).
    pub box_money: u32,
    /// `m_gOptionId` (`GossipOption`).
    pub option_id: u32,
    pub action_menu_id: i32,
    pub action_poi_id: u32,
    pub action_script_id: u32,
}

/// `PrepareGossipMenu` + `SendPreparedGossip` inputs for
/// the actor-side quest menu assembly: the session layer
/// computes `PrepareQuestMenu` itself (quest state lives
/// there) and applies the `SendPreparedGossip` branch
/// (just-discovered taxi node, quest-list redirect).
#[derive(Clone, Debug)]
pub struct DefaultGossipMenu {
    pub text_id: u32,
    pub items: Vec<DefaultGossipItem>,
    /// `canSeeQuests` from `PrepareGossipMenu`.
    pub can_see_quests: bool,
    /// Creature `UNIT_NPC_FLAGS` for the
    /// `SendPreparedGossip` quest-list redirect.
    pub npc_flags: u32,
    /// `GossipMenu::IsJustDiscoveredNode`.
    pub just_discovered_node: bool,
}

/// Result of a default-menu gossip select
/// (`Player::OnGossipSelect` creature arm). Service
/// packets the map emits itself (vendor list, stable,
/// taxi menu, talent wipe, bank) ride `MapEvent`s;
/// `Done` means the actor sends nothing further.
#[derive(Clone, Debug)]
pub enum DefaultGossipSelect {
    /// `CloseGossip` only.
    Close,
    /// A follow-up default menu (actor re-assembles
    /// with its quest menu).
    ShowMenu(DefaultGossipMenu),
    /// `SendPreparedQuest` via the actor quest list.
    QuestList,
    /// `SendTrainerList` (actor supplies the default
    /// title string).
    TrainerList,
    /// `CloseGossip` + `SendPetitionShowList` (actor).
    PetitionShowList,
    /// `CloseGossip` + `SendTabardVendorActivate`
    /// (actor).
    TabardActivate,
    /// `SendAuctionHello` (actor).
    AuctionHello,
    /// `SendBattleGroundList` for this BG type (actor).
    BattlefieldList(u32),
    /// `CloseGossip` + `SetBindPoint`
    /// (`SMSG_BINDER_CONFIRM`, actor).
    BinderConfirm,
    /// Map already emitted the service packet(s), or
    /// the fork returns silently.
    Done,
}

/// `GossipOption` (`GossipDef.h:37-55`).
const GOSSIP_OPTION_GOSSIP: u32 = 1;
const GOSSIP_OPTION_QUESTGIVER: u32 = 2;
const GOSSIP_OPTION_VENDOR: u32 = 3;
const GOSSIP_OPTION_TAXIVENDOR: u32 = 4;
const GOSSIP_OPTION_TRAINER: u32 = 5;
const GOSSIP_OPTION_SPIRITHEALER: u32 = 6;
const GOSSIP_OPTION_SPIRITGUIDE: u32 = 7;
const GOSSIP_OPTION_INNKEEPER: u32 = 8;
const GOSSIP_OPTION_BANKER: u32 = 9;
const GOSSIP_OPTION_PETITIONER: u32 = 10;
const GOSSIP_OPTION_TABARDDESIGNER: u32 = 11;
const GOSSIP_OPTION_BATTLEFIELD: u32 = 12;
const GOSSIP_OPTION_AUCTIONEER: u32 = 13;
const GOSSIP_OPTION_STABLEPET: u32 = 14;
const GOSSIP_OPTION_ARMORER: u32 = 15;
const GOSSIP_OPTION_UNLEARNTALENTS: u32 = 16;
const GOSSIP_OPTION_UNLEARNPETSKILLS: u32 = 17;

/// `UNIT_NPC_FLAG_QUESTGIVER`.
const NPC_FLAG_QUESTGIVER: u32 = 0x2;
/// `CLASS_HUNTER`.
const CLASS_HUNTER: u8 = 3;
/// `TRAINER_TYPE_CLASS`.
const TRAINER_TYPE_CLASS_GOSSIP: u32 = 0;
/// `TRAINER_TYPE_PETS`.
const TRAINER_TYPE_PETS_GOSSIP: u32 = 3;
/// Spirit healer resurrect cast
/// (`OnGossipSelect` spirit-healer arm).
const SPELL_SPIRIT_HEAL: u32 = 17251;

/// `DEFAULT_GOSSIP_MESSAGE` (`GossipDef.h:33`).
const DEFAULT_GOSSIP_MESSAGE: u32 = 16_777_215;

/// `SPELL_ID_PASSIVE_RESURRECTION_SICKNESS`
/// (`SharedDefines.h:1183`).
const SPELL_PASSIVE_RESURRECTION_SICKNESS: u32 = 15_007;

/// `QUEST_STATUS_NONE` (`QuestDef.h:93`).
const EKB_QUEST_STATUS_NONE: i32 = 0;
/// `QUEST_STATUS_COMPLETE` (`QuestDef.h:94`).
const EKB_QUEST_STATUS_COMPLETE: i32 = 1;
/// `QUEST_STATUS_INCOMPLETE` (`QuestDef.h:96`).
const EKB_QUEST_STATUS_INCOMPLETE: i32 = 3;

/// `QUEST_STORMWIND_RENDEZVOUS` (`quest_stormwind_rendezvous.h:32`).
const EKB_QUEST_STORMWIND_RENDEZVOUS: u32 = 6402;
/// `QUEST_THE_GREAT_MASQUERADE` (`quest_stormwind_rendezvous.h:33`).
const EKB_QUEST_THE_GREAT_MASQUERADE: u32 = 6403;
/// `GOSSIP_ROWE_READY` (`quest_stormwind_rendezvous.h:52`).
const EKB_GOSSIP_ROWE_READY: u32 = 9065;
/// `GOSSIP_ROWE_BUSY` (`quest_stormwind_rendezvous.h:53`).
#[allow(dead_code)]
const EKB_GOSSIP_ROWE_BUSY: u32 = 9064;
/// `GOSSIP_ROWE_COMPLETED` (`quest_stormwind_rendezvous.h:51`).
const EKB_GOSSIP_ROWE_COMPLETED: u32 = 9066;
/// `GOSSIP_ROWE_NOTHING` (`quest_stormwind_rendezvous.h:54`).
const EKB_GOSSIP_ROWE_NOTHING: u32 = 9063;
/// Windsor gossip menu text (`quest_stormwind_rendezvous.cpp:813`).
const EKB_WINDSOR_GOSSIP_TEXT: u32 = 5633;
/// Windsor gossip item broadcast text (`quest_stormwind_rendezvous.cpp:809`).
const EKB_WINDSOR_GOSSIP_ITEM_TEXT: u32 = 8256;
/// `SAY_WINDSOR12` (`quest_stormwind_rendezvous.h:80`).
const EKB_SAY_WINDSOR12: u32 = 8208;
/// `QUEST_BEAT` (`stormwind_city.cpp:39`).
const EKB_QUEST_BEAT: u32 = 1640;
/// `FACTION_ENEMY` (`stormwind_city.cpp:38`).
const EKB_FACTION_ENEMY: u32 = 168;
/// `QUEST_MISSING_DIPLO_PT8` (`stormwind_city.cpp:114`).
const EKB_QUEST_MISSING_DIPLO_PT8: u32 = 1447;
/// `SAY_PROGRESS_1_DAS` broadcast text (`stormwind_city.cpp:109`).
#[allow(dead_code)]
const EKB_SAY_PROGRESS_1_DAS: u32 = 1961;
/// `FACTION_NEUTRAL` (`stormwind_city.cpp:119`).
const EKB_FACTION_NEUTRAL: u32 = 189;
/// `NPC_OLD_TOWN_THUG` (`stormwind_city.cpp:117`).
const EKB_NPC_OLD_TOWN_THUG: u32 = 4969;
/// `QUEST_GALENS_ESCAPE` (`swamp_of_sorrows.cpp:36`).
const EKB_QUEST_GALENS_ESCAPE: u32 = 1393;
/// `SAY_QUEST_ACCEPTED` (`swamp_of_sorrows.cpp:41`).
#[allow(dead_code)]
const EKB_SAY_GALEN_ACCEPTED: u32 = 1854;
/// `QUEST_TOME_VALOR` (`westfall.cpp:44`).
const EKB_QUEST_TOME_VALOR: u32 = 1651;
/// `SAY_DS_START` (`westfall.cpp:37`).
#[allow(dead_code)]
const EKB_SAY_DS_START: u32 = 2360;
/// Previous Missing Diplomat part 10 (`wetlands.cpp:450`, literal 1248).
const EKB_QUEST_MISSING_DIPLOMAT_PART10: u32 = 1248;
/// `QUEST_MISSING_DIPLOMAT_PART11` (`wetlands.cpp:45`).
const EKB_QUEST_MISSING_DIPLOMAT_PART11: u32 = 1249;
/// Mikhail fallback menu text (`wetlands.cpp:464`, `npc_text` 1713).
const EKB_MIKHAIL_FALLBACK_TEXT: u32 = 1713;
/// `QUEST_WAIT_FOR_SIRRA_TO_FINISH` (`duskwood.cpp:321`).
const EKB_QUEST_WAIT_FOR_SIRRA_TO_FINISH: u32 = 401;
/// `NPC_STITCHES` (`duskwood.cpp:319`).
const EKB_NPC_STITCHES: u32 = 412;
/// `NPC_TOWN_CRIER` (`duskwood.cpp:376`).
const EKB_NPC_TOWN_CRIER: u32 = 468;
/// `QUEST_STRANHLETHORN_FEVER` (fork spelling, `stranglethorn_vale.cpp:189`).
const EKB_QUEST_STRANHLETHORN_FEVER: u32 = 349;

#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::match_same_arms,
    clippy::match_single_binding
)]
impl Map {
    /// Creature `script_name` for gossip/quest
    /// dispatch.
    /// GO `script_name` for GO script dispatch
    /// (`GameObject::GetScriptId`).
    fn go_script_name(&self, go: ObjectGuid) -> Option<String> {
        self.gameobject(go).map(|g| g.info.script_name.clone())
    }

    /// `pGOGossipHello` dispatch (`ScriptMgr::OnGossipHello`
    /// GO overload): `true` when a script handles the
    /// interaction. The one registration (scarab gong)
    /// sends the quest menu itself in the fork; the
    /// Rust quest-list send is actor-side either way
    /// (see the type-2 `Use` arm in content.rs).
    pub fn gossip_script_on_hello_go(&mut self, player: ObjectGuid, go: ObjectGuid) -> bool {
        let Some(name) = self.go_script_name(go) else {
            return false;
        };
        if name.as_str() == "go_scarab_gong" {
            self.go_scarab_gong_hello(player, go)
        } else {
            let _ = player;
            false
        }
    }

    /// `pGOQuestAccept` dispatch.
    pub fn go_quest_script_on_accept(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        let Some(name) = self.go_script_name(go) else {
            return;
        };
        match name.as_str() {
            "go_crystalline_tear" => self.go_crystalline_tear_accept(player, go, quest_id),
            "go_beached_quest" => self.go_beached_quest_accept(player, go, quest_id),
            _ => {}
        }
        let _ = (player, quest_id);
    }

    /// `pQuestRewardedGO` dispatch.
    pub fn go_quest_script_on_rewarded(
        &mut self,
        player: ObjectGuid,
        go: ObjectGuid,
        quest_id: u32,
    ) {
        let Some(name) = self.go_script_name(go) else {
            return;
        };
        match name.as_str() {
            "go_helcular_s_grave" => self.go_helcular_s_grave_rewarded(player, go, quest_id),
            "go_dusty_rug" => self.go_dusty_rug_rewarded(player, go, quest_id),
            "go_corrupted_plant" => self.go_corrupted_plant_rewarded(player, go, quest_id),
            "go_broken_trap" => self.go_broken_trap_rewarded(player, go, quest_id),
            "go_scarab_gong" => self.go_scarab_gong_rewarded(player, go, quest_id),
            _ => {}
        }
        let _ = (player, quest_id);
    }

    fn gossip_script_name(&self, npc: ObjectGuid) -> Option<String> {
        self.creature(npc).map(|c| c.info.script_name.clone())
    }

    /// `ObjectMgr::GetBroadcastText` subset for
    /// gossip item texts: gendered male/female pick
    /// by the player's `BYTES_0` gender byte,
    /// empty on a miss (fork returns the entry's
    /// text or an error string; scripts here only
    /// feed existing ids).
    fn gossip_broadcast_text(&self, text_id: u32, player: ObjectGuid) -> String {
        let Some(bct) = self.stores.scripts.broadcast_texts.get(&text_id) else {
            return String::new();
        };
        let gender = self.unit_base(player).map_or(0, |u| {
            (u.world.object.values().get_u32(offsets::UNIT_FIELD_BYTES_0) >> 16) & 0xFF
        });
        if gender == 1 && !bct.female_text.is_empty() {
            bct.female_text.clone()
        } else {
            bct.male_text.clone()
        }
    }

    /// `Player::GetGossipTextId(creature)`
    /// (`Player.cpp:12231`): the spawn's
    /// `npc_gossip` text id, else
    /// `DEFAULT_GOSSIP_MESSAGE`.
    fn gossip_text_id(&self, npc: ObjectGuid) -> u32 {
        self.stores
            .scripts
            .npc_gossips
            .get(&npc.counter())
            .copied()
            .filter(|id| *id != 0)
            .unwrap_or(DEFAULT_GOSSIP_MESSAGE)
    }

    /// `Player::GetGossipTextId(menuId, pSource)`
    /// (`Player.cpp:12242`): highest-condition fitting
    /// `gossip_menu` text, else `DEFAULT_GOSSIP_MESSAGE`.
    /// UNMODELED: the row `script_id` `ScriptsStart`
    /// (`sGossipScripts` are not loaded).
    fn gossip_menu_text_id(&self, menu_id: u32, player: ObjectGuid, npc: ObjectGuid) -> u32 {
        if menu_id == 0 {
            return DEFAULT_GOSSIP_MESSAGE;
        }
        let mut text_id = DEFAULT_GOSSIP_MESSAGE;
        let mut last_condition_id = 0;
        for row in self.stores.gossip.menu_texts(menu_id) {
            let fits = (row.condition_id == 0 && last_condition_id == 0)
                || (row.condition_id > last_condition_id
                    && self.condition_satisfied(row.condition_id, Some(player), Some(npc)));
            if fits {
                last_condition_id = row.condition_id;
                text_id = row.text_id;
            }
        }
        text_id
    }

    /// `Creature::CanInteractWithBattleMaster(player,
    /// false)` subset: battlemaster entry + level access
    /// via the battleground template's level bracket.
    /// UNMODELED: the exact `GetBGAccessByLevel` bracket
    /// table (template min/max level used instead).
    fn can_interact_with_battle_master(&self, player: ObjectGuid, npc: ObjectGuid) -> bool {
        let Some(creature) = self.creature(npc) else {
            return false;
        };
        let bg_type = self.stores.battlemasters.bg_type(creature.info.entry);
        if bg_type == 0 {
            return false;
        }
        let level = self.unit_base(player).map_or(0, UnitBase::level);
        match self.stores.battlegrounds.get(bg_type) {
            Some(tpl) => level >= tpl.min_level && (tpl.max_level == 0 || level <= tpl.max_level),
            None => true,
        }
    }
}
