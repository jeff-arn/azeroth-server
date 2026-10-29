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

use mangos_entities::unit::Team;
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

    /// `pGossipHello` dispatch: `Some(menu)` when a
    /// script handles the interaction.
    pub fn gossip_script_on_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        match self.gossip_script_name(npc)?.as_str() {
            "npc_res_fixes" => Some(self.res_fixer_on_gossip_hello(player)),
            "npc_klinfran" => self.npc_klinfran_gossip_hello(player, npc),
            "npc_joseph_redpath" => self.npc_joseph_redpath_gossip_hello(player, npc),
            "npc_dying_archaeologist" => self.npc_dying_archaeologist_gossip_hello(player, npc),
            "npc_squire_rowe" => self.npc_squire_rowe_gossip_hello(player, npc),
            "npc_reginald_windsor" => self.npc_reginald_windsor_gossip_hello(player, npc),
            "npc_mikhail" => self.npc_mikhail_gossip_hello(player, npc),
            "npc_mistress_nagmara" => self.npc_mistress_nagmara_gossip_hello(player, npc),
            "boss_vaelastrasz" => self.boss_vael_gossip_hello(player, npc),
            "npc_blastmaster_emi_shortfuse" => {
                self.npc_blastmaster_emi_shortfuse_gossip_hello(player, npc)
            }
            "mob_craftsman_omarion" => self.npc_master_craftsman_omarion_gossip_hello(player, npc),
            "npc_knot_thimblejack" => self.npc_knot_thimblejack_gossip_hello(player, npc),
            "boss_kromcrush" => self.boss_kromcrush_gossip_hello(player, npc),
            "npc_sergeant_bly" => self.npc_sergeant_bly_gossip_hello(player, npc),
            "npc_weegli_blastfuse" => self.npc_weegli_blastfuse_gossip_hello(player, npc),
            "npc_threshwackonator" => self.npc_threshwackonator_gossip_hello(player, npc),
            "npc_lady_jaina_proudmoore" => self.npc_lady_jaina_proudmoore_gossip_hello(player, npc),
            "npc_screecher_spirit" => self.npc_screecher_spirit_gossip_hello(player, npc),
            "npc_plucky_johnson" => self.plucky_johnson_gossip_hello(player, npc),
            "npc_simone_the_inconspicuous" => {
                self.simone_the_inconspicuous_gossip_hello(player, npc)
            }
            "npc_Krug_SkullSplit" => self.krug_skullsplit_gossip_hello(player, npc),
            "npc_kwee_peddlefeet" => self.npc_kwee_peddlefeet_gossip_hello(player, npc),
            "npc_argent_emissary" => self.npc_argent_emissary_gossip_hello(player, npc),
            "npc_spirit_guide" => Some(self.npc_spirit_guide_gossip_hello(player, npc)),
            "npc_AV_blood_collector" => self.npc_av_blood_collector_gossip_hello(player, npc),
            "npc_AQwar_collector" => self.npc_aqwar_collector_gossip_hello(player, npc),
            "custom_teleport_npc" => self.teleport_npc_gossip_hello(player, npc),
            "custom_enchant_npc" => self.enchant_npc_gossip_hello(player, npc),
            "custom_professions_npc" => self.profession_npc_gossip_hello(player, npc),
            "custom_premade_gear_npc" => self.premade_gear_npc_gossip_hello(player, npc),
            "custom_premade_spec_npc" => self.premade_spec_npc_gossip_hello(player, npc),
            _ => None,
        }
    }

    /// `pGossipSelect` dispatch: `Some(outcome)`
    /// when a script handles the selection.
    pub fn gossip_script_on_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        match self.gossip_script_name(npc)?.as_str() {
            "npc_res_fixes" => Some(self.res_fixer_on_gossip_select(player, sender, action)),
            "npc_klinfran" => self.npc_klinfran_gossip_select(player, npc, sender, action),
            "npc_squire_rowe" => self.npc_squire_rowe_gossip_select(player, npc, sender, action),
            "npc_reginald_windsor" => {
                self.npc_reginald_windsor_gossip_select(player, npc, sender, action)
            }
            "npc_mistress_nagmara" => {
                self.npc_mistress_nagmara_gossip_select(player, npc, sender, action)
            }
            "boss_vaelastrasz" => self.boss_vael_gossip_select(player, npc, sender, action),
            "npc_blastmaster_emi_shortfuse" => {
                self.npc_blastmaster_emi_shortfuse_gossip_select(player, npc, sender, action)
            }
            "mob_craftsman_omarion" => {
                self.npc_master_craftsman_omarion_gossip_select(player, npc, sender, action)
            }
            "npc_knot_thimblejack" => {
                self.npc_knot_thimblejack_gossip_select(player, npc, sender, action)
            }
            "boss_kromcrush" => self.boss_kromcrush_gossip_select(player, npc, sender, action),
            "npc_sergeant_bly" => self.npc_sergeant_bly_gossip_select(player, npc, sender, action),
            "npc_weegli_blastfuse" => {
                self.npc_weegli_blastfuse_gossip_select(player, npc, sender, action)
            }
            "npc_threshwackonator" => {
                self.npc_threshwackonator_gossip_select(player, npc, sender, action)
            }
            "npc_lady_jaina_proudmoore" => {
                self.npc_lady_jaina_proudmoore_gossip_select(player, npc, sender, action)
            }
            "npc_plucky_johnson" => self.plucky_johnson_gossip_select(player, npc, sender, action),
            "npc_simone_the_inconspicuous" => {
                self.simone_the_inconspicuous_gossip_select(player, npc, sender, action)
            }
            "npc_Krug_SkullSplit" => {
                self.krug_skullsplit_gossip_select(player, npc, sender, action)
            }
            "npc_AV_blood_collector" => {
                Some(self.npc_av_blood_collector_gossip_select(player, npc, sender, action))
            }
            "npc_argent_emissary" => {
                self.npc_argent_emissary_gossip_select(player, npc, sender, action)
            }
            "custom_teleport_npc" => self.teleport_npc_gossip_select(player, npc, sender, action),
            "custom_enchant_npc" => self.enchant_npc_gossip_select(player, npc, sender, action),
            "custom_professions_npc" => {
                self.profession_npc_gossip_select(player, npc, sender, action)
            }
            "custom_premade_gear_npc" => {
                self.premade_gear_npc_gossip_select(player, npc, sender, action)
            }
            "custom_premade_spec_npc" => {
                self.premade_spec_npc_gossip_select(player, npc, sender, action)
            }
            _ => None,
        }
    }

    /// `pQuestAcceptNPC` dispatch (fork
    /// `OnQuestAccept` after `AddQuest` succeeds).
    pub fn quest_script_on_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        let Some(name) = self.gossip_script_name(npc) else {
            return;
        };
        match name.as_str() {
            "npc_professor_phizzlethorpe" => {
                self.npc_professor_phizzlethorpe_quest_accept(player, npc, quest_id);
            }
            "npc_shakes_o_breen" => self.npc_shakes_o_breen_quest_accept(player, npc, quest_id),
            "npc_kinelory" => self.npc_kinelory_quest_accept(player, npc, quest_id),
            "npc_grark_lorkrub" => self.npc_grark_lorkrub_quest_accept(player, npc, quest_id),
            "npc_eris_havenfire" => self.npc_eris_havenfire_quest_accept(player, npc, quest_id),
            "npc_rinji" => self.npc_rinji_quest_accept(player, npc, quest_id),
            "npc_miran" => self.npc_miran_quest_accept(player, npc, quest_id),
            "npc_corporal_keeshan" => self.npc_corporal_keeshan_quest_accept(player, npc, quest_id),
            "npc_dying_archaeologist" => {
                self.npc_dying_archaeologist_quest_accept(player, npc, quest_id);
            }
            "npc_deathstalker_erland" => {
                self.npc_deathstalker_erland_quest_accept(player, npc, quest_id);
            }
            "npc_reginald_windsor" => self.npc_reginald_windsor_quest_accept(player, npc, quest_id),
            "npc_bartleby" => self.npc_bartleby_quest_accept(player, npc, quest_id),
            "npc_dashel_stonefist" => self.npc_dashel_stonefist_quest_accept(player, npc, quest_id),
            "npc_galen_goodward" => self.npc_galen_goodward_quest_accept(player, npc, quest_id),
            "npc_daphne_stilwell" => self.npc_daphne_stilwell_quest_accept(player, npc, quest_id),
            "npc_mikhail" => self.npc_mikhail_quest_accept(player, npc, quest_id),
            "boss_vaelastrasz" => self.vaelastrasz_quest_accept(player, npc, quest_id),
            "npc_marshal_windsor" => self.npc_marshal_windsor_quest_accept(player, npc, quest_id),
            "npc_kernobee" => self.npc_kernobee_quest_accept(player, npc, quest_id),
            "npc_ruul_snowhoof" => self.npc_ruul_snowhoof_quest_accept(player, npc, quest_id),
            "npc_torek" => self.npc_torek_quest_accept(player, npc, quest_id),
            "npc_feero_ironhand" => self.npc_feero_ironhand_quest_accept(player, npc, quest_id),
            "npc_kerlonian" => self.npc_kerlonian_quest_accept(player, npc, quest_id),
            "npc_prospector_remtravel" => {
                self.npc_prospector_remtravel_quest_accept(player, npc, quest_id);
            }
            "npc_therylune" => self.npc_therylune_quest_accept(player, npc, quest_id),
            "npc_volcor" => self.npc_volcor_quest_accept(player, npc, quest_id),
            "npc_melizza_brimbuzzle" => {
                self.npc_melizza_brimbuzzle_quest_accept(player, npc, quest_id);
            }
            "npc_dalinda_malem" => self.npc_dalinda_malem_quest_accept(player, npc, quest_id),
            "npc_cork_gizelton" => self.npc_cork_gizelton_quest_accept(player, npc, quest_id),
            "npc_rigger_gizelton" => self.npc_rigger_gizelton_quest_accept(player, npc, quest_id),
            "celebras_spirit" => self.celebras_spirit_quest_accept(player, npc, quest_id),
            "npc_stinky_ignatz" => self.npc_stinky_ignatz_quest_accept(player, npc, quest_id),
            "npc_captured_arkonarin" => {
                self.npc_captured_arkonarin_quest_accept(player, npc, quest_id);
            }
            "npc_arei" => self.npc_arei_quest_accept(player, npc, quest_id),
            "npc_shay_leafrunner" => self.npc_shay_leafrunner_quest_accept(player, npc, quest_id),
            "npc_kindal_moonweaver" => {
                self.npc_kindal_moonweaver_quest_accept(player, npc, quest_id);
            }
            "npc_keeper_remulos" => self.keeper_remulos_quest_accept(player, npc, quest_id),
            "npc_piznik" => self.piznik_quest_accept(player, npc, quest_id),
            "npc_tooga" => self.tooga_quest_accept(player, npc, quest_id),
            "npc_mist" => self.mist_quest_accept(player, npc, quest_id),
            "npc_gilthares" => self.gilthares_quest_accept(player, npc, quest_id),
            "npc_wizzlecranks_shredder" => {
                self.wizzlecranks_shredder_quest_accept(player, npc, quest_id);
            }
            "npc_belnistrasz" => self.belnistrasz_quest_accept(player, npc, quest_id),
            "npc_willix_the_importer" => {
                self.willix_the_importer_quest_accept(player, npc, quest_id);
            }
            "npc_lakota_windsong" => self.lakota_windsong_quest_accept(player, npc, quest_id),
            "npc_paoka_swiftmountain" => {
                self.paoka_swiftmountain_quest_accept(player, npc, quest_id);
            }
            "npc_ame01" => self.ame01_quest_accept(player, npc, quest_id),
            "npc_ringo" => self.ringo_quest_accept(player, npc, quest_id),
            "npc_doctor" => self.npc_doctor_quest_accept(player, npc, quest_id),
            _ => {}
        }
        let _ = (player, npc, quest_id);
    }

    /// `pQuestRewardedNPC` dispatch (fork
    /// `OnQuestRewarded` at the end of
    /// `RewardQuest`).
    pub fn quest_script_on_rewarded(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        let Some(name) = self.gossip_script_name(npc) else {
            return;
        };
        match name.as_str() {
            "npc_AV_blood_collector" => {
                self.npc_av_blood_collector_quest_rewarded(player, npc, quest_id);
            }
            "npc_AV_troops_chief" => {
                self.npc_av_troops_chief_quest_rewarded(player, npc, quest_id);
            }
            "npc_sirra_vonindi" => self.npc_sirra_vonindi_quest_rewarded(player, npc, quest_id),
            "npc_witch_doctor_unbagwa" => {
                self.npc_witch_doctor_unbagwa_quest_rewarded(player, npc, quest_id);
            }
            "npc_mistress_nagmara" => {
                self.npc_mistress_nagmara_quest_rewarded(player, npc, quest_id);
            }
            "npc_rocknot" => self.npc_rocknot_quest_rewarded(player, npc, quest_id),
            "npc_knot_thimblejack" => {
                self.npc_knot_thimblejack_quest_rewarded(player, npc, quest_id);
            }
            "npc_terenthis" => self.npc_terenthis_quest_rewarded(player, npc, quest_id),
            "npc_archmage_tervosh" => {
                self.npc_archmage_tervosh_quest_rewarded(player, npc, quest_id);
            }
            "npc_yehkinya" => self.yehkinya_quest_rewarded(player, npc, quest_id),
            "npc_treshala_fallowbrook" => {
                self.treshala_fallowbrook_quest_rewarded(player, npc, quest_id);
            }
            "npc_Geologist_Larksbane" => {
                self.geologist_larksbane_quest_rewarded(player, npc, quest_id);
            }
            "npc_riggle_bassbait" => self.npc_riggle_bassbait_quest_rewarded(player, npc, quest_id),
            "npc_kwee_peddlefeet" => self.npc_kwee_peddlefeet_quest_rewarded(player, npc, quest_id),
            "npc_AQwar_collector" => self.npc_aqwar_collector_quest_rewarded(player, npc, quest_id),
            _ => {}
        }
        let _ = (player, npc, quest_id);
    }

    /// `GossipHello_npc_res_fixer`
    /// (`npcs_special.cpp:2105-2110`): one chat item
    /// (broadcast text 8995), menu text 6440.
    /// `GossipHello_npc_spirit_guide`
    /// (`battleground.cpp:103-107`): the player
    /// self-casts `SPELL_WAITING_TO_RESURRECT` 2584
    /// (opt-in for the 30 s spirit-heal wave). The
    /// fork sends no menu (returns true); the
    /// default-text empty menu is the host's
    /// closest shape (klinfran precedent).
    fn npc_spirit_guide_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> GossipMenuData {
        let _ = self.cast_spell(
            player,
            2584,
            crate::spells::CastTarget::SelfCast,
            crate::spells::CastOptions {
                triggered: true,
                skip_known: true,
                ..Default::default()
            },
        );
        GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: vec![],
        }
    }

    fn res_fixer_on_gossip_hello(&mut self, player: ObjectGuid) -> GossipMenuData {
        GossipMenuData {
            text_id: 6440,
            items: vec![GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(8995, player),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF + 1,
            }],
        }
    }

    /// `GossipSelect_npc_res_fixer`
    /// (`npcs_special.cpp:2112-2123`): remove the
    /// race resurrection sickness aura, repair all
    /// durability free, close. Any other action
    /// also returns handled (fork returns true).
    fn res_fixer_on_gossip_select(
        &mut self,
        player: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> GossipSelectOutcome {
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            let race = self.unit_base(player).map_or(0, |u| u32::from(u.race()));
            let spell_id = self
                .stores
                .player_infos
                .res_sickness_spell_id(race)
                .unwrap_or(SPELL_PASSIVE_RESURRECTION_SICKNESS);
            self.remove_auras_due_to_spell(player, spell_id);
            self.durability_repair_all(player);
        }
        GossipSelectOutcome::Close
    }

    /// `Player::DurabilityRepairAll` subset: every
    /// inventory item with a max durability is
    /// restored to full (no cost handling — the
    /// res fixer passes 0 discount/free).
    fn durability_repair_all(&mut self, player: ObjectGuid) {
        let entries: Vec<u32> = self
            .player(player)
            .map(|p| {
                p.saved
                    .inventory
                    .iter()
                    .map(|inv| inv.item.item_id)
                    .collect()
            })
            .unwrap_or_default();
        let fixes: Vec<u32> = entries
            .iter()
            .map(|entry| {
                self.stores
                    .item_prototypes
                    .get(*entry)
                    .map_or(0, |p| p.max_durability)
            })
            .collect();
        if let Some(p) = self.player_mut(player) {
            for (inv, max) in p.saved.inventory.iter_mut().zip(fixes) {
                if max > 0 {
                    inv.item.durability = u16::try_from(max).unwrap_or(u16::MAX);
                }
            }
        }
    }

    /// Keeps `CastTarget` referenced for lanes that
    /// cast from gossip selects.
    #[allow(dead_code)]
    fn _cast_target_use(_: CastTarget) {}

    // ---- lane gq_ek_a.rs ----
    /// `QuestAccept_npc_professor_phizzlethorpe` (`arathi_highlands.cpp:102-113`):
    /// quest 665 (Sunken Treasure) says text 845, then starts the escort.
    fn npc_professor_phizzlethorpe_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_SUNKEN_TREASURE: u32 = 665;
        const SAY_PROGRESS_1: i32 = 845;
        if quest_id == QUEST_SUNKEN_TREASURE {
            self.do_script_text(SAY_PROGRESS_1, npc, Some(player), -1);
            // UNMODELED: `pCreature->SetFactionTemplateId(FACTION_ESCORT_N_NEUTRAL_PASSIVE 113)` (no faction helper on Map).
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    /// `QuestAccept_npc_shakes_o_breen` (`arathi_highlands.cpp:275-289`): quest
    /// 667 (Death from Below) monster-yells, then starts the paused escort.
    fn npc_shakes_o_breen_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_DEATH_FROM_BELOW: u32 = 667;
        const BREEN_YELL_1: i32 = 6372;
        let _ = player;
        if quest_id == QUEST_DEATH_FROM_BELOW {
            // Fork `MonsterYell`; numeric id transcribed via `do_script_text`.
            self.do_script_text(BREEN_YELL_1, npc, None, -1);
            // UNMODELED: `pCreature->SetOrientation(2.67)` + `npc_escortAI::Start` + `SetEscortPaused(true)` escort (no escort framework on Map).
        }
    }

    /// `QuestAccept_npc_kinelory` (`arathi_highlands.cpp:411-423`): quest 660
    /// (Hints of a New Plague) says text 816, then starts the escort.
    fn npc_kinelory_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_HINTS_NEW_PLAGUE: u32 = 660;
        const SAY_START: i32 = 816;
        let _ = player;
        if quest_id == QUEST_HINTS_NEW_PLAGUE {
            self.do_script_text(SAY_START, npc, None, -1);
            // UNMODELED: `RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC)` (no unit-flags helper on Map).
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    /// `QuestAccept_npc_grark_lorkrub` (`burning_steppes.cpp:365-376`): quest
    /// 4121 (Precarious Predicament) starts the escort; nothing else modelable.
    fn npc_grark_lorkrub_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_PRECARIOUS_PREDICAMENT: u32 = 4121;
        let _ = (player, npc);
        if quest_id == QUEST_PRECARIOUS_PREDICAMENT {
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
        // Fork returns false when the quest does not match (declines the escort);
        // the host accept dispatch returns ().
    }

    /// `GossipHello_npc_klinfran` (`burning_steppes.cpp:617-624`): offers the
    /// reveal item while quest 7636 is incomplete, then sends the creature's
    /// default gossip text; fork always returns true.
    fn npc_klinfran_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_ICON_CHAT: u8 = 0; // `GossipDef.h:60`.
        const GOSSIP_SENDER_MAIN: u32 = 1; // `ScriptedGossip.h:55`.
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000; // `ScriptedGossip.h:53`.
        const QUEST_STAVE_OF_THE_ANCIENTS: u32 = 7636;
        let mut items = Vec::new();
        if self.player_quest_incomplete(player, QUEST_STAVE_OF_THE_ANCIENTS) {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: "Show me your real face, demon.".to_string(),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF,
            });
        }
        // Fork `pPlayer->GetGossipTextId(pCreature)`: the creature's default menu id.
        let text_id = self.creature(npc).map_or(0, |c| c.gossip_menu_id);
        Some(GossipMenuData { text_id, items })
    }

    /// `GossipSelect_npc_klinfran` (`burning_steppes.cpp:626-631`): closes the
    /// menu and begins the Franklin->Klinfran transform event.
    fn npc_klinfran_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        let _ = (player, npc, sender, action);
        // UNMODELED: `npc_klinfranAI::BeginEvent(player)` (transform scheduling + UNIT_NPC_FLAGS clear; no event-AI framework on Map).
        Some(GossipSelectOutcome::Close)
    }

    /// `QuestAccept_npc_eris_havenfire` (`eastern_plaguelands.cpp:572-582`):
    /// quest 7622 (Balance of Light and Shadow) begins the chapel event.
    fn npc_eris_havenfire_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_BALANCE_OF_LIGHT: u32 = 7622;
        let _ = (player, npc);
        if quest_id == QUEST_BALANCE_OF_LIGHT {
            // UNMODELED: `npc_eris_havenfireAI::BeginEvent(player)` (archer/peasant wave event AI; no event-AI framework on Map).
        }
    }

    /// `GossipHello_npc_joseph_redpath` (`eastern_plaguelands.cpp:1679-1691`):
    /// menu 3861; on incomplete quest 5721 grants the Joseph credit, bows, and
    /// starts the reunion walk. Fork always returns true.
    fn npc_joseph_redpath_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const QUEST_BATTLE_DARROWSHIRE: u32 = 5721;
        const NPC_JOSEPH_REDPATH: u32 = 10936;
        const EMOTE_ONESHOT_BEG: u32 = 20; // `SharedDefines.h:489`.
        let _ = NPC_JOSEPH_REDPATH;
        if self.player_quest_incomplete(player, QUEST_BATTLE_DARROWSHIRE) {
            // UNMODELED: `KilledMonsterCredit(NPC_JOSEPH_REDPATH 10936, guid)` (no quest-credit helper on Map).
            self.handle_emote_command(npc, EMOTE_ONESHOT_BEG);
            // UNMODELED: `npc_joseph_redpathAI::BeginEvent()` (waypoint reunion event; no escort framework on Map).
        }
        Some(GossipMenuData {
            text_id: 3861,
            items: Vec::new(),
        })
    }

    /// `QuestAccept_npc_rinji` (`hinterlands.cpp:195-209`): quest 2742 (Rin'ji
    /// is Trapped) opens the cage and starts the escort.
    fn npc_rinji_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_RINJI_TRAPPED: u32 = 2742;
        const GO_RINJI_CAGE: u32 = 142036;
        const FACTION_ESCORTEE: u32 = 33;
        let _ = (player, npc, GO_RINJI_CAGE, FACTION_ESCORTEE);
        if quest_id == QUEST_RINJI_TRAPPED {
            // UNMODELED: `GetClosestGameObjectWithEntry(GO_RINJI_CAGE 142036)->UseDoorOrButton()` (no GO-state helper for this gate on Map).
            // UNMODELED: `SetFactionTemporary(FACTION_ESCORTEE 33, RESTORE_RESPAWN)` + `RemoveFlag(UNIT_FLAG_IMMUNE_TO_NPC)` (no faction/flag helpers on Map).
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    /// `QuestAccept_npc_miran` (`loch_modan.cpp:99-107`): quest 309
    /// (Protecting the Shipment) starts the escort; nothing else modelable.
    fn npc_miran_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_PROTECTING_THE_SHIPMENT: u32 = 309;
        let _ = (player, npc);
        if quest_id == QUEST_PROTECTING_THE_SHIPMENT {
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    /// `QuestAccept_npc_corporal_keeshan` (`redridge_mountains.cpp:124-136`):
    /// quest 219 (Missing in Action) says text 25, then starts the escort.
    fn npc_corporal_keeshan_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_MISSING_IN_ACTION: u32 = 219;
        const FACTION_ESCORTEE: u32 = 10;
        const SAY_CORPORAL_KEESHAN_1: i32 = 25;
        let _ = FACTION_ESCORTEE;
        if quest_id == QUEST_MISSING_IN_ACTION {
            self.do_script_text(SAY_CORPORAL_KEESHAN_1, npc, Some(player), -1);
            // UNMODELED: `SetFactionTemporary(FACTION_ESCORTEE 10, RESTORE_RESPAWN)` (no faction helper on Map).
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    /// `GossipHello_npc_dying_archaeologist` (`searing_gorge.cpp:186-194`):
    /// silent (fork false) while Obsidion is near; otherwise the default message
    /// plus the quest 3566 menu. Fork returns true in the menu arm.
    fn npc_dying_archaeologist_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const NPC_OBSIDION: u32 = 8400;
        const VISIBLE_RANGE: f32 = 166.0; // `ScriptMgr.h:70`.
        const DEFAULT_GOSSIP_MESSAGE: u32 = 0xffffff; // `GossipDef.h:33`.
        const QUEST_RISE_OBSIDION: u32 = 3566;
        let _ = (player, QUEST_RISE_OBSIDION);
        // Fork `GetClosestCreatureWithEntry(pCreature, NPC_OBSIDION, VISIBLE_RANGE)`; `None` is fork `false`.
        if self
            .nearest_creature_with_entry(npc, NPC_OBSIDION, VISIBLE_RANGE)
            .is_some()
        {
            return None;
        }
        // UNMODELED: `PrepareQuestMenu(QUEST_RISE_OBSIDION 3566)` (quest items have no menu model on Map).
        Some(GossipMenuData {
            text_id: DEFAULT_GOSSIP_MESSAGE,
            items: Vec::new(),
        })
    }

    /// `QuestAccept_npc_dying_archaeologist` (`searing_gorge.cpp:196-219`):
    /// quest 3566 (Rise, Obsidion!) wakes Obsidion via its event AI.
    fn npc_dying_archaeologist_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_RISE_OBSIDION: u32 = 3566;
        const NPC_OBSIDION: u32 = 8400;
        let _ = (player, npc, NPC_OBSIDION);
        if quest_id == QUEST_RISE_OBSIDION {
            // UNMODELED: `GetClosestCreatureWithEntry(NPC_OBSIDION 8400, VISIBLE_RANGE 166.0)` + `npc_obsidionAI::StartEvent` + escort player-list push (no event-AI framework on Map).
        }
        // Fork returns false except when Obsidion is absent (quest proceeds without
        // starting); the host accept dispatch returns ().
    }

    /// `QuestAccept_npc_deathstalker_erland` (`silverpine_forest.cpp:164-175`):
    /// quest 435 says text 481, then starts the escort.
    fn npc_deathstalker_erland_quest_accept(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_ERLAND: u32 = 435;
        const FACTION_ESCORTEE: u32 = 232;
        const SAY_START_1: i32 = 481;
        let _ = FACTION_ESCORTEE;
        if quest_id == QUEST_ERLAND {
            self.do_script_text(SAY_START_1, npc, None, -1);
            // UNMODELED: `SetFactionTemporary(FACTION_ESCORTEE 232, RESTORE_RESPAWN)` (no faction helper on Map).
            // UNMODELED: `npc_escortAI::Start` escort (no escort framework on Map).
        }
    }

    // ---- lane gq_ek_b.rs ----
    /// `GossipHello_npc_squire_rowe` (`quest_stormwind_rendezvous.cpp:937-960`):
    /// ready item (chat icon, literal text, sender 1, action 1000) plus menu
    /// 9065; busy menu 9064; completed menu 9066; default menu 9063.
    fn npc_squire_rowe_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        let rendezvous = self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
            p.quest_status(EKB_QUEST_STORMWIND_RENDEZVOUS)
        });
        let masquerade = self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
            p.quest_status(EKB_QUEST_THE_GREAT_MASQUERADE)
        });
        if rendezvous == EKB_QUEST_STATUS_COMPLETE && masquerade != EKB_QUEST_STATUS_COMPLETE {
            // UNMODELED: npc_squire_roweAI::m_bWindsorUp (AI state, cpp:946);
            // assume Windsor not yet up, offer the ready item.
            return Some(GossipMenuData {
                text_id: EKB_GOSSIP_ROWE_READY,
                items: vec![GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Let Marshal Windsor know that I am ready.".to_string(),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF,
                }],
            });
        }
        if masquerade == EKB_QUEST_STATUS_COMPLETE {
            return Some(GossipMenuData {
                text_id: EKB_GOSSIP_ROWE_COMPLETED,
                items: vec![],
            });
        }
        Some(GossipMenuData {
            text_id: EKB_GOSSIP_ROWE_NOTHING,
            items: vec![],
        })
    }

    /// `GossipSelect_npc_squire_rowe` (`quest_stormwind_rendezvous.cpp:962-983`):
    /// action 1000 starts the Windsor event, then closes.
    fn npc_squire_rowe_gossip_select(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        if action == GOSSIP_ACTION_INFO_DEF {
            if self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
                p.quest_status(EKB_QUEST_STORMWIND_RENDEZVOUS)
            }) == EKB_QUEST_STATUS_COMPLETE
            {
                // UNMODELED: npc_squire_roweAI::m_bEventProcessed /
                // m_playerGuid, walk-off and gossip-flag clear (cpp:969-974).
            }
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true with no packet for other actions; nothing to send.
        None
    }

    /// `GossipHello_npc_reginald_windsor`
    /// (`quest_stormwind_rendezvous.cpp:804-816`): gossip item 8256 plus menu
    /// 5633; quest list comes from `PrepareQuestMenu`.
    fn npc_reginald_windsor_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        // UNMODELED: npc_reginald_windsorAI::playerGUID/QuestAccepted (AI
        // state, cpp:808); proxy is a taken-but-unrewarded masquerade quest.
        let masquerade = self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
            p.quest_status(EKB_QUEST_THE_GREAT_MASQUERADE)
        });
        let mut items = Vec::new();
        if masquerade != EKB_QUEST_STATUS_NONE {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(EKB_WINDSOR_GOSSIP_ITEM_TEXT, player),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF,
            });
        }
        // UNMODELED: PrepareQuestMenu quest list (cpp:811); the core
        // quest-giver path owns it, not the gossip menu.
        Some(GossipMenuData {
            text_id: EKB_WINDSOR_GOSSIP_TEXT,
            items,
        })
    }

    /// `GossipSelect_npc_reginald_windsor`
    /// (`quest_stormwind_rendezvous.cpp:818-833`): action 1000 begins the
    /// escort dialogue, then closes.
    fn npc_reginald_windsor_gossip_select(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        if action == GOSSIP_ACTION_INFO_DEF {
            // UNMODELED: npc_reginald_windsorAI::BeginQuest = true,
            // DoScriptText(SAY_WINDSOR12 = 8208), UNIT_NPC_FLAGS := 0
            // (cpp:823-828).
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true with no packet for other actions; nothing to send.
        None
    }

    /// `QuestAccept_npc_reginald_windsor`
    /// (`quest_stormwind_rendezvous.cpp:787-802`): arms the Windsor event on
    /// quest 6403 accept.
    fn npc_reginald_windsor_quest_accept(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_THE_GREAT_MASQUERADE {
            // UNMODELED: npc_reginald_windsorAI BeginQuest/QuestAccepted/
            // playerGUID/despawn-timer arming and questgiver-flag clear
            // (cpp:791-799); all AI/npc-flag state.
        }
        let _ = EKB_SAY_WINDSOR12;
    }

    /// `QuestAccept_npc_bartleby` (`stormwind_city.cpp:83-94`): quest 1640
    /// turns Bartleby hostile and starts the beating.
    fn npc_bartleby_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        if quest_id == EKB_QUEST_BEAT {
            if let Some(c) = self.creature_mut(npc) {
                c.unit.set_faction_template(EKB_FACTION_ENEMY);
            }
            self.attack_start(npc, player);
        }
    }

    /// `QuestAccept_npc_dashel_stonefist` (`stormwind_city.cpp:419-465`):
    /// quest 1447 starts the brawl that ends the Missing Diplomat chain.
    fn npc_dashel_stonefist_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_MISSING_DIPLO_PT8 {
            // UNMODELED: DoScriptText(SAY_PROGRESS_1_DAS = 1961) (cpp:431);
            // no creature-speech helper on Map.
            if let Some(c) = self.creature_mut(npc) {
                c.unit.set_faction_template(EKB_FACTION_NEUTRAL);
            }
            // UNMODELED: RemoveFlag(UNIT_NPC_FLAG_QUESTGIVER) (cpp:434); no
            // npc-flag helper on Map.
            // UNMODELED: summon 2x NPC_OLD_TOWN_THUG (4969) at
            // (-8676.075195, 443.744019, 99.632210, 3.981758) and
            // (-8685.416992, 443.130829, 99.526917, 5.759635), thug
            // AttackStarts, and quest-fight AI arming (cpp:439-459); no
            // summon helper on Map.
            let _ = EKB_NPC_OLD_TOWN_THUG;
            self.attack_start(npc, player);
        }
    }

    /// `QuestAccept_npc_galen_goodward`
    /// (`swamp_of_sorrows.cpp:132-146`): quest 1393 starts Galen's escort.
    fn npc_galen_goodward_quest_accept(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_GALENS_ESCAPE {
            // UNMODELED: escort Start(false, player, quest) (cpp:139); escort
            // control lives in AI, not on Map.
            // UNMODELED: SetFactionTemporary(495, TEMPFACTION_RESTORE_RESPAWN)
            // (cpp:141); no temporary-faction helper on Map.
            // UNMODELED: DoScriptText(SAY_QUEST_ACCEPTED = 1854) (cpp:142);
            // no creature-speech helper on Map.
        }
    }

    /// `QuestAccept_npc_daphne_stilwell` (`westfall.cpp:168-179`): quest 1651
    /// starts Daphne's escort.
    fn npc_daphne_stilwell_quest_accept(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_TOME_VALOR {
            // UNMODELED: DoScriptText(SAY_DS_START = 2360) (cpp:172); no
            // creature-speech helper on Map.
            // UNMODELED: escort Start(true, player, quest, true) (cpp:175);
            // escort control lives in AI, not on Map.
        }
    }

    /// `QuestAccept_npc_mikhail` (`wetlands.cpp:417-445`): quest 1249 hands
    /// the escort to Tapoke Slim Jahn.
    fn npc_mikhail_quest_accept(&mut self, _player: ObjectGuid, _npc: ObjectGuid, quest_id: u32) {
        if quest_id == EKB_QUEST_MISSING_DIPLOMAT_PART11 {
            // UNMODELED: Tapoke Slim Jahn lookup (20.0f), friend despawn, and
            // escort Start(false, player, quest) (cpp:425-440); creature
            // proximity/escort control lives in AI, not on Map. (Fork returns
            // false when Slim is missing; unobservable in the () dispatch.)
        }
    }

    /// `GossipHello_npc_mikhail` (`wetlands.cpp:447-506`): Missing Diplomat
    /// gate; every branch ends in a plain menu or the core quest list.
    fn npc_mikhail_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        let part10 = self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
            p.quest_status(EKB_QUEST_MISSING_DIPLOMAT_PART10)
        });
        let part11 = self.player(player).map_or(EKB_QUEST_STATUS_NONE, |p| {
            p.quest_status(EKB_QUEST_MISSING_DIPLOMAT_PART11)
        });
        if part10 == EKB_QUEST_STATUS_COMPLETE && part11 == EKB_QUEST_STATUS_NONE {
            // UNMODELED: Tapoke Slim Jahn proximity/alive/escorting check
            // (cpp:458-489); without it fall through to the core quest path.
            // Fork shows npc_text 1713 when Slim is missing or busy.
            let _ = EKB_MIKHAIL_FALLBACK_TEXT;
            return None;
        }
        // UNMODELED: PrepareQuestMenu/SendPreparedQuest (cpp:480-481,
        // 494-495, 501-502) are core quest-giver packets, not gossip menus.
        None
    }

    /// `QuestRewarded_npc_sirra_vonindi` (`duskwood.cpp:665-674`): quest 401
    /// reward launches the Stitches event.
    fn npc_sirra_vonindi_quest_rewarded(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_WAIT_FOR_SIRRA_TO_FINISH {
            // UNMODELED: npc_sirra_vonindiAI::SummonStitches (cpp:627-646):
            // SummonCreature NPC_STITCHES (412) at (-10277.63, 54.27, 42.2,
            // 4.22), scale 2.0, launch via town crier (NPC 468, 400.0f); no
            // summon helper on Map. (Fork returns !SummonStitches();
            // unobservable in the () dispatch.)
            let _ = (EKB_NPC_STITCHES, EKB_NPC_TOWN_CRIER);
        }
    }

    /// `QuestRewarded_npc_witch_doctor_unbagwa`
    /// (`stranglethorn_vale.cpp:320-329`): quest 349 starts Unbagwa's event.
    fn npc_witch_doctor_unbagwa_quest_rewarded(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if quest_id == EKB_QUEST_STRANHLETHORN_FEVER {
            // UNMODELED: npc_witch_doctor_unbagwaAI::StartEvent (cpp:325);
            // event state lives in AI, not on Map.
        }
    }

    /// Keeps `EKB_QUEST_STATUS_INCOMPLETE` referenced for lanes whose proxy
    /// checks only use NONE/COMPLETE today.
    #[allow(dead_code)]
    fn gq_ek_b_status_use(&self, status: i32) -> bool {
        status == EKB_QUEST_STATUS_INCOMPLETE
    }

    // ---- lane gq_instances.rs ----

    /// `GossipHello_npc_mistress_nagmara` (`blackrock_depths.cpp:820-834`): quest
    /// menu plus the love-potion item (broadcast 5040) when quest 4201 is
    /// rewarded, else the plain menu. Fork always returns true.
    fn npc_mistress_nagmara_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_QUEST_POTION_LOVE: u32 = 4201; // `QUEST_POTION_LOVE`.
        const GQI_BCT_NAGMARA_ITEM: u32 = 5040; // `GOSSIP_ITEM_NAGMARA` (broadcast text).
        const GQI_MENU_NAGMARA: u32 = 2727; // `GOSSIP_ID_NAGMARA`.
        const GQI_MENU_NAGMARA_REWARDED: u32 = 2729; // `GOSSIP_ID_NAGMARA_2`.
        let _ = npc;
        // UNMODELED: `PrepareQuestMenu` when the creature is a quest giver (quest items have no menu model on Map).
        let rewarded = self.player(player).is_some_and(|p| {
            p.quest_row(GQI_QUEST_POTION_LOVE)
                .is_some_and(|r| r.rewarded != 0)
        });
        let mut items = Vec::new();
        let text_id = if rewarded {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_NAGMARA_ITEM, player),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF + 1,
            });
            GQI_MENU_NAGMARA_REWARDED
        } else {
            GQI_MENU_NAGMARA
        };
        Some(GossipMenuData { text_id, items })
    }

    /// `GossipSelect_npc_mistress_nagmara` (`blackrock_depths.cpp:836-847`):
    /// action 1001 closes the menu and starts Nagmara's Potion of Love walk.
    fn npc_mistress_nagmara_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        let _ = (player, npc, sender);
        if action != GOSSIP_ACTION_INFO_DEF + 1 {
            // Fork returns true with no packet for other actions (unrepresentable outcome).
            return None;
        }
        // UNMODELED: `npc_mistress_nagmaraAI::DoPotionOfLoveIfCan` (flag clears + follow walk; event AI lives outside this lane).
        Some(GossipSelectOutcome::Close)
    }

    /// `QuestRewarded_npc_mistress_nagmara` (`blackrock_depths.cpp:849-863`):
    /// quest 4201 (Love Potion) starts Nagmara's Potion of Love walk.
    /// Fork always returns true.
    fn npc_mistress_nagmara_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const GQI_QUEST_POTION_LOVE: u32 = 4201; // `QUEST_POTION_LOVE`.
        let _ = (player, npc);
        if quest_id == GQI_QUEST_POTION_LOVE {
            // UNMODELED: `npc_mistress_nagmaraAI::DoPotionOfLoveIfCan` (see select arm).
        }
    }

    /// `QuestRewarded_npc_rocknot` (`blackrock_depths.cpp:1091-1118`): quest 4295
    /// (Ale) says text 5172 and moves the Rocknot bar state to SPECIAL.
    /// Fork always returns true.
    fn npc_rocknot_quest_rewarded(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const GQI_QUEST_ALE: u32 = 4295; // `QUEST_ALE`.
        const GQI_SAY_GOT_BEER: i32 = 5172; // `SAY_GOT_BEER`.
        let _ = player;
        if self.brd_get_data(BRD_TYPE_ROCKNOT) == Some(DONE)
            || self.brd_get_data(BRD_TYPE_ROCKNOT) == Some(BRD_SPECIAL)
        {
            return;
        }
        if quest_id != GQI_QUEST_ALE {
            return;
        }
        if self.brd_get_data(BRD_TYPE_ROCKNOT) != Some(IN_PROGRESS) {
            self.brd_set_data(BRD_TYPE_ROCKNOT, IN_PROGRESS);
        }
        // UNMODELED: `SetFacingToObject(pPlayer)` (no facing helper on Map).
        self.do_script_text(GQI_SAY_GOT_BEER, npc, None, -1);
        // UNMODELED: `npc_rocknotAI::m_uiEmoteTimer = 1500` (cheer/event AI timing lives outside this lane).
        self.brd_set_data(BRD_TYPE_ROCKNOT, BRD_SPECIAL);
    }

    /// `QuestAccept_npc_marshal_windsor` (`blackrock_depths.cpp:2172-2190`):
    /// quest 4322 (Jail Break!) starts the Windsor escort. Fork returns true.
    fn npc_marshal_windsor_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const GQI_QUEST_JAIL_BREAK: u32 = 4322; // `QUEST_JAIL_BREAK` (blackrock_depths.h:89).
        const GQI_FACTION_WINDSOR: u32 = 11; // `SetFactionTemplateId(11)` (cpp 2181, literal).
        let _ = player;
        if quest_id != GQI_QUEST_JAIL_BREAK {
            return;
        }
        // Fork reads the player's instance data; this Map is that instance.
        if self.brd_get_data(BRD_TYPE_QUEST_JAIL_BREAK) != Some(BRD_NOT_STARTED) {
            return;
        }
        self.brd_set_data(BRD_TYPE_QUEST_JAIL_BREAK, IN_PROGRESS);
        let _ = self.object_set_field(
            npc,
            u32::from(offsets::UNIT_FIELD_FACTIONTEMPLATE),
            GQI_FACTION_WINDSOR,
        );
        // UNMODELED: `npc_marshal_windsorAI::Start(false, player, quest)` escort (no escort framework on Map).
    }

    /// `GossipHello_boss_vael` (`boss_vaelastrasz.cpp:455-473`): Razorgore must be
    /// DONE; offers the first Vaelastrasz item (broadcast 9847), menu 7156.
    /// Fork returns false without an instance or while Razorgore lives.
    fn boss_vael_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_BCT_VAEL_ITEM_1: u32 = 9847; // `GOSSIP_ITEM_VAEL_1` (broadcast text).
        const GQI_TEXT_VAEL_1: u32 = 7156; // `GOSSIP_TEXT_VAEL_1`.
        let _ = npc;
        // Fork exempts game masters from the Razorgore gate; no GM helper exists on Map.
        if self.bwl_get_data(BWL_TYPE_RAZORGORE) != Some(DONE) {
            return None;
        }
        // UNMODELED: `PrepareQuestMenu` when the creature is a quest giver and `TYPE_SCEPTER_RUN == NOT_STARTED` (quest items have no menu model on Map).
        let items = vec![GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: self.gossip_broadcast_text(GQI_BCT_VAEL_ITEM_1, player),
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF + 1,
        }];
        Some(GossipMenuData {
            text_id: GQI_TEXT_VAEL_1,
            items,
        })
    }

    /// `GossipSelect_boss_vael` (`boss_vaelastrasz.cpp:433-453`): action 1001
    /// shows the second menu (7256); action 1002 closes and starts the speech.
    fn boss_vael_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GQI_BCT_VAEL_ITEM_2: u32 = 10011; // `GOSSIP_ITEM_VAEL_2` (broadcast text).
        const GQI_TEXT_VAEL_2: u32 = 7256; // `GOSSIP_TEXT_VAEL_2`.
        let _ = (npc, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            let menu = GossipMenuData {
                text_id: GQI_TEXT_VAEL_2,
                items: vec![GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: self.gossip_broadcast_text(GQI_BCT_VAEL_ITEM_2, player),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 2,
                }],
            };
            return Some(GossipSelectOutcome::ShowMenu(menu));
        }
        if action == GOSSIP_ACTION_INFO_DEF + 2 {
            // UNMODELED: `boss_vaelAI::BeginSpeech(player)` (speech/aggro scheduling; event AI lives outside this lane).
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true with no packet for other actions (unrepresentable outcome).
        None
    }

    /// `QuestAccept_vaelastrasz` (`boss_vaelastrasz.cpp:475-501`): quest 8730
    /// (Nefarius Corruption) starts the scepter run for a single champion.
    /// Fork returns false without an instance or for other quests.
    fn vaelastrasz_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const GQI_QUEST_NEFARIUS_CORRUPTION: u32 = 8730; // `QUEST_NEFARIUS_CORRUPTION` (blackwing_lair.h:93).
        let _ = npc;
        if quest_id != GQI_QUEST_NEFARIUS_CORRUPTION {
            return;
        }
        let Some(run) = self.bwl_get_data(BWL_TYPE_SCEPTER_RUN) else {
            return;
        };
        if run != BWL_NOT_STARTED {
            // UNMODELED: `FailQuest(QUEST_NEFARIUS_CORRUPTION)` (no quest-fail helper on Map).
            return;
        }
        self.bwl_set_data(BWL_TYPE_SCEPTER_RUN, BWL_SPECIAL);
        // Fork stores the champion's full guid; the data slot is u32 so the low bits ride (counter-only player raws round-trip).
        self.bwl_set_data(BWL_DATA_SCEPTER_CHAMPION, player.raw() as u32);
        // UNMODELED: `BindToInstanceOrRaid(player, respawn, true)` permanent bind (no binding helper on Map).
    }

    /// `GossipHello_npc_blastmaster_emi_shortfuse` (`gnomeregan.cpp:609-620`):
    /// offers the start item (broadcast 4084) while Grubbis is `NOT_STARTED` or
    /// FAIL; menu text is the creature's default gossip text.
    /// Fork always returns true.
    fn npc_blastmaster_emi_shortfuse_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_NOT_STARTED: u32 = 0; // `EncounterState::NOT_STARTED` (no GNO_ variant in content.rs).
        const GQI_BCT_SHORTFUSE_START: u32 = 4084; // `GOSSIP_ITEM_START` (broadcast text).
        let grubbis = self.gno_get_data(GNO_TYPE_GRUBBIS);
        let mut items = Vec::new();
        if grubbis == Some(GQI_NOT_STARTED) || grubbis == Some(GNO_FAIL) {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_SHORTFUSE_START, player),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF + 1,
            });
        }
        // Fork `pPlayer->GetGossipTextId(pCreature)`: DB gossip_menu_id of the creature.
        let text_id = self.creature(npc).map_or(0, |c| c.gossip_menu_id);
        Some(GossipMenuData { text_id, items })
    }

    /// `GossipSelect_npc_blastmaster_emi_shortfuse` (`gnomeregan.cpp:622-638`):
    /// action 1001 starts the Grubbis event while Grubbis is `NOT_STARTED` or FAIL,
    /// then always closes. Fork always returns true.
    fn npc_blastmaster_emi_shortfuse_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GQI_NOT_STARTED: u32 = 0; // `EncounterState::NOT_STARTED` (no GNO_ variant in content.rs).
        let _ = (player, npc, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            let grubbis = self.gno_get_data(GNO_TYPE_GRUBBIS);
            if grubbis == Some(GQI_NOT_STARTED) || grubbis == Some(GNO_FAIL) {
                // UNMODELED: `npc_blastmaster_emi_shortfuseAI::StartEvent(player)` escort (no escort framework on Map).
            }
        }
        Some(GossipSelectOutcome::Close)
    }

    /// `QuestAccept_npc_kernobee` (`gnomeregan.cpp:866-874`): quest 2904
    /// (A Fine Mess) starts Kernobee's follow. Fork always returns true.
    fn npc_kernobee_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const GQI_QUEST_A_FINE_MESS: u32 = 2904; // `QUEST_A_FINE_MESS`.
        let _ = (player, npc);
        if quest_id == GQI_QUEST_A_FINE_MESS {
            // UNMODELED: `npc_kernobeeAI::StartQuest(player, quest)` follower escort (no escort framework on Map).
        }
    }

    /// `GossipHello_npc_MasterCraftsmanOmarion`
    /// (`instance_naxxramas.cpp:1960-2005`): profession items (tailoring 197 /
    /// blacksmithing 164 / leatherworking 165 at 225+) plus the no-craftsman
    /// item, menu 8507, then Omarion laughs. Fork always returns true.
    fn npc_master_craftsman_omarion_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_SKILL_TAILORING: u32 = 197; // `SKILL_TAILORING` (SharedDefines.h:1001).
        const GQI_SKILL_BLACKSMITHING: u32 = 164; // `SKILL_BLACKSMITHING` (SharedDefines.h:988).
        const GQI_SKILL_LEATHERWORKING: u32 = 165; // `SKILL_LEATHERWORKING` (SharedDefines.h:989).
        const GQI_PROFESSION_GATE: u32 = 225;
        const GQI_BCT_TAILOR: u32 = 12251; // `BC_TAILOR_TEXT` (broadcast text).
        const GQI_BCT_BLACKSMITH: u32 = 12269; // `BC_BLACKSMITH_TEXT` (broadcast text).
        const GQI_BCT_LEATHERWORKER: u32 = 12257; // `BC_LEATHERWORKER_TEXT` (broadcast text).
        const GQI_BCT_NO_CRAFT: u32 = 12279; // `BC_NO_CRAFT_TEXT` (broadcast text).
        const GQI_MENU_INTRO: u32 = 8507; // `GOSSIP_MENU_INTRO`.
        const GQI_SELECT_TAILOR: u32 = 1001; // `GOSSIP_SELECT_TAILOR`.
        const GQI_SELECT_BS: u32 = 1002; // `GOSSIP_SELECT_BS`.
        const GQI_SELECT_LW: u32 = 1003; // `GOSSIP_SELECT_LW`.
        const GQI_SELECT_NOCRAFT: u32 = 1004; // `GOSSIP_SELECT_NOCRAFT`.
        const GQI_EMOTE_ONESHOT_LAUGH: u32 = 11; // `EMOTE_ONESHOT_LAUGH` (SharedDefines.h:480).
        let tailor = self.player(player).map_or(0, |p| {
            p.deferred
                .skills
                .iter()
                .find(|s| s.skill == GQI_SKILL_TAILORING)
                .map_or(0, |s| s.value)
        });
        let blacksmith = self.player(player).map_or(0, |p| {
            p.deferred
                .skills
                .iter()
                .find(|s| s.skill == GQI_SKILL_BLACKSMITHING)
                .map_or(0, |s| s.value)
        });
        let leatherwork = self.player(player).map_or(0, |p| {
            p.deferred
                .skills
                .iter()
                .find(|s| s.skill == GQI_SKILL_LEATHERWORKING)
                .map_or(0, |s| s.value)
        });
        let mut items = Vec::new();
        if tailor >= GQI_PROFESSION_GATE {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_TAILOR, player),
                sender: GQI_SELECT_TAILOR,
                action: GQI_SELECT_TAILOR,
            });
        }
        if blacksmith >= GQI_PROFESSION_GATE {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_BLACKSMITH, player),
                sender: GQI_SELECT_BS,
                action: GQI_SELECT_BS,
            });
        }
        if leatherwork >= GQI_PROFESSION_GATE {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_LEATHERWORKER, player),
                sender: GQI_SELECT_LW,
                action: GQI_SELECT_LW,
            });
        }
        items.push(GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: self.gossip_broadcast_text(GQI_BCT_NO_CRAFT, player),
            sender: GOSSIP_SENDER_MAIN,
            action: GQI_SELECT_NOCRAFT,
        });
        self.handle_emote_command(npc, GQI_EMOTE_ONESHOT_LAUGH);
        Some(GossipMenuData {
            text_id: GQI_MENU_INTRO,
            items,
        })
    }

    /// `GossipSelect_npc_MasterCraftsmanOmarion`
    /// (`instance_naxxramas.cpp:1798-1958`): Argent Dawn rank gates (revered 6 /
    /// exalted 7) pick the craft submenus; craft arms teach the pattern spell
    /// when the trade skill is 300+ and fall through to the crafter menu (8508).
    /// Fork always returns true.
    fn npc_master_craftsman_omarion_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GQI_SKILL_TAILORING: u32 = 197; // `SKILL_TAILORING` (SharedDefines.h:1001).
        const GQI_SKILL_BLACKSMITHING: u32 = 164; // `SKILL_BLACKSMITHING` (SharedDefines.h:988).
        const GQI_SKILL_LEATHERWORKING: u32 = 165; // `SKILL_LEATHERWORKING` (SharedDefines.h:989).
        const GQI_FACTION_ARGENT_DAWN: u32 = 529;
        const GQI_REP_REVERED: u32 = 6; // `REP_REVERED` (SharedDefines.h:123).
        const GQI_REP_EXALTED: u32 = 7; // `REP_EXALTED` (SharedDefines.h:124).
        const GQI_CRAFT_SKILL_GATE: u32 = 300;
        const GQI_BCT_CLOSE_CRAFTER: u32 = 12270; // `BC_CLOSE_CRAFTER` (broadcast text).
        const GQI_BCT_CLOSE_NO_CRAFTER: u32 = 12281; // `BC_CLOSE_NO_CRAFTER` (broadcast text).
        const GQI_MENU_CRAFTER: u32 = 8508; // `GOSSIP_MENU_CRAFTER`.
        const GQI_MENU_NOCRAFT: u32 = 8516; // `GOSSIP_MENU_NOCRAFT`.
        const GQI_SELECT_TAILOR: u32 = 1001;
        const GQI_SELECT_BS: u32 = 1002;
        const GQI_SELECT_LW: u32 = 1003;
        const GQI_SELECT_NOCRAFT: u32 = 1004;
        const GQI_CLOSE: u32 = 100; // `GOSSIP_CLOSE`.
        const GQI_POINTS_IN_RANK: [i32; 8] = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000]; // `POINTS_IN_RANK` (world_actor.rs:1923).
        const GQI_ITEM_ARCANE_TOME: u32 = 22719; // granted by the no-craftsman arm.
        let _ = npc;
        let skill_value = |skill: u32| {
            self.player(player).map_or(0, |p| {
                p.deferred
                    .skills
                    .iter()
                    .find(|s| s.skill == skill)
                    .map_or(0, |s| s.value)
            })
        };
        let tailor_skill = skill_value(GQI_SKILL_TAILORING);
        let blacksmith_skill = skill_value(GQI_SKILL_BLACKSMITHING);
        let leatherwork_skill = skill_value(GQI_SKILL_LEATHERWORKING);
        // `ReputationMgr::GetRank` for Argent Dawn: base + standing through
        // `ReputationToRank` (world_actor.rs `reputation_rank`).
        let standing = self.player(player).map_or(0, |p| {
            p.deferred
                .reputation
                .iter()
                .find(|r| r.faction == GQI_FACTION_ARGENT_DAWN)
                .map_or(0, |r| r.standing)
        });
        let (race_mask, class_mask) = self.unit_base(player).map_or((0, 0), |u| {
            (
                1u32 << u32::from(u.race()).saturating_sub(1),
                1u32 << u32::from(u.class()).saturating_sub(1),
            )
        });
        let base = self
            .stores
            .factions
            .get(GQI_FACTION_ARGENT_DAWN)
            .map_or(0, |e| {
                e.index_fit_to(race_mask, class_mask)
                    .map_or(0, |i| e.base_values[i])
            });
        let reputation = base + standing;
        let mut argent_dawn_rep = 0u32;
        let mut limit = 43_000i32; // Reputation_Cap + 1.
        for rank in (0..8u32).rev() {
            limit -= GQI_POINTS_IN_RANK[rank as usize];
            if reputation >= limit {
                argent_dawn_rep = rank;
                break;
            }
        }
        if action == GQI_CLOSE {
            return Some(GossipSelectOutcome::Close);
        }
        // Fork spits on the player below honored/revered, then closes; the emote has no helper.
        if argent_dawn_rep < GQI_REP_REVERED {
            return Some(GossipSelectOutcome::Close);
        }
        if action == GQI_SELECT_TAILOR {
            let mut items = Vec::new();
            if argent_dawn_rep >= GQI_REP_REVERED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Glacial Gloves".to_string(),
                    sender: GQI_SELECT_TAILOR,
                    action: 1011,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Glacial Wrists".to_string(),
                    sender: GQI_SELECT_TAILOR,
                    action: 1012,
                });
            }
            if argent_dawn_rep >= GQI_REP_EXALTED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Glacial Vest".to_string(),
                    sender: GQI_SELECT_TAILOR,
                    action: 1013,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Glacial Cloak".to_string(),
                    sender: GQI_SELECT_TAILOR,
                    action: 1014,
                });
            }
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_CLOSE_CRAFTER, player),
                sender: GQI_SELECT_TAILOR,
                action: GQI_CLOSE,
            });
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: GQI_MENU_CRAFTER,
                items,
            }));
        }
        if action == GQI_SELECT_BS {
            let mut items = Vec::new();
            if argent_dawn_rep >= GQI_REP_REVERED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icebane Gauntlets".to_string(),
                    sender: GQI_SELECT_BS,
                    action: 1021,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icebane Bracers".to_string(),
                    sender: GQI_SELECT_BS,
                    action: 1022,
                });
            }
            if argent_dawn_rep >= GQI_REP_EXALTED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icebane Breastplate".to_string(),
                    sender: GQI_SELECT_BS,
                    action: 1023,
                });
            }
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_CLOSE_CRAFTER, player),
                sender: GQI_SELECT_BS,
                action: GQI_CLOSE,
            });
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: GQI_MENU_CRAFTER,
                items,
            }));
        }
        if action == GQI_SELECT_LW {
            let mut items = Vec::new();
            if argent_dawn_rep >= GQI_REP_REVERED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Polar Gloves".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1015,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icy Scale Gauntlets".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1018,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Polar Bracers".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1016,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icy Scale Bracers".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1019,
                });
            }
            if argent_dawn_rep >= GQI_REP_EXALTED {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Polar Tunic".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1017,
                });
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Icy Scale Breastplate".to_string(),
                    sender: GQI_SELECT_LW,
                    action: 1020,
                });
            }
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_CLOSE_CRAFTER, player),
                sender: GQI_SELECT_LW,
                action: GQI_CLOSE,
            });
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: GQI_MENU_CRAFTER,
                items,
            }));
        }
        if action == GQI_SELECT_NOCRAFT {
            let items = vec![GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_CLOSE_NO_CRAFTER, player),
                sender: GOSSIP_SENDER_MAIN,
                action: GQI_CLOSE,
            }];
            if self
                .player(player)
                .is_some_and(|p| p.quest_item_count(GQI_ITEM_ARCANE_TOME) == 0)
            {
                // UNMODELED: `AddItem(22719)` (no item-grant helper on Map).
            }
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: GQI_MENU_NOCRAFT,
                items,
            }));
        }
        // Craft arms (`LearnCraftIfCan`: rank gate, 300 skill gate, skip when
        // already known, then the fork falls through to the crafter menu).
        let craft = match action {
            1011 => Some((28212, 28205, GQI_REP_REVERED, tailor_skill)),
            1012 => Some((28215, 28209, GQI_REP_REVERED, tailor_skill)),
            1013 => Some((28213, 28207, GQI_REP_EXALTED, tailor_skill)),
            1014 => Some((28214, 28208, GQI_REP_EXALTED, tailor_skill)),
            1015 => Some((28229, 28220, GQI_REP_REVERED, leatherwork_skill)),
            1016 => Some((28230, 28221, GQI_REP_REVERED, leatherwork_skill)),
            1017 => Some((28228, 28219, GQI_REP_EXALTED, leatherwork_skill)),
            1018 => Some((28232, 28223, GQI_REP_REVERED, leatherwork_skill)),
            1019 => Some((28233, 28224, GQI_REP_REVERED, leatherwork_skill)),
            1020 => Some((28231, 28222, GQI_REP_EXALTED, leatherwork_skill)),
            1021 => Some((28248, 28243, GQI_REP_REVERED, blacksmith_skill)),
            1022 => Some((28249, 28244, GQI_REP_REVERED, blacksmith_skill)),
            1023 => Some((28245, 28242, GQI_REP_EXALTED, blacksmith_skill)),
            _ => None,
        };
        if let Some((learn_id, know_id, min_rank, skill)) = craft {
            if argent_dawn_rep >= min_rank
                && skill >= GQI_CRAFT_SKILL_GATE
                && !self.player_knows_spell(player, know_id)
            {
                let _ = self.cast_spell(
                    player,
                    learn_id,
                    crate::spells::CastTarget::SelfCast,
                    crate::spells::CastOptions {
                        triggered: false,
                        skip_known: true,
                        ..Default::default()
                    },
                );
            }
        } else {
            // Fork returns true with no packet for other actions (unrepresentable outcome).
            return None;
        }
        let menu = GossipMenuData {
            text_id: GQI_MENU_CRAFTER,
            items: vec![GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: self.gossip_broadcast_text(GQI_BCT_CLOSE_CRAFTER, player),
                sender,
                action: GQI_CLOSE,
            }],
        };
        Some(GossipSelectOutcome::ShowMenu(menu))
    }

    /// `GossipHello_npc_knot_thimblejack` (`instance_dire_maul.cpp:949-967`):
    /// the "why bother" item (action 1001) plus the suit-teaching items for
    /// leatherworkers/tailors at 275+ who completed quest 5518 but lack the
    /// pattern, menu 6795. Fork always returns true.
    fn npc_knot_thimblejack_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_QUEST_GORDOK_OGRE_SUIT: u32 = 5518; // `QUEST_GORDOK_OGRE_SUIT`.
        const GQI_SKILL_LEATHERWORKING: u32 = 165; // `SKILL_LEATHERWORKING` (SharedDefines.h:989).
        const GQI_SKILL_TAILORING: u32 = 197; // `SKILL_TAILORING` (SharedDefines.h:1001).
        const GQI_SUIT_SKILL_GATE: u32 = 275;
        const GQI_SPELL_SUIT_KNOWN_L: u32 = 22815; // `SPELL_GORDOK_OGRE_SUIT_L`.
        const GQI_SPELL_SUIT_KNOWN_T: u32 = 22813; // `SPELL_GORDOK_OGRE_SUIT_T`.
        const GQI_MENU_1: u32 = 6795; // `GOSSIP_MENU_1`.
        const GQI_QUEST_COMPLETE: u32 = 1; // `QUEST_STATUS_COMPLETE` (QuestDef.h:94).
        let _ = npc;
        // UNMODELED: `PrepareQuestMenu` when the creature is a quest giver (quest items have no menu model on Map).
        let mut items = vec![GossipItem {
        icon: GOSSIP_ICON_CHAT,
        text: "Why should I bother fixing the trap? Why not just eliminate the guard the old fashioned way?".to_string(),
        sender: GOSSIP_SENDER_MAIN,
        action: GOSSIP_ACTION_INFO_DEF + 1,
    }];
        let rewarded_suit = self.player(player).is_some_and(|p| {
            p.quest_row(GQI_QUEST_GORDOK_OGRE_SUIT)
                .is_some_and(|r| r.rewarded != 0)
        });
        let suit_complete = self.player(player).is_some_and(|p| {
            u32::try_from(p.quest_status(GQI_QUEST_GORDOK_OGRE_SUIT)).unwrap_or(u32::MAX)
                == GQI_QUEST_COMPLETE
        });
        if rewarded_suit && suit_complete {
            let leatherwork = self.player(player).map_or(0, |p| {
                p.deferred
                    .skills
                    .iter()
                    .find(|s| s.skill == GQI_SKILL_LEATHERWORKING)
                    .map_or(0, |s| s.value)
            });
            if leatherwork >= GQI_SUIT_SKILL_GATE
                && !self.player_knows_spell(player, GQI_SPELL_SUIT_KNOWN_L)
            {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Please teach me how to make a Gordok Ogre Suit!".to_string(),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 2,
                });
            }
            let tailor = self.player(player).map_or(0, |p| {
                p.deferred
                    .skills
                    .iter()
                    .find(|s| s.skill == GQI_SKILL_TAILORING)
                    .map_or(0, |s| s.value)
            });
            if tailor >= GQI_SUIT_SKILL_GATE
                && !self.player_knows_spell(player, GQI_SPELL_SUIT_KNOWN_T)
            {
                items.push(GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: "Please teach me how to make a Gordok Ogre Suit!".to_string(),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 3,
                });
            }
        }
        Some(GossipMenuData {
            text_id: GQI_MENU_1,
            items,
        })
    }

    /// `GossipSelect_npc_knot_thimblejack` (`instance_dire_maul.cpp:969-987`):
    /// action 1001 shows menu 6883; actions 1002/1003 teach the suit pattern.
    /// Fork always returns true.
    fn npc_knot_thimblejack_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GQI_SPELL_LEARN_GOS_L: u32 = 22816; // `SPELL_LEARN_GOS_L`.
        const GQI_SPELL_LEARN_GOS_T: u32 = 22814; // `SPELL_LEARN_GOS_T`.
        const GQI_MENU_2: u32 = 6883; // `GOSSIP_MENU_2`.
        let _ = (npc, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            let menu = GossipMenuData {
                text_id: GQI_MENU_2,
                items: Vec::new(),
            };
            return Some(GossipSelectOutcome::ShowMenu(menu));
        }
        // Fork `pPlayer->CastSpell(pPlayer, spell, true)` (triggered player cast).
        if action == GOSSIP_ACTION_INFO_DEF + 2 {
            let _ = self.cast_spell(
                player,
                GQI_SPELL_LEARN_GOS_L,
                crate::spells::CastTarget::SelfCast,
                crate::spells::CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..Default::default()
                },
            );
            // Fork clears the menus with no follow-up packet; Close is the closest outcome.
            return Some(GossipSelectOutcome::Close);
        }
        if action == GOSSIP_ACTION_INFO_DEF + 3 {
            let _ = self.cast_spell(
                player,
                GQI_SPELL_LEARN_GOS_T,
                crate::spells::CastTarget::SelfCast,
                crate::spells::CastOptions {
                    triggered: true,
                    skip_known: true,
                    ..Default::default()
                },
            );
            // Fork clears the menus with no follow-up packet; Close is the closest outcome.
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true with no packet for other actions (unrepresentable outcome).
        None
    }

    /// `QuestRewarded_npc_knot_thimblejack` (`instance_dire_maul.cpp:989-1008`):
    /// quests 5525 / 7429 free Knot: the ball-and-chain is deleted and Knot
    /// walks out. Fork returns false when nothing applies.
    fn npc_knot_thimblejack_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const GQI_QUEST_FREE_KNOT: u32 = 5525; // `QUEST_FREE_KNOT`.
        const GQI_QUEST_FREE_KNOT_REPEATABLE: u32 = 7429; // `QUEST_FREE_KNOT_REPEATABLE`.
        const GQI_GO_KNOTS_BALL_AND_CHAIN: u32 = 179511; // `GO_KNOTS_BALL_AND_CHAIN`.
        const GQI_UNIT_NPC_FLAG_GOSSIP: u32 = 0x01; // `UNIT_NPC_FLAG_GOSSIP` (UnitDefines.h:653).
        const GQI_UNIT_NPC_FLAG_QUESTGIVER: u32 = 0x02; // `UNIT_NPC_FLAG_QUESTGIVER` (UnitDefines.h:654).
        let _ = (player, GQI_GO_KNOTS_BALL_AND_CHAIN);
        if self.dire_maul().is_none() {
            return;
        }
        if quest_id != GQI_QUEST_FREE_KNOT && quest_id != GQI_QUEST_FREE_KNOT_REPEATABLE {
            return;
        }
        // UNMODELED: `FindNearestGameObject(GO_KNOTS_BALL_AND_CHAIN 179511, 20.0f)->Delete()` (no nearest-GO lookup + delete helper on Map).
        let flags = self.object_get_field(npc, offsets::UNIT_NPC_FLAGS);
        let _ = self.object_set_field(
            npc,
            u32::from(offsets::UNIT_NPC_FLAGS),
            flags & !(GQI_UNIT_NPC_FLAG_GOSSIP | GQI_UNIT_NPC_FLAG_QUESTGIVER),
        );
        // Fork `MovePoint(1, 518.325f, 542.00f, -23.901f)` (no orientation given).
        self.creature_move_point(
            npc,
            1,
            Position::new(518.325, 542.00, -23.901, 0.0),
            0,
            0.0,
            0.0,
        );
    }

    /// `GossipHello_boss_kromcrush` (`instance_dire_maul.cpp:1487-1504`):
    /// tribute-DONE picks the king row, else the guard row. Fork returns false
    /// without an instance.
    fn boss_kromcrush_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_KROMCRUSH_TEXT: [(u32, &str); 4] = [
            (
                6913,
                "Um, I'm taking some prisoners we found outside before the king for punishment.",
            ),
            (
                6915,
                "Er... that's how I found them. I wanted to show the king that they were a threat. Say Captain... I overhead Guard Fengus calling you a fat, useless knoll lover. ",
            ),
            (
                6914,
                "So, now that I'm the king... what have you got for me?!",
            ),
            (6920, "This sounds like a task worthy of the new king!"),
        ]; // `sKromcrushGossips` (dire_maul.h:96).
        let _ = player;
        self.dire_maul()?;
        // UNMODELED: `PrepareQuestMenu` when the creature is a quest giver (quest items have no menu model on Map).
        let menu_item = if self.dm_get_data(DM_TYPE_GORDOK_TRIBUTE) == Some(DONE) {
            2
        } else {
            0
        };
        let (text_id, text) = GQI_KROMCRUSH_TEXT[menu_item];
        let items = vec![GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: text.to_string(),
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF,
        }];
        let _ = npc;
        Some(GossipMenuData { text_id, items })
    }

    /// `GossipSelect_boss_kromcrush` (`instance_dire_maul.cpp:1506-1534`):
    /// action 1000 shows the follow-up row; action 1001 closes and either flags
    /// the king questgiver (tribute DONE) or sends Kromcrush to Fengus.
    /// Fork always returns true.
    fn boss_kromcrush_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GQI_KROMCRUSH_TEXT: [(u32, &str); 4] = [
            (
                6913,
                "Um, I'm taking some prisoners we found outside before the king for punishment.",
            ),
            (
                6915,
                "Er... that's how I found them. I wanted to show the king that they were a threat. Say Captain... I overhead Guard Fengus calling you a fat, useless knoll lover. ",
            ),
            (
                6914,
                "So, now that I'm the king... what have you got for me?!",
            ),
            (6920, "This sounds like a task worthy of the new king!"),
        ]; // `sKromcrushGossips` (dire_maul.h:96).
        const GQI_UNIT_NPC_FLAG_QUESTGIVER: u32 = 0x02; // `UNIT_NPC_FLAG_QUESTGIVER` (UnitDefines.h:654).
        let _ = (player, sender);
        let tribute_done = self.dm_get_data(DM_TYPE_GORDOK_TRIBUTE) == Some(DONE);
        let menu_item = if tribute_done { 3 } else { 1 };
        if action == GOSSIP_ACTION_INFO_DEF {
            let (text_id, text) = GQI_KROMCRUSH_TEXT[menu_item];
            let menu = GossipMenuData {
                text_id,
                items: vec![GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: text.to_string(),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 1,
                }],
            };
            return Some(GossipSelectOutcome::ShowMenu(menu));
        }
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            if menu_item == 3 {
                let flags = self.object_get_field(npc, offsets::UNIT_NPC_FLAGS);
                let _ = self.object_set_field(
                    npc,
                    u32::from(offsets::UNIT_NPC_FLAGS),
                    flags | GQI_UNIT_NPC_FLAG_QUESTGIVER,
                );
            } else {
                // UNMODELED: `boss_kromcrushAI::goToFengus()` (scripted walk; event AI lives outside this lane).
            }
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true with no packet for other actions (unrepresentable outcome).
        None
    }

    /// `OnGossipHello_npc_sergeant_bly` (`zulfarrak.cpp:166-182`): pyramid
    /// `KILLED_ALL_TROLLS` offers the battlefield item (menu 1517); `NOT_STARTED`
    /// shows 1515; otherwise 1516. Fork returns false without an instance.
    fn npc_sergeant_bly_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_MENU_BLY_FIGHT: u32 = 1517;
        const GQI_MENU_BLY_IDLE: u32 = 1515;
        const GQI_MENU_BLY_BUSY: u32 = 1516;
        let _ = npc;
        let phase = self.zf_get_data(ZF_EVENT_PYRAMID)?;
        let mut items = Vec::new();
        let text_id = if phase == ZF_PYRAMID_KILLED_ALL_TROLLS {
            items.push(GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: "That's it! I'm tired of helping you out.  It's time we settled things on the battlefield!".to_string(),
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF + 1,
        });
            GQI_MENU_BLY_FIGHT
        } else if phase == ZF_PYRAMID_NOT_STARTED {
            GQI_MENU_BLY_IDLE
        } else {
            GQI_MENU_BLY_BUSY
        };
        let _ = player;
        Some(GossipMenuData { text_id, items })
    }

    /// `OnGossipSelect_npc_sergeant_bly` (`zulfarrak.cpp:152-164`): action 1001
    /// closes the menu and starts Bly's crew event. Fork always returns true.
    fn npc_sergeant_bly_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        let _ = (player, npc, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            // UNMODELED: `npc_sergeant_blyAI::OnScriptEventHappened` (post-gossip yell/faction-attack scheduling; event AI lives outside this lane).
        }
        Some(GossipSelectOutcome::Close)
    }

    /// `OnGossipHello_npc_weegli_blastfuse` (`zulfarrak.cpp:450-469`): pyramid
    /// `KILLED_ALL_TROLLS` offers the blow-the-door item (menu 1514);
    /// `NOT_STARTED` shows 1511; otherwise 1513. Fork returns false without
    /// an instance.
    fn npc_weegli_blastfuse_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GQI_MENU_WEEGLI_READY: u32 = 1514;
        const GQI_MENU_WEEGLI_IDLE: u32 = 1511;
        const GQI_MENU_WEEGLI_BUSY: u32 = 1513;
        let _ = npc;
        let phase = self.zf_get_data(ZF_EVENT_PYRAMID)?;
        let mut items = Vec::new();
        let text_id = if phase == ZF_PYRAMID_KILLED_ALL_TROLLS {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: "Will you blow up that door now?".to_string(),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF + 1,
            });
            GQI_MENU_WEEGLI_READY
        } else if phase == ZF_PYRAMID_NOT_STARTED {
            GQI_MENU_WEEGLI_IDLE
        } else {
            GQI_MENU_WEEGLI_BUSY
        };
        let _ = player;
        Some(GossipMenuData { text_id, items })
    }

    /// `OnGossipSelect_npc_weegli_blastfuse` (`zulfarrak.cpp:439-448`): action
    /// 1001 closes the menu and sends Weegli to blow the door. Fork always
    /// returns true.
    fn npc_weegli_blastfuse_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        let _ = (player, npc, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            // UNMODELED: `npc_weegli_blastfuseAI::OnScriptEventHappened` (`DestroyDoor` run + charge + door use; event AI and GO summons live outside this lane).
        }
        Some(GossipSelectOutcome::Close)
    }

    // ---- lane gq_kal_a.rs ----
    /// `QuestAccept_npc_ruul_snowhoof` (ashenvale.cpp:81-93):
    /// quest 6482 sets escort faction/stand and starts the escort.
    fn npc_ruul_snowhoof_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_FREEDOM_TO_RUUL: u32 = 6482;
        if quest_id != QUEST_FREEDOM_TO_RUUL {
            return;
        }
        // UNMODELED: pCreature->SetFactionTemplateId(33) (FACTION_ESCORTEE).
        // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
        // UNMODELED: npc_ruul_snowhoofAI escort Start(false, player, quest).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_torek` (ashenvale.cpp:207-227):
    /// quest 6544 plays `SAY_READY`, flips raider faction, starts escort.
    fn npc_torek_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_TOREK_ASSULT: u32 = 6544;
        const SAY_READY: i32 = 8284;
        if quest_id != QUEST_TOREK_ASSULT {
            return;
        }
        // TODO comment in fork about companions is informational only.
        self.do_script_text(SAY_READY, npc, Some(player), -1);
        // UNMODELED: pCreature->SetFactionTemporary(1174 FACTION_ORGRIMMAR, TEMPFACTION_RESTORE_RESPAWN) + same for nearby NPC_SPLINTERTREE_RAIDER (12859).
        // UNMODELED: npc_torekAI escort Start(true, player, quest).
    }

    /// `QuestAccept_npc_feero_ironhand` (ashenvale.cpp:386-399):
    /// quest 976 plays `SAY_QUEST_START`, sets escort faction, starts escort.
    fn npc_feero_ironhand_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_SUPPLIES_TO_AUBERDINE: u32 = 976;
        const SAY_QUEST_START: i32 = 1292;
        if quest_id != QUEST_SUPPLIES_TO_AUBERDINE {
            return;
        }
        self.do_script_text(SAY_QUEST_START, npc, Some(player), -1);
        // UNMODELED: pCreature->SetFactionTemplateId(FACTION_ESCORT_A_NEUTRAL_PASSIVE = 10).
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        // UNMODELED: npc_feero_ironhandAI escort Start(true, player, quest).
    }

    /// `QuestAccept_npc_kerlonian` (darkshore.cpp:181-194):
    /// quest 5321 wakes Kerlonian and starts the follow.
    fn npc_kerlonian_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_SLEEPER_AWAKENED: u32 = 5321;
        const SAY_KER_START: i32 = 6540;
        if quest_id != QUEST_SLEEPER_AWAKENED {
            return;
        }
        // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        self.do_script_text(SAY_KER_START, npc, Some(player), -1);
        // UNMODELED: npc_kerlonianAI StartFollow(player, FACTION_ESCORT_A_NEUTRAL_PASSIVE, quest).
    }

    /// `QuestAccept_npc_prospector_remtravel` (darkshore.cpp:311-326):
    /// quest 731 sets escort faction, emotes, starts escort.
    fn npc_prospector_remtravel_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_ABSENT_MINDED_PT2: u32 = 731;
        const SAY_REM_START: i32 = 925;
        const EMOTE_ONESHOT_QUESTION: u32 = 6;
        if quest_id != QUEST_ABSENT_MINDED_PT2 {
            return;
        }
        // UNMODELED: pCreature->SetFactionTemplateId(FACTION_ESCORT_A_NEUTRAL_PASSIVE = 10).
        self.do_script_text(SAY_REM_START, npc, Some(player), -1);
        self.handle_emote_command(npc, EMOTE_ONESHOT_QUESTION);
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        // UNMODELED: npc_prospector_remtravelAI escort Start(false, player, quest, true).
    }

    /// `GossipHello_npc_threshwackonator` (darkshore.cpp:377-384):
    /// quest 2078-incomplete offers the insert-key item; menu text is the
    /// creature DB gossip id.
    fn npc_threshwackonator_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const QUEST_GYROMAST_REV: u32 = 2078;
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        let mut items = Vec::new();
        if self
            .player(player)
            .is_some_and(|p| p.quest_status(QUEST_GYROMAST_REV) == 3)
        {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: "[PH] Insert key".to_string(),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF + 1,
            });
        }
        // Fork GetGossipTextId(pCreature): DB gossip_menu_id of the creature.
        let text_id = self.creature(npc).map_or(0, |c| c.gossip_menu_id);
        Some(GossipMenuData { text_id, items })
    }

    /// `GossipSelect_npc_threshwackonator` (darkshore.cpp:386-398):
    /// action 1001 closes the menu, emotes, and starts the follow.
    fn npc_threshwackonator_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const EMOTE_START: i32 = 3012;
        let _ = (player, sender);
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            self.do_script_text(EMOTE_START, npc, None, -1);
            // UNMODELED: npc_threshwackonatorAI StartFollow(player).
            return Some(GossipSelectOutcome::Close);
        }
        // Fork returns true for any action (handled); non-1001 selects only
        // closed the menu implicitly with no follow-up menu to show.
        Some(GossipSelectOutcome::Close)
    }

    /// `QuestAccept_npc_therylune` (darkshore.cpp:447-461):
    /// quest 945 starts the escort, plays `SAY_THERYLUNE_START`, sets faction.
    fn npc_therylune_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_ID_THERYLUNE_ESCAPE: u32 = 945;
        const SAY_THERYLUNE_START: i32 = 1189;
        if quest_id != QUEST_ID_THERYLUNE_ESCAPE {
            return;
        }
        // UNMODELED: npc_theryluneAI escort Start(false, player, quest).
        self.do_script_text(SAY_THERYLUNE_START, npc, Some(player), -1);
        // UNMODELED: pCreature->SetFactionTemporary(79, TEMPFACTION_RESTORE_RESPAWN).
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
    }

    /// `QuestAccept_npc_volcor` (darkshore.cpp:751-759):
    /// quests 994/995 start the volcor escort.
    fn npc_volcor_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_ESCAPE_THROUGH_FORCE: u32 = 994;
        const QUEST_ESCAPE_THROUGH_STEALTH: u32 = 995;
        if quest_id != QUEST_ESCAPE_THROUGH_FORCE && quest_id != QUEST_ESCAPE_THROUGH_STEALTH {
            return;
        }
        // UNMODELED: npc_volcorAI StartEscort(player, quest).
        let _ = (player, npc);
    }

    /// `QuestComplete_npc_terenthis` (darkshore.cpp:855-887, registered as
    /// pQuestRewardedNPC): fork return value gates the DB quest-end script;
    /// quest 994/995 summons Sentinel Selarin, quest 985 checks Grimclaw.
    fn npc_terenthis_quest_rewarded(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_ESCAPE_THROUGH_FORCE: u32 = 994;
        const QUEST_ESCAPE_THROUGH_STEALTH: u32 = 995;
        const QUEST_HOW_BIG_A_THREAT_2: u32 = 985;
        const NPC_SENTINEL_SELARIN: u32 = 3694;
        const NPC_GRIMCLAW: u32 = 3695;
        const TEMPSUMMON_TIMED_COMBAT_OR_DEAD_DESPAWN: u32 = 9;
        let _ = player;
        if quest_id == QUEST_ESCAPE_THROUGH_FORCE || quest_id == QUEST_ESCAPE_THROUGH_STEALTH {
            // Fork returns true (run DB script) when the sentinel already
            // stands nearby; that bool gate has no Rust equivalent.
            // UNMODELED: quest_end_script gating by fork return value.
            if self
                .nearest_creature_with_entry(npc, NPC_SENTINEL_SELARIN, 40.0)
                .is_some()
            {
                return;
            }
            let at = mangos_movement::movement_info::Position::new(6409.01, 381.597, 13.7997, 1.0);
            if let Some(summoned) = self.summon_creature(
                NPC_SENTINEL_SELARIN,
                at,
                npc,
                TEMPSUMMON_TIMED_COMBAT_OR_DEAD_DESPAWN,
                120000,
            ) {
                self.set_walk(summoned, false);
            }
        } else if quest_id == QUEST_HOW_BIG_A_THREAT_2 {
            // Fork returns true (run DB script) when Grimclaw is nearby,
            // else false (let DB script take over); bool gate unmodeled.
            // UNMODELED: quest_end_script gating by fork return value.
            let _ = self.nearest_creature_with_entry(npc, NPC_GRIMCLAW, 40.0);
        }
    }

    /// `QuestAccept_npc_melizza_brimbuzzle` (desolace.cpp:225-235):
    /// quest 6132 starts the escort.
    fn npc_melizza_brimbuzzle_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_GET_ME_OUT_OF_HERE: u32 = 6132;
        if quest_id != QUEST_GET_ME_OUT_OF_HERE {
            return;
        }
        // UNMODELED: npc_melizza_brimbuzzleAI escort Start(false, player, quest).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_dalinda_malem` (desolace.cpp:271-284):
    /// quest 1440 sets escort faction and starts the escort.
    fn npc_dalinda_malem_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_RETURN_TO_VAHLARRIEL: u32 = 1440;
        if quest_id != QUEST_RETURN_TO_VAHLARRIEL {
            return;
        }
        // UNMODELED: pCreature->SetFactionTemporary(FACTION_ESCORT_A_NEUTRAL_PASSIVE = 10, TEMPFACTION_RESTORE_RESPAWN).
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        // UNMODELED: npc_dalinda_malemAI escort Start(false, player, quest).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_cork_gizelton` (desolace.cpp:956-965):
    /// quest 5821 resumes the caravan path.
    fn npc_cork_gizelton_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_TOP: u32 = 5821;
        if quest_id != QUEST_TOP {
            return;
        }
        // UNMODELED: npc_cork_gizeltonAI ResumePath(player).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_rigger_gizelton` (desolace.cpp:967-979):
    /// quest 5943 resumes Cork's caravan path via the nearby Cork.
    fn npc_rigger_gizelton_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_BOTTOM: u32 = 5943;
        const NPC_CORK_GIZELTON: u32 = 11625;
        if quest_id != QUEST_BOTTOM {
            return;
        }
        let _ = self.nearest_creature_with_entry(npc, NPC_CORK_GIZELTON, 100.0);
        // UNMODELED: npc_cork_gizeltonAI ResumePath(player) on the found Cork.
        let _ = player;
    }

    /// `QuestAccept_celebras_spirit`
    /// (desolace/maraudon/boss_celebras_the_cursed.cpp:309-317):
    /// quest 7046 resets the spirit AI, plays `SAY_ACCEPT`, starts escort.
    fn celebras_spirit_quest_accept(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_SCEPTER: u32 = 7046;
        const SAY_ACCEPT: i32 = 8952;
        if quest_id != QUEST_SCEPTER {
            return;
        }
        self.do_script_text(SAY_ACCEPT, npc, None, -1);
        // UNMODELED: celebrasSpiritAI escort Start(true, player, quest) (incl. AI Reset).
    }

    /// `GossipHello_npc_lady_jaina_proudmoore`
    /// (dustwallow_marsh.cpp:369-386): autograph item on quest 558;
    /// menu text follows Missing Diplomat chain (1267 -> 3158, else 3157).
    fn npc_lady_jaina_proudmoore_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const QUEST_JAINAS_AUTOGRAPH: u32 = 558;
        const QUEST_MISSING_DIPLO_PT16: u32 = 1324;
        const QUEST_MISSING_DIPLO_PT17: u32 = 1267;
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        // UNMODELED: pPlayer->PrepareQuestMenu (quest-offer overlay when the creature is a quest giver).
        let incomplete = |qid: u32| {
            self.player(player)
                .is_some_and(|p| p.quest_status(qid) == 3)
        };
        let complete = |qid: u32| {
            self.player(player)
                .is_some_and(|p| p.quest_status(qid) == 1)
        };
        let mut items = Vec::new();
        if incomplete(QUEST_JAINAS_AUTOGRAPH) {
            items.push(GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: "I know this is rather silly but i have a young ward who is a bit shy and would like your autograph.".to_string(),
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF,
        });
        }
        if complete(QUEST_MISSING_DIPLO_PT17) {
            return Some(GossipMenuData {
                text_id: 3158,
                items,
            });
        }
        if complete(QUEST_MISSING_DIPLO_PT16) {
            // UNMODELED: pPlayer->SendPreparedQuest (quest-offer menu, not a gossip text menu; no GossipMenuData equivalent).
            return None;
        }
        let _ = npc;
        Some(GossipMenuData {
            text_id: 3157,
            items,
        })
    }

    /// `GossipSelect_npc_lady_jaina_proudmoore`
    /// (dustwallow_marsh.cpp:388-396): action 1000 shows menu 7012 and
    /// casts Jaina's autograph spell on the player.
    fn npc_lady_jaina_proudmoore_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const SPELL_JAINAS_AUTOGRAPH: u32 = 23122;
        let _ = (npc, sender);
        if action != GOSSIP_ACTION_INFO_DEF {
            return None;
        }
        let menu = GossipMenuData {
            text_id: 7012,
            items: Vec::new(),
        };
        let _ = self.cast_spell(
            player,
            SPELL_JAINAS_AUTOGRAPH,
            crate::spells::CastTarget::Unit(player),
            crate::spells::CastOptions {
                triggered: false,
                skip_known: true,
                ..Default::default()
            },
        );
        Some(GossipSelectOutcome::ShowMenu(menu))
    }

    /// `QuestRewarded_npc_archmage_tervosh`
    /// (dustwallow_marsh.cpp:182-201): quest 1265 plays text 1751 and
    /// casts Proudmoore's Defense on the player.
    fn npc_archmage_tervosh_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_MISSING_DIPLO_PT14: u32 = 1265;
        const TERVOSH_SAY_ON_QUEST_MD_PT14: i32 = 1751;
        const SPELL_PROUDMOORES_DEFENSE: u32 = 7120;
        if quest_id != QUEST_MISSING_DIPLO_PT14 {
            return;
        }
        // UNMODELED: npc_archmage_tervoshAI dynamic_cast gate (event AI lives outside this lane).
        self.do_script_text(TERVOSH_SAY_ON_QUEST_MD_PT14, npc, None, -1);
        let _ = self.cast_spell(
            npc,
            SPELL_PROUDMOORES_DEFENSE,
            crate::spells::CastTarget::Unit(player),
            crate::spells::CastOptions {
                triggered: true,
                skip_known: true,
                ..Default::default()
            },
        );
    }

    /// `QuestAccept_npc_stinky_ignatz` (dustwallow_marsh.cpp:573-584):
    /// quests 1222/1270 set faction 113/stand and start the escort.
    fn npc_stinky_ignatz_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_STINKYS_ESCAPE_A: u32 = 1222;
        const QUEST_STINKYS_ESCAPE_H: u32 = 1270;
        if quest_id != QUEST_STINKYS_ESCAPE_A && quest_id != QUEST_STINKYS_ESCAPE_H {
            return;
        }
        // UNMODELED: pCreature->SetFactionTemplateId(113) (FACTION_ESCORT_N_NEUTRAL_PASSIVE).
        // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
        // UNMODELED: npc_stinky_ignatzAI escort Start(false, player, quest).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_captured_arkonarin` (felwood.cpp:275-292):
    /// quest 5203 stands Arkonarin, opens the cage, starts the escort.
    fn npc_captured_arkonarin_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_ID_RESCUE_JAEDENAR: u32 = 5203;
        if quest_id != QUEST_ID_RESCUE_JAEDENAR {
            return;
        }
        // UNMODELED: npc_captured_arkonarinAI escort Start(false, player, quest).
        // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        // UNMODELED: GO_ARKONARIN_CAGE (176306) Use(pCreature).
        let _ = (player, npc);
    }

    /// `QuestAccept_npc_arei` (felwood.cpp:471-483):
    /// quest 4261 starts the escort, sets faction, plays `SAY_AREI_ESCORT_START`.
    fn npc_arei_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_ID_ANCIENT_SPIRIT: u32 = 4261;
        const SAY_AREI_ESCORT_START: i32 = 5004;
        if quest_id != QUEST_ID_ANCIENT_SPIRIT {
            return;
        }
        // UNMODELED: npc_areiAI escort Start(false, player, quest).
        // UNMODELED: pCreature->SetFactionTemporary(FACTION_ESCORT_A_NEUTRAL_PASSIVE = 10, TEMPFACTION_RESTORE_RESPAWN).
        self.do_script_text(SAY_AREI_ESCORT_START, npc, Some(player), -1);
    }

    /// `GossipHello_npc_screecher_spirit` (feralas.cpp:30-38):
    /// fixed menu 2039, kill credit, then unselectable.
    fn npc_screecher_spirit_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        let entry = self.creature(npc).map_or(0, |c| c.info.entry);
        // Fork TalkedToCreature(entry, guid): kill credit for the spirit.
        self.quest_kill_credit_one(player, npc, entry);
        // UNMODELED: pCreature->SetFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_NOT_SELECTABLE).
        Some(GossipMenuData {
            text_id: 2039,
            items: Vec::new(),
        })
    }

    /// `QuestAccept_npc_shay_leafrunner` (feralas.cpp:214-224):
    /// quest 2845 plays `SAY_ESCORT_START` and starts the follow.
    fn npc_shay_leafrunner_quest_accept(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_ID_WANDERING_SHAY: u32 = 2845;
        const SAY_ESCORT_START: i32 = 3921;
        if quest_id != QUEST_ID_WANDERING_SHAY {
            return;
        }
        self.do_script_text(SAY_ESCORT_START, npc, None, -1);
        // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
        // UNMODELED: npc_shay_leafrunnerAI BeforeStartFollow(player, FACTION_ESCORT_A_NEUTRAL_PASSIVE = 10, quest).
    }

    /// `QuestAccept_npc_kindal_moonweaver` (feralas.cpp:852-884):
    /// quest 2969 plays `SAY_KINDAL_BEGIN`, starts the follow + sprite event.
    fn npc_kindal_moonweaver_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_FREEDOM_FOR_ALL_CREATURES: u32 = 2969;
        const SAY_KINDAL_BEGIN: i32 = 4079;
        if quest_id != QUEST_FREEDOM_FOR_ALL_CREATURES {
            return;
        }
        // UNMODELED: SetStandState(STAND) / SetFacingToObject(player) / RemoveFlag(IMMUNE_TO_NPC).
        self.do_script_text(SAY_KINDAL_BEGIN, npc, Some(player), -1);
        // UNMODELED: npc_kindal_moonweaverAI StartFollow(player, 231 FACTION_ESCORTEE_KINDAL, quest).
        // UNMODELED: npc_kindal_moonweaverAI BeginEvent() (cage GO state + sprite-darter wiring).
        // UNMODELED: delayed m_Events lambda (SetFollowPaused(false) after 3000ms).
    }

    // ---- lane gq_kal_b.rs ----

    /// `GossipHello_npc_plucky_johnson` (`thousand_needles.cpp:278-285`).
    fn plucky_johnson_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const QUEST_SCOOP: u32 = 1950;
        let mut items = Vec::new();
        if self.player_quest_incomplete(player, QUEST_SCOOP) {
            items.push(GossipItem {
                icon: GOSSIP_ICON_CHAT,
                text: String::from("Please tell me the Phrase.."),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF,
            });
        }
        Some(GossipMenuData {
            text_id: 720,
            items,
        })
    }

    /// `GossipSelect_npc_plucky_johnson` (`thousand_needles.cpp:287-296`).
    fn plucky_johnson_gossip_select(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const QUEST_SCOOP: u32 = 1950;
        if action == GOSSIP_ACTION_INFO_DEF {
            // UNMODELED: pPlayer->AreaExploredOrEventHappens(QUEST_SCOOP).
            let _ = (player, QUEST_SCOOP);
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: 738,
                items: Vec::new(),
            }));
        }
        // Fork returns true with no menu for other actions; no modeled behavior.
        None
    }

    /// `GossipHello_npc_simone_the_inconspicuous` (`ungoro_crater.cpp:864-871`).
    fn simone_the_inconspicuous_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const QUEST_STAVE_OF_THE_ANCIENTS: u32 = 7636;
        let mut items = Vec::new();
        if self.player_quest_incomplete(player, QUEST_STAVE_OF_THE_ANCIENTS) {
            items.push(GossipItem {
                icon: 0,
                text: String::from("Show me your real face, demon."),
                sender: GOSSIP_SENDER_MAIN,
                action: GOSSIP_ACTION_INFO_DEF,
            });
        }
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items,
        })
    }

    /// `GossipSelect_npc_simone_the_inconspicuous` (`ungoro_crater.cpp:873-879`).
    fn simone_the_inconspicuous_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        _sender: u32,
        _action: u32,
    ) -> Option<GossipSelectOutcome> {
        const EMOTE_ONESHOT_LAUGH: u32 = 11;
        // UNMODELED: npc_simone_the_inconspicuousAI::BeginEvent(player guid).
        let _ = player;
        self.handle_emote_command(npc, EMOTE_ONESHOT_LAUGH);
        Some(GossipSelectOutcome::Close)
    }

    /// `GossipHello_npc_Krug_SkullSplit` (`silithus.cpp:2690-2713`).
    fn krug_skullsplit_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const QUEST_FIELD_DUTY: u32 = 8731;
        // UNMODELED: npc_Krug_SkullSplitAI::GetEventStatus() (EVENT_NOT_STARTED /
        // EVENT_COMPLETE); the quest-incomplete arm below is the NOT_STARTED branch.
        if self.player_quest_incomplete(player, QUEST_FIELD_DUTY) {
            return Some(GossipMenuData {
                text_id: self.gossip_text_id(npc),
                items: vec![GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: String::from("Continue."),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 1,
                }],
            });
        }
        // UNMODELED: pPlayer->PrepareQuestMenu for the EVENT_COMPLETE branch.
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: Vec::new(),
        })
    }

    /// `GossipSelect_npc_Krug_SkullSplit` (`silithus.cpp:2716-2732`).
    fn krug_skullsplit_gossip_select(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        if action == GOSSIP_ACTION_INFO_DEF + 1 {
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: self.gossip_text_id(npc),
                items: vec![GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: String::from("Very well, let's go!"),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 2,
                }],
            }));
        }
        if action == GOSSIP_ACTION_INFO_DEF + 2 {
            // UNMODELED: npc_Krug_SkullSplitAI::StartEvent().
            return Some(GossipSelectOutcome::Close);
        }
        None
    }

    /// `QuestAccept_npc_keeper_remulos` (`moonglade.cpp:895-917`).
    fn keeper_remulos_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_NIGHTMARE_MANIFESTS: u32 = 8736;
        const QUEST_WAKING_LEGENDS: u32 = 8447;
        if quest_id == QUEST_NIGHTMARE_MANIFESTS {
            // UNMODELED: pCreature->RemoveFlag(UNIT_NPC_FLAGS, UNIT_NPC_FLAG_QUESTGIVER).
            // UNMODELED: npc_keeper_remulosAI escort Start(false, player guid, quest) + SetMaxPlayerDistance(0).
            let _ = (player, npc);
        }
        if quest_id == QUEST_WAKING_LEGENDS {
            // UNMODELED: pCreature->RemoveFlag(UNIT_NPC_FLAGS, UNIT_NPC_FLAG_QUESTGIVER).
            // UNMODELED: npc_keeper_remulosAI escort Start(false, player guid, quest).
            let _ = (player, npc);
        }
    }

    /// `QuestAccept_npc_piznik` (`stonetalon_mountains.cpp:138-146`).
    fn piznik_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_GERENOS_ORDERS: u32 = 1090;
        if quest_id == QUEST_GERENOS_ORDERS {
            // UNMODELED: npc_piznikAI::StartEvent(player).
            let _ = (player, npc);
        }
    }

    /// `QuestAccept_npc_tooga` (`tanaris.cpp:177-186`).
    fn tooga_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_TOOGA: u32 = 1560;
        if quest_id == QUEST_TOOGA {
            // UNMODELED: npc_toogaAI::StartFollow(player, FACTION_ESCORT_N_FRIEND_PASSIVE, quest).
            let _ = (player, npc);
        }
    }

    /// `QuestAccept_npc_mist` (`teldrassil.cpp:95-106`).
    fn mist_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_MIST: u32 = 938;
        if quest_id == QUEST_MIST {
            // UNMODELED: pCreature->SetFactionTemporary(FACTION_DARNASSUS = 79, TEMPFACTION_RESTORE_RESPAWN).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: npc_mistAI::StartFollow(player, FACTION_DARNASSUS, quest).
            let _ = (player, npc);
        }
    }

    /// `QuestAccept_npc_gilthares` (`the_barrens.cpp:160-175`).
    fn gilthares_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_FREE_FROM_HOLD: u32 = 898;
        const SAY_GIL_START: i32 = 1065;
        if quest_id == QUEST_FREE_FROM_HOLD {
            // UNMODELED: pCreature->SetFactionTemplateId(FACTION_ESCORT_H_NEUTRAL_ACTIVE = 232).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
            self.do_script_text(SAY_GIL_START, npc, Some(player), -1);
            // UNMODELED: npc_giltharesAI escort Start(false, player guid, quest).
        }
    }

    /// `QuestAccept_npc_wizzlecranks_shredder` (`the_barrens.cpp:549-559`).
    fn wizzlecranks_shredder_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_ESCAPE: u32 = 863;
        const SAY_START: i32 = 1031;
        if quest_id == QUEST_ESCAPE {
            self.do_script_text(SAY_START, npc, None, -1);
            // UNMODELED: pCreature->SetFactionTemplateId(FACTION_RATCHET = 637).
            // UNMODELED: npc_wizzlecranks_shredderAI escort Start(true, player guid, quest).
            let _ = player;
        }
    }

    /// `QuestAccept_npc_belnistrasz` (`razorfen_downs.cpp:275-288`).
    fn belnistrasz_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_EXTINGUISHING_THE_IDOL: u32 = 3525;
        const SAY_BELNISTRASZ_READY: i32 = 4493;
        if quest_id == QUEST_EXTINGUISHING_THE_IDOL {
            // UNMODELED: npc_belnistraszAI escort Start(false, player guid, quest).
            self.do_script_text(SAY_BELNISTRASZ_READY, npc, Some(player), -1);
            // UNMODELED: pCreature->SetFactionTemplateId(FACTION_ESCORT_N_NEUTRAL_ACTIVE = 250).
        }
    }

    /// `QuestAccept_npc_willix_the_importer` (`razorfen_kraul.cpp:137-152`).
    fn willix_the_importer_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_WILLIX_THE_IMPORTER: u32 = 1144;
        const SAY_WILLIX_READY: i32 = 1482;
        if quest_id == QUEST_WILLIX_THE_IMPORTER {
            // UNMODELED: npc_willix_the_importerAI escort Start(false, player guid, quest).
            self.do_script_text(SAY_WILLIX_READY, npc, Some(player), -1);
            // UNMODELED: pCreature->SetFactionTemporary(FACTION_ESCORT_N_NEUTRAL_PASSIVE = 113, TEMPFACTION_RESTORE_RESPAWN).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            let _ = player;
        }
    }

    /// `QuestAccept_npc_lakota_windsong` (`thousand_needles.cpp:103-116`).
    fn lakota_windsong_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_FREE_AT_LAST: u32 = 4904;
        const SAY_LAKO_START: i32 = 5926;
        if quest_id == QUEST_FREE_AT_LAST {
            self.do_script_text(SAY_LAKO_START, npc, Some(player), -1);
            // UNMODELED: pCreature->SetFactionTemporary(FACTION_ESCORTEE = 33, TEMPFACTION_RESTORE_RESPAWN).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: npc_lakota_windsongAI escort Start(false, player guid, quest).
            let _ = player;
        }
    }

    /// `QuestAccept_npc_paoka_swiftmountain` (`thousand_needles.cpp:173-186`).
    fn paoka_swiftmountain_quest_accept(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_HOMEWARD: u32 = 4770;
        const SAY_START: i32 = 5648;
        if quest_id == QUEST_HOMEWARD {
            self.do_script_text(SAY_START, npc, Some(player), -1);
            // UNMODELED: pCreature->SetFactionTemplateId(FACTION_ESCORT_H_NEUTRAL_ACTIVE = 232).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: npc_paoka_swiftmountainAI escort Start(false, player guid, quest).
            let _ = player;
        }
    }

    /// `QuestAccept_npc_ame01` (`ungoro_crater.cpp:102-115`).
    fn ame01_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_CHASING_AME: u32 = 4245;
        if quest_id == QUEST_CHASING_AME {
            // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
            // UNMODELED: pCreature->SetFactionTemporary(FACTION_ESCORT_N_NEUTRAL_PASSIVE = 113, TEMPFACTION_RESTORE_RESPAWN).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: npc_ame01AI escort Start(false, player guid, quest).
            let _ = (player, npc);
        }
    }

    /// `QuestAccept_npc_ringo` (`ungoro_crater.cpp:308-321`).
    fn ringo_quest_accept(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_A_LITTLE_HELP: u32 = 4491;
        if quest_id == QUEST_A_LITTLE_HELP {
            // UNMODELED: pCreature->SetStandState(UNIT_STAND_STATE_STAND).
            // UNMODELED: pCreature->RemoveFlag(UNIT_FIELD_FLAGS, UNIT_FLAG_IMMUNE_TO_NPC).
            // UNMODELED: npc_ringoAI::StartFollow(player, FACTION_ESCORT_N_NEUTRAL_PASSIVE = 113, quest).
            let _ = (player, npc);
        }
    }

    /// `QuestRewarded_npc_yehkinya` (`tanaris.cpp:361-374`).
    fn yehkinya_quest_rewarded(&mut self, player: ObjectGuid, npc: ObjectGuid, quest_id: u32) {
        const QUEST_HAKKAR_EVENT: u32 = 8181;
        const SAY_HAKKAR_EVENT_1: i32 = 10456;
        if quest_id == QUEST_HAKKAR_EVENT {
            self.do_script_text(SAY_HAKKAR_EVENT_1, npc, Some(player), -1);
            // UNMODELED: npc_yehkinyaAI escort Start(true, 0, NULL, true).
            // UNMODELED: pCreature->SetWalk(false).
            let _ = player;
        }
    }

    /// `QuestComplete_npc_treshala_fallowbrook` (`teldrassil.cpp:116-124`,
    /// registered as `pQuestRewardedNPC`).
    fn treshala_fallowbrook_quest_rewarded(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_MORTALITY_WANES: u32 = 1142;
        const EMOTE_ONESHOT_CRY: u32 = 18;
        // Fork is a `pQuestCompleteNPC`-style handler filed under REWARDED;
        // fork returns false so DB quest handling takes over.
        if quest_id == QUEST_MORTALITY_WANES {
            self.handle_emote_command(npc, EMOTE_ONESHOT_CRY);
        }
    }

    /// `QuestComplete_npc_Geologist_Larksbane` (`silithus.cpp:906-914`,
    /// registered as `pQuestRewardedNPC`).
    fn geologist_larksbane_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        // Fork is a `pQuestCompleteNPC`-style handler filed under REWARDED;
        // fork returns false unless quest 8315 (The Calling).
        if quest_id == 8315 {
            // UNMODELED: npc_Geologist_LarksbaneAI::QuestCompleted(player, quest).
            let _ = (player, npc);
        }
    }

    // ---- lane gq_world_custom.rs ----
    /// `QuestAccept_npc_doctor` (`world/npcs_special.cpp:498-507`): quest
    /// 6622/6624 starts the triage event.
    /// UNMODELED: `npc_doctorAI::BeginEvent` (patient summon/update AI,
    /// `GroupEventHappens`/`FailQuest` credit on `EndEvent`).
    fn npc_doctor_quest_accept(&mut self, _player: ObjectGuid, _npc: ObjectGuid, quest_id: u32) {
        const QUEST_TRIAGE_H: u32 = 6622;
        const QUEST_TRIAGE_A: u32 = 6624;
        if quest_id != QUEST_TRIAGE_A && quest_id != QUEST_TRIAGE_H {}
        // UNMODELED: npc_doctorAI::BeginEvent(pPlayer) — triage patient summon AI.
    }

    /// `QuestRewarded_npc_riggle_bassbait` (`world/npcs_special.cpp:1257-1268`):
    /// quest 8193 records the fishing winner and yells to zone.
    /// `VAR_STV_*` (`game/ObjectMgr.h:518-521`); texts 10608/10610/10609.
    fn npc_riggle_bassbait_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_MASTER_ANGLER: u32 = 8193;
        const VAR_STV_FISHING_PREV_WIN_TIME: u32 = 30021;
        const VAR_STV_FISHING_HAS_WINNER: u32 = 30056;
        const YELL_WINNER: i32 = 10610;
        if quest_id != QUEST_MASTER_ANGLER {
            return;
        }
        // UNMODELED: SetSavedVariable(VAR_STV_FISHING_PREV_WIN_TIME, time(nullptr))
        // (needs wall-clock epoch seconds; Map only tracks ms-since-boot).
        let _ = VAR_STV_FISHING_PREV_WIN_TIME;
        self.saved_variables.insert(VAR_STV_FISHING_HAS_WINNER, 1);
        // UNMODELED: pCreature->RemoveFlag(UNIT_NPC_FLAGS, UNIT_NPC_FLAG_QUESTGIVER).
        self.do_script_text(YELL_WINNER, npc, Some(player), -1);
    }

    /// `GossipHello_npc_kwee_peddlefeet` (`world/npcs_special.cpp:1990-2009`):
    /// contest-result menu (texts 8315/8316/8320); `false` (no menu) when the
    /// Love-is-in-the-Air event is running.
    /// `VAR_KWEE_*` 2200-2207, quests 8981/8993, `TEXT_ID_*` (`cpp:1831-1847`).
    fn npc_kwee_peddlefeet_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const VAR_KWEE_HORDE: u32 = 2207;
        const VAR_KWEE_ALLIANCE: u32 = 2206;
        const TEXT_ID_VICTORY_A: u32 = 8315;
        const TEXT_ID_VICTORY_H: u32 = 8316;
        const TEXT_ID_TIE: u32 = 8320;
        // UNMODELED: SendUpdateWorldState(i, GetSavedVariable(i)) for 2200-2207.
        // UNMODELED: sGameEventMgr.IsActiveEvent(EVENT_LOVE_IS_IN_THE_AIR = 8)
        // gate (no game-event helper; arm assumes the contest is over, as the
        // fork does once the event flag is off).
        // UNMODELED: npc_kwee_peddlefeetAI::winningFaction (AI state); fork
        // SetVariables recomputes it from the 2206/2207 totals (cpp:1881-1892),
        // reproduced here.
        let horde = self
            .saved_variables
            .get(&VAR_KWEE_HORDE)
            .copied()
            .unwrap_or(0);
        let alliance = self
            .saved_variables
            .get(&VAR_KWEE_ALLIANCE)
            .copied()
            .unwrap_or(0);
        let text_id = match horde.cmp(&alliance) {
            std::cmp::Ordering::Greater => TEXT_ID_VICTORY_H,
            std::cmp::Ordering::Less => TEXT_ID_VICTORY_A,
            std::cmp::Ordering::Equal => TEXT_ID_TIE,
        };
        let _ = player;
        Some(GossipMenuData {
            text_id,
            items: Vec::new(),
        })
    }

    /// `QuestRewarded_npc_kwee_peddlefeet` (`world/npcs_special.cpp:2011-2040`):
    /// quest 8981/8993 adds `ReqItemCount[0]` to the city boss var and the
    /// faction total (`CityZones`, `cpp:1851-1858`).
    fn npc_kwee_peddlefeet_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        const QUEST_GIFT_H: u32 = 8981;
        const QUEST_GIFT_A: u32 = 8993;
        const VAR_KWEE_THRALL: u32 = 2200;
        const VAR_KWEE_TYRANDE: u32 = 2205;
        const VAR_KWEE_BOLVAR: u32 = 2203;
        const VAR_KWEE_ALLIANCE: u32 = 2206;
        const VAR_KWEE_HORDE: u32 = 2207;
        const CITY_ZONES: [u32; 6] = [1637, 1638, 1497, 1519, 1537, 1657];
        if quest_id != QUEST_GIFT_A && quest_id != QUEST_GIFT_H {
            return;
        }
        let Some(template) = self.stores.quest_templates.get(quest_id).cloned() else {
            return;
        };
        let count = template.req_item_counts[0];
        if count == 0 {
            return;
        }
        // Fork reads the creature's zone; the player shares the interaction
        // position, and only the player zone helper exists on Map.
        let zone = self.player_zone_id(player);
        let mut boss_var = 0;
        let mut faction_var = 0;
        for i in VAR_KWEE_THRALL..=VAR_KWEE_TYRANDE {
            if zone == CITY_ZONES[(i - VAR_KWEE_THRALL) as usize] {
                boss_var = i;
                faction_var = if i < VAR_KWEE_BOLVAR {
                    VAR_KWEE_HORDE
                } else {
                    VAR_KWEE_ALLIANCE
                };
            }
        }
        if boss_var != 0 {
            let stock = self.saved_variables.get(&boss_var).copied().unwrap_or(0);
            self.saved_variables.insert(boss_var, stock + count);
        }
        if faction_var != 0 {
            let stock = self.saved_variables.get(&faction_var).copied().unwrap_or(0);
            self.saved_variables.insert(faction_var, stock + count);
        }
    }

    /// `GossipHello_npc_argent_emissary`
    /// (`world/scourge_invasion.cpp:1121-1150`): world-state pushes plus the
    /// four-question menu (text 8434).
    /// `VARIABLE_SI_*` = 17000 + subevent (`scourge_invasion.h:360-370` with
    /// `GAME_EVENT_SCOURGE_INVASION` = 17): attack count 17000, Winterspring
    /// 17090, Tanaris 17091, Azshara 17092, Blasted Lands 17093, Eastern
    /// Plaguelands 17094, Burning Steppes 17095. World-state ids 2219,
    /// 2279-2284 (`game/WorldStates.h:102-110`).
    fn npc_argent_emissary_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const BCT_OPTION_0: u32 = 12176;
        const BCT_OPTION_1: u32 = 12207;
        const BCT_OPTION_2: u32 = 12434;
        const BCT_OPTION_3: u32 = 12398;
        const BCT_GOSSIP: u32 = 8434;
        // UNMODELED: 7x SendUpdateWorldState (2219 battles-won; 2279 Azshara,
        // 2280 Blasted Lands, 2281 Burning Steppes, 2282 Plaguelands, 2283
        // Tanaris, 2284 Winterspring remaining) from saved vars
        // 17000/17092/17093/17095/17094/17091/17090 (no Map world-state helper).
        Some(GossipMenuData {
            text_id: BCT_GOSSIP,
            items: vec![
                GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: self.gossip_broadcast_text(BCT_OPTION_0, player),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 1,
                },
                GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: self.gossip_broadcast_text(BCT_OPTION_1, player),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 2,
                },
                GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: self.gossip_broadcast_text(BCT_OPTION_2, player),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 3,
                },
                GossipItem {
                    icon: GOSSIP_ICON_CHAT,
                    text: self.gossip_broadcast_text(BCT_OPTION_3, player),
                    sender: GOSSIP_SENDER_MAIN,
                    action: GOSSIP_ACTION_INFO_DEF + 4,
                },
            ],
        })
    }

    /// `GossipSelect_npc_argent_emissary`
    /// (`world/scourge_invasion.cpp:1009-1119`): every arm rebuilds a menu
    /// (fork never closes; falls through to `return true`).
    /// Answer texts: 8471/8486/8573, sub-answers 8480/8482/8483/8484/8593/
    /// 8594, battle-count answers 8551/8554/8555, area-free 8481, back-option
    /// 12478, sub-options 12198/12201/12203/12205/12476/12477
    /// (`scourge_invasion.h:459-492`).
    fn npc_argent_emissary_gossip_select(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const GOSSIP_ICON_CHAT: u8 = 0;
        const GOSSIP_SENDER_MAIN: u32 = 1;
        const GOSSIP_ACTION_INFO_DEF: u32 = 1000;
        const BCT_OPTION: u32 = 12478;
        const BCT_OPTION_0: u32 = 12176;
        const BCT_OPTION_1: u32 = 12207;
        const BCT_OPTION_2: u32 = 12434;
        const BCT_OPTION_3: u32 = 12398;
        const BCT_SUB_0: u32 = 12198;
        const BCT_SUB_1: u32 = 12201;
        const BCT_SUB_2: u32 = 12203;
        const BCT_SUB_3: u32 = 12205;
        const BCT_SUB_4: u32 = 12476;
        const BCT_SUB_5: u32 = 12477;
        const BCT_GOSSIP: u32 = 8434;
        const BCT_A0: u32 = 8471;
        const BCT_A1: u32 = 8486;
        const BCT_A2: u32 = 8573;
        const BCT_A2S0: u32 = 8480;
        const BCT_A2S1: u32 = 8482;
        const BCT_A2S2: u32 = 8483;
        const BCT_A2S3: u32 = 8484;
        const BCT_A2S4: u32 = 8593;
        const BCT_A2S5: u32 = 8594;
        const BCT_B0: u32 = 8551;
        const BCT_B1: u32 = 8554;
        const BCT_B2: u32 = 8555;
        const BCT_FREE: u32 = 8481;
        const VAR_SI_WINTERSPRING_REMAINING: u32 = 17090;
        const VAR_SI_TANARIS_REMAINING: u32 = 17091;
        const VAR_SI_AZSHARA_REMAINING: u32 = 17092;
        const VAR_SI_BLASTED_LANDS_REMAINING: u32 = 17093;
        const VAR_SI_EASTERN_PLAGUELANDS_REMAINING: u32 = 17094;
        const VAR_SI_BURNING_STEPPES_REMAINING: u32 = 17095;
        // Fork `urand(0, 2)` over the three battle-count answers (rolled up
        // front so the menu closures below can hold `&self`).
        let battle_pick = if self.roll_chance_i(34) {
            BCT_B0
        } else if self.roll_chance_i(50) {
            BCT_B1
        } else {
            BCT_B2
        };
        let item = |text: String| GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text,
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF + 5,
        };
        let back = |text: String| GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text,
            sender: GOSSIP_SENDER_MAIN,
            action: GOSSIP_ACTION_INFO_DEF + 3,
        };
        let menu = |text_id: u32, items: Vec<GossipItem>| GossipMenuData { text_id, items };
        let t = |id: u32| self.gossip_broadcast_text(id, player);
        let remaining = |var: u32| self.saved_variables.get(&var).copied().unwrap_or(0);
        let area_menu = |text_id: u32| menu(text_id, vec![back(t(BCT_OPTION))]);
        match action {
            a if a == GOSSIP_ACTION_INFO_DEF + 1 => Some(GossipSelectOutcome::ShowMenu(menu(
                BCT_A0,
                vec![item(t(BCT_OPTION))],
            ))),
            a if a == GOSSIP_ACTION_INFO_DEF + 2 => Some(GossipSelectOutcome::ShowMenu(menu(
                BCT_A1,
                vec![item(t(BCT_OPTION))],
            ))),
            a if a == GOSSIP_ACTION_INFO_DEF + 3 => Some(GossipSelectOutcome::ShowMenu(menu(
                BCT_A2,
                vec![
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_4),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 10,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_2),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 8,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_3),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 9,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_1),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 7,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_5),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 11,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_SUB_0),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 6,
                    },
                    item(t(BCT_OPTION)),
                ],
            ))),
            a if a == GOSSIP_ACTION_INFO_DEF + 4 => Some(GossipSelectOutcome::ShowMenu(menu(
                battle_pick,
                vec![item(t(BCT_OPTION))],
            ))),
            a if a == GOSSIP_ACTION_INFO_DEF + 5 => Some(GossipSelectOutcome::ShowMenu(menu(
                BCT_GOSSIP,
                vec![
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_OPTION_0),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 1,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_OPTION_1),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 2,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_OPTION_2),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 3,
                    },
                    GossipItem {
                        icon: GOSSIP_ICON_CHAT,
                        text: t(BCT_OPTION_3),
                        sender: GOSSIP_SENDER_MAIN,
                        action: GOSSIP_ACTION_INFO_DEF + 4,
                    },
                ],
            ))),
            a if a == GOSSIP_ACTION_INFO_DEF + 6 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_WINTERSPRING_REMAINING) > 0 {
                    area_menu(BCT_A2S0)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            a if a == GOSSIP_ACTION_INFO_DEF + 7 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_TANARIS_REMAINING) > 0 {
                    area_menu(BCT_A2S1)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            a if a == GOSSIP_ACTION_INFO_DEF + 8 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_BLASTED_LANDS_REMAINING) > 0 {
                    area_menu(BCT_A2S2)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            a if a == GOSSIP_ACTION_INFO_DEF + 9 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_BURNING_STEPPES_REMAINING) > 0 {
                    area_menu(BCT_A2S3)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            a if a == GOSSIP_ACTION_INFO_DEF + 10 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_AZSHARA_REMAINING) > 0 {
                    area_menu(BCT_A2S4)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            a if a == GOSSIP_ACTION_INFO_DEF + 11 => Some(GossipSelectOutcome::ShowMenu(
                if remaining(VAR_SI_EASTERN_PLAGUELANDS_REMAINING) > 0 {
                    area_menu(BCT_A2S5)
                } else {
                    area_menu(BCT_FREE)
                },
            )),
            // Fork switch has no default arm; unknown actions fall to `return
            // true` with no menu sent.
            _ => None,
        }
    }

    /// `GossipHello_npc_AQwar_collector`
    /// (`world/world_event_wareffort.cpp:559-590`): objective-gated menu with
    /// the per-resource gossip text (`GetWarEffortGossipTextId`, cpp:211-243).
    /// Table (`world_event_wareffort.h:115-146`): shared items map to
    /// (progress ally, progress horde, complete ally, complete horde).
    fn npc_aqwar_collector_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const GOSSIP_ROWS: [[u32; 5]; 25] = [
            [2840, 7824, 7866, 7823, 7867],
            [8831, 7835, 7907, 7836, 7908],
            [4304, 7845, 7912, 7846, 7846],
            [6887, 7861, 7927, 7862, 7928],
            [14529, 7853, 7923, 7854, 7924],
            [3575, 7829, 0, 7830, 0],
            [12359, 7831, 0, 7832, 0],
            [8836, 7837, 0, 7838, 0],
            [3820, 7833, 0, 7834, 0],
            [2318, 7840, 0, 7842, 0],
            [2319, 7843, 0, 7844, 0],
            [12210, 7859, 0, 7860, 0],
            [5095, 7856, 0, 7857, 0],
            [1251, 7847, 0, 7848, 0],
            [6450, 7851, 0, 7852, 0],
            [3576, 0, 7870, 0, 7871],
            [3860, 0, 7876, 0, 7877],
            [2447, 0, 7878, 0, 7879],
            [4625, 0, 7905, 0, 7906],
            [4234, 0, 7909, 0, 7910],
            [8170, 0, 7914, 0, 7915],
            [12209, 0, 7925, 0, 7926],
            [13935, 0, 7929, 0, 7930],
            [3530, 0, 7918, 0, 7919],
            [8544, 0, 7920, 0, 7921],
        ];
        // UNMODELED: npc_AQwar_collectorAI::{ObjectiveReached,
        // RemoveQuestGiverFlag, SendWorldStateUpdateToPlayer, resourceItemId}
        // (collector AI + world-state push have no Map equivalent); the arm
        // assumes the objective is not yet reached, as the live turn-in path.
        // UNMODELED: pCreature->HandleEmote(EMOTE_ONESHOT_BOW = 2).
        // UNMODELED: pPlayer->PrepareQuestMenu (quest-menu items unmodeled).
        let alliance = self
            .player(player)
            .is_some_and(|p| p.team == Team::Alliance);
        let row = GOSSIP_ROWS[0];
        let mut text_id = if alliance { row[1] } else { row[2] };
        if text_id == 0 {
            text_id = 2;
        }
        Some(GossipMenuData {
            text_id,
            items: Vec::new(),
        })
    }

    /// `QuestComplete_npc_AQwar_collector` (`world/world_event_wareffort.cpp:592-628`,
    /// registered as `pQuestRewardedNPC`: treat as REWARDED): adds
    /// `ReqItemCount[0]` to the resource stock var and cheers on completion.
    /// Shared items {2840, 8831, 4304, 6887, 14529} map per team via
    /// `GetSharedSavedVar` to 30024/30026/30028/30030/30032 (alliance) and
    /// 30025/30027/30029/30031/30033 (horde) (`game/ObjectMgr.h:524-533`);
    /// faction items use the item id itself as the var.
    fn npc_aqwar_collector_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        const SHARED_ITEMS: [u32; 5] = [2840, 8831, 4304, 6887, 14529];
        const SHARED_VARS_ALLIANCE: [u32; 5] = [30024, 30026, 30028, 30030, 30032];
        const SHARED_VARS_HORDE: [u32; 5] = [30025, 30027, 30029, 30031, 30033];
        let Some(template) = self.stores.quest_templates.get(quest_id).cloned() else {
            return;
        };
        let req_item = template.req_item_ids[0];
        let req_count = template.req_item_counts[0];
        if req_item == 0 || req_count == 0 {
            return;
        }
        let alliance = self
            .player(player)
            .is_some_and(|p| p.team == Team::Alliance);
        let mut saved_var = req_item;
        for (i, item) in SHARED_ITEMS.iter().enumerate() {
            if req_item == *item {
                saved_var = if alliance {
                    SHARED_VARS_ALLIANCE[i]
                } else {
                    SHARED_VARS_HORDE[i]
                };
                break;
            }
        }
        let stock = self.saved_variables.get(&saved_var).copied().unwrap_or(0);
        self.saved_variables.insert(saved_var, stock + req_count);
        // UNMODELED: npc_AQwar_collectorAI::ObjectiveReached(pQuest) +
        // RemoveQuestGiverFlag (collector AI state).
        // UNMODELED: pQuestGiver->HandleEmote(EMOTE_ONESHOT_CHEER = 4).
    }

    /// `GossipHello_TeleportNPC` (`custom/custom_creatures.cpp:24-49`): horde
    /// cities action 1 / alliance 2, starting areas 3/4, instances 5, raids
    /// 101, Gurubashi 4015, Kalimdor zones 6010, EK zones 6020; menu text
    /// `DEFAULT_GOSSIP_MESSAGE` (16777215). Icon 5 is `GOSSIP_ICON_INTERACT_2`.
    fn teleport_npc_gossip_hello(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        Some(self.teleport_npc_menu(player, 0))
    }

    /// `SendDefaultMenu_TeleportNPC` (`custom/custom_creatures.cpp:51-549`)
    /// as a menu builder: submenu actions rebuild the menu, `action` 0 is the
    /// hello main menu (picked by team, as the fork hello does).
    fn teleport_npc_menu(&self, player: ObjectGuid, action: u32) -> GossipMenuData {
        const INTERACT_2: u8 = 5;
        const TALK: u8 = 7;
        const SENDER_MAIN: u32 = 1;
        const DEFAULT_GOSSIP_MESSAGE: u32 = 16777215;
        let horde = self.player(player).is_some_and(|p| p.team == Team::Horde);
        let gi = |icon: u8, text: &str, action: u32| GossipItem {
            icon,
            text: text.to_string(),
            sender: SENDER_MAIN,
            action,
        };
        let main: Vec<GossipItem> = if horde {
            vec![
                gi(INTERACT_2, "Major Cities", 1),
                gi(INTERACT_2, "Starting Areas", 3),
                gi(INTERACT_2, "Instances", 5),
                gi(INTERACT_2, "Raids", 101),
                gi(INTERACT_2, "Gurubashi Arena", 4015),
                gi(INTERACT_2, "Zones - Kalimdor", 6010),
                gi(INTERACT_2, "Zones - Eastern Kingdoms", 6020),
            ]
        } else {
            vec![
                gi(INTERACT_2, "Major Cities", 2),
                gi(INTERACT_2, "Starting Areas", 4),
                gi(INTERACT_2, "Instances", 5),
                gi(INTERACT_2, "Raids", 101),
                gi(INTERACT_2, "Gurubashi Arena", 4015),
                gi(INTERACT_2, "Zones - Kalimdor", 6010),
                gi(INTERACT_2, "Zones - Eastern Kingdoms", 6020),
            ]
        };
        let back_main = vec![gi(INTERACT_2, "<-[Main Menu]", 100)];
        let tail = |mut items: Vec<GossipItem>| -> Vec<GossipItem> {
            items.extend(back_main.clone());
            items
        };
        let items = match action {
            0 => main,
            1 => tail(vec![
                gi(INTERACT_2, "Orgrimmar", 20),
                gi(INTERACT_2, "Undercity", 21),
                gi(INTERACT_2, "Thunderbluff", 22),
            ]),
            2 => tail(vec![
                gi(INTERACT_2, "Stormwind City", 23),
                gi(INTERACT_2, "Ironforge", 24),
                gi(INTERACT_2, "Darnassus", 25),
            ]),
            3 => tail(vec![
                gi(INTERACT_2, "Shadow Grave", 40),
                gi(INTERACT_2, "Valley of Trials", 41),
                gi(INTERACT_2, "Camp Narache", 42),
            ]),
            4 => tail(vec![
                gi(INTERACT_2, "Northshire Valley", 43),
                gi(INTERACT_2, "Coldridge Valley", 44),
                gi(INTERACT_2, "Shadowglen", 45),
            ]),
            5 => tail(vec![
                gi(INTERACT_2, "The Wailing Caverns", 1249),
                gi(INTERACT_2, "Deadmines", 1250),
                gi(INTERACT_2, "Shadowfang Keep", 1251),
                gi(INTERACT_2, "Blackfathom Deeps", 1252),
                gi(INTERACT_2, "Razorfen Kraul", 1254),
                gi(INTERACT_2, "Razorfen Downs", 1256),
                gi(INTERACT_2, "Scarlet Monastery", 1257),
                gi(TALK, "[More] ->", 5551),
            ]),
            5551 => {
                let mut items = vec![
                    gi(INTERACT_2, "Uldaman", 1258),
                    gi(INTERACT_2, "Zul'Farrak", 1259),
                    gi(INTERACT_2, "Maraudon", 1260),
                    gi(INTERACT_2, "The Sunken Temple", 1261),
                    gi(INTERACT_2, "Blackrock Depths", 1262),
                    gi(INTERACT_2, "Dire Maul", 1263),
                    gi(INTERACT_2, "Blackrock Spire", 1264),
                    gi(INTERACT_2, "Stratholme", 1265),
                    gi(INTERACT_2, "Scholomance", 1266),
                    gi(TALK, "<- [Back]", 5),
                ];
                items.extend(back_main.clone());
                items
            }
            101 => tail(vec![
                gi(INTERACT_2, "Zul'Gurub", 4000),
                gi(INTERACT_2, "Onyxia's Lair", 4001),
                gi(INTERACT_2, "Molten Core", 4002),
                gi(INTERACT_2, "Blackwing Lair", 4003),
                gi(INTERACT_2, "Ruins of Ahn'Qiraj", 4004),
                gi(INTERACT_2, "Temple of Ahn'Qiraj", 4005),
                gi(INTERACT_2, "Naxxramas", 4006),
            ]),
            6010 => tail(vec![
                gi(INTERACT_2, "Ashenvale", 601),
                gi(INTERACT_2, "Darkshore", 602),
                gi(INTERACT_2, "Desolace", 603),
                gi(INTERACT_2, "Durotar", 604),
                gi(INTERACT_2, "Dustwallow Marsh", 605),
                gi(INTERACT_2, "Feralas", 606),
                gi(INTERACT_2, "Silithus", 607),
                gi(INTERACT_2, "Stonetalon Mountains", 608),
                gi(INTERACT_2, "Tanaris", 609),
                gi(INTERACT_2, "The Barrens", 610),
                gi(INTERACT_2, "Thousand Needles", 611),
                gi(INTERACT_2, "Winterspring", 612),
            ]),
            6020 => tail(vec![
                gi(INTERACT_2, "Arathi Highlands", 613),
                gi(INTERACT_2, "Badlands", 614),
                gi(INTERACT_2, "Dun Morogh", 615),
                gi(INTERACT_2, "Duskwood", 616),
                gi(INTERACT_2, "Eastern Plaguelands", 617),
                gi(INTERACT_2, "Elwynn Forest", 618),
                gi(INTERACT_2, "Hillsbrad Foothills", 619),
                gi(INTERACT_2, "Redridge Mountains", 620),
                gi(INTERACT_2, "Silverpine Forest", 621),
                gi(TALK, "[More] ->", 6021),
            ]),
            6021 => {
                let mut items = vec![
                    gi(INTERACT_2, "Stranglethorn Vale", 622),
                    gi(INTERACT_2, "Swamp of Sorrows", 623),
                    gi(INTERACT_2, "The Hinterlands", 624),
                    gi(INTERACT_2, "Tirisfal Glades", 625),
                    gi(INTERACT_2, "Westfall", 626),
                    gi(INTERACT_2, "Wetlands", 627),
                    gi(TALK, "<- [Back]", 6020),
                ];
                items.extend(back_main.clone());
                items
            }
            100 => main,
            _ => Vec::new(),
        };
        GossipMenuData {
            text_id: DEFAULT_GOSSIP_MESSAGE,
            items,
        }
    }

    /// `GossipSelect_TeleportNPC` (`custom/custom_creatures.cpp:550-557`):
    /// `GOSSIP_SENDER_MAIN` re-enters the menu builder; teleport arms
    /// `CLOSE_GOSSIP_MENU` + `TeleportTo` (map ids in the header comment).
    /// Actions 50-58/70-73 have no menu link in the fork (dead arms, kept
    /// fork-exact); action 74 is commented out in the fork (no-op).
    fn teleport_npc_gossip_select(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const SENDER_MAIN: u32 = 1;
        const MAP_EASTERN_KINGDOMS: u16 = 0;
        const MAP_KALIMDOR: u16 = 1;
        const MAP_BLACKROCK_DEPTHS: u16 = 230;
        const MAP_BLACKWING_LAIR: u16 = 469;
        const MAP_NAXXRAMAS: u16 = 533;
        if sender != SENDER_MAIN {
            return None;
        }
        match action {
            1 | 2 | 3 | 4 | 5 | 5551 | 101 | 6010 | 6020 | 6021 | 100 => Some(
                GossipSelectOutcome::ShowMenu(self.teleport_npc_menu(player, action)),
            ),
            // Fork action 74 (PreTBC Mall) is commented out: no-op arm.
            74 => None,
            _ => {
                let dest = match action {
                    20 => (MAP_KALIMDOR, 1437.0, -4421.0, 25.24, 1.65),
                    21 => (MAP_EASTERN_KINGDOMS, 1822.0999, 238.638_85, 60.694_81, 0.0),
                    22 => (MAP_KALIMDOR, -1_272.703_7, 116.886_49, 131.016_86, 0.0),
                    23 => (MAP_EASTERN_KINGDOMS, -8_828.231, 627.927_5, 94.055664, 0.0),
                    24 => (MAP_EASTERN_KINGDOMS, -4917.0, -955.0, 502.0, 0.0),
                    25 => (MAP_KALIMDOR, 9_962.713, 2_280.142_8, 1_341.394_4, 0.0),
                    40 => (MAP_EASTERN_KINGDOMS, 1663.517, 1_678.187_7, 120.5303, 0.0),
                    41 => (MAP_KALIMDOR, -602.1253, -4_262.421, 38.956_34, 0.0),
                    42 => (MAP_KALIMDOR, -2_914.17, -266.061_8, 53.658_21, 0.0),
                    43 => (
                        MAP_EASTERN_KINGDOMS,
                        -8_943.134,
                        -132.934_92,
                        83.704_27,
                        0.0,
                    ),
                    44 => (
                        MAP_EASTERN_KINGDOMS,
                        -6_231.106_4,
                        332.270_48,
                        383.153_93,
                        0.0,
                    ),
                    45 => (MAP_KALIMDOR, 10_329.919, 833.500_3, 1_326.260_6, 0.0),
                    50 => (MAP_KALIMDOR, -3_762.340_8, 1_216.537_6, 127.434_61, 0.0),
                    51 => (
                        MAP_EASTERN_KINGDOMS,
                        -7_528.554_7,
                        -1_222.907_2,
                        285.732_94,
                        0.0,
                    ),
                    52 => (MAP_EASTERN_KINGDOMS, -11_916.18, -1_190.977, 85.137_9, 0.0),
                    53 => (MAP_KALIMDOR, -4_682.391_6, -3_709.857_4, 46.792862, 0.0),
                    54 => (
                        MAP_EASTERN_KINGDOMS,
                        -7_342.27,
                        -1_096.863_9,
                        277.069_3,
                        0.0,
                    ),
                    55 => (
                        MAP_EASTERN_KINGDOMS,
                        3_121.061_8,
                        -3_689.973_4,
                        133.458_79,
                        0.0,
                    ),
                    56 => (
                        MAP_EASTERN_KINGDOMS,
                        3_160.417,
                        -4_038.750_2,
                        104.177376,
                        0.0,
                    ),
                    57 => (
                        MAP_EASTERN_KINGDOMS,
                        3_357.214_4,
                        -3_379.713_1,
                        144.780_85,
                        0.0,
                    ),
                    58 => (
                        MAP_EASTERN_KINGDOMS,
                        1_252.319_8,
                        -2_587.151_1,
                        92.886_77,
                        0.0,
                    ),
                    70 => (MAP_KALIMDOR, -7_430.070_3, 1_002.554_57, 1.249787, 0.0),
                    71 => (MAP_KALIMDOR, 1_006.426_8, -4_439.259, 11.352882, 0.0),
                    72 => (MAP_KALIMDOR, 1_922.842_7, -2_169.429_7, 94.327_4, 0.0),
                    73 => (MAP_KALIMDOR, -7_180.401_4, -3_773.328_4, 8.728_32, 0.0),
                    4015 => (MAP_EASTERN_KINGDOMS, -13261.3, 168.294, 35.0792, 1.00688),
                    4017 => (MAP_KALIMDOR, 315.721, -4743.4, 10.4867, 0.0),
                    4018 => (MAP_EASTERN_KINGDOMS, -9464.0, 62.0, 56.0, 0.0),
                    1249 => (MAP_KALIMDOR, -722.53, -2226.30, 16.94, 2.71),
                    1250 => (MAP_EASTERN_KINGDOMS, -11212.04, 1658.58, 25.67, 1.45),
                    1251 => (MAP_EASTERN_KINGDOMS, -254.47, 1524.68, 76.89, 1.56),
                    1252 => (MAP_KALIMDOR, 4254.58, 664.74, -29.04, 1.97),
                    1254 => (MAP_KALIMDOR, -4484.04, -1739.40, 86.47, 1.23),
                    1256 => (MAP_KALIMDOR, -4645.08, -2470.85, 85.53, 4.39),
                    1257 => (MAP_EASTERN_KINGDOMS, 2843.89, -693.74, 139.32, 5.11),
                    1258 => (MAP_EASTERN_KINGDOMS, -6119.70, -2957.30, 204.11, 0.03),
                    1259 => (MAP_KALIMDOR, -6839.39, -2911.03, 8.87, 0.41),
                    1260 => (MAP_KALIMDOR, -1433.33, 2955.34, 96.21, 4.82),
                    1261 => (MAP_EASTERN_KINGDOMS, -10346.92, -3851.90, -43.41, 6.09),
                    1262 => (MAP_EASTERN_KINGDOMS, -7301.03, -913.19, 165.37, 0.08),
                    1263 => (MAP_KALIMDOR, -3982.47, 1127.79, 161.02, 0.05),
                    1264 => (MAP_EASTERN_KINGDOMS, -7535.43, -1212.04, 285.45, 5.29),
                    1265 => (MAP_EASTERN_KINGDOMS, 3263.54, -3379.46, 143.59, 0.0),
                    1266 => (MAP_EASTERN_KINGDOMS, 1219.01, -2604.66, 85.61, 0.50),
                    4000 => (MAP_EASTERN_KINGDOMS, -11916.7, -1212.82, 92.2868, 4.6095),
                    4001 => (MAP_KALIMDOR, -4707.44, -3726.82, 54.6723, 3.8),
                    4002 => (
                        MAP_BLACKROCK_DEPTHS,
                        1_121.451_2,
                        -454.316_77,
                        -101.329_54,
                        3.5,
                    ),
                    4003 => (MAP_BLACKWING_LAIR, -7665.55, -1102.49, 400.679, 0.0),
                    4004 => (MAP_KALIMDOR, -8_409.032, 1_498.830_9, 27.361542, 2.497567),
                    4005 => (MAP_KALIMDOR, -8_245.838, 1_983.736_2, 129.071_69, 0.936195),
                    4006 => (MAP_NAXXRAMAS, 3005.87, -3435.0, 293.89, 0.0),
                    601 => (MAP_KALIMDOR, 3469.43, 847.62, 6.36476, 0.0),
                    602 => (MAP_KALIMDOR, 6207.5, -152.833, 80.8185, 0.0),
                    603 => (MAP_KALIMDOR, -93.1614, 1691.15, 90.0649, 0.0),
                    604 => (MAP_KALIMDOR, 341.42, -4684.7, 31.9493, 0.0),
                    605 => (MAP_KALIMDOR, -3463.26, -4123.13, 18.1043, 0.0),
                    606 => (MAP_KALIMDOR, -4458.93, 243.415, 65.6136, 0.0),
                    607 => (MAP_KALIMDOR, -6824.15, 821.273, 50.6675, 0.0),
                    608 => (MAP_KALIMDOR, 1145.00, 85.664, -6.64, 0.0),
                    609 => (MAP_KALIMDOR, -7373.69, -2950.2, 11.7598, 0.0),
                    610 => (MAP_KALIMDOR, -456.263, -2652.7, 96.615, 0.0),
                    611 => (MAP_KALIMDOR, -4941.66, -1595.42, -33.07, 0.0),
                    612 => (MAP_KALIMDOR, 6107.62, -4181.6, 853.322, 0.0),
                    613 => (MAP_EASTERN_KINGDOMS, -1544.93, -2495.01, 54.11, 0.0),
                    614 => (MAP_EASTERN_KINGDOMS, -6084.95, -3328.44, 253.76, 0.0),
                    615 => (MAP_EASTERN_KINGDOMS, -5660.33, 755.299, 390.605, 0.0),
                    616 => (MAP_EASTERN_KINGDOMS, -10879.85, -327.06, 37.78, 0.0),
                    617 => (MAP_EASTERN_KINGDOMS, 2280.12, -5313.16, 87.74, 0.0),
                    618 => (MAP_EASTERN_KINGDOMS, -9545.78, -51.81, 56.72, 0.0),
                    619 => (MAP_EASTERN_KINGDOMS, -436.03, -583.27, 53.58, 0.0),
                    620 => (MAP_EASTERN_KINGDOMS, -9633.80, -1875.09, 68.39, 0.0),
                    621 => (MAP_EASTERN_KINGDOMS, 641.48, 1297.84, 85.45, 0.0),
                    622 => (MAP_EASTERN_KINGDOMS, -11393.77, -288.94, 58.99, 0.0),
                    623 => (MAP_EASTERN_KINGDOMS, -10522.55, -3028.30, 21.79, 0.0),
                    624 => (MAP_EASTERN_KINGDOMS, 155.14, -2014.95, 122.55, 0.0),
                    625 => (MAP_EASTERN_KINGDOMS, 2255.5, 288.511, 35.1138, 0.0),
                    626 => (MAP_EASTERN_KINGDOMS, -10919.09, 998.47, 35.47, 0.0),
                    627 => (MAP_EASTERN_KINGDOMS, -4086.36, -2610.95, 47.0143, 0.0),
                    _ => return None,
                };
                self.teleport_to_location(
                    player,
                    WorldLocation {
                        map_id: dest.0,
                        x: dest.1,
                        y: dest.2,
                        z: dest.3,
                        o: dest.4,
                    },
                );
                Some(GossipSelectOutcome::Close)
            }
        }
    }

    /// `GossipHello_EnchantNPC` (`custom/custom_creatures.cpp:620-632`): slot
    /// picker (chest 4, back 14, wrists 8, hands 9, feet 7, mainhand 15,
    /// offhand 16; `game/Objects/Player.h:481-500`).
    /// UNMODELED: menu text `GetGossipTextId(creature)` (no creature
    /// gossip-text lookup on Map; 0 used).
    fn enchant_npc_gossip_hello(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const INTERACT: u8 = 5;
        const SENDER_MAIN: u32 = 1;
        const EQUIPMENT_SLOT_CHEST: u32 = 4;
        const EQUIPMENT_SLOT_FEET: u32 = 7;
        const EQUIPMENT_SLOT_WRISTS: u32 = 8;
        const EQUIPMENT_SLOT_HANDS: u32 = 9;
        const EQUIPMENT_SLOT_BACK: u32 = 14;
        const EQUIPMENT_SLOT_MAINHAND: u32 = 15;
        const EQUIPMENT_SLOT_OFFHAND: u32 = 16;
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: vec![
                GossipItem {
                    icon: INTERACT,
                    text: "Chest".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_CHEST,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Cloak".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_BACK,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Bracers".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_WRISTS,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Gloves".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_HANDS,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Boots".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_FEET,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Mainhand".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_MAINHAND,
                },
                GossipItem {
                    icon: INTERACT,
                    text: "Offhand".to_string(),
                    sender: SENDER_MAIN,
                    action: EQUIPMENT_SLOT_OFFHAND,
                },
            ],
        })
    }

    /// `GossipSelect_EnchantNPC` (`custom/custom_creatures.cpp:634-873`):
    /// slot actions (< 20) rebuild the enchant picker; enchant actions apply
    /// the enchant id and close. Enchant-action ids (`cpp:559-599`):
    /// 20-32 weapons, 33-36 offhand, 37-38 chest, 39-43 cloak, 44-48 bracer,
    /// 49-54 gloves, 55-57 boots.
    fn enchant_npc_gossip_select(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const INTERACT: u8 = 5;
        const SENDER_MAIN: u32 = 1;
        const DEFAULT_GOSSIP_MESSAGE: u32 = 16777215;
        const EQUIPMENT_SLOT_CHEST: u32 = 4;
        const EQUIPMENT_SLOT_FEET: u32 = 7;
        const EQUIPMENT_SLOT_WRISTS: u32 = 8;
        const EQUIPMENT_SLOT_HANDS: u32 = 9;
        const EQUIPMENT_SLOT_BACK: u32 = 14;
        const EQUIPMENT_SLOT_MAINHAND: u32 = 15;
        const EQUIPMENT_SLOT_OFFHAND: u32 = 16;
        if sender != SENDER_MAIN {
            return None;
        }
        let gi = |text: &str, action: u32| GossipItem {
            icon: INTERACT,
            text: text.to_string(),
            sender: SENDER_MAIN,
            action,
        };
        if action < 20 {
            let items = match action {
                a if a == EQUIPMENT_SLOT_CHEST => {
                    vec![gi("Greater Stats", 37), gi("Greater Health", 38)]
                }
                a if a == EQUIPMENT_SLOT_BACK => vec![
                    gi("Agility", 42),
                    gi("Armor", 41),
                    gi("Dodge", 39),
                    gi("Subtlety", 40),
                    gi("Greater Resistance", 43),
                ],
                a if a == EQUIPMENT_SLOT_WRISTS => vec![
                    gi("Stamina", 44),
                    gi("Strength", 45),
                    gi("Healing", 46),
                    gi("Intellect", 47),
                    gi("MP5", 48),
                ],
                a if a == EQUIPMENT_SLOT_HANDS => vec![
                    gi("Strength", 49),
                    gi("Agility", 50),
                    gi("Fire Power", 51),
                    gi("Frost Power", 52),
                    gi("Shadow Power", 53),
                    gi("Healing", 54),
                ],
                a if a == EQUIPMENT_SLOT_FEET => {
                    vec![gi("Stamina", 57), gi("Minor Speed", 56), gi("Agility", 55)]
                }
                a if a == EQUIPMENT_SLOT_MAINHAND => vec![
                    gi("Crusader", 22),
                    gi("1H Strength", 27),
                    gi("1H Agility", 28),
                    gi("2H Agility", 21),
                    gi("2H INT", 31),
                    gi("2H Spirit", 32),
                    gi("Spellpower", 29),
                    gi("Healing", 30),
                    gi("Lifesteal", 23),
                    gi("Fiery", 24),
                    gi("Icy", 25),
                    gi("Demonslaying", 26),
                ],
                a if a == EQUIPMENT_SLOT_OFFHAND => vec![
                    gi("Spirit", 33),
                    gi("Stamina", 34),
                    gi("Frost Resistance", 35),
                    gi("Shield Spike", 36),
                ],
                _ => return None,
            };
            return Some(GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id: DEFAULT_GOSSIP_MESSAGE,
                items,
            }));
        }
        let enchant_id = match action {
            20 => 1896,
            21 => 2646,
            31 => 2568,
            32 => 2567,
            22 => 1900,
            27 => 2563,
            28 => 2564,
            29 => 2504,
            30 => 2505,
            23 => 1898,
            25 => 1894,
            24 => 803,
            26 => 912,
            33 => 1890,
            35 => 926,
            34 => 929,
            36 => 1704,
            37 => 1891,
            38 => 1892,
            39 => 2622,
            40 => 2621,
            41 => 1889,
            42 => 849,
            43 => 1888,
            44 => 1886,
            45 => 1885,
            46 => 2566,
            47 => 1883,
            48 => 2565,
            49 => 927,
            50 => 2564,
            51 => 2616,
            52 => 2615,
            53 => 2614,
            54 => 2617,
            55 => 904,
            56 => 911,
            57 => 929,
            _ => return None,
        };
        // UNMODELED: GetItemByPos(INVENTORY_SLOT_BAG_0 = 255, slot) +
        // 2H-weapon subclass check (axe2/mace2/sword2/polearm/staff) and shield
        // subclass check + SendNotification failures (no equipped-item or
        // session-notification helpers on Map).
        // UNMODELED: Enchant(item, id) — Clear/SetEnchantment(PERM_SLOT = 0)
        // (no item-enchant helper on Map).
        let _ = enchant_id;
        Some(GossipSelectOutcome::Close)
    }

    /// `GossipHello_ProfessionNPC` (`custom/custom_creatures.cpp:928-949`):
    /// profession picker, icon 5 (`GOSSIP_ICON_INTERACT_2`).
    /// UNMODELED: menu text `GetGossipTextId(creature)` (0 used).
    fn profession_npc_gossip_hello(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const INTERACT_2: u8 = 5;
        const SENDER_MAIN: u32 = 1;
        let gi = |text: &str, action: u32| GossipItem {
            icon: INTERACT_2,
            text: text.to_string(),
            sender: SENDER_MAIN,
            action,
        };
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: vec![
                gi("Alchemy", 1),
                gi("Blacksmithing", 2),
                gi("Leatherworking", 3),
                gi("Tailoring", 4),
                gi("Engineering", 5),
                gi("Enchanting", 6),
                gi("Herbalism", 9),
                gi("Skinning", 10),
                gi("Mining", 11),
                gi("First Aid", 12),
                gi("Fishing", 13),
                gi("Cooking", 14),
            ],
        })
    }

    /// `GossipSelect_ProfessionNPC` (`custom/custom_creatures.cpp:964-1023`,
    /// via `CompleteLearnProfession` + `LearnAllRecipesInProfession`, cpp
    /// 909-962): learns the picked profession at 300/300 with all recipes,
    /// then closes. Skill ids (`game/SharedDefines.h`): alchemy 171,
    /// blacksmithing 164, leatherworking 165, tailoring 197, engineering 202,
    /// enchanting 333, herbalism 182, skinning 393, mining 186, first aid 129,
    /// fishing 356, cooking 185.
    fn profession_npc_gossip_select(
        &mut self,
        _player: ObjectGuid,
        _npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const SKILL_BY_ACTION: [u32; 15] = [
            0, 171, 164, 165, 197, 202, 333, 0, 0, 182, 393, 186, 129, 356, 185,
        ];
        let action_idx = action as usize;
        if action_idx >= SKILL_BY_ACTION.len() || SKILL_BY_ACTION[action_idx] == 0 {
            // Fork actions 7/8 are empty arms; all arms then CLOSE_GOSSIP_MENU.
            return Some(GossipSelectOutcome::Close);
        }
        // UNMODELED: HasSkill check + CompleteLearnProfession/LearnAllRecipes-
        // InProfession — SetSkill(300/300) + recipe scan (GetMaxSkillLineAbility-
        // Id/SkillLineAbility/LearnSpell over class/race masks) have no Map
        // equivalent.
        let _ = SKILL_BY_ACTION[action_idx];
        Some(GossipSelectOutcome::Close)
    }

    /// `GossipHello_PremadeGearNPC` (`custom/custom_creatures.cpp:1031-1043`):
    /// one item per gear template of the player's class.
    /// UNMODELED: `sObjectMgr.GetPlayerPremadeGearTemplates()` (no premade
    /// template store on Map); empty menu keeps the fork `return true`.
    /// UNMODELED: menu text `GetGossipTextId(creature)` (0 used).
    fn premade_gear_npc_gossip_hello(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: Vec::new(),
        })
    }

    /// `GossipSelect_PremadeGearNPC` (`custom/custom_creatures.cpp:1045-1051`):
    /// lightning visual (spell 24240) + template apply, then close.
    fn premade_gear_npc_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const SPELL_LIGHTNING_VISUAL: u32 = 24240;
        self.send_spell_go_simple(npc, SPELL_LIGHTNING_VISUAL, player, 0);
        // UNMODELED: sObjectMgr.ApplyPremadeGearTemplateToPlayer(action,
        // player) (no premade template store on Map).
        let _ = action;
        Some(GossipSelectOutcome::Close)
    }

    /// `GossipHello_PremadeSpecNPC` (`custom/custom_creatures.cpp:1053-1065`):
    /// one item per spec template of the player's class.
    /// UNMODELED: `sObjectMgr.GetPlayerPremadeSpecTemplates()` (no premade
    /// template store on Map); empty menu keeps the fork `return true`.
    /// UNMODELED: menu text `GetGossipTextId(creature)` (0 used).
    fn premade_spec_npc_gossip_hello(
        &mut self,
        _player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        Some(GossipMenuData {
            text_id: self.gossip_text_id(npc),
            items: Vec::new(),
        })
    }

    /// `GossipSelect_PremadeSpecNPC` (`custom/custom_creatures.cpp:1067-1073`):
    /// lightning visual (spell 24240) + template apply, then close.
    fn premade_spec_npc_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> Option<GossipSelectOutcome> {
        const SPELL_LIGHTNING_VISUAL: u32 = 24240;
        self.send_spell_go_simple(npc, SPELL_LIGHTNING_VISUAL, player, 0);
        // UNMODELED: sObjectMgr.ApplyPremadeSpecTemplateToPlayer(action,
        // player) (no premade template store on Map).
        let _ = action;
        Some(GossipSelectOutcome::Close)
    }
    // ---- lane go_quest.rs ----
    /// `GOHello_scarab_gong` (silithus.cpp:2184-2193): questgiver-type gate,
    /// then the fork sends the prepared quest menu. Always true past the gate.
    fn go_scarab_gong_hello(&mut self, player: ObjectGuid, go: ObjectGuid) -> bool {
        const GAMEOBJECT_TYPE_QUESTGIVER: u32 = 2;
        let go_type = self
            .gameobjects
            .get(&go)
            .map_or(u32::MAX, |g| g.info.go_type);
        if go_type != GAMEOBJECT_TYPE_QUESTGIVER {
            return false;
        }
        // UNMODELED: player->PrepareQuestMenu(go) + player->SendPreparedQuest(go)
        // (no quest-menu helpers on Map).
        let _ = player;
        true
    }

    /// `QuestAcceptGO_crystalline_tear` (silithus.cpp:2006-2029): quest 8519
    /// summons Anachronos at the tear unless already nearby; the scene start
    /// is creature-AI owned.
    fn go_crystalline_tear_accept(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        const QUEST_A_PAWN_ON_THE_ETERNAL_BOARD: u32 = 8519;
        const NPC_ANACHRONOS_THE_ANCIENT: u32 = 15381;
        const TEMPSUMMON_CORPSE_DESPAWN: u32 = 5;
        if quest_id != QUEST_A_PAWN_ON_THE_ETERNAL_BOARD {
            return;
        }
        if self
            .nearest_creature_with_entry(go, NPC_ANACHRONOS_THE_ANCIENT, 180.0)
            .is_some()
        {
            return;
        }
        let Some(pos) = self.object_position(go) else {
            return;
        };
        let at = Position::new(pos.x, pos.y, pos.z, 3.75);
        if self
            .summon_creature(
                NPC_ANACHRONOS_THE_ANCIENT,
                at,
                player,
                TEMPSUMMON_CORPSE_DESPAWN,
                0,
            )
            .is_none()
        {}
        // UNMODELED: anachronosAI->m_uiPlayerGUID = player guid + BeginScene()
        // (escort/scene sequencing lives in creature AI, done elsewhere).
    }

    /// `QuestAcceptGO_beached_sea` (darkshore.cpp:889-913, script
    /// "`go_beached_quest")`: quest id selects which beach-found item to grant.
    /// The fork always returns true (even with no item match).
    fn go_beached_quest_accept(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        let id: u32 = if quest_id == 4722 || quest_id == 4727 || quest_id == 4732 {
            12289
        } else if quest_id == 4725 || quest_id == 4731 {
            12292
        } else if quest_id == 4723 || quest_id == 4728 || quest_id == 4730 || quest_id == 4733 {
            12242
        } else {
            0
        };
        if id == 0 {
            return;
        }
        // UNMODELED: player->CanStoreNewItem + StoreNewItem(id, 1) +
        // SendNewItem (no item-grant helpers on Map). Fork grants count 1.
        let _ = (player, go);
    }

    /// `QuestRewarded_go_helcular_s_grave` (hillsbrad_foothills.cpp:31-42):
    /// summon Helcular (2433) at the grave unless already spawned. The fork
    /// tracks the spawn via GO-AI-stored guid; here the map-wide entry lookup
    /// stands in (no GO AI state in this lane).
    fn go_helcular_s_grave_rewarded(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        const NPC_HELCULAR: u32 = 2433;
        const TEMPSUMMON_DEAD_DESPAWN: u32 = 7;
        let _ = (player, quest_id);
        if self.find_creature_by_entry(NPC_HELCULAR).is_some() {
            return;
        }
        let at = Position::new(-741.982, -621.186, 18.3853, 2.05043);
        let _ = self.summon_creature(NPC_HELCULAR, at, go, TEMPSUMMON_DEAD_DESPAWN, 0);
    }

    /// `QuestRewarded_go_dusty_rug` (hillsbrad_foothills.cpp:145-150): kicks
    /// the rug step-machine (`go_dusty_rugAI::StartEvent`, cpp 129-141).
    fn go_dusty_rug_rewarded(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        // UNMODELED: go_dusty_rugAI::StartEvent step machine (needs GO AI tick
        // state; no GO AI in this lane). Fork-exact sequence for the driver:
        // StartEvent sets step=1, timer=2000, keg RespawnTime 120 + Refresh.
        // NPC_CAPTURED_FARMER = 2284, GO_TAINTED_KEG = 1729,
        // GO_TAINTED_KEG_SMOKE = 1730. Step 1 (timer 4500): farmers in 30.0f
        // grid move to keg contact point (MOVE_PATHFINDING). Step 2 (timer
        // 2000): front farmer UNIT_STAND_STATE_KNEEL. Step 3 (timer 20000):
        // SummonGameObject(1730, keg x/y/z+1, 120s), front farmer
        // UNIT_STAND_STATE_STAND, then DealDamage(self-kill) each farmer.
        // Step 4: step=0, timer=0.
        let _ = (player, go, quest_id);
    }

    /// `QuestRewarded_go_corrupted_plant` (felwood.cpp:534-540): cleanse the
    /// plant (`PlantQuestRewarded`, cpp 516-529). Entry pairing is pure and
    /// ported; the GO summon/despawn half has no Map helper.
    fn go_corrupted_plant_rewarded(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        /// Corrupted->cleansed entry pairs (felwood.cpp:492).
        const PLANT_PAIRS: [[u32; 2]; 30] = [
            [164885, 164881],
            [173324, 173325],
            [174608, 174609],
            [174684, 174685],
            [164886, 164882],
            [171939, 171940],
            [171942, 171943],
            [174594, 174612],
            [174595, 174613],
            [174596, 174614],
            [174598, 174615],
            [174712, 174714],
            [174713, 174715],
            [164888, 164883],
            [173284, 174622],
            [174605, 174623],
            [174606, 174624],
            [174607, 174625],
            [174686, 174687],
            [164887, 164884],
            [173327, 173326],
            [174599, 174616],
            [174600, 174617],
            [174601, 174618],
            [174602, 174619],
            [174603, 174620],
            [174604, 174621],
            [174708, 174710],
            [174709, 174711],
            [0, 0],
        ];
        /// `CLEANSED_PLANT_RESPAWN_TIMER` = 25 * 60 (felwood.cpp:501).
        const CLEANSED_PLANT_RESPAWN_SECS: u32 = 1500;
        let _ = (player, quest_id);
        // Fork returns false; the Rust rewarded host returns ().
        let Some(entry) = self.object_entry(go) else {
            return;
        };
        let Some(cleansed_entry) = PLANT_PAIRS.iter().find_map(|p| {
            if p[0] == entry || p[1] == entry {
                Some(p[1])
            } else {
                None
            }
        }) else {
            return;
        };
        // UNMODELED: SummonGameObject(cleansed_entry, go x/y/z, respawn
        // CLEANSED_PLANT_RESPAWN_SECS) + SetSpawnedByDefault(true) +
        // SetRespawnDelay(GOData) + me->Despawn() + DestroyForNearbyPlayers()
        // + cleansedGuid tracking/UpdateAI removal (no GO-summon helper).
        let _ = (cleansed_entry, CLEANSED_PLANT_RESPAWN_SECS);
    }

    /// `QuestRewarded_go_broken_trap` (instance_dire_maul.cpp:806-821): quest
    /// 1193 marks the trap done, locks interaction, swaps in the fixed trap.
    fn go_broken_trap_rewarded(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        const QUEST_A_BROKEN_TRAP: u32 = 1193;
        const TYPE_BROKEN_TRAP: u32 = 7;
        const ENCOUNTER_DONE: u32 = 3;
        const GO_FLAG_NO_INTERACT: u32 = 0x10;
        // Fork returns false on quest mismatch; the Rust host returns ().
        if quest_id != QUEST_A_BROKEN_TRAP {
            return;
        }
        let _ = player;
        let _ = self.instance_set_data(TYPE_BROKEN_TRAP, ENCOUNTER_DONE);
        let flags = self.object_get_field(go, offsets::GAMEOBJECT_FLAGS);
        let _ = self.object_set_field(
            go,
            u32::from(offsets::GAMEOBJECT_FLAGS),
            flags | GO_FLAG_NO_INTERACT,
        );
        // UNMODELED: player->SummonGameObject(GO_FIXED_TRAP = 179512, go
        // x/y/z + orientation, 0,0,0,0, respawn 43200s) + pGo->Delete()
        // (no GO-summon/delete helpers on Map).
    }

    /// `QuestRewarded_scarab_gong` (silithus.cpp:2195-2213): quest 8743 bumps
    /// the gong counter; the first bang opens the AQ gates.
    fn go_scarab_gong_rewarded(&mut self, player: ObjectGuid, go: ObjectGuid, quest_id: u32) {
        const QUEST_BANG_A_GONG: u32 = 8743;
        // Fork returns false on quest mismatch; the Rust host returns ().
        if quest_id != QUEST_BANG_A_GONG {
            return;
        }
        // UNMODELED: sObjectMgr saved variables VAR_WE_GONG_BANG_TIMES = 30053
        // (read/inc/write) + VAR_WE_GONG_TIME = 30052, and on first bang
        // scarab_gongAI::BeginAQOpeningEvent (GO AI event state; no GO AI in
        // this lane). Fork-exact sequence for the driver: nearest GOs
        // GO_AQ_BARRIER = 176146, GO_AQ_GATE_RUNES = 176148,
        // GO_AQ_GATE_ROOTS = 176147 (radius 150); abort Anachronos scene if
        // NPC_ANACHRONOS_THE_ANCIENT = 15381 within 180.0f (+8000ms timer);
        // skip when barrier GO_STATE_ACTIVE; SendBroadcastTextToWorld
        // GLOBAL_TEXT_CHAMPION = 11427; stages STAGE_OPEN_GATES = 0 /
        // STAGE_WAR = 1 / STAGE_RESET = 2; step sounds SOUND_ROOTS_OPEN = 7114
        // (timer 5000), SOUND_RUNES_OPEN = 7116 (timer 8000),
        // SOUND_DOOR_OPEN = 7115 then war stage after 10000; war stage sets
        // gong time + world-event timer nudge; reset restores READY states.
        let _ = (player, go);
    }
}

