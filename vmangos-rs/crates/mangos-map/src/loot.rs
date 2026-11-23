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
//! Recorded UNMODELED at this layer: AI `FillLoot`
//! overrides, `CAN_WIELD_LOOT` spawn pre-generation,
//! actor-side round-robin looter rotation
//! (`Group::UpdateLooterGuid` / `SendLooter` /
//! `SMSG_PARTYKILLLOG` — group-rule chests fill for
//! the whole group instead; the creature RR player
//! here is the kill-time tap recipient),
//! `BindToInstanceOrRaid`, chest lock skill-up
//! (`m_SkillupSet`), disenchant-on-roll,
//! damage-origin loot gating (shared gap with XP,
//! `xp.rs`), `lootForCreator`, BG-group
//! tap/allowed arms, random property ids (global
//! grant-path gap).

use std::collections::{HashMap, HashSet};

use mangos_db::DbError;
use mangos_db::character_load::ItemLootRow;
use mangos_db::character_save::LootSave;
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::item_prototype::ItemPrototype;
use mangos_world_data::loot::LootRng as _;
use mangos_world_data::loot::{GeneratedLootItem, LootRollCtx, generate_loot, generate_money_loot};

use crate::inventory::{
    ERR_ALREADY_LOOTED, ERR_CANT_DO_RIGHT_NOW, ERR_ITEM_LOCKED, ERR_ITEM_NOT_FOUND,
    ERR_YOU_ARE_DEAD, InvPos, NULL_BAG, NULL_SLOT,
};
use crate::map::Map;

/// `LOOT_CORPSE`.
pub const LOOT_CORPSE: u8 = 1;
/// `LOOT_PICKPOCKETING`: the wire byte pickpocket
/// windows (and remapped skinning windows) send
/// (`Player.cpp:8054-8056`; `LootMgr.h:50-56`).
pub const LOOT_PICKPOCKETING: u8 = 2;
/// `LOOT_SKINNING` (`LootMgr.h:56`): the internal
/// loot type `EffectOpenLock` passes for GO chests
/// (remapped to [`LOOT_PICKPOCKETING`] on the wire,
/// `Player.cpp:8051-8056`).
pub const LOOT_SKINNING: u8 = 6;
/// `LOOT_INSIGNIA` (`LootMgr.h:62`): the internal
/// loot type `EffectSkinPlayerCorpse` passes for bones
/// (remapped to [`LOOT_CORPSE`] on the wire, fork
/// `Player.cpp:8051-8068`).
pub const LOOT_INSIGNIA: u8 = 22;
/// `LOOT_FISHING` (`LootMgr.h:58`): the wire byte for
/// bobber windows, and — via the fork remap at
/// `Player.cpp:8063-8068` — for fishing-hole and
/// junk-fail windows too.
pub const LOOT_FISHING: u8 = 3;
/// `LOOT_FISHINGHOLE` (`LootMgr.h:59`): the internal
/// kind the hole Use arm passes (wired as
/// [`LOOT_FISHING`]).
pub const LOOT_FISHINGHOLE: u8 = 20;
/// `LOOT_FISHING_FAIL` (`LootMgr.h:60`): the internal
/// kind for the junk catch (entry 0 of the fishing
/// store, wired as [`LOOT_FISHING`]).
pub const LOOT_FISHING_FAIL: u8 = 21;
/// `SKILL_MINING` (`SharedDefines.h:998`): vein
/// depletion rolls scale on it.
const SKILL_MINING: u32 = 186;
/// Chest open/close anims (`GameObjectDefines.h:180`).
const GO_STATE_ACTIVE: u32 = 0;
const GO_STATE_READY: u32 = 1;
/// Vein fallback `ReqValue` when the chest lock row
/// is missing (`LootHandler.cpp:485-487`).
const VEIN_REQ_FALLBACK: i32 = 175;
/// Partial-loot chest cooldown (`LootHandler.cpp:518`:
/// `SetCooldownTime(now + 5 * MINUTE)`).
const CHEST_PARTIAL_LOOT_COOLDOWN_SECS: u64 = 5 * 60;
/// `MAX_NR_LOOT_ITEMS`.
const MAX_NR_LOOT_ITEMS: usize = 16;
/// `INTERACTION_DISTANCE` (`ObjectDefines.h:24`):
/// the bones money-take band (fork
/// `LootHandler.cpp:264-272`).
const CORPSE_MONEY_DISTANCE: f32 = 5.0;
/// `AURA_INTERRUPT_LOOTING_CANCELS`.
const AURA_INTERRUPT_LOOTING_CANCELS: u32 = 0x0000_0800;
/// `UNIT_FLAG_LOOTING`.
const UNIT_FLAG_LOOTING: u32 = 0x0000_0400;
/// `UNIT_FLAG_SKINNABLE` (`UnitDefines.h:567`).
const UNIT_FLAG_SKINNABLE: u32 = 0x0400_0000;
/// `CREATURE_TYPE_CRITTER` (`CreatureDefines.h:50`).
const CREATURE_TYPE_CRITTER: u32 = 8;
/// `SKILL_SKINNING` (`SharedDefines.h:1041`).
const SKILL_SKINNING: u32 = 393;
/// `SKILL_FISHING` (`SharedDefines.h:1037`): the
/// bobber Use roll and `UpdateFishingSkill` scale on it.
pub(crate) const SKILL_FISHING: u32 = 356;
/// `SkillChance` grey/green/yellow/orange defaults
/// (`World.cpp:686-689`: 0/25/75/100).
const SKILL_CHANCE_GREY: i32 = 0;
const SKILL_CHANCE_GREEN: i32 = 25;
const SKILL_CHANCE_YELLOW: i32 = 75;
const SKILL_CHANCE_ORANGE: i32 = 100;
/// `SkillChance.SkinningSteps` default
/// (`World.cpp:692`: 75).
const SKINNING_CHANCE_STEPS: u32 = 75;
/// `SkillGain.Gathering` default (`World.cpp:696`: 1).
const SKILL_GAIN_GATHERING: u32 = 1;
/// `Creature::skinningForOthersTimer` duration
/// (`Creature.cpp:3209`: 5000 ms).
const SKINNING_FOR_OTHERS_MS: u64 = 5000;

/// `SkillGainChance` (fork `Player.cpp:5320-5329`):
/// grey/green/yellow thresholds pick the x10 table
/// value, else orange. Pure fork math, unit-tested.
fn skill_gain_chance(skill: i32, gray: i32, green: i32, yellow: i32) -> i32 {
    if skill >= gray {
        SKILL_CHANCE_GREY * 10
    } else if skill >= green {
        SKILL_CHANCE_GREEN * 10
    } else if skill >= yellow {
        SKILL_CHANCE_YELLOW * 10
    } else {
        SKILL_CHANCE_ORANGE * 10
    }
}

/// Orange-attempt fail predicate (fork `Spell.cpp:6091`:
/// `(ReqValue < 0 ? 0 : ReqValue) > irand(...)`).
/// Pure fork math, unit-tested.
fn skinning_try_again_fails(req: i32, roll: i32) -> bool {
    req.max(0) > roll
}
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

/// `ITEM_DYNFLAG_UNLOCKED` (locked containers refuse
/// `CMSG_OPEN_ITEM` without a lock-template source).
const ITEM_DYNFLAG_UNLOCKED: u32 = 0x0000_0004;
/// `ITEM_DYNFLAG_WRAPPED` (gift containers unwrap
/// through `character_gifts`, never through a loot
/// window).
const ITEM_DYNFLAG_WRAPPED: u32 = 0x0000_0008;

/// `ItemLootUpdateState` (`ItemDefines.h:187-195`).
/// Absent from [`Map::item_loots`] is
/// `ITEM_LOOT_NONE`; destruction drops the entry
/// (the save shape falls back to the persisted
/// `generated_loot` word, which is exactly the fork
/// `DeleteAllFromDB` gate input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemLootUpdateState {
    Unchanged,
    Changed,
    Temporary,
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

/// Creature-loot window kinds (`SendLoot` callers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreatureLootKind {
    Corpse,
    Pickpocket,
    Skinning,
}

