//! Alterac Valley gossip and quest hooks.

#[allow(clippy::wildcard_imports)]
use super::*;

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
    pub(crate) fn npc_av_blood_collector_quest_rewarded(
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
    pub(crate) fn npc_av_troops_chief_quest_rewarded(
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
    pub(crate) fn npc_av_blood_collector_gossip_hello(
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
    pub(crate) fn npc_av_blood_collector_gossip_select(
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