// ---- `battleground_alterac.cpp` gossip/quest hooks (Tier 3 Slice 3
// step 5b): the Alterac Valley challenge-invocation consumers of the
// `BattleGroundAV` engine state in `bg_av.rs`. Escort-dependent arms
// (`AV_NpcEventAI::Start`, troll/ram rider marches) stay UNMODELED
// behind the Slice 2-recorded `npc_escortAI` framework remainder.

/// `AV_NPC_QUARTERMASTER_A` (Stormpike quartermaster entry).
const AVC_NPC_QUARTERMASTER_A: u32 = 13236;
/// `AV_NPC_QUARTERMASTER` (Frostwolf quartermaster entry).
const AVC_NPC_QUARTERMASTER: u32 = 12096;
/// `AV_NPC_ENTRY_MURGOT` (smith, horde).
const AVC_NPC_MURGOT: u32 = 13257;
/// `AV_NPC_ENTRY_REGZAR` (smith, alliance).
const AVC_NPC_REGZAR: u32 = 13176;
/// `AV_NPC_MARSHAL_TERAVAINE`.
const AVC_NPC_TERAVAINE: u32 = 13449;
/// `AV_NPC_WARMASTER_CMD` (warmaster commander entry).
const AVC_NPC_WARMASTER_CMD: u32 = 13446;
/// `AV_NPC_ENTRY_GUSE`.
const AVC_NPC_GUSE: u32 = 13179;
/// `AV_NPC_ENTRY_JEZTOR`.
const AVC_NPC_JEZTOR: u32 = 13180;
/// `AV_NPC_ENTRY_MULVERICK`.
const AVC_NPC_MULVERICK: u32 = 13181;
/// `AV_NPC_ENTRY_SLIDORE`.
const AVC_NPC_SLIDORE: u32 = 13438;
/// `AV_NPC_ENTRY_ICHMAN`.
const AVC_NPC_ICHMAN: u32 = 13437;
/// `AV_NPC_ENTRY_VIPORE`.
const AVC_NPC_VIPORE: u32 = 13439;
/// `AV_NPC_WOLFRIDER_CMD`.
const AVC_NPC_WOLFRIDER_CMD: u32 = 13441;
/// `AV_NPC_RAMRIDER_CMD`.
const AVC_NPC_RAMRIDER_CMD: u32 = 13577;
/// `AV_NPC_ARCHDRUID_RENFERAL`.
const AVC_NPC_RENFERAL: u32 = 12097;
/// `AV_NPC_PRIMALIST_THURLOGA`.
const AVC_NPC_THURLOGA: u32 = 13236;
/// `GOSSIP_ACTION_TRADE` (vendor list request).
const AVC_GOSSIP_ACTION_TRADE: u32 = 3;