/// Which template store a creature-window fill
/// draws items from (fork `FillLoot` store arm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LootFillStore {
    Creature,
    Pickpocket,
    Skinning,
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
#[allow(clippy::struct_excessive_bools)] // fork `Loot` flag cluster
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
    /// `lootForSkin`: skinning generation has run
    /// (fork `Creature::lootForSkin`); the entry then
    /// holds skin loot, never body loot.
    pub loot_for_skin: bool,
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

    /// `Loot::clear` (fork `LootMgr.h:287`): full
    /// content wipe that keeps the entry alive. The
    /// bones release arm uses it so the emptied window
    /// re-opens empty (the entry is the fork
    /// `lootForBody` marker: `loot->clear()` never
    /// drops the `Loot` itself).
    pub fn clear(&mut self) {
        self.player_quest_items.clear();
        self.quest_items.clear();
        self.player_ffa_items.clear();
        self.player_cond_items.clear();
        self.players_looting.clear();
        self.items.clear();
        self.gold = 0;
        self.unlooted_count = 0;
        self.round_robin_player = 0;
        self.allowed_looters.clear();
        self.personal = true;
        self.team = 0;
        self.has_ffa_quest_items = false;
    }

    /// `Loot::leaveOnlyQuestItems` (fork
    /// `LootMgr.h:320` = `clear(false)`): stale-viewer
    /// pickpocket refills keep quest items and their
    /// per-player marks; plain items, gold, FFA and
    /// conditional marks, viewers, RR and the FFA flag
    /// drop.
    pub fn leave_only_quest_items(&mut self) {
        self.player_ffa_items.clear();
        self.player_cond_items.clear();
        self.players_looting.clear();
        self.items.clear();
        self.gold = 0;
        self.unlooted_count = 0;
        self.round_robin_player = 0;
        self.allowed_looters.clear();
        self.personal = true;
        self.has_ffa_quest_items = false;
    }

    /// Fork `Loot::empty`: no plain or quest items
    /// left and no gold (note: unlike `is_looted`,
    /// per-player marks don't count).
    #[must_use]
    pub fn is_empty_loot(&self) -> bool {
        self.items.is_empty() && self.quest_items.is_empty() && self.gold == 0
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
        self.skinning_for_others.remove(&creature);
        self.pickpocketed.remove(&creature);
        // Fork `Creature.cpp:2193` (`JUST_ALIVED`)
        // clears the flag on the same path the corpse
        // state drops here.
        if let Some(u) = self.unit_base_mut(creature) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur & !UNIT_FLAG_SKINNABLE);
        }
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
        // Fork `GenerateLootForBody` clears
        // `lootForPickPocketed` before refilling the
        // body: a killed mark wipes any pickpocket
        // window.
        self.pickpocketed.remove(&victim);
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
        let state = self.generate_window_state(
            victim,
            looter,
            loot_id,
            Some((gold_min, gold_max)),
            LootFillStore::Creature,
        );
        let allowed = state.allowed_looters.clone();
        self.loots.insert(victim, state);
        for member in allowed {
            self.loot_fill_dependent(victim, member);
        }
    }

    /// Shared creature-window fill (`Loot::FillLoot` +
    /// `FillPlayerDependentLoot` shape): group-aware
    /// allowed set around `looter` at reward distance,
    /// items from the body store (with money), the
    /// pickpocketing store (fork adds its extra money
    /// separately, `Player.cpp:7917-7919`) or the
    /// skinning store (fork adds no money on the skin
    /// arm, `Player.cpp:7904-7939` vs `:7985-8001`).
    fn generate_window_state(
        &mut self,
        victim: ObjectGuid,
        looter: ObjectGuid,
        loot_id: u32,
        money: Option<(u32, u32)>,
        store_kind: LootFillStore,
    ) -> LootState {
        let team = self.player_team_id(looter);
        let looter_group = self.players.get(&looter).and_then(|e| e.player.group_id);
        let victim_pos = self.unit_position(victim).unwrap_or_default();
        let victim_info = self
            .creatures
            .get(&victim)
            .map(|c| c.info.clone())
            .unwrap_or_default();
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
            allowed_looters: allowed,
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
            let store = match store_kind {
                LootFillStore::Creature => &stores.loot.creature,
                LootFillStore::Pickpocket => &stores.loot.pickpocketing,
                LootFillStore::Skinning => &stores.loot.skinning,
            };
            let generated = generate_loot(&mut ctx, &stores.loot, store, loot_id);
            let gold = money.map_or(0, |(min, max)| generate_money_loot(&mut ctx, min, max));
            state.gold = gold;
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
        state
    }

    /// Skinning generation (fork `Player.cpp:7985-8001`):
    /// fresh fill from the skinning store over the shared
    /// creature slot (fork `loot->clear()` + `FillLoot(
    /// skinning_loot_id, LootTemplates_Skinning, this,
    /// false)`), no money. Rolls survive the transition
    /// (fork `clear()` never owned them); the body flag
    /// is already set by the window prelude.
    fn fill_skinning_loot(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        let entry = self.creatures.get(&victim).map_or(0, |c| c.info.entry);
        let skinning_id = self
            .stores
            .creature_templates
            .get(entry)
            .map_or(0, |t| t.skinning_loot_id);
        let old_rolls = self
            .loots
            .get(&victim)
            .map(|s| s.rolls.clone())
            .unwrap_or_default();
        let mut state =
            self.generate_window_state(victim, caster, skinning_id, None, LootFillStore::Skinning);
        state.body_processed = true;
        state.loot_for_skin = true;
        state.rolls = old_rolls;
        self.loots.insert(victim, state);
        let allowed: Vec<ObjectGuid> = self
            .loots
            .get(&victim)
            .map(|s| s.allowed_looters.clone())
            .unwrap_or_default();
        for member in allowed {
            self.loot_fill_dependent(victim, member);
        }
    }

    /// Pickpocket first fill (fork `Player.cpp:7909-7921`):
    /// fresh fill from the pickpocketing store over the
    /// shared creature slot (fork `loot->clear()` +
    /// `FillLoot(pickpocket_loot_id,
    /// LootTemplates_Pickpocketing, this, false)`, skipped
    /// when the template id is 0) plus the extra money
    /// `10 * (urand(0, victimLvl / 2) + urand(0,
    /// casterLvl / 2)) * Rate.Drop.Money` (fork
    /// `:7917-7919`). Rolls survive like skinning.
    fn fill_pickpocket_loot(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        let entry = self.creatures.get(&victim).map_or(0, |c| c.info.entry);
        let pickpocket_id = self
            .stores
            .creature_templates
            .get(entry)
            .map_or(0, |t| t.pickpocket_loot_id);
        let old_rolls = self
            .loots
            .get(&victim)
            .map(|s| s.rolls.clone())
            .unwrap_or_default();
        let mut state = self.generate_window_state(
            victim,
            caster,
            pickpocket_id,
            None,
            LootFillStore::Pickpocket,
        );
        let victim_half = self.unit_level(victim) / 2;
        let caster_half = self.unit_level(caster) / 2;
        let rate = self.config.loot_rates.money;
        let (first, second) = (
            self.rng.urand(0, victim_half),
            self.rng.urand(0, caster_half),
        );
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let extra = ((10 * (first + second)) as f32 * rate) as u32;
        state.gold += extra;
        state.body_processed = true;
        state.rolls = old_rolls;
        self.loots.insert(victim, state);
        self.pickpocketed.insert(victim);
        let allowed: Vec<ObjectGuid> = self
            .loots
            .get(&victim)
            .map(|s| s.allowed_looters.clone())
            .unwrap_or_default();
        for member in allowed {
            self.loot_fill_dependent(victim, member);
        }
    }

    /// Pickpocket stale-viewer refill (fork
    /// `Player.cpp:7923-7933`): the window is empty or
    /// the viewer is not the original looter, so the
    /// slot is cleared, refilled, and stripped to quest
    /// items only (`leaveOnlyQuestItems`). No extra
    /// money on this arm (fork adds it on first fill
    /// only).
    fn refill_pickpocket_quest_only(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        let entry = self.creatures.get(&victim).map_or(0, |c| c.info.entry);
        let pickpocket_id = self
            .stores
            .creature_templates
            .get(entry)
            .map_or(0, |t| t.pickpocket_loot_id);
        let old_rolls = self
            .loots
            .get(&victim)
            .map(|s| s.rolls.clone())
            .unwrap_or_default();
        let mut state = self.generate_window_state(
            victim,
            caster,
            pickpocket_id,
            None,
            LootFillStore::Pickpocket,
        );
        state.leave_only_quest_items();
        state.body_processed = true;
        state.rolls = old_rolls;
        self.loots.insert(victim, state);
    }

    /// `Creature::SetDeathState(JUST_DIED)` skinnable
    /// gate (fork `Creature.cpp:2143-2146`): the flag is
    /// set only for non-pets whose template has skinning
    /// loot with rows and whose kill was tapped (fork
    /// `HasLootRecipient`).
    pub(crate) fn set_skinnable_on_kill(&mut self, victim: ObjectGuid) {
        if self.is_pet(victim) {
            return;
        }
        let entry = self.creatures.get(&victim).map_or(0, |c| c.info.entry);
        let skinning_id = self
            .stores
            .creature_templates
            .get(entry)
            .map_or(0, |t| t.skinning_loot_id);
        if skinning_id == 0 || !self.stores.loot.skinning.have_loot_for(skinning_id) {
            return;
        }
        if !self.loot_taps.contains_key(&victim) {
            return;
        }
        if let Some(u) = self.unit_base_mut(victim) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur | UNIT_FLAG_SKINNABLE);
        }
    }

    /// `Creature::IsSkinnableBy` (fork `Creature.h:312`):
    /// anyone once the head-start deadline passes, else
    /// only the tapper (fork `!skinningForOthersTimer
    /// || IsTappedBy`).
    fn is_skinnable_by(&self, player: ObjectGuid, creature: ObjectGuid) -> bool {
        if self
            .skinning_for_others
            .get(&creature)
            .is_none_or(|until| u64::from(self.now_ms) >= *until)
        {
            return true;
        }
        let Some(tap) = self.loot_taps.get(&creature) else {
            return false;
        };
        self.is_tapped_by(player, tap)
    }

    /// `GetConfigMaxSkillValue` (fork `World.h:753-758`):
    /// the skill ceiling for the server's max player
    /// level, gating the orange-attempt roll.
    fn max_skill_value(&self) -> u32 {
        let cap = self.stores.player_infos.max_player_level();
        if cap > 60 {
            300 + ((cap - 60) * 75) / 10
        } else {
            cap * 5
        }
    }

    /// Pickpocket cast gate (fork `Spell.cpp:6188-6198`):
    /// the target must be a creature with no player
    /// owner, else `BAD_TARGETS`; without a pickpocket
    /// template id and without a humanoid-or-undead
    /// type mask, `TARGET_NO_POCKETS`. Returns the fork
    /// fail byte, or `None` when the cast may proceed
    /// (alive/range/hostile gates stay in the generic
    /// unit arm).
    pub(crate) fn pickpocket_cast_error(
        &self,
        _caster: ObjectGuid,
        victim: ObjectGuid,
    ) -> Option<u8> {
        use mangos_spells::defines::cast_result;
        use mangos_world_data::creature_template::{CREATURE_TYPE_HUMANOID, CREATURE_TYPE_UNDEAD};
        // Fork `:6191`: creature target with a
        // non-player owner only.
        let Some(creature) = self.creatures.get(&victim) else {
            return Some(cast_result::FAILED_BAD_TARGETS);
        };
        if self.pet_owner(victim).is_some_and(ObjectGuid::is_player) {
            return Some(cast_result::FAILED_BAD_TARGETS);
        }
        // Fork `:6194`: pockets need a template id or
        // the humanoid/undead type mask (fork
        // `CreatureDefines.h:56`).
        let entry = creature.info.entry;
        let template = self.stores.creature_templates.get(entry);
        let pickpocket_id = template.map_or(0, |t| t.pickpocket_loot_id);
        let creature_type = creature.info.creature_type;
        if pickpocket_id == 0
            && creature_type != CREATURE_TYPE_HUMANOID
            && creature_type != CREATURE_TYPE_UNDEAD
        {
            return Some(cast_result::FAILED_TARGET_NO_POCKETS);
        }
        // The caster-player gate lives with the effect
        // arm (fork `EffectPickPocket` silent return);
        // the cast check keys on the victim only.
        None
    }

    /// Skinning cast gate (fork `Spell.cpp:6069-6094`),
    /// checked before the alive gate: corpses are dead
    /// by definition. Returns the fork fail byte, or
    /// `None` when the cast may proceed.
    pub(crate) fn skinning_cast_error(
        &mut self,
        caster: ObjectGuid,
        victim: ObjectGuid,
        triggered: bool,
    ) -> Option<u8> {
        use mangos_spells::defines::cast_result;
        // Player caster, unit target (fork `:6071`).
        if !caster.is_player()
            || !self.players.contains_key(&caster)
            || !victim.is_creature()
            || !self.creatures.contains_key(&victim)
        {
            return Some(cast_result::FAILED_BAD_TARGETS);
        }
        // `UNIT_FLAG_SKINNABLE` (fork `:6074`).
        if self.unit_value_u32(
            victim,
            mangos_protocol::update_field_table::offsets::UNIT_FIELD_FLAGS,
        ) & UNIT_FLAG_SKINNABLE
            == 0
        {
            return Some(cast_result::FAILED_TARGET_UNSKINNABLE);
        }
        // `IsSkinnableBy` (fork `:6077`).
        if !self.is_skinnable_by(caster, victim) {
            return Some(cast_result::FAILED_TARGET_NOT_LOOTED);
        }
        let skill = self
            .players
            .get(&caster)
            .map_or(0, |e| Self::skill_value(&e.player, SKILL_SKINNING));
        let level = self.unit_level(victim);
        // Fork `:6083` wraps `int32` math: levels below
        // 10 go negative and pass this gate.
        #[allow(clippy::cast_possible_wrap)]
        let req = if skill < 100 {
            (level as i32 - 10) * 10
        } else {
            level as i32 * 5
        };
        #[allow(clippy::cast_possible_wrap)]
        if req > skill as i32 {
            return Some(cast_result::FAILED_LOW_CASTLEVEL);
        }
        // Non-critters need a fully-looted, unskinned
        // body (fork `:6087`).
        let creature_type = self
            .creatures
            .get(&victim)
            .map_or(0, |c| c.info.creature_type);
        let skinned = self.loots.get(&victim).is_some_and(|s| s.loot_for_skin);
        let looted = self.loots.get(&victim).is_none_or(LootState::is_looted);
        if creature_type != CREATURE_TYPE_CRITTER && (skinned || !looted) {
            return Some(cast_result::FAILED_TARGET_NOT_LOOTED);
        }
        // Orange-attempt fail roll (fork `:6091`):
        // non-triggered casts below the skill ceiling
        // fail when the clamped req beats
        // `irand(skill-25, skill+37)`.
        let max_skill = self.max_skill_value();
        if !triggered && skill < max_skill {
            #[allow(clippy::cast_possible_wrap)]
            let roll = self.spell_rng().irand(skill as i32 - 25, skill as i32 + 37);
            if skinning_try_again_fails(req, roll) {
                return Some(cast_result::FAILED_TRY_AGAIN);
            }
        }
        None
    }

    /// `Spell::EffectPickPocket` (fork
    /// `SpellEffects.cpp:2580-2590`): the caster must be
    /// a player and the victim an alive hostile
    /// creature, else silent return; then the
    /// pickpocket window opens.
    pub(crate) fn effect_pickpocket(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        if !caster.is_player() || !self.players.contains_key(&caster) {
            return;
        }
        if !victim.is_creature() || !self.creatures.contains_key(&victim) {
            return;
        }
        if !self.unit_is_alive(victim) {
            return;
        }
        if !Self::is_hostile_pair(caster, victim) {
            return;
        }
        self.open_creature_loot_window(caster, victim, CreatureLootKind::Pickpocket);
    }

    /// `Spell::EffectSkinning` (fork
    /// `SpellEffects.cpp:5265-5284`): opens the skinning
    /// window, clears `UNIT_FLAG_SKINNABLE`, then rolls
    /// the gather skill gain (elite victims double the
    /// chance).
    pub(crate) fn effect_skinning(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        if !victim.is_creature() || !self.creatures.contains_key(&victim) {
            return;
        }
        if !caster.is_player() || !self.players.contains_key(&caster) {
            return;
        }
        self.open_creature_loot_window(caster, victim, CreatureLootKind::Skinning);
        // Fork `:5275` clears the flag even when the
        // window holds nothing.
        if let Some(u) = self.unit_base_mut(victim) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur & !UNIT_FLAG_SKINNABLE);
        }
        let level = self.unit_level(victim);
        // Fork `:5278` (note the negative-req divergence
        // from the cast check below level 10).
        let req = if level < 10 {
            0
        } else if level < 20 {
            (level - 10) * 10
        } else {
            level * 5
        };
        // Fork `Creature::IsElite` (`Creature.h:197-204`;
        // pets never count) doubles the gain chance.
        let elite = !self.is_pet(victim)
            && self
                .creatures
                .get(&victim)
                .map(|c| c.info.entry)
                .is_some_and(|entry| {
                    self.stores
                        .creature_templates
                        .get(entry)
                        .is_some_and(mangos_world_data::creature_template::CreatureInfo::is_elite)
                });
        let skill = self
            .players
            .get(&caster)
            .map_or(0, |e| Self::skill_value(&e.player, SKILL_SKINNING));
        self.update_gather_skinning(caster, skill, req, if elite { 2 } else { 1 });
    }

    /// `Spell::EffectSkinPlayerCorpse` (fork
    /// `SpellEffects.cpp:5735-5760`): the caster must be
    /// a player. A corpse-guid victim takes the convert
    /// arm (fork arm (a): the owner is offline — convert
    /// with the caster as looter, then the insignia
    /// window on the bones; no range/LOS gate, no
    /// repop, no skinned notify). A player-guid victim
    /// takes the `RemovedInsignia` arm (fork arm (b));
    /// the window opens on the converted bones with the
    /// victim as the gold/reference-loot source.
    /// Anything else returns silently.
    pub(crate) fn effect_skin_player_corpse(&mut self, caster: ObjectGuid, victim: ObjectGuid) {
        if !caster.is_player() || !self.players.contains_key(&caster) {
            return;
        }
        if victim.is_corpse() {
            let Some(owner) = self.corpses.get(&victim).map(|c| c.owner) else {
                return;
            };
            if !owner.is_player() {
                return;
            }
            self.convert_corpse_for_player(owner, Some(caster));
            if self.corpses.contains_key(&victim) {
                self.open_corpse_loot_window(caster, victim, Some(owner), true);
            } else {
                // Fork `SendLoot` on the removed corpse
                // releases (the effect's direct send
                // lands after the convert removed it).
                self.send_loot_release(caster, victim);
            }
            return;
        }
        if !victim.is_player() || !self.players.contains_key(&victim) {
            return;
        }
        let Some(bones) = self.removed_insignia(victim, caster) else {
            return;
        };
        if self.corpses.contains_key(&bones) {
            self.open_corpse_loot_window(caster, bones, Some(victim), true);
        } else {
            self.send_loot_release(caster, bones);
        }
    }

    /// Insignia first-open fill (fork
    /// `Player.cpp:7837-7871` `lootForBody` gate):
    /// victim-level gold (fork `:7857`) plus the BG
    /// reference fill gated on the map's
    /// `player_skin_ref_loot_id` (fork `:7861-7863`),
    /// personal, the looter as the only allowed viewer.
    fn fill_insignia_loot(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        victim: Option<ObjectGuid>,
    ) {
        let team = self.player_team_id(player);
        let ref_loot_id = self.bg.as_ref().map_or(0, |b| b.player_skin_ref_loot_id);
        let victim_level = victim.map_or(0, |v| self.unit_level(v));
        let mut state = LootState {
            personal: true,
            team,
            loot_id: ref_loot_id,
            allowed_looters: vec![player],
            ..LootState::default()
        };
        let (gold, generated) = {
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
            // Fork `Player.cpp:7857`: `urand(50, 150)
            // * 0.016 * (level / 5.76)^2.5 * Rate`.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let gold = if victim_level > 0 {
                (ctx.rng.urand(50, 150) as f32
                    * 0.016
                    * ((victim_level as f32) / 5.76).powf(2.5)
                    * ctx.rates.money) as u32
            } else {
                0
            };
            // Fork `:7859` gates gold AND the reference
            // fill on `pVictim != nullptr` (a bare
            // `CMSG_LOOT` on bones fills nothing).
            let generated = if victim.is_some()
                && ref_loot_id != 0
                && stores.loot.reference.have_loot_for(ref_loot_id)
            {
                generate_loot(&mut ctx, &stores.loot, &stores.loot.reference, ref_loot_id)
            } else {
                None
            };
            (gold, generated)
        };
        state.gold = gold;
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                self.loot_add_item(&mut state, item, player);
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
            }
        }
        self.loots.insert(guid, state);
        let allowed: Vec<ObjectGuid> = self
            .loots
            .get(&guid)
            .map(|s| s.allowed_looters.clone())
            .unwrap_or_default();
        for member in allowed {
            self.loot_fill_dependent(guid, member);
        }
    }

    /// `Player::SendLoot` `HIGHGUID_CORPSE` arm (fork
    /// `Player.cpp:7837-7871`): the insignia window.
    /// Shared prelude (disabled-map release,
    /// prior-window release, looting-cancel aura strip —
    /// every non-pickpocket window, fork `:7721`); then,
    /// unlike every other window, no distance gate (fork
    /// `SendLoot` corpse arm has none). First open runs
    /// the `lootForBody` fill; the bones always gain
    /// `CORPSE_DYNFLAG_LOOTABLE` (fork `:7868-7869`);
    /// permission is `ALL_PERMISSION`. An insignia
    /// open remaps to wire `LOOT_PICKPOCKETING` and
    /// sets no `UNIT_FLAG_LOOTING`; a plain corpse
    /// `CMSG_LOOT` remaps to `LOOT_CORPSE` and does
    /// (fork `:8051-8081`).
    pub(crate) fn open_corpse_loot_window(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        victim: Option<ObjectGuid>,
        insignia: bool,
    ) {
        // `SendLoot`: disabled maps release at once.
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            self.send_loot_release(player, guid);
            return;
        }
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        if !self.corpses.contains_key(&guid) {
            self.send_loot_release(player, guid);
            return;
        }
        // Fork `:7721` strips looting-cancel auras for
        // every window except pickpocket.
        self.remove_auras_with_interrupt_flags(player, AURA_INTERRUPT_LOOTING_CANCELS, 0);
        if !self.loots.contains_key(&guid) {
            self.fill_insignia_loot(player, guid, victim);
        }
        if let Some(bones) = self.corpses.get_mut(&guid) {
            use mangos_entities::update::CORPSE_DYNFLAG_LOOTABLE;
            use mangos_protocol::update_field_table::offsets;
            let values = bones.world.object.values_mut();
            let cur = values.get_u32(offsets::CORPSE_FIELD_DYNAMIC_FLAGS);
            values.set_u32(
                offsets::CORPSE_FIELD_DYNAMIC_FLAGS,
                cur | CORPSE_DYNFLAG_LOOTABLE,
            );
            bones
                .world
                .object
                .force_values_update_at(offsets::CORPSE_FIELD_DYNAMIC_FLAGS);
        }
        let view = self.loot_view_bytes(player, guid, LootPermission::All);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(if insignia {
            LOOT_PICKPOCKETING
        } else {
            LOOT_CORPSE
        });
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if let Some(state) = self.loots.get_mut(&guid) {
            state.players_looting.insert(player);
        }
        self.player_loot_guid.insert(player, guid);
        // Fork `:8080-8081` keys on the post-remap wire
        // byte: only the plain corpse window sets it.
        if !insignia && let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur | UNIT_FLAG_LOOTING);
        }
    }

    /// `Player::UpdateGatherSkill` skinning arm (fork
    /// `Player.cpp:5373-5384`) over `UpdateSkillPro`
    /// (fork `:5409-5449`): the `SkillGainChance`
    /// ladder at `req+100/+50/+25` (fork `:5320-5328`,
    /// x10 tables) times the elite multiplier, shifted
    /// down as skill passes `SkinningSteps`; the gain
    /// roll is `irand(1,1000) <= chance`, the row clamps
    /// at its trained max, and both the deferred row and
    /// the skill-field triplet move (fork
    /// `SetUInt32Value(MAKE_SKILL_VALUE)`; the save diff
    /// reads the fields back).
    fn update_gather_skinning(
        &mut self,
        player: ObjectGuid,
        skill: u32,
        req: u32,
        multiplicator: u32,
    ) {
        #[allow(clippy::cast_possible_wrap)]
        let base = skill_gain_chance(
            skill as i32,
            req as i32 + 100,
            req as i32 + 50,
            req as i32 + 25,
        );
        #[allow(clippy::cast_possible_wrap)]
        let chance = (base * multiplicator as i32) >> (skill / SKINNING_CHANCE_STEPS);
        // Fork `UpdateSkillPro` speedup on 0 chance.
        if chance <= 0 {
            return;
        }
        let roll = self.spell_rng().irand(1, 1000);
        if roll > chance {
            return;
        }
        let Some(entry) = self.players.get_mut(&player) else {
            return;
        };
        let Some(row) = entry
            .player
            .deferred
            .skills
            .iter_mut()
            .find(|s| s.skill == SKILL_SKINNING)
        else {
            return;
        };
        if row.max == 0 || row.value == 0 || row.value >= row.max {
            return;
        }
        row.value = (row.value + SKILL_GAIN_GATHERING).min(row.max);
        let (skill_id, value, max) = (row.skill, row.value, row.max);
        // Refresh the packed skill triplet (see
        // `write_skill_fields` for the layout).
        if let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            for slot in 0..127u16 {
                let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
                if u.values().get_u32(base) == skill_id {
                    u.values().set_u32(base + 1, value | (max << 16));
                    break;
                }
            }
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
    /// GO guids route to the chest window (deviation:
    /// fork `HandleLootOpcode` rejects non-unit guids
    /// with `DIDNT_KILL` + anticheat since chests
    /// only open via the `OpenLock` spell; the
    /// gameobject-use flow shares this window, so
    /// accepting the guid here keeps both opcode
    /// paths on one window).
    pub fn handle_loot(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        if guid.is_game_object() {
            self.open_go_loot_window(player, guid);
            return;
        }
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
        if guid.is_corpse() {
            self.open_corpse_loot_window(player, guid, None, false);
            return;
        }
        self.open_creature_loot_window(player, guid, CreatureLootKind::Corpse);
    }

    /// `Player::SendLoot` `HIGHGUID_UNIT` arm (fork
    /// `Player.cpp:7696-8082`): corpse `CMSG_LOOT`,
    /// `EffectPickPocket` and `EffectSkinning` share
    /// this window. Shared prelude (fork `:7698-7722`):
    /// disabled-map release, prior-window release,
    /// looting-cancel aura strip (every non-pickpocket
    /// window, fork `:7721`). Then the alive-XOR gate
    /// (dead creatures, except pickpocket which needs
    /// a live victim), the range gate (skipped for
    /// skinning only), the recipient/body-roll block
    /// (corpse and skinning only — pickpocket has
    /// neither), and the kind fill; the wire byte
    /// remaps SKINNING to PICKPOCKETING (fork
    /// `:8051-8068`).
    pub(crate) fn open_creature_loot_window(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        kind: CreatureLootKind,
    ) {
        // `SendLoot`: disabled maps release at once.
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            self.send_loot_release(player, guid);
            return;
        }
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        if !guid.is_creature() {
            // Player/corpse arms UNMODELED (insignia);
            // spell windows target units only (fork
            // `Spell.cpp:6071`, `SpellEffects.cpp:2585`).
            if kind == CreatureLootKind::Corpse {
                self.send_loot_release(player, guid);
            } else {
                self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            }
            return;
        }
        // Fork `:7721` strips looting-cancel auras for
        // every window except pickpocket.
        if kind != CreatureLootKind::Pickpocket {
            self.remove_auras_with_interrupt_flags(player, AURA_INTERRUPT_LOOTING_CANCELS, 0);
        }
        // Fork `:7877`: the creature must be alive for
        // pickpocket and dead for every other window.
        if !self.creatures.contains_key(&guid)
            || self.unit_is_alive(guid) != (kind == CreatureLootKind::Pickpocket)
        {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        // Skinning skips the range gate: the spell range
        // was already checked at cast (fork
        // `Player.cpp:7883-7888`).
        if kind != CreatureLootKind::Skinning && !self.within_loot_range(player, guid) {
            self.send_loot_error(player, guid, loot_error::TOO_FAR);
            return;
        }
        // Fork `:7890`: pickpocketing a friendly
        // creature releases the window.
        if kind == CreatureLootKind::Pickpocket && !Self::is_hostile_pair(player, guid) {
            self.send_loot_release(player, guid);
            return;
        }
        // The AI `CanBeLooted()` arm (fork `:7896`) is
        // UNMODELED: no script-hook surface for it yet.
        let recipient = if kind == CreatureLootKind::Pickpocket {
            None
        } else {
            let Some(recipient) = self.loot_recipient_player(guid) else {
                self.send_loot_release(player, guid);
                return;
            };
            Some(recipient)
        };
        // Body block (`lootForBody` gate in `SendLoot`;
        // fork `Player.cpp:7956-7982`): runs on the
        // skinning path too when the body was never
        // opened. A fully-looted body leaves no entry
        // behind, so the skinning path materializes an
        // empty one for the flag first. Pickpocket has
        // no body block (fork `:7904-7939`).
        if kind != CreatureLootKind::Pickpocket {
            if !self.loots.contains_key(&guid) && kind == CreatureLootKind::Skinning {
                self.loots.insert(
                    guid,
                    LootState {
                        personal: true,
                        ..LootState::default()
                    },
                );
            }
            let needs_rolls = self.loots.get(&guid).is_some_and(|s| !s.body_processed);
            if needs_rolls {
                if let Some(state) = self.loots.get_mut(&guid) {
                    state.body_processed = true;
                }
                let tap_grouped = self
                    .loot_taps
                    .get(&guid)
                    .is_some_and(|t| t.group_id.is_some());
                let method = recipient
                    .and_then(|r| self.players.get(&r))
                    .map_or(3, |e| e.player.group_loot_method);
                if tap_grouped && (method == 3 || method == 4) {
                    self.start_loot_rolls(guid, method == 4);
                }
            }
        }
        let permission = self.creature_loot_permission_fill(player, guid, kind);
        self.player_loot_guid.insert(player, guid);
        let view = self.loot_view_bytes(player, guid, permission);
        let wire = match kind {
            CreatureLootKind::Corpse => LOOT_CORPSE,
            CreatureLootKind::Pickpocket | CreatureLootKind::Skinning => LOOT_PICKPOCKETING,
        };
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(wire);
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if permission != LootPermission::None
            && let Some(state) = self.loots.get_mut(&guid)
        {
            state.players_looting.insert(player);
        }
        // Fork `:8080-8081` checks the post-remap wire
        // byte, so spell windows set no LOOTING flag.
        if wire == LOOT_CORPSE
            && let Some(u) = self.unit_base_mut(player)
        {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur | UNIT_FLAG_LOOTING);
        }
    }

    /// Kind fill + permission for the shared creature
    /// window (fork `SendLoot` `:7904-8033` arms):
    /// corpse keeps the group ladder; pickpocket fills
    /// once then refills quest-only for stale viewers;
    /// skinning fills once. Spell kinds always view as
    /// OWNER and set no `UNIT_FLAG_LOOTING` (the wire
    /// byte is remapped at the call site, fork
    /// `:8080-8081`).
    fn creature_loot_permission_fill(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        kind: CreatureLootKind,
    ) -> LootPermission {
        match kind {
            CreatureLootKind::Corpse => self.loot_permission(player, guid),
            CreatureLootKind::Pickpocket => {
                // Fork `Player.cpp:7904-7939`: first
                // open fills; a later open refills
                // quest-only when the window is empty
                // or the viewer is not the original
                // looter. Permission is always OWNER
                // and instance binding is UNMODELED
                // (fork `:7937`; no binding state may
                // be added under the append-only lock).
                if !self.pickpocketed.contains(&guid) {
                    self.fill_pickpocket_loot(player, guid);
                } else if self
                    .loots
                    .get(&guid)
                    .is_none_or(|s| s.is_empty_loot() || !s.is_allowed_looter(player))
                {
                    // Missing entry: a full take + release
                    // removed it, but the mark (fork
                    // `Creature::lootForPickPocketed`)
                    // survives release — refill quest-only.
                    self.refill_pickpocket_quest_only(player, guid);
                }
                LootPermission::Owner
            }
            CreatureLootKind::Skinning => {
                // Possible only if the body is fully
                // looted (fork `Player.cpp:7984`); the
                // cast gate already enforced that.
                if !self.loots.get(&guid).is_some_and(|s| s.loot_for_skin) {
                    self.fill_skinning_loot(player, guid);
                    // `SetUInt32Value(UNIT_DYNAMIC_FLAGS,
                    // UNIT_DYNFLAG_LOOTABLE)` on non-empty
                    // skin loot (fork `:7993-7995`).
                    if self.loots.get(&guid).is_some_and(|s| !s.is_looted()) {
                        self.set_dynflag_lootable(guid, true);
                    }
                }
                // Group rights are skipped on this arm
                // (fork `:8002-8033`); skinning is
                // always owner-viewed.
                LootPermission::Owner
            }
        }
    }

    // --------------------------------------------------
    // Gameobject chest loot (`Player::SendLoot`
    // HIGHGUID_GAMEOBJECT arm, `Player.cpp:7726-7793`
    // + `DoLootRelease` GO arm,
    // `LootHandler.cpp:398-523`; mines/herb nodes are
    // the same type-3 arm with `minSuccessOpens > 0`).
    // State lives in [`Map::loots`] keyed by GO guid
    // (fork `GameObject::loot`); the loot-state word
    // rides `GoRuntime::loot_state`
    // (`GameObject.h:280`). Loot windows opened here
    // carry the internal `LOOT_SKINNING` type, so the
    // wire byte is [`LOOT_PICKPOCKETING`] and no
    // `UNIT_FLAG_LOOTING` is set (fork `:8051-8081`).
    // UNMODELED: `BindToInstanceOrRaid` (no binding
    // state may be added under the append-only lock),
    // `Group::UpdateLooterGuid` rotation (group-rule
    // chests fill for the whole group with
    // `ALL_PERMISSION`, RR pointer parked on the
    // opener), lock skill-up (`m_SkillupSet` has no
    // Rust surface yet).
    // --------------------------------------------------

    /// `GameObject::GetSpellForLock` subset
    /// (`GameObject.cpp:2519-2549`): whether the
    /// player may open this lock through direct use.
    /// Lock 0 always opens (fork `CanOpenLock`
    /// `:7924-7925`); a missing lock row refuses;
    /// skill cases need a known `OPEN_LOCK` spell
    /// whose misc matches the case index and whose
    /// base value covers the case skill (the fork
    /// `CalculateSpellEffectValue` dice/level tail
    /// is unmodeled, base points gate alone); item
    /// cases have no spell, so they refuse here
    /// (fork `break`s the same way). Key-item
    /// inventory checks ride the door `PlayerCanUse`
    /// arm only, matching the fork.
    pub(crate) fn go_open_lock_known(&self, player: ObjectGuid, lock_id: u32) -> bool {
        use mangos_world_data::locks::lock_key;
        if lock_id == 0 {
            return true;
        }
        let Some(lock) = self.stores.locks.get(lock_id) else {
            return false;
        };
        let Some(entry) = self.players.get(&player) else {
            return false;
        };
        let mut known: Vec<u32> = entry
            .player
            .deferred
            .spells
            .iter()
            .filter(|s| s.active != 0)
            .map(|s| s.spell)
            .collect();
        known.extend(
            self.stores
                .player_infos
                .default_spells(
                    u32::from(entry.player.unit.race()),
                    u32::from(entry.player.unit.class()),
                )
                .iter()
                .copied(),
        );
        for j in 0..mangos_world_data::locks::MAX_LOCK_CASE {
            let case = lock.lock_type[j];
            if case == lock_key::NONE {
                continue;
            }
            // Fork `break`s on the first non-skill
            // case (key items open through no spell).
            if case != lock_key::SKILL {
                break;
            }
            #[allow(clippy::cast_possible_wrap)]
            let (want_index, want_skill) = (lock.index[j] as i32, lock.skill[j] as i32);
            for spell_id in &known {
                let Some(spell) = self.stores.spells.get(*spell_id) else {
                    continue;
                };
                if spell.effects.iter().any(|eff| {
                    eff.effect == mangos_spells::defines::effect::OPEN_LOCK
                        && eff.misc_value == want_index
                        && eff.base_points >= want_skill
                }) {
                    return true;
                }
            }
        }
        false
    }

    /// Chest loot-state word, `GO_NOT_READY` when no
    /// runtime row exists yet.
    fn go_loot_state(&self, guid: ObjectGuid) -> u8 {
        use crate::gameobjects::GO_LOOT_NOT_READY;
        self.go_runtime
            .get(&guid)
            .map_or(GO_LOOT_NOT_READY, |r| r.loot_state)
    }

    /// `IsAtInteractDistance` subset: the display-box
    /// test (`GameObject.cpp:2497`) is unmodeled (no
    /// display-addon store), so the established
    /// 2D use band (`gameobject_use`) gates the
    /// window, money, and take paths alike.
    fn go_within_use_range(&self, player: ObjectGuid, guid: ObjectGuid) -> bool {
        let (Some(pp), Some(gp)) = (self.unit_position(player), self.object_position(guid)) else {
            return false;
        };
        crate::visibility::dist2d(pp, gp) <= 10.0
    }

    /// `Player::SendLoot` `HIGHGUID_GAMEOBJECT` arm
    /// (fork `Player.cpp:7726-7793`): disabled-map
    /// release, prior-window release, looting-cancel
    /// aura strip, interact-distance gate, chest
    /// level gate (anticheat log has no session hook
    /// here, the release carries it), then the
    /// `GO_READY && isSpawned` fill (`gameobject`
    /// template loot id + money, group-rules fill
    /// for grouped viewers, `GO_ACTIVATED` +
    /// `GO_STATE_ACTIVE` anim) or the `GO_ACTIVATED`
    /// late-viewer refill.
    pub(crate) fn open_go_loot_window(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        use crate::gameobjects::{GO_LOOT_ACTIVATED, GO_LOOT_JUST_DEACTIVATED, GO_LOOT_READY};
        use mangos_protocol::update_field_table::offsets;

        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            self.send_loot_release(player, guid);
            return;
        }
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        let Some(go) = self.gameobjects.get(&guid) else {
            self.send_loot_release(player, guid);
            return;
        };
        let (info, spawned) = (go.info.clone(), go.is_spawned());
        // Fork `:7721` strips for every window except
        // pickpocket; the GO window's internal type is
        // skinning, so it strips.
        self.remove_auras_with_interrupt_flags(player, AURA_INTERRUPT_LOOTING_CANCELS, 0);
        if !self.go_within_use_range(player, guid) {
            self.send_loot_release(player, guid);
            return;
        }
        // Fork `:7737-7745`: over-level chests release
        // (plus a `CHEAT_ACTION_LOG | REPORT_GMS`
        // anticheat sink with no Rust hook).
        if info.go_type == mangos_world_data::gameobject_template::TYPE_CHEST
            && info.data_u32(9) > self.unit_level(player) + 10
        {
            self.send_loot_release(player, guid);
            return;
        }
        let loot_state = self.go_loot_state(guid);
        if loot_state == GO_LOOT_READY && spawned {
            let loot_id = if info.go_type == mangos_world_data::gameobject_template::TYPE_CHEST {
                info.data_u32(1)
            } else {
                0
            };
            if loot_id != 0 {
                // Fork `loot->clear()` + fill.
                self.loots.remove(&guid);
                let grouped = self
                    .players
                    .get(&player)
                    .and_then(|e| e.player.group_id)
                    .is_some();
                let group_rules = info.go_type
                    == mangos_world_data::gameobject_template::TYPE_CHEST
                    && info.data_u32(15) != 0
                    && grouped;
                self.fill_go_loot(player, guid, loot_id, group_rules);
            }
            if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.loot_state = GO_LOOT_ACTIVATED;
            } else {
                self.go_runtime.insert(
                    guid,
                    crate::gameobjects::GoRuntime {
                        loot_state: GO_LOOT_ACTIVATED,
                        ..crate::gameobjects::GoRuntime::default()
                    },
                );
            }
        } else if loot_state == GO_LOOT_ACTIVATED {
            self.loot_fill_dependent(guid, player);
        } else if loot_state == GO_LOOT_JUST_DEACTIVATED {
            // Deactivation races the despawn tick:
            // serve the empty window through the same
            // release below (fork keeps `SendLoot`
            // flowing for non-ready states too).
            self.loot_fill_dependent(guid, player);
        }
        // Chest open animation (fork `:7787-7789`).
        if info.go_type == mangos_world_data::gameobject_template::TYPE_CHEST {
            let _ =
                self.object_set_field(guid, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_ACTIVE);
        }
        let view = self.loot_view_bytes(player, guid, LootPermission::All);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(LOOT_PICKPOCKETING);
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if let Some(state) = self.loots.get_mut(&guid) {
            state.players_looting.insert(player);
        }
        self.player_loot_guid.insert(player, guid);
        // Fork `:8080-8081` sets `UNIT_FLAG_LOOTING`
        // only for the `LOOT_CORPSE` wire byte, so GO
        // windows set no flag.
    }

    /// GO chest fill (`Loot::FillLoot` over the
    /// `gameobject` store + `GenerateMoneyLoot`,
    /// fork `Player.cpp:7759-7783`): solo windows
    /// are personal to the opener, group-rule
    /// windows admit the opener's on-map group with
    /// `ALL_PERMISSION` (no eligibility rolls and no
    /// RR-pointer rotation, see the section note).
    fn fill_go_loot(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        loot_id: u32,
        group_rules: bool,
    ) {
        let team = self.player_team_id(player);
        let group_id = self.players.get(&player).and_then(|e| e.player.group_id);
        let allowed: Vec<ObjectGuid> = if group_rules {
            let group_id = group_id.expect("group-rule fill needs a group");
            self.players
                .iter()
                .filter(|(_, e)| e.player.group_id == Some(group_id))
                .map(|(member, _)| *member)
                .collect()
        } else {
            vec![player]
        };
        let (min_money, max_money) = self
            .gameobjects
            .get(&guid)
            .map_or((0, 0), |g| (g.info.min_money_loot, g.info.max_money_loot));
        let mut state = LootState {
            personal: !group_rules,
            team,
            loot_id,
            round_robin_player: if group_rules { player.raw() } else { 0 },
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
            let generated = generate_loot(&mut ctx, &stores.loot, &stores.loot.gameobject, loot_id);
            state.gold = generate_money_loot(&mut ctx, min_money, max_money);
            generated
        };
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                self.loot_add_item(&mut state, item, player);
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
            }
        }
        self.loots.insert(guid, state);
        for member in allowed {
            self.loot_fill_dependent(guid, member);
        }
    }

    /// `DoLootRelease` `HIGHGUID_GAMEOBJECT` arm
    /// (fork `LootHandler.cpp:398-523`): chest close
    /// anim (`GO_STATE_READY`), the FFA temp-chest
    /// exception (keeps `GO_READY` + loot), the
    /// full-loot vein branch (event scripts, then
    /// the `minSuccessOpens`/`maxSuccessOpens` use
    /// roll with `ReqValue` from the lock row, fork
    /// `Rate.Mining.*` defaults of 1.0) or
    /// `JUST_DEACTIVATED` + clear, and the partial
    /// arm (`GO_ACTIVATED` + 5-minute chest
    /// cooldown). Non-chest GOs deactivate once
    /// looted. Restock chests re-arm through the
    /// tick pass; delay-less statics re-arm at once
    /// (fork `!m_respawnDelayTime` return).
    fn do_go_loot_release(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        use crate::gameobjects::{
            GO_LOOT_ACTIVATED, GO_LOOT_JUST_DEACTIVATED, GO_LOOT_NOT_READY, GO_LOOT_READY,
        };
        use mangos_protocol::update_field_table::offsets;

        let Some(go) = self.gameobjects.get(&guid) else {
            return;
        };
        let (info, spawned_by_default, respawn_delay) = (
            go.info.clone(),
            go.spawned_by_default,
            go.respawn_delay_time.max(0),
        );
        let is_chest = info.go_type == mangos_world_data::gameobject_template::TYPE_CHEST;
        // Chest close animation (fork `:408-410`).
        if is_chest {
            let _ =
                self.object_set_field(guid, u32::from(offsets::GAMEOBJECT_STATE), GO_STATE_READY);
        }
        let has_ffa = self.loots.get(&guid).is_some_and(|s| s.has_ffa_quest_items);
        // FFA temp-chest exception (fork `:414-419`).
        if has_ffa && !spawned_by_default && is_chest {
            if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.loot_state = GO_LOOT_READY;
            }
            return;
        }
        let looted = self.loots.get(&guid).is_none_or(LootState::is_looted);
        // Holes deplete only on full takes (fork
        // `LootHandler.cpp:498-507` sits inside the
        // `isLooted()` branch); partial hole releases
        // fall to the partial arm below. Bobbers always
        // take the full path (fork `|| FISHINGNODE`).
        let is_hole = info.go_type == mangos_world_data::gameobject_template::TYPE_FISHINGHOLE;
        if looted || (!is_chest && !is_hole) {
            // Door arms never reach here (doors open no
            // loot window); `FISHINGNODE` expiry is a
            // tick arm, not a window arm.
            if is_chest {
                // Looted-event scripts (fork
                // `:461-466`): C++ process event,
                // else the DB `event_scripts` arm.
                let event_id = info.data_u32(6);
                if event_id != 0 && !self.process_event(event_id, Some(player), Some(guid), true) {
                    self.scripts_start_event(event_id, player, guid);
                }
                let go_min = info.data_u32(4);
                let go_max = info.data_u32(5);
                if go_min != 0 && go_max > go_min {
                    // Vein/mine/herb arm (fork
                    // `:469-497`, amount/next rates at
                    // their 1.0 defaults).
                    let uses = self.go_runtime.get_mut(&guid).map_or(1, |r| {
                        r.uses += 1;
                        r.uses
                    });
                    let req = self.stores.locks.get(info.data_u32(0)).map_or(
                        VEIN_REQ_FALLBACK,
                        #[allow(clippy::cast_possible_wrap)]
                        |lock| lock.skill[0] as i32,
                    );
                    let skill = self
                        .players
                        .get(&player)
                        .map_or(0, |e| Self::skill_value(&e.player, SKILL_MINING));
                    let state = Self::vein_release_state(
                        uses,
                        go_min,
                        go_max,
                        req,
                        skill,
                        self.rng.chance100(),
                    );
                    if state == GO_LOOT_READY {
                        if let Some(r) = self.go_runtime.get_mut(&guid) {
                            r.loot_state = GO_LOOT_READY;
                        }
                    } else {
                        self.go_schedule_deactivation(guid, &info);
                    }
                } else {
                    // Plain chest: depleted at once.
                    self.go_schedule_deactivation(guid, &info);
                }
            } else if is_hole {
                self.go_hole_release(guid, &info, spawned_by_default, respawn_delay);
            } else if self.go_loot_state(guid) != GO_LOOT_NOT_READY
                && let Some(r) = self.go_runtime.get_mut(&guid)
            {
                r.loot_state = GO_LOOT_JUST_DEACTIVATED;
                // Retired bobbers delete on the next
                // tick (fork `GO_JUST_DEACTIVATED`
                // `Update` arm); other non-chest windows
                // have no despawn clock.
                if info.go_type == mangos_world_data::gameobject_template::TYPE_FISHINGNODE {
                    self.go_set_just_deactivated(guid);
                }
            }
            self.loots.remove(&guid);
            return;
        }
        // Partial loot (fork `:511-520`).
        if let Some(r) = self.go_runtime.get_mut(&guid) {
            r.loot_state = GO_LOOT_ACTIVATED;
            if is_chest {
                r.cooldown_until = self.now_unix + CHEST_PARTIAL_LOOT_COOLDOWN_SECS;
            }
        }
    }

    /// Pool depletion (fork `LootHandler.cpp:498-507`
    /// and the `GameObject::Update`
    /// `GO_JUST_DEACTIVATED` arm): one more use, then
    /// retire once the re-rolled `urand(min, max)`
    /// threshold is met, else `READY` again. Reached
    /// only on full takes (see the gate in
    /// `do_go_loot_release`);
    /// partials ride the partial arm below. Dynamic
    /// pools delete on the next tick; delay-less
    /// statics re-arm at once (fork
    /// `!m_respawnDelayTime` return); delayed statics
    /// re-arm through the tick pass (map-local, no
    /// persist write).
    fn go_hole_release(
        &mut self,
        guid: ObjectGuid,
        info: &mangos_world_data::gameobject_template::GameObjectInfo,
        spawned_by_default: bool,
        respawn_delay: i32,
    ) {
        use crate::gameobjects::{GO_LOOT_JUST_DEACTIVATED, GO_LOOT_NOT_READY, GO_LOOT_READY};
        let uses = self.go_runtime.get_mut(&guid).map_or(1, |r| {
            r.uses += 1;
            r.uses
        });
        let min = info.data_u32(2);
        let max = info.data_u32(3);
        let threshold = mangos_world_data::loot::LootRng::urand(&mut self.rng, min, max);
        if uses >= threshold {
            if !spawned_by_default || respawn_delay > 0 {
                if self.go_loot_state(guid) != GO_LOOT_NOT_READY
                    && let Some(r) = self.go_runtime.get_mut(&guid)
                {
                    r.loot_state = GO_LOOT_JUST_DEACTIVATED;
                }
                self.go_set_just_deactivated(guid);
            } else if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.loot_state = GO_LOOT_READY;
            }
        } else if let Some(r) = self.go_runtime.get_mut(&guid) {
            r.loot_state = GO_LOOT_READY;
        }
    }

    /// Vein depletion roll (fork
    /// `LootHandler.cpp:469-497`): maxed uses
    /// deplete at once; below-min uses always
    /// re-arm; between, the roll is `roll <
    /// 100 * 0.8^(4/max * uses) + skill/(Req+25)`
    /// (fork `roll_chance_f`). Pure fork math,
    /// unit-tested.
    fn vein_release_state(
        uses: u32,
        go_min: u32,
        go_max: u32,
        req: i32,
        skill: u32,
        roll: f32,
    ) -> u8 {
        use crate::gameobjects::{GO_LOOT_JUST_DEACTIVATED, GO_LOOT_READY};
        #[allow(clippy::cast_precision_loss)]
        let (uses_f, min_f, max_f) = (uses as f32, go_min as f32, go_max as f32);
        if uses_f >= max_f {
            return GO_LOOT_JUST_DEACTIVATED;
        }
        if uses_f < min_f {
            return GO_LOOT_READY;
        }
        #[allow(clippy::cast_precision_loss)]
        let req_f = req.max(0) as f32;
        #[allow(clippy::cast_precision_loss)]
        let skill_f = skill as f32;
        let chance = 100.0 * (0.8f32).powf(4.0 * (1.0 / max_f) * uses_f) + skill_f / (req_f + 25.0);
        if roll < chance {
            GO_LOOT_READY
        } else {
            GO_LOOT_JUST_DEACTIVATED
        }
    }

    /// Full-loot deactivation scheduling (fork
    /// `SetLootState(GO_JUST_DEACTIVATED)` +
    /// `Update` `GO_JUST_DEACTIVATED` chest arm):
    /// restock chests wait out `chestRestockTime` at
    /// `NOT_READY`; delay-less statics re-arm
    /// `READY` at once; everything else despawns on
    /// the respawn delay through the tick pass (no
    /// persist write, no despawn-anim/flag packets).
    fn go_schedule_deactivation(
        &mut self,
        guid: ObjectGuid,
        info: &mangos_world_data::gameobject_template::GameObjectInfo,
    ) {
        use crate::gameobjects::{GO_LOOT_JUST_DEACTIVATED, GO_LOOT_NOT_READY, GO_LOOT_READY};
        let restock = info.data_u32(2);
        let consumable = info.data_u32(3) != 0;
        if restock != 0 && !consumable {
            if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.cooldown_until = self.now_unix.saturating_add(u64::from(restock));
                r.loot_state = GO_LOOT_NOT_READY;
            }
            return;
        }
        let delay = self
            .gameobjects
            .get(&guid)
            .map_or(0, |g| g.respawn_delay_time.max(0));
        if delay == 0 {
            self.loots.remove(&guid);
            if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.loot_state = GO_LOOT_READY;
            }
            return;
        }
        if let Some(r) = self.go_runtime.get_mut(&guid) {
            r.loot_state = GO_LOOT_JUST_DEACTIVATED;
        }
        self.go_set_just_deactivated(guid);
    }

    // --------------------------------------------------
    // Fishing loot (`Player::SendLoot` `HIGHGUID_GAMEOBJECT`
    // arm for `LOOT_FISHING` / `LOOT_FISHINGHOLE` /
    // `LOOT_FISHING_FAIL`, fork `Player.cpp:7726-7793`
    // with the wire remap at `:8063-8068`, and
    // `DoLootRelease` hole arm, `LootHandler.cpp:498-507`).
    // State rides the same GO-keyed [`Map::loots`]
    // host as chests; releases flow through
    // `do_go_loot_release` (holes roll their deplete
    // uses there, bobbers always retire). `SkillFail.*`
    // config tails are fork defaults, not plumped
    // config: `SkillFail.Loot.Fishing` false (fail
    // escapes, no junk window), `SkillFail.Gain.Fishing`
    // false (gains on success only),
    // `SkillFail.Possible.FishingPool` true (a nearby
    // hole overwrites a failed roll); `SkillGain.Gathering`
    // already rides `SKILL_GAIN_GATHERING`.
    // --------------------------------------------------

    /// `GameObject::getFishLoot` (`GameObject.cpp:848-863`):
    /// subzone template first, zone template when the
    /// subzone differs and has none, nothing (empty
    /// window) inside the hidden wetlands lake
    /// (subzone 11 within 100 yd of the fork point).
    fn fill_fish_loot(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        const WETLANDS_SUBZONE: u32 = 11;
        const WETLANDS_LAKE_X: f32 = -4074.74;
        const WETLANDS_LAKE_Y: f32 = -1315.79;
        const WETLANDS_LAKE_RADIUS: f32 = 100.0;
        self.loots.remove(&guid);
        let Some(pos) = self.object_position(guid) else {
            return;
        };
        let (zone, subzone) = self.zone_and_area_id(pos);
        // Fork `:854-856`: hidden lake, no fish.
        if subzone == WETLANDS_SUBZONE
            && ((pos.x - WETLANDS_LAKE_X).powi(2) + (pos.y - WETLANDS_LAKE_Y).powi(2)).sqrt()
                <= WETLANDS_LAKE_RADIUS
        {
            return;
        }
        let team = self.player_team_id(player);
        let mut state = LootState {
            personal: true,
            team,
            loot_id: subzone,
            round_robin_player: 0,
            allowed_looters: vec![player],
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
            // `FillLoot` returns false only for a missing
            // template, which is exactly the `Option`
            // contract here.
            generate_loot(&mut ctx, &stores.loot, &stores.loot.fishing, subzone).or_else(|| {
                if subzone == zone {
                    None
                } else {
                    generate_loot(&mut ctx, &stores.loot, &stores.loot.fishing, zone)
                }
            })
        };
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                self.loot_add_item(&mut state, item, player);
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
            }
        }
        self.loots.insert(guid, state);
        self.loot_fill_dependent(guid, player);
    }

    /// Junk-fail fill (fork `Player.cpp:7770-7772`):
    /// entry 0 of the fishing store.
    fn fill_fish_junk(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        self.loots.remove(&guid);
        let team = self.player_team_id(player);
        let mut state = LootState {
            personal: true,
            team,
            loot_id: 0,
            round_robin_player: 0,
            allowed_looters: vec![player],
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
            generate_loot(&mut ctx, &stores.loot, &stores.loot.fishing, 0)
        };
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                self.loot_add_item(&mut state, item, player);
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
            }
        }
        self.loots.insert(guid, state);
        self.loot_fill_dependent(guid, player);
    }

    /// `Player::UpdateFishingSkill`
    /// (`Player.cpp:5394-5407`) over the skinning gain
    /// shape: chance 100 below skill 75, else
    /// `2500 / (skill - 50)` (x10 for the shared roll
    /// scale), one point capped at max (the trial
    /// restriction arm has no Rust trial state).
    pub(crate) fn update_fishing_skill(&mut self, player: ObjectGuid) {
        let skill = self
            .players
            .get(&player)
            .map_or(0, |e| Self::skill_value(&e.player, SKILL_FISHING));
        #[allow(clippy::cast_possible_wrap)]
        let chance = if skill < 75 {
            100 * 10
        } else {
            2500 / (skill as i32 - 50) * 10
        };
        if chance <= 0 {
            return;
        }
        if self.spell_rng().irand(1, 1000) > chance {
            return;
        }
        let Some(entry) = self.players.get_mut(&player) else {
            return;
        };
        let Some(row) = entry
            .player
            .deferred
            .skills
            .iter_mut()
            .find(|s| s.skill == SKILL_FISHING)
        else {
            return;
        };
        if row.max == 0 || row.value == 0 || row.value >= row.max {
            return;
        }
        row.value = (row.value + SKILL_GAIN_GATHERING).min(row.max);
        let (skill_id, value, max) = (row.skill, row.value, row.max);
        if let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            for slot in 0..127u16 {
                let base = offsets::PLAYER_SKILL_INFO_1_1 + slot * 3;
                if u.values().get_u32(base) == skill_id {
                    u.values().set_u32(base + 1, value | (max << 16));
                    break;
                }
            }
        }
    }

    /// `Player::SendLoot` `HIGHGUID_GAMEOBJECT` arm for
    /// the three fishing kinds (fork `Player.cpp:7726-7793`):
    /// disabled-map release, prior-window release,
    /// looting-cancel aura strip, the fork distance
    /// bypass (holes skip it; bobbers skip it for the
    /// owner), then the `GO_READY && isSpawned` fill
    /// (`getFishLoot` / entry-0 junk / hole loot id +
    /// money) or the late-viewer refill. The wire byte
    /// is always [`LOOT_FISHING`] (fork `:8063-8068`).
    pub(crate) fn open_fishing_loot_window(
        &mut self,
        player: ObjectGuid,
        guid: ObjectGuid,
        kind: u8,
    ) {
        use crate::gameobjects::{GO_LOOT_ACTIVATED, GO_LOOT_JUST_DEACTIVATED, GO_LOOT_READY};
        debug_assert!(matches!(
            kind,
            LOOT_FISHING | LOOT_FISHINGHOLE | LOOT_FISHING_FAIL
        ));
        if self.stores.loot.disabled_maps.contains(&self.map_id) {
            self.send_loot_release(player, guid);
            return;
        }
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        let Some(go) = self.gameobjects.get(&guid) else {
            self.send_loot_release(player, guid);
            return;
        };
        let (info, spawned) = (go.info.clone(), go.is_spawned());
        self.remove_auras_with_interrupt_flags(player, AURA_INTERRUPT_LOOTING_CANCELS, 0);
        // Fork `:7729-7733`: no distance check for
        // holes, and none for owned bobbers.
        let owned_bobber = (kind == LOOT_FISHING || kind == LOOT_FISHING_FAIL)
            && self.go_owner_guid(guid) == player;
        if kind != LOOT_FISHINGHOLE && !owned_bobber && !self.go_within_use_range(player, guid) {
            self.send_loot_release(player, guid);
            return;
        }
        let loot_state = self.go_loot_state(guid);
        if loot_state == GO_LOOT_READY && spawned {
            match kind {
                LOOT_FISHING => self.fill_fish_loot(player, guid),
                LOOT_FISHING_FAIL => self.fill_fish_junk(player, guid),
                _ => {
                    let loot_id = info.data_u32(1);
                    if loot_id != 0 {
                        self.loots.remove(&guid);
                        // Group-rule chests are the only
                        // group fill; holes are personal
                        // (fork `Player.cpp:7759` gates
                        // on `GAMEOBJECT_TYPE_CHEST`).
                        self.fill_go_loot(player, guid, loot_id, false);
                    }
                }
            }
            if let Some(r) = self.go_runtime.get_mut(&guid) {
                r.loot_state = GO_LOOT_ACTIVATED;
            } else {
                self.go_runtime.insert(
                    guid,
                    crate::gameobjects::GoRuntime {
                        loot_state: GO_LOOT_ACTIVATED,
                        ..crate::gameobjects::GoRuntime::default()
                    },
                );
            }
        } else if loot_state == GO_LOOT_ACTIVATED || loot_state == GO_LOOT_JUST_DEACTIVATED {
            self.loot_fill_dependent(guid, player);
        }
        let view = self.loot_view_bytes(player, guid, LootPermission::All);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(guid.raw());
        writer.write_u8(LOOT_FISHING);
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if let Some(state) = self.loots.get_mut(&guid) {
            state.players_looting.insert(player);
        }
        self.player_loot_guid.insert(player, guid);
    }

    /// `CMSG_LOOT_RELEASE` (guid ignored, stored
    /// window released).
    pub fn handle_loot_release(&mut self, player: ObjectGuid) {
        self.do_loot_release(player);
    }

    /// `WorldSession::DoLootRelease`: non-item loot
    /// releases before cleanup; the item arm releases
    /// only after its destroy/autostore work, and not
    /// at all when the item no longer exists.
    fn do_loot_release(&mut self, player: ObjectGuid) {
        let Some(guid) = self.player_loot_guid.remove(&player) else {
            return;
        };
        if guid.is_item() {
            if self.do_item_loot_release(player, guid) {
                self.send_loot_release(player, guid);
            }
            return;
        }
        self.send_loot_release(player, guid);
        if let Some(u) = self.unit_base_mut(player) {
            use mangos_protocol::update_field_table::offsets;
            let cur = u.values().get_u32(offsets::UNIT_FIELD_FLAGS);
            u.values()
                .set_u32(offsets::UNIT_FIELD_FLAGS, cur & !UNIT_FLAG_LOOTING);
        }
        if guid.is_game_object() {
            // The window guid, not the player's
            // target, authorizes the release (fork
            // `HandleLootReleaseOpcode` reads the
            // stored guid).
            if self.gameobjects.contains_key(&guid) {
                self.do_go_loot_release(player, guid);
            }
            return;
        }
        if guid.is_corpse() {
            // Fork `LootHandler.cpp:524-540` corpse
            // arm: a fully-looted window clears (the
            // entry survives as the `lootForBody`
            // marker, so reopen serves empty instead
            // of refilling) and the bones drop
            // `LOOTABLE`.
            if let Some(state) = self.loots.get_mut(&guid) {
                if state.is_looted() {
                    state.clear();
                }
                state.players_looting.remove(&player);
            }
            // Fork clears the flag only on the looted
            // arm but force-updates unconditionally
            // (fork `:536-539`).
            if self.loots.get(&guid).is_none_or(LootState::is_looted)
                && let Some(bones) = self.corpses.get_mut(&guid)
            {
                use mangos_entities::update::CORPSE_DYNFLAG_LOOTABLE;
                use mangos_protocol::update_field_table::offsets;
                let values = bones.world.object.values_mut();
                let cur = values.get_u32(offsets::CORPSE_FIELD_DYNAMIC_FLAGS);
                values.set_u32(
                    offsets::CORPSE_FIELD_DYNAMIC_FLAGS,
                    cur & !CORPSE_DYNFLAG_LOOTABLE,
                );
            }
            if let Some(bones) = self.corpses.get_mut(&guid) {
                use mangos_protocol::update_field_table::offsets;
                bones
                    .world
                    .object
                    .force_values_update_at(offsets::CORPSE_FIELD_DYNAMIC_FLAGS);
            }
            return;
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
            // `AllLootRemovedFromCorpse` decay (fork
            // `Creature.cpp:3214-3223`, non-skinned arm;
            // the skinned-decay-0 distinction is
            // deferred): a positive
            // `Rate.Corpse.Decay.Looted` scales the
            // timer, otherwise the corpse lingers a
            // third of the spawn's respawn delay (fork
            // default 0.0 takes this arm, so the timer
            // must NOT be zeroed — the taps have to
            // survive release for skinning).
            if let Some(creature) = self.creatures.get_mut(&guid) {
                if self.config.corpse_decay_looted_rate > 0.0 {
                    #[allow(clippy::cast_precision_loss)]
                    let scaled = creature.corpse_decay_timer as f64
                        * f64::from(self.config.corpse_decay_looted_rate);
                    creature.corpse_decay_timer = scaled as i64;
                } else {
                    creature.corpse_decay_timer = i64::from(creature.respawn_delay) * 1000 / 3;
                }
            }
            // `skinningForOthersTimer = 5000` (fork
            // `Creature.cpp:3209`): the tapper-only
            // skinning head-start after full loot. It
            // lives outside `loots` so it survives the
            // entry removal below.
            self.skinning_for_others
                .insert(guid, u64::from(self.now_ms) + SKINNING_FOR_OTHERS_MS);
            self.set_dynflag_lootable(guid, false);
            self.loots.remove(&guid);
        }
    }

    /// `CMSG_LOOT_MONEY` creature arm (fork
    /// `LootHandler.cpp:239+`): a live pickpocket
    /// victim pays its window out solo (fork
    /// `shareMoneyWithGroup = false`; the fork also
    /// keys on rogue class — UNMODELED here, the
    /// pickpocket mark from the spell window is the
    /// gate); corpses take the group-split path.
    /// GO chests (`HIGHGUID_GAMEOBJECT`) pay out of
    /// the stored window at the use band (fork gates
    /// on owner-or-`INTERACTION_DISTANCE`; the owner
    /// arm is fishing-only and unmodeled, so the
    /// band gates alone) with the shared group
    /// split (`shareMoneyWithGroup` stays true).
    pub fn handle_loot_money(&mut self, player: ObjectGuid) {
        let Some(guid) = self.player_loot_guid.get(&player).copied() else {
            return;
        };
        if guid.is_item() {
            self.handle_item_loot_money(player, guid);
            return;
        }
        if guid.is_game_object() {
            if !self.gameobjects.contains_key(&guid) || !self.go_within_use_range(player, guid) {
                return;
            }
        } else if guid.is_corpse() {
            // Fork `LootHandler.cpp:264-272` corpse arm:
            // bones within `INTERACTION_DISTANCE` pay
            // out; the shared group split below still
            // applies (`shareMoneyWithGroup` stays
            // true).
            let (Some(pp), Some(bones)) = (self.unit_position(player), self.corpses.get(&guid))
            else {
                return;
            };
            if crate::visibility::dist3d(pp, bones.world.position()) > CORPSE_MONEY_DISTANCE {
                return;
            }
        } else if !guid.is_creature()
            || !self.creatures.contains_key(&guid)
            || !self.within_loot_range(player, guid)
        {
            return;
        }
        // Fork `ok_loot`: live victims pay only out of
        // a pickpocket window.
        let pickpocket_take = self.unit_is_alive(guid) && self.pickpocketed.contains(&guid);
        if self.unit_is_alive(guid) && !pickpocket_take {
            return;
        }
        let Some(gold) = self.loots.get(&guid).map(|s| s.gold) else {
            return;
        };
        self.notify_money_removed(guid);
        if pickpocket_take {
            self.modify_money(player, i64::from(gold));
            if let Some(state) = self.loots.get_mut(&guid) {
                state.gold = 0;
            }
            return;
        }
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

    // --------------------------------------------------
    // Item container loot (`HandleOpenItemOpcode` +
    // `Player::SendLoot` HIGHGUID_ITEM arm +
    // `DoLootRelease` item arm; single-player
    // `OWNER_PERMISSION`, no group rolls, no
    // `UNIT_FLAG_LOOTING` for item guids).
    // --------------------------------------------------

    /// `Item::HasGeneratedLoot`: a live entry whose
    /// window still holds unlooted content (fork
    /// `Item.h:122`: loot non-empty and state neither
    /// NONE nor REMOVED; destruction drops the entry,
    /// so presence plus `!is_looted` is exact).
    #[must_use]
    pub fn item_has_generated_loot(&self, item_low: u32) -> bool {
        let guid = ObjectGuid::with_entry(HighGuid::ITEM, 0, item_low);
        self.item_loots
            .get(&guid)
            .is_some_and(|(state, _)| !state.is_looted())
    }

    /// Login rebuild from the persisted `item_loot`
    /// rows (`Player::_LoadItemLoot`: `item_id == 0`
    /// carries gold, else one unlooted slot; state
    /// `UNCHANGED`).
    pub(crate) fn rebuild_item_loot(&mut self, player: ObjectGuid) {
        let rows: Vec<ItemLootRow> = self
            .players
            .get(&player)
            .map_or(Vec::new(), |e| e.player.deferred.item_loot.clone());
        let mut grouped: HashMap<u32, Vec<ItemLootRow>> = HashMap::new();
        for row in rows {
            grouped.entry(row.guid).or_default().push(row);
        }
        for (item_low, rows) in grouped {
            let guid = ObjectGuid::with_entry(HighGuid::ITEM, 0, item_low);
            let mut state = LootState {
                personal: true,
                ..LootState::default()
            };
            for row in rows {
                if row.item_id == 0 {
                    state.gold = row.amount.max(0) as u32;
                } else {
                    state.items.push(LootSlotItem {
                        itemid: row.item_id,
                        count: row.amount.clamp(1, 255) as u8,
                        condition_id: 0,
                        needs_quest: false,
                        freeforall: false,
                        is_looted: false,
                        is_blocked: false,
                        is_counted: false,
                        is_underthreshold: false,
                        loot_owner: ObjectGuid::EMPTY,
                    });
                    state.unlooted_count += 1;
                }
            }
            state.allowed_looters.push(player);
            self.item_loots
                .insert(guid, (state, ItemLootUpdateState::Unchanged));
        }
    }

    /// Save projection of one container (`Item::SaveToDB`
    /// loot arms): generated or taken loot rewrites the
    /// rows (`Replace`), untouched rebuilds write
    /// nothing, disenchant loot is session-only.
    /// `has_saved_loot` is the `DeleteAllFromDB` gate:
    /// a live entry always qualifies; a removed row
    /// falls back to its persisted `generated_loot`
    /// word (set exactly when rows were first
    /// generated, so the fallback never over-deletes).
    #[must_use]
    pub fn item_loot_saves(&self) -> HashMap<u32, (LootSave, bool)> {
        let mut out = HashMap::new();
        for (guid, (state, ustate)) in &self.item_loots {
            let loot = match ustate {
                ItemLootUpdateState::Unchanged | ItemLootUpdateState::Temporary => {
                    LootSave::Unchanged
                }
                ItemLootUpdateState::Changed => {
                    let rows = state
                        .items
                        .iter()
                        .filter(|i| !i.is_looted)
                        .map(|i| ItemLootRow {
                            guid: guid.counter(),
                            item_id: i.itemid,
                            amount: i32::from(i.count),
                            property: 0,
                        })
                        .collect();
                    LootSave::Replace {
                        gold: state.gold,
                        rows,
                    }
                }
            };
            out.insert(guid.counter(), (loot, true));
        }
        out
    }

    /// Mark one container slot taken (single-player
    /// take tail shared by the take and disenchant
    /// autostore paths).
    fn mark_item_slot_taken(state: &mut LootState, slot: u8, is_quest: bool) {
        if is_quest {
            if let Some(qi) = state.quest_items.get_mut(slot as usize - state.items.len()) {
                qi.is_looted = true;
            }
        } else if let Some(slot_item) = state.items.get_mut(slot as usize) {
            slot_item.is_looted = true;
        }
        state.unlooted_count = state.unlooted_count.saturating_sub(1);
    }

    /// Pure slot resolution over one container state
    /// (single-player: no per-player quest/FFA marks;
    /// plain slots index `items`, the tail indexes
    /// `quest_items`). Returns the slot item plus
    /// whether it lives in the quest tail.
    fn item_slot_in(state: &LootState, slot: u8) -> Option<(LootSlotItem, bool)> {
        let at = usize::from(slot);
        if at < state.items.len() {
            let item = state.items[at];
            (!item.is_looted).then_some((item, false))
        } else {
            let item = *state.quest_items.get(at - state.items.len())?;
            (!item.is_looted).then_some((item, true))
        }
    }

    /// Item-arm view (`LootView` with `LOOT_CORPSE`
    /// type 1, `OWNER_PERMISSION`): gold plus every
    /// unlooted slot as `ALLOW_LOOT`.
    fn item_loot_view_bytes(&self, guid: ObjectGuid) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        let Some((state, _)) = self.item_loots.get(&guid) else {
            writer.write_u32_le(0);
            writer.write_u8(0);
            return writer.into_vec();
        };
        writer.write_u32_le(state.gold);
        writer.write_u8(0);
        let mut shown: u8 = 0;
        for (i, item) in state.items.iter().enumerate() {
            if item.is_looted {
                continue;
            }
            writer.write_u8(i as u8);
            self.write_loot_item(&mut writer, item);
            writer.write_u8(slot_type::ALLOW_LOOT);
            shown += 1;
        }
        for (qi, item) in state.quest_items.iter().enumerate() {
            if item.is_looted {
                continue;
            }
            writer.write_u8((state.items.len() + qi) as u8);
            self.write_loot_item(&mut writer, item);
            writer.write_u8(slot_type::ALLOW_LOOT);
            shown += 1;
        }
        let mut bytes = writer.into_vec();
        bytes[4] = shown;
        bytes
    }

    /// `CMSG_OPEN_ITEM` (`SpellHandler.cpp:160` gate
    /// order: mover/item/proto, taxi, dead, lock,
    /// cast interrupt, wrapped gifts, then the
    /// `SendLoot(itemGuid, LOOT_CORPSE)` arm).
    /// Gate half of `handle_open_item`: `GetItemByPos`
    /// plus the mover, taxi, dead, lock, and wrapped
    /// gates. Resolves the container guid, its
    /// prototype, and whether it ever generated.
    /// Every refusal sends its own equip error.
    /// `gift_lookup` runs the actor-side
    /// `character_gifts` query at the wrapped gate
    /// (the map layer holds no gift source); the fork
    /// order after the lock gate is preserved exactly.
    fn open_item_target(
        &mut self,
        player: ObjectGuid,
        bag: u8,
        slot: u8,
        gift_lookup: &mut dyn FnMut(u32) -> Result<Option<(u32, u32)>, DbError>,
    ) -> Option<(ObjectGuid, ItemPrototype, bool)> {
        let row = {
            let entry = self.players.get(&player)?;
            let inv = &entry.player.deferred.inventory;
            if bag == 255 {
                inv.iter().find(|r| r.bag == 0 && r.slot == slot).cloned()
            } else {
                inv.iter()
                    .find(|r| r.bag == 0 && r.slot == bag)
                    .and_then(|bag_row| {
                        inv.iter()
                            .find(|r| r.bag == bag_row.item.item_guid && r.slot == slot)
                    })
                    .cloned()
            }
        };
        let fail = |this: &mut Self, msg: u8, item: ObjectGuid| {
            this.send_inventory_error(player, msg, item, ObjectGuid::EMPTY, 0);
        };
        let Some(row) = row else {
            fail(self, ERR_ITEM_NOT_FOUND, ObjectGuid::EMPTY);
            return None;
        };
        let container = ObjectGuid::with_entry(HighGuid::ITEM, 0, row.item.item_guid);
        let Some(proto) = self.stores.item_prototypes.get(row.item.item_id).cloned() else {
            fail(self, ERR_ITEM_NOT_FOUND, container);
            return None;
        };
        if !self.unit_is_alive(player) {
            fail(self, ERR_YOU_ARE_DEAD, container);
            return None;
        }
        // `IsTaxiFlying`: fail closed while the taxi
        // flag rides (read-only unit word, no state
        // change).
        if self.unit_value_u32(
            player,
            mangos_protocol::update_field_table::offsets::UNIT_FIELD_FLAGS,
        ) & crate::motion::UNIT_FLAG_TAXI_FLIGHT
            != 0
        {
            fail(self, ERR_CANT_DO_RIGHT_NOW, container);
            return None;
        }
        // Lock gate: with no lock-template store every
        // set `LockID` on a still-locked container is
        // the fork unknown-lock refusal.
        if proto.lock_id != 0 && row.item.flags & ITEM_DYNFLAG_UNLOCKED == 0 {
            fail(self, ERR_ITEM_LOCKED, container);
            return None;
        }
        // Wrapped gifts never open a loot window: the
        // actor-side `character_gifts` lookup restores
        // the real entry/flags and clears the giver
        // (fork `SpellHandler.cpp:221-246` live-row
        // `SetEntry`/`SetFlags` + `ITEM_CHANGED`, then
        // the unwrap-time gift delete), or destroys
        // the wrapper when no gift row exists (fork
        // miss arm: log + `DestroyItem`). A lookup
        // error fails closed without touching state.
        if row.item.flags & ITEM_DYNFLAG_WRAPPED != 0 {
            let item_low = row.item.item_guid;
            match gift_lookup(item_low) {
                Ok(Some((entry, flags))) => {
                    let pre = self.snapshot(player);
                    if let Some(player_entry) = self.players.get_mut(&player)
                        && let Some(live) = player_entry
                            .player
                            .deferred
                            .inventory
                            .iter_mut()
                            .find(|r| r.item.item_guid == item_low)
                    {
                        live.item.item_id = entry;
                        live.item.flags = flags;
                        live.item.gift_creator_guid = 0;
                    }
                    self.sync_inventory(player, &pre);
                    return None;
                }
                Ok(None) => {
                    let pos = if row.bag == 0 {
                        InvPos::new(crate::inventory::BAG_MAIN, row.slot)
                    } else {
                        let bag_slot = self.players.get(&player).and_then(|entry| {
                            entry
                                .player
                                .deferred
                                .inventory
                                .iter()
                                .find(|r| r.item.item_guid == row.bag)
                                .map(|b| b.slot)
                        });
                        let bag_slot = bag_slot?;
                        InvPos::new(bag_slot, row.slot)
                    };
                    self.destroy_item(player, pos, 0);
                    return None;
                }
                Err(_) => {
                    fail(self, ERR_CANT_DO_RIGHT_NOW, container);
                    return None;
                }
            }
        }
        Some((container, proto, row.item.generated_loot != 0))
    }

    /// Generation half of `handle_open_item` (fork
    /// `SendLoot` item arm): open-item always passes
    /// `LOOT_CORPSE`, so generation uses the item
    /// template plus money and sets
    /// `SetGeneratedLoot(true)` so the row diff
    /// persists the word. A prototype disenchant id
    /// does not reroute open-item generation.
    fn deal_item_loot(&mut self, player: ObjectGuid, container: ObjectGuid, proto: &ItemPrototype) {
        let team = self.player_team_id(player);
        let mut state = LootState {
            personal: true,
            team,
            ..LootState::default()
        };
        let (loot_id, money) = (proto.item_id, (proto.min_money_loot, proto.max_money_loot));
        // One roll context (fork `FillLoot` then
        // `GenerateMoneyLoot` order).
        let (generated, gold) = {
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
            (
                generate_loot(&mut ctx, &stores.loot, &stores.loot.item, loot_id),
                generate_money_loot(&mut ctx, money.0, money.1),
            )
        };
        state.gold = gold;
        if let Some(generated) = generated {
            state.has_ffa_quest_items = generated.has_ffa_quest_items;
            for item in generated.items {
                state.items.push(LootSlotItem::from(item));
                state.unlooted_count += 1;
            }
            for item in generated.quest_items {
                state.quest_items.push(LootSlotItem::from(item));
                state.unlooted_count += 1;
            }
        }
        // `SetGeneratedLoot(true)`: the `Changed` row
        // persists the generated-loot word.
        let item_low = container.counter();
        if let Some(entry) = self.players.get_mut(&player)
            && let Some(live) = entry
                .player
                .deferred
                .inventory
                .iter_mut()
                .find(|r| r.item.item_guid == item_low)
        {
            live.item.generated_loot = 1;
        }
        let ustate = ItemLootUpdateState::Changed;
        state.allowed_looters.push(player);
        self.item_loots.insert(container, (state, ustate));
    }

    pub fn handle_open_item(
        &mut self,
        player: ObjectGuid,
        bag: u8,
        slot: u8,
        gift_lookup: &mut dyn FnMut(u32) -> Result<Option<(u32, u32)>, DbError>,
    ) {
        let Some((container, proto, generated)) =
            self.open_item_target(player, bag, slot, gift_lookup)
        else {
            return;
        };
        if self.casts.contains_key(&player) {
            self.cancel_cast(player, false);
        }
        // `SendLoot` releases the current window first.
        if self.player_loot_guid.contains_key(&player) {
            self.do_loot_release(player);
        }
        if self.item_loots.contains_key(&container) {
            // Generated before: serve the stored state
            // untouched, never clear or refill (fork
            // skips `FillLoot` when `HasGeneratedLoot`).
            self.player_loot_guid.insert(player, container);
            let view = self.item_loot_view_bytes(container);
            let mut writer = PacketWriter::new();
            writer.write_u64_le(container.raw());
            writer.write_u8(LOOT_CORPSE);
            writer.write_bytes(&view);
            let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
            self.send_to_player(player, packet);
            if let Some((state, _)) = self.item_loots.get_mut(&container) {
                state.players_looting.insert(player);
            }
            return;
        }
        // Anti-regen: the bool outlived the entry, so a
        // fresh `FillLoot` would fabricate loot (fork
        // `HasGeneratedLootSecondary` arm: error log +
        // anticheat + release; the map layer has no
        // anticheat hook, so the release carries it).
        if generated {
            self.send_loot_release(player, container);
            return;
        }
        self.deal_item_loot(player, container, &proto);
        self.player_loot_guid.insert(player, container);
        let view = self.item_loot_view_bytes(container);
        let mut writer = PacketWriter::new();
        writer.write_u64_le(container.raw());
        writer.write_u8(LOOT_CORPSE);
        writer.write_bytes(&view);
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_RESPONSE, writer.into_vec());
        self.send_to_player(player, packet);
        if let Some((state, _)) = self.item_loots.get_mut(&container) {
            state.players_looting.insert(player);
        }
        // Fork sets `UNIT_FLAG_LOOTING` only when
        // `!guid.IsItem`: item windows set no flag.
    }

    /// `CMSG_AUTOSTORE_LOOT_ITEM` item arm (fork
    /// `LootHandler.cpp:84-96`: entry gate, then the
    /// shared grant tail). Take marks `Changed`, even
    /// out of `Temporary`.
    pub fn handle_item_autostore(&mut self, player: ObjectGuid, guid: ObjectGuid, slot: u8) {
        let Some((state, _)) = self.item_loots.get(&guid) else {
            self.send_loot_release(player, guid);
            return;
        };
        let Some((item, is_quest)) = Self::item_slot_in(state, slot) else {
            self.send_inventory_error(
                player,
                ERR_ALREADY_LOOTED,
                ObjectGuid::EMPTY,
                ObjectGuid::EMPTY,
                0,
            );
            return;
        };
        let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() else {
            return;
        };
        if let Err(msg) = self.grant_loot_item(player, &proto, u32::from(item.count)) {
            self.send_inventory_error(player, msg, ObjectGuid::EMPTY, ObjectGuid::EMPTY, 0);
            return;
        }
        if let Some((state, ustate)) = self.item_loots.get_mut(&guid) {
            Self::mark_item_slot_taken(state, slot, is_quest);
            *ustate = ItemLootUpdateState::Changed;
        }
        self.send_loot_removed(player, slot);
    }

    /// `CMSG_LOOT_MONEY` item arm (fork item arm:
    /// same gate, no group share, gold zeroed,
    /// `Changed`).
    pub fn handle_item_loot_money(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        let Some(gold) = self.item_loots.get(&guid).map(|(s, _)| s.gold) else {
            self.send_loot_release(player, guid);
            return;
        };
        self.modify_money(player, i64::from(gold));
        let packet = WorldPacket::with_body(opcode_id::SMSG_LOOT_CLEAR_MONEY, Vec::new());
        self.send_to_player(player, packet);
        if let Some((state, ustate)) = self.item_loots.get_mut(&guid) {
            state.gold = 0;
            *ustate = ItemLootUpdateState::Changed;
        }
    }

    /// Grant every remaining slot (fork
    /// `AutoStoreLoot` on the disenchant release
    /// path). Slots that do not fit stay unlooted.
    fn autostore_item_remainder(&mut self, player: ObjectGuid, guid: ObjectGuid) {
        let slots: Vec<(u8, ItemPrototype)> =
            self.item_loots.get(&guid).map_or(Vec::new(), |(state, _)| {
                let mut out = Vec::new();
                for (i, item) in state.items.iter().enumerate() {
                    if item.is_looted {
                        continue;
                    }
                    if let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() {
                        out.push((i as u8, proto));
                    }
                }
                for (qi, item) in state.quest_items.iter().enumerate() {
                    if item.is_looted {
                        continue;
                    }
                    if let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() {
                        out.push(((state.items.len() + qi) as u8, proto));
                    }
                }
                out
            });
        let mut granted_any = false;
        for (slot, proto) in slots {
            let count = self
                .item_loots
                .get(&guid)
                .and_then(|(state, _)| Self::item_slot_in(state, slot).map(|(i, _)| i.count));
            let Some(count) = count else {
                continue;
            };
            if self
                .grant_loot_item(player, &proto, u32::from(count))
                .is_err()
            {
                continue;
            }
            granted_any = true;
            if let Some((state, _)) = self.item_loots.get_mut(&guid) {
                let plain = usize::from(slot) < state.items.len();
                Self::mark_item_slot_taken(state, slot, !plain);
                self.send_loot_removed(player, slot);
            }
        }
        // Only successful grants dirty the save
        // shape: a failed autostore — like a
        // `Temporary` disenchant window nobody could
        // take from — must not promote to `Changed`.
        if granted_any && let Some((_, ustate)) = self.item_loots.get_mut(&guid) {
            *ustate = ItemLootUpdateState::Changed;
        }
    }

    /// `DoLootRelease` item arm (fork `LootHandler.cpp:541-573`):
    /// temporary (disenchant) loot autostores the
    /// remainder then the container is destroyed;
    /// normal containers are destroyed only fully
    /// looted and not a bag. Returns whether the
    /// item still existed, which gates the caller's
    /// trailing `SendLootRelease`.
    fn do_item_loot_release(&mut self, player: ObjectGuid, guid: ObjectGuid) -> bool {
        let found = self
            .players
            .get(&player)
            .and_then(|entry| {
                entry
                    .player
                    .deferred
                    .inventory
                    .iter()
                    .find(|r| r.item.item_guid == guid.counter())
                    .cloned()
            })
            .and_then(|row| {
                self.stores
                    .item_prototypes
                    .get(row.item.item_id)
                    .cloned()
                    .map(|proto| (row, proto))
            });
        let Some((row, proto)) = found else {
            self.item_loots.remove(&guid);
            return false;
        };
        // Packet position of the container row: main
        // rows hang off `BAG_MAIN`, nested rows off
        // the containing bag's own slot.
        let pos = if row.bag == 0 {
            InvPos::new(crate::inventory::BAG_MAIN, row.slot)
        } else {
            let bag_slot = self.players.get(&player).and_then(|entry| {
                entry
                    .player
                    .deferred
                    .inventory
                    .iter()
                    .find(|r| r.item.item_guid == row.bag)
                    .map(|b| b.slot)
            });
            let Some(bag_slot) = bag_slot else {
                self.item_loots.remove(&guid);
                return true;
            };
            InvPos::new(bag_slot, row.slot)
        };
        let is_bag = proto.class == 1;
        let temporary = self
            .item_loots
            .get(&guid)
            .is_some_and(|(_, ustate)| *ustate == ItemLootUpdateState::Temporary);
        if temporary {
            let looted = self
                .item_loots
                .get(&guid)
                .is_none_or(|(state, _)| state.is_looted());
            if !looted {
                self.autostore_item_remainder(player, guid);
            }
            // A remainder that fits nowhere stays live:
            // destroying the container would delete
            // unrecoverable loot.
            let looted = self
                .item_loots
                .get(&guid)
                .is_none_or(|(state, _)| state.is_looted());
            if !looted {
                return true;
            }
            self.item_loots.remove(&guid);
            self.destroy_item(player, pos, 0);
            return true;
        }
        let looted = self
            .item_loots
            .get(&guid)
            .is_none_or(|(state, _)| state.is_looted());
        if looted && !is_bag {
            self.item_loots.remove(&guid);
            self.destroy_item(player, pos, 0);
        }
        true
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

    /// Bones take tail (fork `LootHandler.cpp:126-134`
    /// corpse arm): bones existence only — no distance
    /// gate, no alive/pickpocket gate, no roll-blocked
    /// or master-loot guards (fork gates the master
    /// guard on `lguid.IsCreature()`).
    fn autostore_corpse_item(&mut self, player: ObjectGuid, guid: ObjectGuid, slot: u8) {
        if !self.corpses.contains_key(&guid) {
            self.send_loot_release(player, guid);
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
        let Some(proto) = self.stores.item_prototypes.get(item.itemid).cloned() else {
            return;
        };
        if let Err(msg) = self.grant_loot_item(player, &proto, u32::from(item.count)) {
            self.send_inventory_error(player, msg, ObjectGuid::EMPTY, ObjectGuid::EMPTY, 0);
            return;
        }
        self.apply_loot_take_marks(player, guid, slot, &resolved);
    }

    /// `CMSG_AUTOSTORE_LOOT_ITEM` creature arm (GO
    /// chests share the take tail at the use band:
    /// no alive/pickpocket gate, no roll-blocked or
    /// master-loot guards — GO fills never set them
    /// and fork leaves `ALL_PERMISSION` on the GO
    /// branch — but the allowed-looter list still
    /// gates group-rule windows).
    pub fn handle_autostore_loot_item(&mut self, player: ObjectGuid, slot: u8) {
        let Some(guid) = self.player_loot_guid.get(&player).copied() else {
            return;
        };
        if guid.is_item() {
            self.handle_item_autostore(player, guid, slot);
            return;
        }
        if guid.is_game_object() {
            if !self.gameobjects.contains_key(&guid) {
                self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
                return;
            }
            if !self.go_within_use_range(player, guid) {
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
            if !self
                .loots
                .get(&guid)
                .is_some_and(|s| s.is_allowed_looter(player))
                || !self.loot_allowed_for_player(&item, player, guid)
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
            return;
        }
        if guid.is_corpse() {
            self.autostore_corpse_item(player, guid, slot);
            return;
        }
        if !guid.is_creature() || !self.creatures.contains_key(&guid) {
            self.send_loot_error(player, guid, loot_error::DIDNT_KILL);
            return;
        }
        // Fork `ok_loot` (`LootHandler.cpp`): live
        // victims serve takes only out of a pickpocket
        // window (mark-gated; the fork's rogue-class
        // half is UNMODELED as in the money arm).
        if self.unit_is_alive(guid) && !self.pickpocketed.contains(&guid) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(itemid: u32, count: u8) -> LootSlotItem {
        LootSlotItem {
            itemid,
            count,
            condition_id: 0,
            needs_quest: false,
            freeforall: false,
            is_looted: false,
            is_blocked: false,
            is_counted: false,
            is_underthreshold: false,
            loot_owner: ObjectGuid::EMPTY,
        }
    }

    fn container_state() -> LootState {
        let mut state = LootState {
            personal: true,
            ..LootState::default()
        };
        state.items.push(slot(2589, 2));
        state.quest_items.push(slot(9001, 1));
        state.gold = 137;
        state.unlooted_count = 2;
        state
    }

    /// A taken slot resolves to nothing afterwards,
    /// while the still-golded window stays unlooted
    /// (the reopen-serves-existing invariant at state
    /// level; the wire shape is covered by the
    /// loopback open-item round trip).
    #[test]
    fn slot_resolution_after_take() {
        let mut state = container_state();
        let (item, is_quest) = Map::item_slot_in(&state, 0).expect("plain slot resolves");
        assert_eq!(item.itemid, 2589);
        assert!(!is_quest);
        let (quest_item, is_quest) =
            Map::item_slot_in(&state, 1).expect("quest tail slot resolves");
        assert_eq!(quest_item.itemid, 9001);
        assert!(is_quest);
        Map::mark_item_slot_taken(&mut state, 0, false);
        assert!(Map::item_slot_in(&state, 0).is_none());
        // Gold keeps the window alive after the plain
        // take.
        assert!(!state.is_looted());
        Map::mark_item_slot_taken(&mut state, 1, true);
        assert!(Map::item_slot_in(&state, 1).is_none());
        assert!(!state.is_looted());
        state.gold = 0;
        assert!(state.is_looted());
    }

    /// Full take + money drains to `is_looted`, which
    /// is exactly the reopen-serves-empty / release-
    /// destroys shape the fork pins to `isLooted`.
    #[test]
    fn take_then_reopen_view_empty() {
        let mut state = container_state();
        Map::mark_item_slot_taken(&mut state, 0, false);
        Map::mark_item_slot_taken(&mut state, 1, true);
        assert!(!state.is_looted());
        state.gold = 0;
        assert!(state.is_looted());
        // Out-of-range slots never resolve.
        assert!(Map::item_slot_in(&state, 9).is_none());
    }

    /// `SkillGainChance` ladder (fork
    /// `Player.cpp:5320-5329`, x10 tables): thresholds
    /// are inclusive, else orange.
    #[test]
    fn skill_gain_chance_ladder() {
        // Grey at/above req+100.
        assert_eq!(skill_gain_chance(160, 160, 110, 85), 0);
        assert_eq!(skill_gain_chance(200, 160, 110, 85), 0);
        // Green at/above req+50.
        assert_eq!(skill_gain_chance(110, 160, 110, 85), 250);
        assert_eq!(skill_gain_chance(130, 160, 110, 85), 250);
        // Yellow at/above req+25.
        assert_eq!(skill_gain_chance(85, 160, 110, 85), 750);
        assert_eq!(skill_gain_chance(100, 160, 110, 85), 750);
        // Orange below.
        assert_eq!(skill_gain_chance(84, 160, 110, 85), 1000);
        assert_eq!(skill_gain_chance(1, 100, 50, 25), 1000);
    }

    /// Pickpocket repeat keeps quest state only (fork
    /// `LootMgr.h:320` `leaveOnlyQuestItems`): items,
    /// gold, FFA/conditional marks, allowed/looting
    /// viewers, RR and the FFA flag drop; quest items
    /// and per-player quest marks survive.
    #[test]
    fn pickpocket_repeat_keeps_quest_items_only() {
        let mut state = container_state();
        let low = 1234u32;
        state.player_quest_items.insert(
            low,
            vec![LootQuestMark {
                index: 0,
                is_looted: false,
            }],
        );
        state.player_ffa_items.insert(
            low,
            vec![LootQuestMark {
                index: 0,
                is_looted: false,
            }],
        );
        state.player_cond_items.insert(
            low,
            vec![LootQuestMark {
                index: 0,
                is_looted: false,
            }],
        );
        state.allowed_looters.push(ObjectGuid::from_raw(7));
        state.players_looting.insert(ObjectGuid::from_raw(7));
        state.round_robin_player = 7;
        state.personal = false;
        state.has_ffa_quest_items = true;
        LootState::leave_only_quest_items(&mut state);
        assert!(state.items.is_empty());
        assert_eq!(state.gold, 0);
        assert_eq!(state.unlooted_count, 0);
        assert_eq!(state.quest_items.len(), 1);
        assert!(state.player_quest_items.contains_key(&low));
        assert!(state.player_ffa_items.is_empty());
        assert!(state.player_cond_items.is_empty());
        assert!(state.allowed_looters.is_empty());
        assert!(state.players_looting.is_empty());
        assert_eq!(state.round_robin_player, 0);
        assert!(state.personal);
        assert!(!state.has_ffa_quest_items);
    }

    /// Vein release math (fork
    /// `LootHandler.cpp:469-497`): maxed uses
    /// deplete, below-min uses re-arm, and the
    /// middle roll compares against
    /// `100 * 0.8^(4/max * uses) + skill/(Req+25)`.
    #[test]
    fn vein_release_state_branches() {
        use crate::gameobjects::{GO_LOOT_JUST_DEACTIVATED, GO_LOOT_READY};
        // Below min always re-arms (uses 1 < min 2).
        assert_eq!(
            Map::vein_release_state(1, 2, 5, 175, 0, 99.9),
            GO_LOOT_READY
        );
        // Maxed uses deplete regardless of the roll.
        assert_eq!(
            Map::vein_release_state(5, 2, 5, 175, 300, 0.0),
            GO_LOOT_JUST_DEACTIVATED
        );
        // Middle band: roll under the chance re-arms
        // (uses 2, max 5: 100 * 0.8^1.6 ~= 69.6).
        assert_eq!(Map::vein_release_state(2, 2, 5, 175, 0, 0.0), GO_LOOT_READY);
        assert_eq!(
            Map::vein_release_state(2, 2, 5, 175, 0, 99.9),
            GO_LOOT_JUST_DEACTIVATED
        );
    }

    /// Orange-attempt fail predicate (fork
    /// `Spell.cpp:6091`): negative reqs clamp to 0,
    /// equality passes.
    #[test]
    fn skinning_try_again_predicate() {
        assert!(skinning_try_again_fails(40, 39));
        assert!(!skinning_try_again_fails(40, 40));
        assert!(!skinning_try_again_fails(40, 97));
        // Negative req clamps: only a negative roll
        // fails.
        assert!(skinning_try_again_fails(-90, -1));
        assert!(!skinning_try_again_fails(-90, 0));
        assert!(!skinning_try_again_fails(0, 0));
    }
}
