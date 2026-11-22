//! Creature corpse loot runtime (fork `Loot`,
//! `Creature::GenerateLootForBody`, `Player::SendLoot`,
//! `LootHandler` creature arms, `Group` loot rolls;
//! Tier 4 B0-10).
//!
//! State: [`Map::loots`] by creature guid,
//! [`Map::loot_taps`] for the recipient tap
//! (`Creature::SetLootRecipient` on first player-side
//! damage), [`Map::player_loot_guid`] for the open loot
//! window (`Player::m_lootGuid`). Generation runs at
//! `kill_unit` after XP (fork `Unit::Kill` order);
//! per-player quest/FFA/conditional fills are eager
//! for allowed looters (`FillPlayerDependentLoot`)
//! and lazy for later viewers (`FillNotNormalLootFor`
//! from `SendLoot` is GO-only in the fork, so creature
//! fills only happen through the eager path plus the
//! take/view guards below).
//!
//! Recorded UNMODELED at this layer: GO chest /
//! fishing / skinning / pickpocket / item-open /
//! corpse-insignia loot sources, AI `FillLoot`
//! overrides, `CAN_WIELD_LOOT` spawn pre-generation,
//! actor-side round-robin looter rotation
//! (`Group::UpdateLooterGuid` / `SendLooter` /
//! `SMSG_PARTYKILLLOG` — the RR player here is the
//! kill-time tap recipient), disenchant-on-roll,
//! damage-origin loot gating (shared gap with XP,
//! `xp.rs`), `lootForCreator`, BG-group
//! tap/allowed arms, random property ids (global
//! grant-path gap).

use std::collections::{HashMap, HashSet};

use mangos_guid::ObjectGuid;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::item_prototype::ItemPrototype;
use mangos_world_data::loot::LootRng as _;
use mangos_world_data::loot::{GeneratedLootItem, LootRollCtx, generate_loot, generate_money_loot};

use crate::inventory::{NULL_BAG, NULL_SLOT};
use crate::map::Map;

/// `LOOT_CORPSE`.
pub const LOOT_CORPSE: u8 = 1;
/// `MAX_NR_LOOT_ITEMS`.
const MAX_NR_LOOT_ITEMS: usize = 16;
/// `AURA_INTERRUPT_LOOTING_CANCELS`.
const AURA_INTERRUPT_LOOTING_CANCELS: u32 = 0x0000_0800;
/// `UNIT_FLAG_LOOTING`.
const UNIT_FLAG_LOOTING: u32 = 0x0000_0400;
/// `ITEM_EXTRA_IGNORE_QUEST_STATUS`.
const ITEM_EXTRA_IGNORE_QUEST_STATUS: u32 = 0x02;

/// `LootError` bytes (fork `SharedDefines.h`).
pub mod loot_error {
    pub const DIDNT_KILL: u8 = 0;
    pub const TOO_FAR: u8 = 4;
    pub const NOTSTANDING: u8 = 8;
    pub const STUNNED: u8 = 9;
    pub const PLAYER_NOT_FOUND: u8 = 10;
    pub const MASTER_INV_FULL: u8 = 12;
    pub const MASTER_UNIQUE_ITEM: u8 = 13;
    pub const MASTER_OTHER: u8 = 14;
}

/// Slot types (`LootSlotType`).
mod slot_type {
    pub const ALLOW_LOOT: u8 = 0;
    pub const ROLL_ONGOING: u8 = 1;
    pub const MASTER: u8 = 2;
    /// `MAX_LOOT_SLOT_TYPE` sentinel (hide slot).
    pub const HIDDEN: u8 = 5;
}

/// Permission ladder (`PermissionTypes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LootPermission {
    All,
    Group,
    Master,
    RoundRobin,
    Owner,
    None,
}

/// One generated slot (`LootItem` runtime state).
#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_excessive_bools)] // fork flag record
pub struct LootSlotItem {
    pub itemid: u32,
    pub count: u8,
    pub condition_id: u16,
    pub needs_quest: bool,
    pub freeforall: bool,
    pub is_looted: bool,
    pub is_blocked: bool,
    pub is_counted: bool,
    pub is_underthreshold: bool,
    pub loot_owner: ObjectGuid,
}

impl From<GeneratedLootItem> for LootSlotItem {
    fn from(g: GeneratedLootItem) -> Self {
        Self {
            itemid: g.itemid,
            count: g.count,
            condition_id: g.condition_id,
            needs_quest: g.needs_quest,
            freeforall: g.freeforall,
            is_looted: false,
            is_blocked: false,
            is_counted: false,
            is_underthreshold: false,
            loot_owner: ObjectGuid::EMPTY,
        }
    }
}

/// `QuestItem` per-player mark.
#[derive(Clone, Copy, Debug)]
pub struct LootQuestMark {
    pub index: u8,
    pub is_looted: bool,
}

/// One in-flight group loot roll (`Roll` subset;
/// map-local, see `start_loot_rolls`). Votes keyed
/// by guid low in fork `std::map` order.
#[derive(Clone, Debug)]
pub struct LootRoll {
    pub item_slot: u8,
    pub itemid: u32,
    pub count: u8,
    /// guid low -> vote (`RollVote`: 0 pass, 1 need,
    /// 2 greed, 3 not emitted yet).
    pub votes: std::collections::BTreeMap<u32, u8>,
    pub total_need: u32,
    pub total_greed: u32,
    pub total_pass: u32,
    pub deadline_ms: u64,
}

/// Corpse loot state (`Loot`).
#[derive(Clone, Debug, Default)]
pub struct LootState {
    pub personal: bool,
    pub items: Vec<LootSlotItem>,
    pub quest_items: Vec<LootSlotItem>,
    pub gold: u32,
    pub unlooted_count: u32,
    /// Fork `Team` numeric (469/67/0).
    pub team: u32,
    pub round_robin_player: u64,
    pub players_looting: HashSet<ObjectGuid>,
    pub allowed_looters: Vec<ObjectGuid>,
    pub has_ffa_quest_items: bool,
    pub loot_id: u32,
    /// `lootForBody`: group roll pass has run.
    pub body_processed: bool,
    pub rolls: Vec<LootRoll>,
    player_quest_items: HashMap<u32, Vec<LootQuestMark>>,
    player_ffa_items: HashMap<u32, Vec<LootQuestMark>>,
    player_cond_items: HashMap<u32, Vec<LootQuestMark>>,
}

impl LootState {
    /// `Loot::isLooted`.
    #[must_use]
    pub fn is_looted(&self) -> bool {
        self.gold == 0 && self.unlooted_count == 0
    }

    /// `Loot::IsAllowedLooter(guid, false)`.
    #[must_use]
    pub fn is_allowed_looter(&self, guid: ObjectGuid) -> bool {
        self.allowed_looters.contains(&guid)
    }

    /// `Loot::hasOverThresholdItem`.
    fn has_over_threshold_item(&self) -> bool {
        self.items
            .iter()
            .any(|i| !i.is_looted && !i.is_underthreshold && !i.freeforall)
    }

    /// `Loot::hasItemFor`: any unlooted quest/FFA/
    /// conditional mark for the player.
    fn has_item_for(&self, player: ObjectGuid) -> bool {
        let low = player.counter();
        let any_unlooted = |marks: Option<&Vec<LootQuestMark>>| {
            marks.is_some_and(|m| m.iter().any(|q| !q.is_looted))
        };
        any_unlooted(self.player_quest_items.get(&low))
            || any_unlooted(self.player_ffa_items.get(&low))
            || any_unlooted(self.player_cond_items.get(&low))
    }
}

/// Tap record (`m_lootRecipientGuid` +
/// `m_lootGroupRecipientId`). `recipient` may be a
/// pet guid (fork stores the pet when the tapper is
/// the owner's own pet).
#[derive(Clone, Copy, Debug)]
pub struct LootTap {
    pub recipient: ObjectGuid,
    pub group_id: Option<u32>,
}

impl Map {
    /// Player behind an attacker (self, pet owner, or
    /// summoner): `GetCharmerOrOwnerPlayerOrPlayerItself`
    /// subset.
    pub(crate) fn affecting_player(&self, attacker: ObjectGuid) -> Option<ObjectGuid> {
        if attacker.is_player() {
            return Some(attacker);
        }
        if let Some(owner) = self.pet_owner(attacker).filter(|g| g.is_player()) {
            return Some(owner);
        }
        self.summons
            .get(&attacker)
            .map(|st| st.summoner)
            .filter(|g| g.is_player())
    }