impl Map {
    /// `GetReputationRank` over a faction standing
    /// (fork reps 729/730 in AV flows).
    fn av_reputation_rank(&self, player: ObjectGuid, faction: u32) -> u32 {
        const POINTS_IN_RANK: [i32; 8] = [36000, 3000, 3000, 3000, 6000, 12000, 21000, 1000];
        let standing = self.player(player).map_or(0, |p| {
            p.deferred
                .reputation
                .iter()
                .find(|r| r.faction == faction)
                .map_or(0, |r| r.standing)
        });
        let (race_mask, class_mask) = self.unit_base(player).map_or((0, 0), |u| {
            (
                1u32 << u32::from(u.race()).saturating_sub(1),
                1u32 << u32::from(u.class()).saturating_sub(1),
            )
        });
        let base = self.stores.factions.get(faction).map_or(0, |e| {
            e.index_fit_to(race_mask, class_mask)
                .map_or(0, |i| e.base_values[i])
        });
        let reputation = base + standing;
        let mut limit = 43_000i32;
        for rank in (0..8u32).rev() {
            limit -= POINTS_IN_RANK[rank as usize];
            if reputation >= limit {
                return rank;
            }
        }
        0
    }

    /// AV entry index of a creature (fork
    /// `GetEntry`), 0 on a miss.
    fn av_entry(&self, npc: ObjectGuid) -> u32 {
        self.creature(npc).map_or(0, |c| c.info.entry)
    }

