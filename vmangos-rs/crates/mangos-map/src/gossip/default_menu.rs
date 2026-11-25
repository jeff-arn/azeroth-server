//! Default (non-script) gossip menu construction and selection.

#![allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::too_many_lines,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    clippy::match_same_arms,
    clippy::match_single_binding
)]

#[allow(clippy::wildcard_imports)]
use super::*;

impl Map {
    /// `HandleGossipHelloOpcode` non-script arm
    /// (`NPCHandler.cpp:378-382`): prepare the
    /// creature's default gossip menu. `None` when the
    /// fork returns silently (cannot interact).
    /// UNMODELED: `PauseOutOfCombatMovement`, the
    /// spirit-guide area query, and GO gossip hello
    /// (GOs reach the quest list through `Use`).
    pub fn gossip_default_hello(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
    ) -> Option<DefaultGossipMenu> {
        if !self.npc_if_can_interact_with(player, npc, 0) {
            return None;
        }
        let menu_id = self.creature(npc)?.gossip_menu_id;
        Some(self.gossip_prepare_menu(player, npc, menu_id))
    }

    /// `Player::PrepareGossipMenu` (creature source)
    /// plus the `SendPreparedGossip` text pick; the
    /// quest menu itself is assembled actor-side from
    /// `can_see_quests`.
    fn gossip_prepare_menu(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        menu_id: u32,
    ) -> DefaultGossipMenu {
        let (npc_flags, default_menu_id, info) = {
            let Some(creature) = self.creature(npc) else {
                return DefaultGossipMenu {
                    text_id: DEFAULT_GOSSIP_MESSAGE,
                    items: Vec::new(),
                    can_see_quests: false,
                    npc_flags: 0,
                    just_discovered_node: false,
                };
            };
            (
                creature
                    .unit
                    .world
                    .object
                    .values()
                    .get_u32(offsets::UNIT_NPC_FLAGS),
                creature.gossip_menu_id,
                creature.info.clone(),
            )
        };
        let default_menu = menu_id == default_menu_id;
        let mut can_see_quests = default_menu && npc_flags & NPC_FLAG_QUESTGIVER != 0;
        let mut options = self.stores.gossip.menu_options(menu_id);
        if options.is_empty() && default_menu {
            options = self.stores.gossip.menu_options(0);
        }
        let options: Vec<_> = options.to_vec();
        let player_alive = self.unit_base(player).is_some_and(UnitBase::is_alive);
        let player_class = self.unit_base(player).map_or(0, UnitBase::class);
        let player_level = self.unit_base(player).map_or(0, UnitBase::level);
        let mut items = Vec::new();
        let mut just_discovered_node = false;
        for opt in &options {
            if opt.condition_id != 0
                && !self.condition_satisfied(opt.condition_id, Some(player), Some(npc))
            {
                if opt.option_id == GOSSIP_OPTION_QUESTGIVER {
                    can_see_quests = false;
                }
                continue;
            }
            if opt.npc_option_npcflag & npc_flags == 0 {
                continue;
            }
            let mut has_item = true;
            match opt.option_id {
                GOSSIP_OPTION_GOSSIP => {}
                GOSSIP_OPTION_QUESTGIVER | GOSSIP_OPTION_ARMORER => has_item = false,
                GOSSIP_OPTION_SPIRITHEALER => {
                    if player_alive {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_VENDOR => {
                    if self.vendor_rows(npc).is_empty() {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_TRAINER => {
                    if !self.is_trainer_of(player, npc, false) {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_UNLEARNTALENTS => {
                    // `CanTrainAndResetTalentsOf`
                    // (`Creature.cpp:1433`).
                    if !(player_level >= 10
                        && info.trainer_type == TRAINER_TYPE_CLASS_GOSSIP
                        && u32::from(player_class) == info.trainer_class)
                    {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_UNLEARNPETSKILLS => {
                    let pet_ok = self.pet_of(player).is_some_and(|pet| {
                        self.pets
                            .get(&pet)
                            .is_some_and(|st| st.is_hunter() && st.spells.len() > 1)
                    });
                    if !(pet_ok
                        && info.trainer_type == TRAINER_TYPE_PETS_GOSSIP
                        && info.trainer_class == u32::from(CLASS_HUNTER))
                    {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_TAXIVENDOR => {
                    if self.send_learn_new_taxi_node(player, npc) {
                        just_discovered_node = true;
                    }
                }
                GOSSIP_OPTION_BATTLEFIELD => {
                    if !self.can_interact_with_battle_master(player, npc) {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_STABLEPET => {
                    if player_class != CLASS_HUNTER {
                        has_item = false;
                    }
                }
                GOSSIP_OPTION_SPIRITGUIDE
                | GOSSIP_OPTION_INNKEEPER
                | GOSSIP_OPTION_BANKER
                | GOSSIP_OPTION_PETITIONER
                | GOSSIP_OPTION_TABARDDESIGNER
                | GOSSIP_OPTION_AUCTIONEER => {}
                _ => has_item = false,
            }
            if has_item {
                let text = if opt.option_broadcast_text != 0 {
                    self.gossip_broadcast_text(opt.option_broadcast_text, player)
                } else {
                    opt.option_text.clone()
                };
                let box_text = if opt.box_broadcast_text != 0 {
                    self.gossip_broadcast_text(opt.box_broadcast_text, player)
                } else {
                    opt.box_text.clone()
                };
                #[allow(clippy::cast_possible_truncation)]
                items.push(DefaultGossipItem {
                    icon: opt.option_icon as u8,
                    coded: opt.box_coded,
                    text,
                    box_text,
                    box_money: opt.box_money,
                    option_id: opt.option_id,
                    action_menu_id: opt.action_menu_id,
                    action_poi_id: opt.action_poi_id,
                    action_script_id: opt.action_script_id,
                });
            }
        }
        // `SendPreparedGossip` text pick: npc_gossip
        // text first, overwritten by the menu text when
        // a menu is set (fork order).
        let mut text_id = self.gossip_text_id(npc);
        if menu_id != 0 {
            text_id = self.gossip_menu_text_id(menu_id, player, npc);
        }
        DefaultGossipMenu {
            text_id,
            items,
            can_see_quests,
            npc_flags,
            just_discovered_node,
        }
    }

    /// `Player::OnGossipSelect` creature arm
    /// (`Player.cpp:12108`) for default-menu items.
    /// UNMODELED: `SendPointOfInterest` (no
    /// `points_of_interest` load) and the
    /// `action_script_id` `sGossipScripts` start.
    pub fn gossip_default_select(
        &mut self,
        player: ObjectGuid,
        npc: ObjectGuid,
        item: &DefaultGossipItem,
    ) -> DefaultGossipSelect {
        if !self.npc_if_can_interact_with(player, npc, 0) {
            return DefaultGossipSelect::Done;
        }
        match item.option_id {
            GOSSIP_OPTION_GOSSIP => match item.action_menu_id.cmp(&0) {
                std::cmp::Ordering::Greater => {
                    #[allow(clippy::cast_sign_loss)]
                    let menu = self.gossip_prepare_menu(player, npc, item.action_menu_id as u32);
                    DefaultGossipSelect::ShowMenu(menu)
                }
                std::cmp::Ordering::Less => {
                    // `TalkedToCreature` kill credit.
                    let entry = self.creature(npc).map_or(0, |c| c.info.entry);
                    self.quest_kill_credit_one(player, npc, entry);
                    DefaultGossipSelect::Close
                }
                std::cmp::Ordering::Equal => DefaultGossipSelect::Done,
            },
            GOSSIP_OPTION_SPIRITHEALER => {
                if !self.unit_base(player).is_some_and(UnitBase::is_alive) {
                    self.cast_spell_with_original_caster(
                        npc,
                        player,
                        SPELL_SPIRIT_HEAL,
                        CastTarget::Unit(npc),
                        crate::spells::CastOptions {
                            triggered: true,
                            skip_known: true,
                            ..Default::default()
                        },
                    );
                }
                DefaultGossipSelect::Done
            }
            GOSSIP_OPTION_QUESTGIVER => DefaultGossipSelect::QuestList,
            GOSSIP_OPTION_VENDOR | GOSSIP_OPTION_ARMORER => {
                self.send_list_inventory(player, npc);
                DefaultGossipSelect::Done
            }
            GOSSIP_OPTION_STABLEPET => {
                self.list_stabled_pets(player, npc);
                DefaultGossipSelect::Done
            }
            GOSSIP_OPTION_TRAINER => DefaultGossipSelect::TrainerList,
            GOSSIP_OPTION_UNLEARNTALENTS => {
                self.send_talent_wipe_confirm(player, npc);
                DefaultGossipSelect::Close
            }
            GOSSIP_OPTION_UNLEARNPETSKILLS => {
                // `SendPetSkillWipeConfirm`: close, then
                // the confirm packet is emitted map-side
                // by the pet unlearn confirm helper when
                // present; the close itself is fork-exact.
                // UNMODELED: `SMSG_PET_UNLEARN_CONFIRM`
                // send (pet reset-cost packet not ported).
                DefaultGossipSelect::Close
            }
            GOSSIP_OPTION_TAXIVENDOR => {
                self.send_taxi_menu(player, npc);
                DefaultGossipSelect::Done
            }
            GOSSIP_OPTION_INNKEEPER => DefaultGossipSelect::BinderConfirm,
            GOSSIP_OPTION_BANKER => {
                self.banker_activate(player, npc);
                DefaultGossipSelect::Done
            }
            GOSSIP_OPTION_PETITIONER => DefaultGossipSelect::PetitionShowList,
            GOSSIP_OPTION_TABARDDESIGNER => DefaultGossipSelect::TabardActivate,
            GOSSIP_OPTION_AUCTIONEER => DefaultGossipSelect::AuctionHello,
            GOSSIP_OPTION_SPIRITGUIDE => {
                let menu_id = self.creature(npc).map_or(0, |c| c.gossip_menu_id);
                DefaultGossipSelect::ShowMenu(self.gossip_prepare_menu(player, npc, menu_id))
            }
            GOSSIP_OPTION_BATTLEFIELD => {
                let entry = self.creature(npc).map_or(0, |c| c.info.entry);
                let bg_type = self.stores.battlemasters.bg_type(entry);
                if bg_type == 0 {
                    DefaultGossipSelect::Done
                } else {
                    DefaultGossipSelect::BattlefieldList(bg_type)
                }
            }
            _ => DefaultGossipSelect::Done,
        }
    }
}