    /// `Creature::SetLootRecipient` from the damage
    /// path (`Unit::DealDamage`): first tap wins,
    /// except a pet tap is replaced when its owner (or
    /// a member of the tap group) lands a hit. Pet
    /// victims are never tapped.
    pub(crate) fn tap_creature(&mut self, creature: ObjectGuid, attacker: ObjectGuid) {
        if !creature.is_creature() || self.is_pet(creature) {
            return;
        }
        let Some(player) = self.affecting_player(attacker) else {
            return;
        };
        if let Some(tap) = self.loot_taps.get(&creature).copied() {
            // Replacement arm: current recipient is a
            // pet and the new attacker is a player who
            // owns it or shares the tap group.
            let replace = !tap.recipient.is_player()
                && attacker.is_player()
                && (self.pet_owner(tap.recipient) == Some(player)
                    || (tap.group_id.is_some()
                        && self.players.get(&player).and_then(|e| e.player.group_id)
                            == tap.group_id));
            if !replace {
                return;
            }
        }
        let recipient = if !attacker.is_player()
            && self.is_pet(attacker)
            && self.pet_owner(attacker) == Some(player)
        {
            attacker
        } else {
            player
        };
        let group_id = self.players.get(&player).and_then(|e| e.player.group_id);
        self.loot_taps.insert(
            creature,
            LootTap {
                recipient,
                group_id,
            },
        );
        self.touch_dynflags(creature);
    }

    /// `SetLootRecipient(nullptr)` (evade / respawn).
    pub(crate) fn clear_loot_tap(&mut self, creature: ObjectGuid) {
        if self.loot_taps.remove(&creature).is_some() {
            self.touch_dynflags(creature);
        }
    }

    /// Drop all loot state for a creature leaving the
    /// world (despawn / respawn re-materialization).
    pub(crate) fn clear_loot_for_creature(&mut self, creature: ObjectGuid) {
        self.loots.remove(&creature);
        self.loot_taps.remove(&creature);
        self.player_loot_guid.retain(|_, g| *g != creature);
    }

    /// Force `UNIT_DYNAMIC_FLAGS` re-emit (the word is
    /// rewritten per-viewer at build time).
    pub(crate) fn touch_dynflags(&mut self, creature: ObjectGuid) {
        if let Some(u) = self.unit_base_mut(creature) {
            u.world.object.force_values_update_at(
                mangos_protocol::update_field_table::offsets::UNIT_DYNAMIC_FLAGS,
            );
        }
    }

    /// Set/clear the stored lootable dynflag bit
    /// (fork sets it at death when the template has a
    /// loot id or gold; release clears it).
    fn set_dynflag_lootable(&mut self, creature: ObjectGuid, on: bool) {
        use mangos_protocol::update_field_table::offsets;
        if let Some(u) = self.unit_base_mut(creature) {
            let cur = u.values().get_u32(offsets::UNIT_DYNAMIC_FLAGS);
            let next = if on {
                cur | mangos_entities::update::UNIT_DYNFLAG_LOOTABLE
            } else {
                cur & !mangos_entities::update::UNIT_DYNFLAG_LOOTABLE
            };
            u.values().set_u32(offsets::UNIT_DYNAMIC_FLAGS, next);
        }
    }

    /// Per-viewer dynamic flags for a creature (fork
    /// `Object.cpp` `UNIT_DYNAMIC_FLAGS` arm: tapped
    /// bits from the tap, lootable cleared when the
    /// viewer may not loot).
    pub(crate) fn creature_dynamic_flags(
        &self,
        viewer: ObjectGuid,
        creature: ObjectGuid,
        stored: u32,
    ) -> u32 {
        use mangos_entities::update::{
            UNIT_DYNFLAG_LOOTABLE, UNIT_DYNFLAG_TAPPED, UNIT_DYNFLAG_TAPPED_BY_PLAYER,
        };
        let mut flags = stored;
        if let Some(tap) = self.loot_taps.get(&creature) {
            if self.is_tapped_by(viewer, tap) {
                flags |= UNIT_DYNFLAG_TAPPED | UNIT_DYNFLAG_TAPPED_BY_PLAYER;
            } else {
                flags |= UNIT_DYNFLAG_TAPPED;
                flags &= !UNIT_DYNFLAG_TAPPED_BY_PLAYER;
            }
        } else {
            flags &= !(UNIT_DYNFLAG_TAPPED | UNIT_DYNFLAG_TAPPED_BY_PLAYER);
        }
        if flags & UNIT_DYNFLAG_LOOTABLE != 0 && !self.is_allowed_to_loot(viewer, creature) {
            flags &= !UNIT_DYNFLAG_LOOTABLE;
        }
        flags
    }

    /// `Creature::IsTappedBy`.
    fn is_tapped_by(&self, player: ObjectGuid, tap: &LootTap) -> bool {
        if player == tap.recipient {
            return true;
        }
        if !tap.recipient.is_empty() && self.pet_owner(tap.recipient) == Some(player) {
            return true;
        }
        // BG-group arm UNMODELED (no BG group mirror).
        match tap.group_id {
            None => false,
            Some(g) => self
                .players
                .get(&player)
                .is_some_and(|e| e.player.group_id == Some(g)),
        }
    }

    /// `Creature::GetLootRecipient` resolved to a
    /// player on this map (original recipient, else
    /// any tap-group member present).
    fn loot_recipient_player(&self, creature: ObjectGuid) -> Option<ObjectGuid> {
        let tap = self.loot_taps.get(&creature)?;
        if tap.recipient.is_player() && self.players.contains_key(&tap.recipient) {
            let still_grouped = match tap.group_id {
                None => true,
                Some(g) => self
                    .players
                    .get(&tap.recipient)
                    .is_some_and(|e| e.player.group_id == Some(g)),
            };
            if still_grouped {
                return Some(tap.recipient);
            }
        }
        let g = tap.group_id?;
        self.players
            .iter()
            .find(|(_, e)| e.player.group_id == Some(g))
            .map(|(guid, _)| *guid)
    }

    /// `Player::IsAllowedToLoot(creature)` (dynflag
    /// gate). The AI `CanBeLooted()` arm is UNMODELED
    /// (no script-hook surface for it yet).
    pub(crate) fn is_allowed_to_loot(&self, player: ObjectGuid, creature: ObjectGuid) -> bool {
        if self.unit_is_alive(creature) {
            return false;
        }
        let Some(loot) = self.loots.get(&creature) else {
            return false;
        };
        if loot.is_looted() {
            return false;
        }
        // Nostalrius: must have been there at the kill.
        if !loot.is_allowed_looter(player) {
            return false;
        }
        let Some(tap) = self.loot_taps.get(&creature) else {
            return false;
        };
        let Some(entry) = self.players.get(&player) else {
            return false;
        };
        let Some(group_id) = entry.player.group_id else {
            // Solo: only the original recipient (a pet
            // tap counts for its owner in `IsTappedBy`
            // but fork compares the player pointer here;
            // pet taps never generate loot, so the arm
            // is unreachable in practice).
            return player == tap.recipient;
        };
        if tap.group_id != Some(group_id) {
            return false;
        }
        let rr_is_player =
            loot.round_robin_player == 0 || ObjectGuid::from_raw(loot.round_robin_player) == player;
        match entry.player.group_loot_method {
            // FREE_FOR_ALL
            0 => true,
            // MASTER_LOOT: over-threshold items are
            // always viewable, else the ROUND_ROBIN arm.
            2 if loot.has_over_threshold_item() => true,
            // ROUND_ROBIN (+ master fallthrough)
            1 | 2 => rr_is_player || loot.has_item_for(player),
            // GROUP_LOOT / NEED_BEFORE_GREED
            _ => rr_is_player || loot.has_over_threshold_item() || loot.has_item_for(player),
        }
    }

    /// `Player::HasQuestForItem`: an incomplete quest
    /// requires the item and the held count is below
    /// the required count. (Fork also rejects
    /// timer-failed quests; the runtime rows carry no
    /// failed flag distinct from status here.)
    pub(crate) fn has_quest_for_item(&self, player: ObjectGuid, item_id: u32) -> bool {
        let Some(entry) = self.players.get(&player) else {
            return false;
        };
        for row in &entry.player.deferred.quest_status {
            // QUEST_STATUS_INCOMPLETE (3) only.
            if row.status != 3 {
                continue;
            }
            let Some(tpl) = self.stores.quest_templates.get(row.quest) else {
                continue;
            };
            for i in 0..4 {
                if tpl.req_item_ids[i] == item_id
                    && entry.player.quest_item_count(item_id) < tpl.req_item_counts[i]
                {
                    return true;
                }
            }
        }
        false
    }