    /// Whether this map hosts a live Alterac Valley.
    fn av_is_battleground(&self) -> bool {
        self.bg.as_ref().is_some_and(|b| b.bg_type_id == 1)
    }

    /// `QuestComplete_npc_AVBlood_collector`
    /// (`battleground_alterac.cpp:2478`).
    fn npc_av_blood_collector_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        _npc: ObjectGuid,
        quest_id: u32,
    ) {
        if !self.av_is_battleground() {
            return;
        }
        let Some(q) = self.stores.quests.get(quest_id).copied() else {
            return;
        };
        let (req_id, req_count) = (q.req_item_ids[0], q.req_item_counts[0]);
        let tamed = quest_id == 7001 || quest_id == 7027;
        if (req_id == 0 || req_count == 0) && !tamed {
            return;
        }
        let Some(team) = self.player(player).map(|p| p.team) else {
            return;
        };
        let faction_id = team_index(team) as u8;
        let mut delivered = req_count;
        let challenge = if tamed {
            delivered = 1;
            challenge::TAMED_CAVALRY_ASSAULT
        } else {
            match req_id {
                17306 | 17423 => challenge::BLOOD_WORLDBOSS_ASSAULT,
                17643 | 17642 => challenge::HIDE_CAVALRY_ASSAULT,
                17326 | 17502 => challenge::SOLDIER_AIR_ASSAULT,
                17327 | 17503 => challenge::LIEUTENANT_AIR_ASSAULT,
                17328 | 17504 => challenge::COMMANDER_AIR_ASSAULT,
                17522 => challenge::IRONDEEP_GROUND_ASSAULT,
                17542 => challenge::COLDTOOTH_GROUND_ASSAULT,
                _ => return,
            }
        };
        self.av_add_challenge_counter(faction_id, challenge, delivered);
        if challenge == challenge::BLOOD_WORLDBOSS_ASSAULT
            && self.av_world_boss_challenge_ready(faction_id)
        {
            self.av_reset_world_boss_challenge(faction_id);
            self.av_set_player_go_status(faction_id, assault::WORLDBOSS_ASSAULT, true);
            // UNMODELED: `AV_NpcEventAI::Start` escort + flag flips on
            // the quest giver (no escort framework on Map).
        }
    }
}

impl Map {
    /// `QuestComplete_AV_npc_troops_chief`
    /// (`battleground_alterac.cpp:3226`).
    fn npc_av_troops_chief_quest_rewarded(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        quest_id: u32,
    ) {
        let Some(q) = self.stores.quests.get(quest_id).copied() else {
            return;
        };
        if q.req_item_ids[0] == 0 || q.req_item_counts[0] == 0 {
            return;
        }
        let entry = self.av_entry(npc);
        if entry == AVC_NPC_TERAVAINE || entry == AVC_NPC_WARMASTER_CMD {
            // `HandleEmote(EMOTE_ONESHOT_SHOUT 22)`.
            self.handle_emote_command(npc, 22);
        }
        if !self.av_is_battleground() {
            return;
        }
        let Some(team) = self.player(player).map(|p| p.team) else {
            return;
        };
        let faction_id = team_index(team) as u8;
        if quest_id == 6901 || quest_id == 6846 {
            self.av_reset_ground_challenge(faction_id);
            self.av_set_player_go_status(faction_id, assault::GROUND_ASSAULT, true);
            // UNMODELED: `AV_npc_troops_chief_EventAI::Start` escort +
            // flag flips + SetWalk(true) (no escort framework on Map).
        }
    }
}

impl Map {
    /// One CHAT `ADD_GOSSIP_ITEM` of the blood collector.
    fn av_bc_chat_item(&self, player: ObjectGuid, text_id: u32, action: u32) -> GossipItem {
        GossipItem {
            icon: GOSSIP_ICON_CHAT,
            text: self.gossip_broadcast_text(text_id, player),
            sender: GOSSIP_SENDER_MAIN,
            action,
        }
    }