    /// `LootItem::AllowedForPlayer`.
    fn loot_allowed_for_player(
        &self,
        item: &LootSlotItem,
        player: ObjectGuid,
        creature: ObjectGuid,
    ) -> bool {
        if item.condition_id != 0
            && !self.condition_satisfied(u32::from(item.condition_id), Some(player), Some(creature))
        {
            return false;
        }
        let Some(proto) = self.stores.item_prototypes.get(item.itemid) else {
            return false;
        };
        if item.needs_quest {
            if !self.has_quest_for_item(player, item.itemid) {
                return false;
            }
        } else if proto.start_quest != 0
            && let Some(quest) = self.stores.quest_templates.get(proto.start_quest)
            && !quest.is_repeatable()
            && self
                .players
                .get(&player)
                .is_some_and(|e| e.player.quest_status(proto.start_quest) != 0)
            && !self.has_quest_for_item(player, item.itemid)
            && proto.extra_flags & ITEM_EXTRA_IGNORE_QUEST_STATUS == 0
        {
            return false;
        }
        if !item.loot_owner.is_empty() {
            return player == item.loot_owner;
        }
        true
    }

    /// `SendLoot` permission ladder (creature arm of
    /// `Player::SendLoot`).
    pub(crate) fn loot_permission(
        &self,
        player: ObjectGuid,
        creature: ObjectGuid,
    ) -> LootPermission {
        let Some(tap) = self.loot_taps.get(&creature) else {
            return LootPermission::None;
        };
        let Some(entry) = self.players.get(&player) else {
            return LootPermission::None;
        };
        match entry.player.group_id {
            None => {
                if self.loot_recipient_player(creature) == Some(player) {
                    LootPermission::Owner
                } else {
                    LootPermission::None
                }
            }
            Some(g) => {
                if tap.group_id != Some(g) {
                    return LootPermission::None;
                }
                match entry.player.group_loot_method {
                    // FREE_FOR_ALL
                    0 => LootPermission::All,
                    // ROUND_ROBIN
                    1 => LootPermission::RoundRobin,
                    // MASTER_LOOT: every group viewer gets
                    // MASTER_PERMISSION (the give handler
                    // enforces who the master looter is).
                    2 => LootPermission::Master,
                    // GROUP_LOOT / NEED_BEFORE_GREED
                    _ => LootPermission::Group,
                }
            }
        }
    }

    /// `GetMaxLootDistance` gate: center 3D distance
    /// strictly below the no-leeway combat reach
    /// (`IsWithinDistInMap(..., SizeFactor::None)`).
    fn within_loot_range(&self, player: ObjectGuid, creature: ObjectGuid) -> bool {
        let (Some(pa), Some(pb)) = (self.unit_position(player), self.unit_position(creature))
        else {
            return false;
        };
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        let dz = pa.z - pb.z;
        let reach = self.reach_between(player, creature);
        dx * dx + dy * dy + dz * dz < reach * reach
    }

    /// `WorldObject::IsWithinLootXPDist` between two
    /// players (money split band): raid maps always,
    /// else 2D distance strictly below
    /// `GROUP_XP_DISTANCE`.
    fn within_loot_xp_dist_players(&self, a: ObjectGuid, b: ObjectGuid) -> bool {
        if let Some(entry) = self.stores.maps.get(self.map_id)
            && entry.is_raid()
        {
            return true;
        }
        let (Some(pa), Some(pb)) = (self.unit_position(a), self.unit_position(b)) else {
            return false;
        };
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        dx * dx + dy * dy < self.config.max_group_xp_distance * self.config.max_group_xp_distance
    }

    /// Kill-time body looter (`Unit::Kill` `pPlayerTap`
    /// resolution): the online original recipient,
    /// else the killer's affecting player; a pet-guid
    /// tap resolves to nobody (fork sets
    /// `pPlayerTap = nullptr`). Synthesizes the tap
    /// for killer-only kills.
    fn resolve_body_looter(
        &mut self,
        victim: ObjectGuid,
        killer: Option<ObjectGuid>,
    ) -> Option<ObjectGuid> {
        let tap = self.loot_taps.get(&victim).copied();
        let looter = match tap {
            Some(t) if t.recipient.is_player() && self.players.contains_key(&t.recipient) => {
                Some(t.recipient)
            }
            Some(t) if !t.recipient.is_player() => None,
            _ => killer.and_then(|k| self.affecting_player(k)),
        }?;
        if tap.is_none() {
            let group_id = self.players.get(&looter).and_then(|e| e.player.group_id);
            self.loot_taps.insert(
                victim,
                LootTap {
                    recipient: looter,
                    group_id,
                },
            );
        }
        Some(looter)
    }