    /// The vendor `ADD_GOSSIP_ITEM` (`GOSSIP_TEXT_BROWSE_GOODS`).
    fn av_bc_vendor_item(&self, player: ObjectGuid) -> GossipItem {
        GossipItem {
            icon: 1, // GOSSIP_ICON_VENDOR
            text: self.gossip_broadcast_text(3370, player),
            sender: GOSSIP_SENDER_MAIN,
            action: AVC_GOSSIP_ACTION_TRADE,
        }
    }

    /// `HasFlag(UNIT_NPC_FLAGS, UNIT_NPC_FLAG_QUESTGIVER)`.
    fn av_is_questgiver(&self, npc: ObjectGuid) -> bool {
        self.object_get_field(npc, offsets::UNIT_NPC_FLAGS) & 0x02 != 0
    }

    /// `IsVendor` (template npc flags vendor bit).
    fn av_is_vendor(&self, npc: ObjectGuid) -> bool {
        self.object_get_field(npc, offsets::UNIT_NPC_FLAGS) & 0x04 != 0
    }

    /// `SetFlag`/`RemoveFlag(UNIT_NPC_FLAGS, QUESTGIVER)`.
    fn av_set_questgiver_flag(&mut self, npc: ObjectGuid, set: bool) {
        let flags = self.object_get_field(npc, offsets::UNIT_NPC_FLAGS);
        let next = if set { flags | 0x02 } else { flags & !0x02 };
        let _ = self.object_set_field(npc, u32::from(offsets::UNIT_NPC_FLAGS), next);
    }