    /// Kill-time loot generation (`Unit::Kill` loot
    /// block + `Creature::GenerateLootForBody`).
    /// Pets never generate; the lootable dynflag bit
    /// is set from the template alone (fork `Kill`
    /// order), generation itself needs an online
    /// player tap (or killer) and an enabled map.
    pub(crate) fn generate_creature_loot(
        &mut self,
        victim: ObjectGuid,
        killer: Option<ObjectGuid>,
    ) {
        if self.is_pet(victim) {
            return;
        }
        let entry = {
            let Some(c) = self.creatures.get(&victim) else {
                return;
            };
            c.info.entry
        };
        let template = self.stores.creature_templates.get(entry).cloned();
        let (loot_id, gold_min, gold_max) = template
            .as_ref()
            .map_or((0, 0, 0), |t| (t.loot_id, t.gold_min, t.gold_max));
        if loot_id != 0 || gold_max > 0 {
            self.set_dynflag_lootable(victim, true);
        }
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            return;
        }
        // Temp summons: fork generation runs through
        // the same path, but summons carry no tap
        // (their own tap rules) — the killer fallback
        // below would still fire; the observable fork
        // outcome for summoned creatures with a loot
        // template is a generated corpse loot, so no
        // special-casing here.
        let Some(looter) = self.resolve_body_looter(victim, killer) else {
            return;
        };
        let team = self.player_team_id(looter);
        let looter_group = self.players.get(&looter).and_then(|e| e.player.group_id);
        let victim_pos = self.unit_position(victim).unwrap_or_default();
        let victim_info = template.clone().unwrap_or_default();
        // `FillPlayerDependentLoot(looter, false)`:
        // the looter's live group at reward distance.
        let (allowed, personal): (Vec<ObjectGuid>, bool) = match looter_group {
            None => (vec![looter], true),
            Some(g) => {
                let members: Vec<ObjectGuid> = self
                    .players
                    .iter()
                    .filter(|(_, e)| e.player.group_id == Some(g))
                    .filter(|(guid, _)| {
                        self.unit_position(**guid).is_some_and(|p| {
                            self.at_group_reward_distance(p, victim_pos, &victim_info)
                        })
                    })
                    .map(|(guid, _)| *guid)
                    .collect();
                (members, false)
            }
        };
        let mut state = LootState {
            personal,
            team,
            loot_id,
            round_robin_player: if personal { 0 } else { looter.raw() },
            allowed_looters: allowed.clone(),
            ..LootState::default()
        };
        let generated = {
            let Self {
                rng,
                stores,
                config,
                ..
            } = self;
            let conditions = &stores.scripts.conditions;
            let mut ctx = LootRollCtx {
                rng,
                prototypes: &stores.item_prototypes,
                rates: config.loot_rates,
                team,
                condition_team: &|cid| {
                    conditions.get(&u32::from(cid)).map(|c| {
                        if c.cond_type == 6 {
                            c.values[0] as u32
                        } else {
                            0
                        }
                    })
                },
            };
            let generated = generate_loot(&mut ctx, &stores.loot, &stores.loot.creature, loot_id);
            state.gold = generate_money_loot(&mut ctx, gold_min, gold_max);
            generated
        };
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                self.loot_add_item(&mut state, item, looter);
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
            }
        }
        self.loots.insert(victim, state);
        for member in allowed {
            self.loot_fill_dependent(victim, member);
        }
    }

    /// `Loot::AddItem` counting + group threshold
    /// marking (`FillPlayerDependentLoot` tail).
    fn loot_add_item(&self, state: &mut LootState, item: GeneratedLootItem, looter: ObjectGuid) {
        let mut slot = LootSlotItem::from(item);
        if !state.personal {
            let threshold = self
                .players
                .get(&looter)
                .map_or(2, |e| e.player.group_loot_threshold);
            let quality = self
                .stores
                .item_prototypes
                .get(item.itemid)
                .map_or(0, |p| p.quality);
            if quality < u32::from(threshold) {
                slot.is_underthreshold = true;
            }
        }
        // Only plain one-player items count here; FFA
        // and conditional items count per player in
        // the dependent fills.
        let counted = item.condition_id == 0 && !item.freeforall;
        state.items.push(slot);
        if counted {
            state.unlooted_count += 1;
        }
    }

    /// `FillNotNormalLootFor` (quest + FFA +
    /// conditional fills with fork counting rules).
    fn loot_fill_dependent(&mut self, creature: ObjectGuid, player: ObjectGuid) {
        let Some(state) = self.loots.get(&creature) else {
            return;
        };
        let low = player.counter();
        if state.player_quest_items.contains_key(&low) {
            return;
        }
        let items_len = state.items.len();
        // Precompute `AllowedForPlayer` (needs `&self`)
        // before mutating the state.
        let quest_allowed: Vec<bool> = state
            .quest_items
            .iter()
            .map(|it| !it.is_looted && self.loot_allowed_for_player(it, player, creature))
            .collect();
        let item_allowed: Vec<bool> = state
            .items
            .iter()
            .map(|it| !it.is_looted && self.loot_allowed_for_player(it, player, creature))
            .collect();
        let Some(state) = self.loots.get_mut(&creature) else {
            return;
        };
        // `FillQuestLoot`.
        let mut qmarks: Vec<LootQuestMark> = Vec::new();
        if items_len < MAX_NR_LOOT_ITEMS {
            for (i, allowed) in quest_allowed.iter().enumerate() {
                if !allowed {
                    continue;
                }
                let (freeforall, was_blocked) = {
                    let item = &state.quest_items[i];
                    (item.freeforall, item.is_blocked)
                };
                if freeforall || !was_blocked {
                    state.unlooted_count += 1;
                }
                state.quest_items[i].is_blocked = true;
                qmarks.push(LootQuestMark {
                    index: i as u8,
                    is_looted: false,
                });
                if items_len + qmarks.len() >= MAX_NR_LOOT_ITEMS {
                    break;
                }
            }
        }
        // `FillFFALoot`.
        let mut fmarks: Vec<LootQuestMark> = Vec::new();
        for (i, allowed) in item_allowed.iter().enumerate() {
            if *allowed && state.items[i].freeforall {
                fmarks.push(LootQuestMark {
                    index: i as u8,
                    is_looted: false,
                });
                state.unlooted_count += 1;
            }
        }
        // `FillNonQuestNonFFAConditionalLoot`.
        let mut cmarks: Vec<LootQuestMark> = Vec::new();
        for (i, allowed) in item_allowed.iter().enumerate() {
            let item = &state.items[i];
            if *allowed && !item.freeforall && item.condition_id != 0 {
                cmarks.push(LootQuestMark {
                    index: i as u8,
                    is_looted: false,
                });
                if !state.items[i].is_counted {
                    state.items[i].is_counted = true;
                    state.unlooted_count += 1;
                }
            }
        }
        state.player_quest_items.insert(low, qmarks);
        state.player_ffa_items.insert(low, fmarks);
        state.player_cond_items.insert(low, cmarks);
    }

    // --------------------------------------------------
    // Wire: errors, release, view, notifies.
    // --------------------------------------------------

    /// `Player::SendLootError`.
    fn send_loot_error(&mut self, player: ObjectGuid, guid: ObjectGuid, error: u8) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(0);
        writer.write_u8(error);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
    }

    /// `Player::SendLootRelease` (packet only).
    fn send_loot_release(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(1);
        let packet =
            WorldPacket::with_body(opcode_id::SMSG_LOOT_RELEASE_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
    }

    /// `Player::SendNotifyLootItemRemoved`.
    fn send_loot_removed(&mut self, player: ObjectGuid, slot: u8) {
        let mut writer = PacketWriter::new();
        writer.write_u8(slot);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_REMOVED, writer.into_vec());
        self.send_to_player(player, packet);
    }

    /// Current looters of a corpse, pruning players
    /// who left the map (fork notify loops erase
    /// missing players the same way).
    fn prune_looters(&mut self, creature: ObjectGuid) -> Vec<ObjectGuid> {
        let Some(state) = self.loots.get(&creature) else {
            return Vec::new();
        };
        let looters: Vec<ObjectGuid> = state
            .players_looting
            .iter()
            .copied()
            .filter(|g| self.players.contains_key(g))
            .collect();
        if let Some(state) = self.loots.get_mut(&creature) {
            let keep: HashSet<ObjectGuid> = looters.iter().copied().collect();
            state.players_looting.retain(|g| keep.contains(g));
        }
        looters
    }

    /// `Loot::NotifyItemRemoved` (all current looters).
    fn notify_item_removed(&mut self, creature: ObjectGuid, slot: u8) {
        for looter in self.prune_looters(creature) {
            self.send_loot_removed(looter, slot);
        }
    }

    /// `Loot::NotifyQuestItemRemoved`: each looter
    /// gets the removal at their own quest-slot
    /// position.
    fn notify_quest_item_removed(&mut self, creature: ObjectGuid, quest_index: u8) {
        let looters = self.prune_looters(creature);
        let mut sends: Vec<(ObjectGuid, u8)> = Vec::new();
        if let Some(state) = self.loots.get(&creature) {
            let items_len = state.items.len();
            for looter in looters {
                if let Some(marks) = state.player_quest_items.get(&looter.counter())
                    && let Some(pos) = marks.iter().position(|m| m.index == quest_index)
                {
                    sends.push((looter, (items_len + pos) as u8));
                }
            }
        }
        for (looter, slot) in sends {
            self.send_loot_removed(looter, slot);
        }
    }

    /// `Loot::NotifyMoneyRemoved`
    /// (`SMSG_LOOT_CLEAR_MONEY` to all looters).
    fn notify_money_removed(&mut self, creature: ObjectGuid) {
        for looter in self.prune_looters(creature) {
            let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_CLEAR_MONEY, Vec::new());
            self.send_to_player(looter, packet);
        }
    }

    /// One item block (`operator<<(LootItem)`).
    fn write_loot_item(&self, writer: &mut PacketWriter, item: &LootSlotItem) {
        writer.write_u32_le(item.itemid);
        writer.write_u32_le(u32::from(item.count));
        writer.write_u32_le(
            self.stores
                .item_prototypes
                .get(item.itemid)
                .map_or(0, |p| p.display_id),
        );
        writer.write_u32_le(0);
        // Random property id: global grant-path gap.
        writer.write_u32_le(0);
    }

    /// `LootItem::GetSlotTypeForSharedLoot`.
    fn shared_slot_type(
        item: &LootSlotItem,
        permission: LootPermission,
        allowed: bool,
        condition_ok: bool,
    ) -> u8 {
        if item.is_looted
            || item.freeforall
            || (item.condition_id != 0 && !condition_ok)
            || !allowed
        {
            return slot_type::HIDDEN;
        }
        match permission {
            LootPermission::Group => {
                if item.is_blocked || item.is_underthreshold {
                    slot_type::ALLOW_LOOT
                } else {
                    slot_type::ROLL_ONGOING
                }
            }
            LootPermission::All | LootPermission::Owner => slot_type::ALLOW_LOOT,
            LootPermission::Master => {
                if item.is_underthreshold {
                    slot_type::ALLOW_LOOT
                } else {
                    slot_type::MASTER
                }
            }
            LootPermission::RoundRobin | LootPermission::None => slot_type::HIDDEN,
        }
    }

    /// Per-viewer quest/FFA/conditional view
    /// sections of `operator<<(LootView)`. Returns
    /// the number of items written.
    #[allow(clippy::too_many_arguments)] // view state bundle
    fn write_view_personal_sections(
        &self,
        writer: &mut PacketWriter,
        state: &LootState,
        viewer: ObjectGuid,
        rr: u64,
        viewer_is_rr: bool,
        permission: LootPermission,
        allowed: &[bool],
    ) -> u8 {
        let mut shown: u8 = 0;
        let low = viewer.counter();
        let rr_low = ObjectGuid::from_raw(rr).counter();
        // Quest items: wire slot = items + list position.
        if let Some(marks) = state.player_quest_items.get(&low) {
            let rr_has_list = state.player_quest_items.contains_key(&rr_low);
            for (pos, mark) in marks.iter().enumerate() {
                let item = &state.quest_items[mark.index as usize];
                if mark.is_looted || item.is_looted {
                    continue;
                }
                if !item.freeforall && rr != 0 && !viewer_is_rr && rr_has_list {
                    continue;
                }
                writer.write_u8((state.items.len() + pos) as u8);
                self.write_loot_item(writer, item);
                writer.write_u8(slot_type::ALLOW_LOOT);
                shown += 1;
            }
        }
        // FFA items.
        if let Some(marks) = state.player_ffa_items.get(&low) {
            for mark in marks {
                let item = &state.items[mark.index as usize];
                if mark.is_looted || item.is_looted {
                    continue;
                }
                writer.write_u8(mark.index);
                self.write_loot_item(writer, item);
                writer.write_u8(slot_type::ALLOW_LOOT);
                shown += 1;
            }
        }
        // Conditional items.
        if let Some(marks) = state.player_cond_items.get(&low) {
            let rr_has_list = state.player_cond_items.contains_key(&rr_low);
            for mark in marks {
                let item = &state.items[mark.index as usize];
                let slot = Self::shared_slot_type(
                    item,
                    permission,
                    allowed[mark.index as usize],
                    !mark.is_looted,
                );
                if slot >= slot_type::HIDDEN {
                    continue;
                }
                if !item.freeforall && rr != 0 && !viewer_is_rr && rr_has_list {
                    continue;
                }
                writer.write_u8(mark.index);
                self.write_loot_item(writer, item);
                writer.write_u8(slot);
                shown += 1;
            }
        }
        shown
    }

    /// `operator<<(LootView)` for the creature state.
    fn loot_view_bytes(
        &self,
        viewer: ObjectGuid,
        creature: ObjectGuid,
        permission: LootPermission,
    ) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        if permission == LootPermission::None {
            writer.write_u32_le(0);
            writer.write_u8(0);
            return writer.into_vec();
        }
        let empty = LootState::default();
        let state = self.loots.get(&creature).unwrap_or(&empty);
        let allowed: Vec<bool> = state
            .items
            .iter()
            .map(|it| self.loot_allowed_for_player(it, viewer, creature))
            .collect();
        writer.write_u32_le(state.gold);
        // Count placeholder, backpatched below.
        writer.write_u8(0);
        let mut shown: u8 = 0;
        let rr = state.round_robin_player;
        let viewer_is_rr = rr != 0 && ObjectGuid::from_raw(rr) == viewer;
        match permission {
            LootPermission::Group => {
                for (i, item) in state.items.iter().enumerate() {
                    if item.is_looted || item.freeforall || !allowed[i] {
                        continue;
                    }
                    let slot = if item.is_blocked {
                        slot_type::ROLL_ONGOING
                    } else if rr == 0 || !item.is_underthreshold || viewer_is_rr {
                        slot_type::ALLOW_LOOT
                    } else {
                        continue;
                    };
                    writer.write_u8(i as u8);
                    self.write_loot_item(&mut writer, item);
                    writer.write_u8(slot);
                    shown += 1;
                }
            }
            LootPermission::RoundRobin => {
                for (i, item) in state.items.iter().enumerate() {
                    if item.is_looted || item.freeforall || !allowed[i] {
                        continue;
                    }
                    if rr != 0 && !viewer_is_rr {
                        continue;
                    }
                    writer.write_u8(i as u8);
                    self.write_loot_item(&mut writer, item);
                    writer.write_u8(slot_type::ALLOW_LOOT);
                    shown += 1;
                }
            }
            LootPermission::All | LootPermission::Master | LootPermission::Owner => {
                let slot = if permission == LootPermission::Master {
                    slot_type::MASTER
                } else {
                    slot_type::ALLOW_LOOT
                };
                for (i, item) in state.items.iter().enumerate() {
                    if item.is_looted || item.freeforall || !allowed[i] {
                        continue;
                    }
                    writer.write_u8(i as u8);
                    self.write_loot_item(&mut writer, item);
                    writer.write_u8(slot);
                    shown += 1;
                }
            }
            LootPermission::None => unreachable!(),
        }
        shown += self.write_view_personal_sections(
            &mut writer,
            state,
            viewer,
            rr,
            viewer_is_rr,
            permission,
            &allowed,
        );
        let mut bytes = writer.into_vec();
        bytes[4] = shown;
        bytes
    }

    // --------------------------------------------------
    // Handlers (`LootHandler.cpp` creature arms).
    // --------------------------------------------------

    /// Shared grant tail (`CanStoreNewItem` +
    /// `StoreNewItem` + `SendNewItem`). Roll wins
    /// pass `notify = false` (fork roll grants send
    /// no push packet).
    fn grant_loot_item(
        &mut self,
        target: ObjectGuid,
        proto: &ItemPrototype,
        count: u32,
    ) -> Result<(), crate::inventory::InvResult> {
        self.grant_loot_item_notify(target, proto, count, true)
    }

    fn grant_loot_item_notify(
        &mut self,
        target: ObjectGuid,
        proto: &ItemPrototype,
        count: u32,
        notify: bool,
    ) -> Result<(), crate::inventory::InvResult> {
        let dests = {
            let Some(entry) = self.players.get(&target) else {
                return Err(crate::inventory::ERR_ITEM_NOT_FOUND);
            };
            self.can_store_auto(
                &entry.player,
                NULL_BAG,
                NULL_SLOT,
                proto,
                count,
                false,
                None,
            )?
        };
        let template = self.new_item_row(proto, count);
        let Some(placed) = self.store_new_item(target, &template, &dests) else {
            return Err(crate::inventory::ERR_ITEM_NOT_FOUND);
        };
        if notify {
            self.send_new_item(target, &placed, count);
        }
        Ok(())
    }

    /// `CMSG_LOOT` (`HandleLootOpcode` +
    /// `Player::SendLoot` creature arm, `LOOT_CORPSE`).
    /// GO/item/corpse loot sources are UNMODELED.
    pub fn handle_loot(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        if !guid.is_creature() && !guid.is_player() && !guid.is_corpse() {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        if !self.unit_is_alive(player) {
            self.send_loot_error(player, guid, loot_error::PLAYER_NOT_FOUND);
            return;
        }
        let (stand_state, stunned) = self.players.get(&player).map_or((0, false), |e| {
            (
                e.player.unit.stand_state(),
                e.player
                    .unit
                    .has_unit_state(mangos_entities::unit::UNIT_STATE_STUNNED),
            )
        });
        if stand_state != 0 {
            self.send_loot_error(player, guid, loot_error::NOTSTANDING);
            return;
        }
        if stunned {
            self.send_loot_error(player, guid, loot_error::STUNNED);
            return;
        }
        if self.casts.contains_key(&player) {
            self.cancel_cast(player, false);
        }
        // `SendLoot`: disabled maps release at once.
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            self.send_loot_release(player, guid);
            return;
        }
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        if !guid.is_creature() {
            // Player/corpse arms UNMODELED (insignia).
            self.send_loot_release(player, guid);
            return;
        }
        self.remove_auras_with_interrupt_flags(player, AURA_INTERRUPT_LOOTING_CANCELS, 0);
        if !self.creatures.contains_key(&guid) || self.unit_is_alive(guid) {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        if !self.within_loot_range(player, guid) {
            self.send_loot_error(player, guid, loot_error::TOO_FAR);
            return;
        }
        let Some(recipient) = self.loot_recipient_player(guid) else {
            self.send_loot_release(player, guid);
            return;
        };
        // First body view runs the group roll pass
        // (`lootForBody` gate in `SendLoot`).
        let needs_rolls = self.loots.get(&guid).is_some_and(|s| !s.body_processed);
        if needs_rolls {
            if let Some(state) = self.loots.get_mut(&guid) {
                state.body_processed = true;
            }
            let tap_grouped = self
                .loot_taps
                .get(&guid)
                .is_some_and(|t| t.group_id.is_some());
            let method = self
                .players
                .get(&recipient)
                .map_or(3, |e| e.player.group_loot_method);
            if tap_grouped && (method == 3 || method == 4) {
                self.start_loot_rolls(guid, method == 4);
            }
        }
        let permission = self.loot_permission(player, guid);
        self.player_loot_guid.insert(player, guid);
        let view = self.loot_view_bytes(player, guid, permission);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(LOOT_CORPSE);
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if permission != LootPermission::None
            && let Some(state) = self.loots.get_mut(&guid)
        {
            state.players_looting.insert(player);
        }
        if let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur | UNIT_FLAG_LOOTING);
        }
    }

    /// `CMSG_LOOT_RELEASE` (guid ignored, stored
    /// window released).
    pub fn handle_loot_release(&mut self, player: ObjectGuid) {
        self.do_loot_release(player);
    }

    /// `WorldSession::DoLootRelease` creature arm.
    fn do_loot_release(&mut self, player: ObjectGuid) {
        let Some(guid) = self.player_loot_guid.remove(&player) else {
            return;
        };
        self.send_loot_release(player, guid);
        if let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur & !UNIT_FLAG_LOOTING);
        }
        if !self.creatures.contains_key(&guid) {
            return;
        }
        let grouped_master = self
            .players
            .get(&player)
            .is_some_and(|e| e.player.group_id.is_some() && e.player.group_loot_method == 2);
        let mut clear_lootable = false;
        let mut rr_cleared = false;
        if let Some(state) = self.loots.get_mut(&guid) {
            if state.is_looted() {
                clear_lootable = true;
            } else if state.round_robin_player != 0
                && ObjectGuid::from_raw(state.round_robin_player) == player
                && !grouped_master
            {
                state.round_robin_player = 0;
                rr_cleared = true;
            }
            state.players_looting.remove(&player);
        }
        if rr_cleared {
            self.touch_dynflags(guid);
        }
        if clear_lootable {
            // `AllLootRemovedFromCorpse`: decay timer
            // scaled by `Rate.Corpse.Decay.Looted`
            // (fork default 0.0 -> next decay tick
            // removes the corpse).
            if let Some(creature) = self.creatures.get_mut(&guid) {
                #[allow(clippy::cast_precision_loss)]
                let scaled = creature.corpse_decay_timer as f64
                    * f64::from(self.config.corpse_decay_looted_rate);
                creature.corpse_decay_timer = scaled as i64;
            }
            self.set_dynflag_lootable(guid, false);
            self.loots.remove(&guid);
        }
    }

    /// `CMSG_LOOT_MONEY` creature arm.
    pub fn handle_loot_money(&mut self, player: ObjectGuid) {
        let Some(guid) = self.player_loot_guid.get(&player).copied() else {
            return;
        };
        if !guid.is_creature()
            || !self.creatures.contains_key(&guid)
            || self.unit_is_alive(guid)
            || !self.within_loot_range(player, guid)
        {
            return;
        }
        let Some(gold) = self.loots.get(&guid).map(|s| s.gold) else {
            return;
        };
        self.notify_money_removed(guid);
        let group_id = self.players.get(&player).and_then(|e| e.player.group_id);
        if let Some(g) = group_id {
            let near: Vec<ObjectGuid> = self
                .players
                .iter()
                .filter(|(_, e)| e.player.group_id == Some(g))
                .map(|(guid2, _)| *guid2)
                .filter(|m| self.within_loot_xp_dist_players(player, *m))
                .collect();
            if !near.is_empty() {
                let per = gold / near.len() as u32;
                for member in near {
                    self.modify_money(member, i64::from(per));
                    let mut writer = PacketWriter::new();
                    writer.write_u32_le(per);
                    let packet = WorldPacket::with_body(
                        opcode_id::SMSG_LOOT_MONEY_NOTIFY,
                        writer.into_vec(),
                    );
                    self.send_to_player(member, packet);
                }
            }
        } else {
            self.modify_money(player, i64::from(gold));
        }
        if let Some(state) = self.loots.get_mut(&guid) {
            state.gold = 0;
        }
    }

    /// Resolved autostore slot (fork `LootItemInSlot`
    /// out-params flattened).
    fn loot_item_in_slot(
        &self,
        creature: ObjectGuid,
        slot: u8,
        player: ObjectGuid,
    ) -> Option<LootSlotRef> {
        let state = self.loots.get(&creature)?;
        let slot = slot as usize;
        let low = player.counter();
        if slot >= state.items.len() {
            let quest_slot = slot - state.items.len();
            let marks = state.player_quest_items.get(&low)?;
            let mark = marks.get(quest_slot)?;
            let item = state.quest_items[mark.index as usize];
            if item.is_looted || mark.is_looted {
                return None;
            }
            return Some(LootSlotRef {
                item,
                kind: SlotKind::Quest(mark.index),
            });
        }
        let item = state.items[slot];
        if item.freeforall {
            if let Some(marks) = state.player_ffa_items.get(&low)
                && let Some(mark) = marks.iter().find(|m| m.index as usize == slot)
            {
                if mark.is_looted {
                    return None;
                }
                return Some(LootSlotRef {
                    item,
                    kind: SlotKind::Ffa(slot as u8),
                });
            }
            // No FFA mark: fork returns the item with
            // `is_looted` from the slot itself.
            if item.is_looted {
                return None;
            }
            return Some(LootSlotRef {
                item,
                kind: SlotKind::Plain,
            });
        }
        if item.condition_id != 0
            && !item.is_looted
            && let Some(marks) = state.player_cond_items.get(&low)
            && let Some(mark) = marks.iter().find(|m| m.index as usize == slot)
        {
            if mark.is_looted {
                return None;
            }
            return Some(LootSlotRef {
                item,
                kind: SlotKind::Cond(slot as u8),
            });
        }
        if item.is_looted {
            return None;
        }
        Some(LootSlotRef {
            item,
            kind: SlotKind::Plain,
        })
    }

    /// Post-grant marks, notifies and count
    /// bookkeeping for one taken slot (fork order
    /// in `HandleAutostoreLootItemOpcode`).
    fn apply_loot_take_marks(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        slot: u8,
        resolved: &LootSlotRef,
    ) {
        let item = resolved.item;
        let low = player.counter();
        match resolved.kind {
            SlotKind::Quest(index) => {
                let single_quest_list = self
                    .loots
                    .get(&guid)
                    .is_some_and(|s| s.player_quest_items.len() == 1);
                if let Some(state) = self.loots.get_mut(&guid)
                    && let Some(marks) = state.player_quest_items.get_mut(&low)
                    && let Some(mark) = marks.iter_mut().find(|m| m.index == index)
                {
                    mark.is_looted = true;
                }
                if item.freeforall || single_quest_list {
                    self.send_loot_removed(player, slot);
                } else {
                    self.notify_quest_item_removed(guid, index);
                }
            }
            SlotKind::Ffa(index) => {
                if let Some(state) = self.loots.get_mut(&guid)
                    && let Some(marks) = state.player_ffa_items.get_mut(&low)
                    && let Some(mark) = marks.iter_mut().find(|m| m.index == index)
                {
                    mark.is_looted = true;
                }
                self.send_loot_removed(player, slot);
            }
            SlotKind::Cond(index) => {
                if let Some(state) = self.loots.get_mut(&guid)
                    && let Some(marks) = state.player_cond_items.get_mut(&low)
                    && let Some(mark) = marks.iter_mut().find(|m| m.index == index)
                {
                    mark.is_looted = true;
                }
                self.notify_item_removed(guid, slot);
            }
            SlotKind::Plain => {
                self.notify_item_removed(guid, slot);
            }
        }
        if let Some(state) = self.loots.get_mut(&guid) {
            if !item.freeforall {
                match resolved.kind {
                    SlotKind::Quest(index) => {
                        state.quest_items[index as usize].is_looted = true;
                    }
                    _ => {
                        state.items[slot as usize].is_looted = true;
                    }
                }
            }
            // Saturating: fork decrements (wraps) in
            // the no-mark FFA quirk path.
            state.unlooted_count = state.unlooted_count.saturating_sub(1);
        }
    }

    /// `CMSG_AUTOSTORE_LOOT_ITEM` creature arm.
    pub fn handle_autostore_loot_item(&mut self, player: ObjectGuid, slot: u8) {
        let Some(guid) = self.player_loot_guid.get(&player).copied() else {
            return;
        };
        if !guid.is_creature() || !self.creatures.contains_key(&guid) || self.unit_is_alive(guid) {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        if !self.within_loot_range(player, guid) {
            self.send_loot_error(player, guid, loot_error::TOO_FAR);
            return;
        }
        let Some(resolved) = self.loot_item_in_slot(guid, slot, player) else {
            self.send_inventory_error(
                player,
                crate::inventory::ERR_ALREADY_LOOTED,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let item = resolved.item;
        if !self.loot_allowed_for_player(&item, player, guid) {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        // Quest items reuse `is_blocked`; everything
        // else blocked is roll-owned.
        if !matches!(resolved.kind, SlotKind::Quest(_)) && item.is_blocked {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        // Master-loot steal guard.
        if !item.is_underthreshold
            && matches!(resolved.kind, SlotKind::Plain | SlotKind::Cond(_))
            && self
                .players
                .get(&player)
                .is_some_and(|e| e.player.group_id.is_some() && e.player.group_loot_method == 2)
        {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() else {
            return;
        };
        if let Err(msg) = self.grant_loot_item(player, &proto, u32::from(item.count)) {
            self.send_inventory_error(player, msg, ObjectGuid::EMPTY, ObjectGuid::EMPTY, 0);
            return;
        }
        self.apply_loot_take_marks(player, guid, slot, &resolved);
    }

    /// `CMSG_LOOT_MASTER_GIVE`.
    pub fn handle_loot_master_give(
        &mut self,
        player: ObjectGuid,
        loot_guid: ObjectGuid,
        slot: u8,
        target_guid: ObjectGuid,
    ) {
        let master_ok = self.players.get(&player).is_some_and(|e| {
            e.player.group_id.is_some()
                && e.player.group_loot_method == 2
                && e.player.group_loot_master_low == player.counter()
        });
        if !master_ok {
            self.send_loot_error(player, loot_guid, loot_error::DIDNT_KILL);
            return;
        }
        if !self.players.contains_key(&target_guid) {
            self.send_loot_error(player, loot_guid, loot_error::PLAYER_NOT_FOUND);
            return;
        }
        let same_group = {
            let a = self.players.get(&player).and_then(|e| e.player.group_id);
            let b = self
                .players
                .get(&target_guid)
                .and_then(|e| e.player.group_id);
            a.is_some() && a == b
        };
        if !same_group {
            self.send_loot_error(player, loot_guid, loot_error::MASTER_OTHER);
            return;
        }
        if self.player_loot_guid.get(&player) != Some(&loot_guid) {
            self.send_loot_error(player, loot_guid, loot_error::DIDNT_KILL);
            return;
        }
        if !loot_guid.is_creature() || !self.creatures.contains_key(&loot_guid) {
            self.send_loot_error(player, loot_guid, loot_error::DIDNT_KILL);
            return;
        }
        let at_reward = {
            let entry = self.creatures.get(&loot_guid).map_or(0, |c| c.info.entry);
            let info = self
                .stores
                .creature_templates
                .get(entry)
                .cloned()
                .unwrap_or_default();
            let creature_pos = self.unit_position(loot_guid).unwrap_or_default();
            self.unit_position(player)
                .is_some_and(|p| self.at_group_reward_distance(p, creature_pos, &info))
        };
        if !at_reward {
            self.send_loot_error(player, loot_guid, loot_error::TOO_FAR);
            return;
        }
        let Some(item) = self
            .loots
            .get(&loot_guid)
            .and_then(|s| s.items.get(slot as usize).copied())
        else {
            self.send_loot_release(player, loot_guid);
            self.send_inventory_error(
                player,
                crate::inventory::ERR_ITEM_NOT_FOUND,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let allowed = self
            .loots
            .get(&loot_guid)
            .is_some_and(|s| s.is_allowed_looter(target_guid));
        if !allowed {
            self.send_loot_error(player, loot_guid, loot_error::MASTER_OTHER);
            return;
        }
        let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() else {
            self.send_loot_error(player, loot_guid, loot_error::MASTER_OTHER);
            return;
        };
        if let Err(msg) = self.grant_loot_item(target_guid, &proto, u32::from(item.count)) {
            self.send_inventory_error(target_guid, msg, ObjectGuid::EMPTY, ObjectGuid::EMPTY, 0);
            let mapped = if msg == crate::inventory::ERR_BAG_FULL
                || msg == crate::inventory::ERR_INVENTORY_FULL
            {
                loot_error::MASTER_INV_FULL
            } else if msg == crate::inventory::ERR_CANT_CARRY_MORE_OF_THIS {
                loot_error::MASTER_UNIQUE_ITEM
            } else {
                loot_error::MASTER_OTHER
            };
            self.send_loot_error(player, loot_guid, mapped);
            return;
        }
        if let Some(state) = self.loots.get_mut(&loot_guid)
            && let Some(slot_item) = state.items.get_mut(slot as usize)
        {
            slot_item.count = 0;
            slot_item.is_looted = true;
            state.unlooted_count = state.unlooted_count.saturating_sub(1);
        }
        self.notify_item_removed(loot_guid, slot);
    }

    // --------------------------------------------------
    // Group loot rolls (`Group::GroupLoot` /
    // `NeedBeforeGreed` / `CountTheRoll`), map-local:
    // the fork voter set is already restricted to
    // allowed looters within loot XP distance of the
    // corpse, so no voter can live on another map.
    // --------------------------------------------------

    /// `SMSG_LOOT_START_ROLL` to one voter.
    fn send_start_roll(
        &mut self,
        voter: ObjectGuid,
        creature: ObjectGuid,
        roll: &LootRoll,
        countdown_ms: u32,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.raw());
        writer.write_u32_le(u32::from(roll.item_slot));
        writer.write_u32_le(roll.itemid);
        writer.write_u32_le(0);
        writer.write_u32_le(0); // random property id (gap)
        writer.write_u32_le(countdown_ms);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_START_ROLL, writer.into_vec());
        self.send_to_player(voter, packet);
    }

    /// `Group::SendLootRoll` broadcast to voters.
    fn broadcast_loot_roll(
        &mut self,
        creature: ObjectGuid,
        roll: &LootRoll,
        voter: ObjectGuid,
        roll_number: u8,
        roll_type: u8,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.raw());
        writer.write_u32_le(u32::from(roll.item_slot));
        writer.write_u64_le(voter.raw());
        writer.write_u32_le(roll.itemid);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        writer.write_u8(roll_number);
        writer.write_u8(roll_type);
        let body = writer.into_vec();
        for low in roll.votes.keys().copied().collect::<Vec<_>>() {
            let guid = ObjectGuid::new(mangos_guid::HighGuid::PLAYER, low);
            if self.players.contains_key(&guid) {
                let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_ROLL, body.clone());
                self.send_to_player(guid, packet);
            }
        }
    }

    /// `Group::SendLootRollWon` broadcast to voters.
    fn broadcast_loot_roll_won(
        &mut self,
        creature: ObjectGuid,
        roll: &LootRoll,
        winner: ObjectGuid,
        roll_number: u8,
        roll_type: u8,
    ) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.raw());
        writer.write_u32_le(u32::from(roll.item_slot));
        writer.write_u32_le(roll.itemid);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        writer.write_u64_le(winner.raw());
        writer.write_u8(roll_number);
        writer.write_u8(roll_type);
        let body = writer.into_vec();
        for low in roll.votes.keys().copied().collect::<Vec<_>>() {
            let guid = ObjectGuid::new(mangos_guid::HighGuid::PLAYER, low);
            if self.players.contains_key(&guid) {
                let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_ROLL_WON, body.clone());
                self.send_to_player(guid, packet);
            }
        }
    }

    /// `Group::SendLootAllPassed` broadcast to voters.
    fn broadcast_loot_all_passed(&mut self, creature: ObjectGuid, roll: &LootRoll) {
        let mut writer = PacketWriter::new();
        writer.write_u64_le(creature.raw());
        writer.write_u32_le(u32::from(roll.item_slot));
        writer.write_u32_le(roll.itemid);
        writer.write_u32_le(0);
        writer.write_u32_le(0);
        let body = writer.into_vec();
        for low in roll.votes.keys().copied().collect::<Vec<_>>() {
            let guid = ObjectGuid::new(mangos_guid::HighGuid::PLAYER, low);
            if self.players.contains_key(&guid) {
                let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_ALL_PASSED, body.clone());
                self.send_to_player(guid, packet);
            }
        }
    }

    /// `Group::GroupLoot` / `NeedBeforeGreed` pass
    /// over the corpse items.
    fn start_loot_rolls(&mut self, creature: ObjectGuid, need_before_greed: bool) {
        let Some(state) = self.loots.get(&creature) else {
            return;
        };
        let threshold = state
            .allowed_looters
            .first()
            .and_then(|g| self.players.get(g))
            .map_or(2, |e| e.player.group_loot_threshold);
        let slots: Vec<(u8, LootSlotItem)> = state
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (i as u8, *it))
            .collect();
        for (slot, item) in slots {
            if item.freeforall {
                continue;
            }
            let Some(proto) = self.stores.item_prototypes.get(item.itemid) else {
                continue;
            };
            if proto.quality >= u32::from(threshold) {
                self.start_one_roll(creature, slot, &item, need_before_greed);
            } else if let Some(state) = self.loots.get_mut(&creature) {
                state.items[slot as usize].is_underthreshold = true;
            }
        }
    }

    /// `Group::StartLootRoll` for one slot.
    fn start_one_roll(
        &mut self,
        creature: ObjectGuid,
        slot: u8,
        item: &LootSlotItem,
        need_before_greed: bool,
    ) {
        let tap_group = self.loot_taps.get(&creature).and_then(|t| t.group_id);
        let Some(group_id) = tap_group else {
            return;
        };
        let entry_id = self.creatures.get(&creature).map_or(0, |c| c.info.entry);
        let info = self
            .stores
            .creature_templates
            .get(entry_id)
            .cloned()
            .unwrap_or_default();
        let creature_pos = self.unit_position(creature).unwrap_or_default();
        let proto = self.stores.item_prototypes.get(item.itemid).cloned();
        let mut voters: Vec<ObjectGuid> = Vec::new();
        for (guid, entry) in &self.players {
            if entry.player.group_id != Some(group_id) {
                continue;
            }
            if need_before_greed {
                let usable = proto.as_ref().is_some_and(|p| {
                    self.can_use_item(&entry.player, p) == crate::inventory::ERR_OK
                });
                if !usable {
                    continue;
                }
            }
            if !self.loot_allowed_for_player(item, *guid, creature) {
                continue;
            }
            let allowed = self
                .loots
                .get(&creature)
                .is_some_and(|s| s.is_allowed_looter(*guid));
            if !allowed {
                continue;
            }
            let near = self
                .unit_position(*guid)
                .is_some_and(|p| self.at_group_reward_distance(p, creature_pos, &info));
            if near {
                voters.push(*guid);
            }
        }
        if voters.is_empty() {
            return;
        }
        let mut roll = LootRoll {
            item_slot: slot,
            itemid: item.itemid,
            count: item.count,
            votes: voters.iter().map(|g| (g.counter(), 3)).collect(),
            total_need: 0,
            total_greed: 0,
            total_pass: 0,
            deadline_ms: u64::from(self.now_ms) + LOOT_ROLL_TIMEOUT_MS,
        };
        if voters.len() == 1 {
            // `CountSingleLooterRoll`: forced need win
            // at roll 100, no start packet.
            let only = voters[0];
            roll.votes.insert(only.counter(), 1);
            self.broadcast_loot_roll_won(creature, &roll, only, 100, 1);
            self.award_roll_item(creature, &roll, only, true);
            return;
        }
        if let Some(state) = self.loots.get_mut(&creature) {
            state.items[slot as usize].is_blocked = true;
            state.rolls.push(roll.clone());
        }
        for voter in voters {
            self.send_start_roll(voter, creature, &roll, LOOT_ROLL_TIMEOUT_MS as u32);
        }
    }

    /// Winner grant shared by need/greed/single
    /// resolutions. `need` selects the fork failure
    /// arm (need sets `lootOwner`, greed does not).
    fn award_roll_item(
        &mut self,
        creature: ObjectGuid,
        roll: &LootRoll,
        winner: ObjectGuid,
        need: bool,
    ) {
        if !self.players.contains_key(&winner) {
            // Fork: need winner offline still claims
            // the item via `lootOwner` (and it stays
            // blocked); greed winner offline changes
            // nothing.
            if need
                && let Some(state) = self.loots.get_mut(&creature)
                && let Some(item) = state.items.get_mut(roll.item_slot as usize)
            {
                item.loot_owner = winner;
            }
            return;
        }
        let Some(proto) = self.stores.item_prototypes.get(roll.itemid).cloned() else {
            return;
        };
        match self.grant_loot_item_notify(winner, &proto, u32::from(roll.count), false) {
            Ok(()) => {
                if let Some(state) = self.loots.get_mut(&creature)
                    && let Some(item) = state.items.get_mut(roll.item_slot as usize)
                {
                    item.is_looted = true;
                    state.unlooted_count = state.unlooted_count.saturating_sub(1);
                }
                self.notify_item_removed(creature, roll.item_slot);
            }
            Err(msg) => {
                if let Some(state) = self.loots.get_mut(&creature)
                    && let Some(item) = state.items.get_mut(roll.item_slot as usize)
                {
                    item.is_blocked = false;
                    if need {
                        item.loot_owner = winner;
                    }
                }
                self.send_inventory_error(winner, msg, ObjectGuid::EMPTY, ObjectGuid::EMPTY, 0);
            }
        }
    }

    /// `Group::CountTheRoll`.
    fn resolve_roll(&mut self, creature: ObjectGuid, roll: &LootRoll) {
        if roll.total_need > 0 {
            let mut best: u8 = 0;
            let mut winner = ObjectGuid::new(
                mangos_guid::HighGuid::PLAYER,
                roll.votes.keys().next().copied().unwrap_or(0),
            );
            let mut rolled: Vec<(ObjectGuid, u8)> = Vec::new();
            for (low, vote) in &roll.votes {
                if *vote != 1 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                let n = self.rng.urand(1, 100) as u8;
                rolled.push((ObjectGuid::new(mangos_guid::HighGuid::PLAYER, *low), n));
                if best < n {
                    best = n;
                    winner = ObjectGuid::new(mangos_guid::HighGuid::PLAYER, *low);
                }
            }
            for (voter, n) in rolled {
                self.broadcast_loot_roll(creature, roll, voter, n, 1);
            }
            self.broadcast_loot_roll_won(creature, roll, winner, best, 1);
            self.award_roll_item(creature, roll, winner, true);
        } else if roll.total_greed > 0 {
            let mut best: u8 = 0;
            let mut winner = ObjectGuid::new(
                mangos_guid::HighGuid::PLAYER,
                roll.votes.keys().next().copied().unwrap_or(0),
            );
            let mut rolled: Vec<(ObjectGuid, u8)> = Vec::new();
            for (low, vote) in &roll.votes {
                if *vote != 2 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                let n = self.rng.urand(1, 100) as u8;
                rolled.push((ObjectGuid::new(mangos_guid::HighGuid::PLAYER, *low), n));
                if best < n {
                    best = n;
                    winner = ObjectGuid::new(mangos_guid::HighGuid::PLAYER, *low);
                }
            }
            for (voter, n) in rolled {
                self.broadcast_loot_roll(creature, roll, voter, n, 2);
            }
            self.broadcast_loot_roll_won(creature, roll, winner, best, 2);
            self.award_roll_item(creature, roll, winner, false);
        } else {
            self.broadcast_loot_all_passed(creature, roll);
            if let Some(state) = self.loots.get_mut(&creature)
                && let Some(item) = state.items.get_mut(roll.item_slot as usize)
            {
                item.is_blocked = false;
            }
        }
    }

    /// `CMSG_LOOT_ROLL` (`HandleLootRoll` +
    /// `Group::CountRollVote`). Fork counts repeat
    /// votes from the same player again (totals are
    /// incremented per call); replicated as read.
    pub fn handle_loot_roll(
        &mut self,
        player: ObjectGuid,
        creature: ObjectGuid,
        slot: u32,
        choice: u8,
    ) {
        if choice >= 3 {
            return;
        }
        if self
            .players
            .get(&player)
            .is_none_or(|e| e.player.group_id.is_none())
        {
            return;
        }
        let low = player.counter();
        let (roll_snapshot, complete) = {
            let Some(state) = self.loots.get_mut(&creature) else {
                return;
            };
            let Some(roll) = state
                .rolls
                .iter_mut()
                .find(|r| u32::from(r.item_slot) == slot)
            else {
                return;
            };
            if !roll.votes.contains_key(&low) || state.items.is_empty() {
                return;
            }
            roll.votes.insert(low, choice);
            match choice {
                0 => roll.total_pass += 1,
                1 => roll.total_need += 1,
                _ => roll.total_greed += 1,
            }
            let done =
                roll.total_pass + roll.total_need + roll.total_greed >= roll.votes.len() as u32;
            (roll.clone(), done)
        };
        // Vote echo (`SendLootRoll` per choice).
        let (number, kind) = match choice {
            0 => (128u8, 128u8),
            1 => (0u8, 0u8),
            _ => (128u8, 2u8),
        };
        self.broadcast_loot_roll(creature, &roll_snapshot, player, number, kind);
        if complete {
            let roll = {
                let Some(state) = self.loots.get_mut(&creature) else {
                    return;
                };
                let Some(pos) = state
                    .rolls
                    .iter()
                    .position(|r| u32::from(r.item_slot) == slot)
                else {
                    return;
                };
                state.rolls.remove(pos)
            };
            self.resolve_roll(creature, &roll);
        }
    }

    /// Roll timeouts (fork creature group-loot timer
    /// expiring into `Group::EndRoll`, which resolves
    /// every roll on the corpse).
    pub(crate) fn update_loot_rolls(&mut self) {
        let now = u64::from(self.now_ms);
        let due: Vec<ObjectGuid> = self
            .loots
            .iter()
            .filter(|(_, s)| s.rolls.iter().any(|r| now >= r.deadline_ms))
            .map(|(g, _)| *g)
            .collect();
        for creature in due {
            let rolls: Vec<LootRoll> = self
                .loots
                .get_mut(&creature)
                .map(|s| std::mem::take(&mut s.rolls))
                .unwrap_or_default();
            for roll in rolls {
                self.resolve_roll(creature, &roll);
            }
        }
    }

    /// `Group::SendLootStartRollsForPlayer`: re-send
    /// pending roll prompts to a player entering the
    /// map.
    pub(crate) fn send_pending_rolls(&mut self, player: ObjectGuid) {
        let now = u64::from(self.now_ms);
        let low = player.counter();
        let pending: Vec<(ObjectGuid, LootRoll, u32)> = self
            .loots
            .iter()
            .flat_map(|(creature, state)| {
                state
                    .rolls
                    .iter()
                    .filter(|r| r.votes.get(&low) == Some(&3) && r.deadline_ms > now)
                    .map(move |r| {
                        (
                            *creature,
                            r.clone(),
                            (r.deadline_ms - now).min(u64::from(u32::MAX)) as u32,
                        )
                    })
            })
            .collect();
        for (creature, roll, countdown) in pending {
            self.send_start_roll(player, creature, &roll, countdown);
        }
    }
}

/// `LOOT_ROLL_TIMEOUT` (1 minute).
const LOOT_ROLL_TIMEOUT_MS: u64 = 60_000;

/// Which per-player list a resolved slot belongs to.
#[derive(Clone, Copy, Debug)]
enum SlotKind {
    /// `quest_items[index]` via a quest mark.
    Quest(u8),
    /// `items[index]` via an FFA mark.
    Ffa(u8),
    /// `items[index]` via a conditional mark.
    Cond(u8),
    /// `items[slot]` direct.
    Plain,
}

struct LootSlotRef {
    item: LootSlotItem,
    kind: SlotKind,
}

impl mangos_world_data::loot::LootRng for crate::WorldRng {
    fn chance100(&mut self) -> f32 {
        self.rand_norm() * 100.0
    }

    fn urand(&mut self, min: u32, max: u32) -> u32 {
        <crate::WorldRng as mangos_entities::creature::SpawnRng>::urand(self, min, max)
    }
}