    /// `GetDistance(x,y,z)` (3D), far on a miss.
    fn av_distance(&self, npc: ObjectGuid, x: f32, y: f32, z: f32) -> f32 {
        self.object_position(npc).map_or(f32::MAX, |p| {
            let (dx, dy, dz) = (p.x - x, p.y - y, p.z - z);
            (dx * dx + dy * dy + dz * dz).sqrt()
        })
    }

    /// `GossipHello_npc_AVBlood_collector`
    /// (`battleground_alterac.cpp:2611-3058`).
    #[allow(clippy::too_many_lines)]
    fn npc_av_blood_collector_gossip_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<GossipMenuData> {
        const DEF: u32 = GOSSIP_ACTION_INFO_DEF;
        let entry = self.av_entry(npc);
        let team = self.player(player).map_or(Team::None, |p| p.team);
        let faction_id = team_index(team) as u8;
        let alliance = team == Team::Alliance;
        let mut items: Vec<GossipItem> = Vec::new();
        let mut objective_reached = false;
        let (mut global_soldier, mut global_lieutenant, mut global_commander) =
            (false, false, false);
        let mut ui_message: u32 = 0;
        let mut ui_message_global: u32 = 0;
        let mut challenge_v: usize = 0;
        let mut min_rep: u32 = 0;

        let is_qm = entry == AVC_NPC_QUARTERMASTER_A || entry == AVC_NPC_QUARTERMASTER;
        if is_qm {
            challenge_v = challenge::NB_CHALLENGES;
            // UNMODELED: `PrepareQuestMenu` (quest items have no menu model on Map).
            let (mut actual, mut actual2) = (0u32, 0u32);
            if self.av_is_battleground() {
                actual = self.av_challenge_counter(faction_id, challenge::IRONDEEP_GROUND_ASSAULT);
                actual2 =
                    self.av_challenge_counter(faction_id, challenge::COLDTOOTH_GROUND_ASSAULT);
            }
            if actual + actual2 > 230 {
                challenge_v += 14;
            } else if actual + actual2 > 140 {
                challenge_v += 12;
            } else if actual > 210 && actual2 > 50 {
                challenge_v += 14;
            } else if actual > 110 && actual2 > 20 {
                challenge_v += 12;
            } else {
                challenge_v += 13;
            }
            items.push(self.av_bc_chat_item(player, 9128, DEF + challenge_v as u32 + 1));
        }

        if entry == AVC_NPC_MURGOT || entry == AVC_NPC_REGZAR {
            challenge_v = challenge::NB_CHALLENGES;
            // UNMODELED: `PrepareQuestMenu` (quest items have no menu model on Map).
            if self.av_is_battleground() {
                let actual = self.av_armor_resources(faction_id);
                if actual < 500 {
                    if actual > 200 {
                        challenge_v += if actual < 400 { 1 } else { 4 };
                    }
                } else if actual < 1000 {
                    if actual > 700 {
                        challenge_v += if actual < 900 { 2 } else { 5 };
                    }
                } else if actual < 1500 && actual > 1200 {
                    challenge_v += if actual < 1400 { 3 } else { 6 };
                }
                let honored = self.av_reputation_rank(player, 729) >= 5
                    || self.av_reputation_rank(player, 730) >= 5;
                match self.av_reinforcement_level(faction_id) {
                    0 => {
                        items.push(self.av_bc_chat_item(
                            player,
                            9130,
                            DEF + challenge_v as u32 + 1,
                        ));
                        if honored && actual >= 500 {
                            items.push(self.av_bc_chat_item(player, 8718, DEF + 501));
                        }
                        return Some(GossipMenuData {
                            text_id: 6073,
                            items,
                        });
                    }
                    1 => {
                        items.push(self.av_bc_chat_item(
                            player,
                            9130,
                            DEF + challenge_v as u32 + 1,
                        ));
                        if honored && actual >= 1000 {
                            items.push(self.av_bc_chat_item(player, 8719, DEF + 1001));
                        }
                        return Some(GossipMenuData {
                            text_id: 6217,
                            items,
                        });
                    }
                    2 => {
                        items.push(self.av_bc_chat_item(
                            player,
                            9130,
                            DEF + challenge_v as u32 + 1,
                        ));
                        if honored && actual >= 1500 {
                            items.push(self.av_bc_chat_item(player, 8723, DEF + 1501));
                        }
                        return Some(GossipMenuData {
                            text_id: 6218,
                            items,
                        });
                    }
                    3 => {
                        let text_id = self.gossip_text_id(npc);
                        return Some(GossipMenuData { text_id, items });
                    }
                    _ => {}
                }
            }
            // Fork returns true with nothing sent (no BG / default level).
            return None;
        }

        if matches!(entry, AVC_NPC_GUSE | AVC_NPC_JEZTOR | AVC_NPC_MULVERICK) {
            let _ = self.object_set_field(npc, u32::from(offsets::UNIT_DYNAMIC_FLAGS), 0);
            let dist = self.av_distance(npc, -1338.6, -328.16, 90.8);
            if dist > 15.0 && team == Team::Horde {
                // UNMODELED: `AV_NpcEventAI::Start` escort +
                // faction-template flips + SPAWNING/IMMUNE/PVP
                // field flags (no escort framework on Map).
                self.av_set_questgiver_flag(npc, false);
                return None;
            }
            if dist < 15.0 {
                self.av_set_questgiver_flag(npc, true);
            }
        } else if matches!(entry, AVC_NPC_SLIDORE | AVC_NPC_ICHMAN | AVC_NPC_VIPORE) {
            let dist = self.av_distance(npc, 575.116, -51.90, 37.62);
            if dist > 15.0 && alliance {
                // UNMODELED: VIPORE self-cast 5759 +
                // `AV_NpcEventAI::Start` escort + faction/flag flips.
                self.av_set_questgiver_flag(npc, false);
                return None;
            }
            self.av_set_questgiver_flag(npc, true);
        }

        if entry != AVC_NPC_TERAVAINE && self.av_is_battleground() {
            for quest_id in self.stores.quest_relations.creature_starts(entry).to_vec() {
                if quest_id == 0 {
                    continue;
                }
                let Some(q) = self.stores.quests.get(quest_id).copied() else {
                    continue;
                };
                let req = q.req_item_ids[0];
                if req == 0 {
                    continue;
                }
                let mut wrong = false;
                if quest_id == 7001 || quest_id == 7027 {
                    challenge_v = challenge::TAMED_CAVALRY_ASSAULT;
                    min_rep = self.av_min_reputation_needed(assault::CAVALRY_ASSAULT);
                } else {
                    match req {
                        17643 | 17642 => {
                            challenge_v = challenge::HIDE_CAVALRY_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::CAVALRY_ASSAULT);
                        }
                        17326 | 17502 => {
                            challenge_v = challenge::SOLDIER_AIR_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::AIR_BEACON_SOLDIER);
                        }
                        17327 | 17503 => {
                            challenge_v = challenge::LIEUTENANT_AIR_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::AIR_BEACON_LIEUTENANT);
                        }
                        17328 | 17504 => {
                            challenge_v = challenge::COMMANDER_AIR_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::AIR_BEACON_COMMANDER);
                        }
                        17522 => {
                            challenge_v = challenge::IRONDEEP_GROUND_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::GROUND_ASSAULT);
                        }
                        17542 => {
                            challenge_v = challenge::COLDTOOTH_GROUND_ASSAULT;
                            min_rep = self.av_min_reputation_needed(assault::GROUND_ASSAULT);
                        }
                        _ => {
                            wrong = true;
                            min_rep = 0;
                        }
                    }
                }
                let (actual, objective) = if wrong {
                    (0, 1)
                } else {
                    (
                        self.av_challenge_counter(faction_id, challenge_v),
                        self.av_challenge_goals(faction_id, challenge_v),
                    )
                };
                if actual >= objective {
                    match challenge_v {
                        challenge::SOLDIER_AIR_ASSAULT => {
                            objective_reached = self
                                .av_aerial_challenge_ready(faction_id, assault::AIR_BEACON_SOLDIER);
                            ui_message = if alliance { 8796 } else { 8667 };
                        }
                        challenge::LIEUTENANT_AIR_ASSAULT => {
                            objective_reached = self.av_aerial_challenge_ready(
                                faction_id,
                                assault::AIR_BEACON_LIEUTENANT,
                            );
                            ui_message = if alliance { 8799 } else { 8669 };
                        }
                        challenge::COMMANDER_AIR_ASSAULT => {
                            objective_reached = self.av_aerial_challenge_ready(
                                faction_id,
                                assault::AIR_BEACON_COMMANDER,
                            );
                            ui_message = if alliance { 8793 } else { 8671 };
                        }
                        challenge::HIDE_CAVALRY_ASSAULT | challenge::TAMED_CAVALRY_ASSAULT => {
                            objective_reached = self.av_cavalry_challenge_ready(faction_id);
                            ui_message = 8903;
                        }
                        challenge::IRONDEEP_GROUND_ASSAULT
                        | challenge::COLDTOOTH_GROUND_ASSAULT => {
                            objective_reached = self.av_ground_challenge_ready(faction_id);
                            ui_message = 9050;
                        }
                        _ => {}
                    }
                }
                if matches!(entry, AVC_NPC_GUSE | AVC_NPC_SLIDORE)
                    && self.av_aerial_challenge_ready(faction_id, assault::AIR_GLOBAL_SOLDIER)
                {
                    global_soldier = true;
                    ui_message_global = if entry == AVC_NPC_GUSE { 10341 } else { 10351 };
                }
                if matches!(entry, AVC_NPC_JEZTOR | AVC_NPC_VIPORE)
                    && self.av_aerial_challenge_ready(faction_id, assault::AIR_GLOBAL_LIEUTENANT)
                {
                    global_lieutenant = true;
                    ui_message_global = if entry == AVC_NPC_JEZTOR {
                        10343
                    } else {
                        10353
                    };
                }
                if matches!(entry, AVC_NPC_MULVERICK | AVC_NPC_ICHMAN)
                    && self.av_aerial_challenge_ready(faction_id, assault::AIR_GLOBAL_COMMANDER)
                {
                    global_commander = true;
                    ui_message_global = if entry == AVC_NPC_MULVERICK {
                        10346
                    } else {
                        10349
                    };
                }
            }
        }

        if entry == AVC_NPC_TERAVAINE {
            ui_message = 8903;
            objective_reached = true;
            challenge_v = challenge::IRONDEEP_GROUND_ASSAULT;
        }

        if objective_reached {
            // Fork resets the rep gate to `REP_NEUTRAL` here.
            let objective_min_rep: u32 = 3;
            let _ = min_rep;
            // UNMODELED: `PrepareQuestMenu` (non-Teravaine).
            if self.av_reputation_rank(player, 729) >= objective_min_rep
                || self.av_reputation_rank(player, 730) >= objective_min_rep
            {
                items.push(self.av_bc_chat_item(player, ui_message, DEF + challenge_v as u32 + 1));
                if global_soldier && matches!(entry, AVC_NPC_GUSE | AVC_NPC_SLIDORE) {
                    items.push(self.av_bc_chat_item(
                        player,
                        ui_message_global,
                        DEF + assault::AIR_GLOBAL_SOLDIER as u32 + 1 + 50,
                    ));
                } else if global_lieutenant && matches!(entry, AVC_NPC_JEZTOR | AVC_NPC_VIPORE) {
                    items.push(self.av_bc_chat_item(
                        player,
                        ui_message_global,
                        DEF + assault::AIR_GLOBAL_LIEUTENANT as u32 + 1 + 50,
                    ));
                } else if global_commander && matches!(entry, AVC_NPC_MULVERICK | AVC_NPC_ICHMAN) {
                    items.push(self.av_bc_chat_item(
                        player,
                        ui_message_global,
                        DEF + assault::AIR_GLOBAL_COMMANDER as u32 + 1 + 50,
                    ));
                }
                if is_qm {
                    if self.av_is_vendor(npc) {
                        items.push(self.av_bc_vendor_item(player));
                    }
                    return Some(GossipMenuData {
                        text_id: 6255,
                        items,
                    });
                }
                let text_id = self.gossip_text_id(npc);
                // Fork sends the menu, then `HandleEmote(EMOTE_ONESHOT_BOW 2)`.
                self.handle_emote_command(npc, 2);
                return Some(GossipMenuData { text_id, items });
            }
            // Fork: bow emote, return true, nothing sent.
            self.handle_emote_command(npc, 2);
            return None;
        }

        if self.av_is_questgiver(npc) {
            if is_qm {
                if self.av_is_vendor(npc) {
                    items.push(self.av_bc_vendor_item(player));
                }
                return Some(GossipMenuData {
                    text_id: 6255,
                    items,
                });
            }
            if (entry == AVC_NPC_RENFERAL || entry == AVC_NPC_THURLOGA) && self.av_is_battleground()
            {
                let counter =
                    self.av_challenge_counter(faction_id, challenge::BLOOD_WORLDBOSS_ASSAULT);
                let (opt, btext) = if counter >= 160 {
                    (2u32, if alliance { 8761 } else { 8645 })
                } else if counter >= 100 {
                    (1, if alliance { 8759 } else { 8643 })
                } else {
                    (0, if alliance { 8757 } else { 8641 })
                };
                items.push(self.av_bc_chat_item(
                    player,
                    btext,
                    DEF + challenge::BLOOD_WORLDBOSS_ASSAULT as u32 + 200 + opt,
                ));
            }
            // UNMODELED: `PrepareQuestMenu` (quest items have no menu model on Map).
            let text_id = self.gossip_text_id(npc);
            return Some(GossipMenuData { text_id, items });
        }
        // Fork returns true with nothing sent.
        None
    }

    /// `GossipSelect_npc_AVBlood_collector`
    /// (`battleground_alterac.cpp:3286-3556`).
    #[allow(clippy::too_many_lines)]
    fn npc_av_blood_collector_gossip_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        _sender: u32,
        action: u32,
    ) -> GossipSelectOutcome {
        const DEF: u32 = GOSSIP_ACTION_INFO_DEF;
        if action == AVC_GOSSIP_ACTION_TRADE {
            // UNMODELED: `SEND_VENDORLIST` (no vendor-list outcome on Map).
            return GossipSelectOutcome::Close;
        }
        if action < DEF + 1 {
            // Fork returns true, nothing sent.
            return GossipSelectOutcome::Close;
        }
        let entry = self.av_entry(npc);
        let is_smith = entry == AVC_NPC_MURGOT || entry == AVC_NPC_REGZAR;
        let challenge_v = (action - (DEF + 1)) as usize;
        // Fork quirk: non-smith selections with `challenge < NB + 10`
        // are closed first, but the say/emote/reset/menu below
        // still run (a menu case reopens the gossip window).
        if !is_smith && challenge_v < challenge::NB_CHALLENGES + 10 {
            let say: i32 = match entry {
                AVC_NPC_RAMRIDER_CMD => 8905,
                AVC_NPC_WOLFRIDER_CMD => 8887,
                AVC_NPC_QUARTERMASTER_A => 8907,
                AVC_NPC_QUARTERMASTER => 8913,
                AVC_NPC_JEZTOR => 10342,
                AVC_NPC_GUSE => 10339,
                AVC_NPC_MULVERICK => 10347,
                AVC_NPC_ICHMAN => 10358,
                AVC_NPC_VIPORE => 10357,
                _ => 0,
            };
            self.do_script_text(say, npc, None, -1);
            // `HandleEmote(EMOTE_ONESHOT_SHOUT 22)`.
            self.handle_emote_command(npc, 22);
        }

        let mut menu_text: Option<u32> = None;
        if self.av_is_battleground() {
            let team = self.player(player).map_or(Team::None, |p| p.team);
            let faction_id = team_index(team) as u8;
            let horde = team == Team::Horde;
            match challenge_v {
                challenge::SOLDIER_AIR_ASSAULT => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_BEACON_SOLDIER);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_BEACON_SOLDIER, true);
                    // UNMODELED: beacon item grant 17324 (horde) /
                    // 17506 (alliance) when not already held (no
                    // item-grant helper on Map).
                    let _ = horde;
                }
                challenge::LIEUTENANT_AIR_ASSAULT => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_BEACON_LIEUTENANT);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_BEACON_LIEUTENANT, true);
                    // UNMODELED: beacon item grant 17325 (horde) /
                    // 17507 (alliance) when not already held.
                }
                challenge::COMMANDER_AIR_ASSAULT => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_BEACON_COMMANDER);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_BEACON_COMMANDER, true);
                    // UNMODELED: beacon item grant 17323 (horde) /
                    // 17505 (alliance) when not already held.
                }
                c if c == assault::AIR_GLOBAL_SOLDIER + 50 => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_GLOBAL_SOLDIER);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_GLOBAL_SOLDIER, true);
                }
                c if c == assault::AIR_GLOBAL_LIEUTENANT + 50 => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_GLOBAL_LIEUTENANT);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_GLOBAL_LIEUTENANT, true);
                }
                c if c == assault::AIR_GLOBAL_COMMANDER + 50 => {
                    self.av_reset_aerial_challenge(faction_id, assault::AIR_GLOBAL_COMMANDER);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::AIR_GLOBAL_COMMANDER, true);
                }
                challenge::HIDE_CAVALRY_ASSAULT | challenge::TAMED_CAVALRY_ASSAULT => {
                    self.av_reset_cavalry_challenge(faction_id);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::CAVALRY_ASSAULT, true);
                    // UNMODELED: `AV_NpcEventAI::Start` escort +
                    // SPAWNING/IMMUNE/PVP field-flag flips.
                }
                challenge::IRONDEEP_GROUND_ASSAULT | challenge::COLDTOOTH_GROUND_ASSAULT => {
                    self.av_reset_ground_challenge(faction_id);
                    self.av_set_questgiver_flag(npc, true);
                    self.av_set_player_go_status(faction_id, assault::GROUND_ASSAULT, true);
                    // UNMODELED: assault-order grant 17442 (horde) /
                    // 17353 (alliance) when not already held.
                }
                c if c == challenge::NB_CHALLENGES => menu_text = Some(6784),
                c if c == challenge::NB_CHALLENGES + 1 => menu_text = Some(6780),
                c if c == challenge::NB_CHALLENGES + 2 => menu_text = Some(6781),
                c if c == challenge::NB_CHALLENGES + 3 => menu_text = Some(6783),
                c if c == challenge::NB_CHALLENGES + 4 => menu_text = Some(6778),
                c if c == challenge::NB_CHALLENGES + 5 => menu_text = Some(6779),
                c if c == challenge::NB_CHALLENGES + 6 => menu_text = Some(6782),
                c if c == challenge::NB_CHALLENGES + 13 => menu_text = Some(6733),
                c if c == challenge::NB_CHALLENGES + 12 => menu_text = Some(6732),
                c if c == challenge::NB_CHALLENGES + 14 => menu_text = Some(6731),
                c if c == challenge::BLOOD_WORLDBOSS_ASSAULT + 199 => {
                    menu_text = Some(if entry == AVC_NPC_THURLOGA {
                        6098
                    } else {
                        6175
                    });
                }
                c if c == challenge::BLOOD_WORLDBOSS_ASSAULT + 200 => {
                    menu_text = Some(if entry == AVC_NPC_THURLOGA {
                        6099
                    } else {
                        6176
                    });
                }
                c if c == challenge::BLOOD_WORLDBOSS_ASSAULT + 201 => {
                    menu_text = Some(if entry == AVC_NPC_THURLOGA {
                        6100
                    } else {
                        6177
                    });
                }
                500 | 1000 | 1500 => self.av_upgrade_armor(npc, player),
                _ => {}
            }
        }
        match menu_text {
            Some(text_id) => GossipSelectOutcome::ShowMenu(GossipMenuData {
                text_id,
                items: vec![],
            }),
            None => GossipSelectOutcome::Close,
        }
    }
}
