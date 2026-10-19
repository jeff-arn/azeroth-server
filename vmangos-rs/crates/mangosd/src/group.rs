//! Group/party/raid service (Slice B.3): `Group` from
//! `Group.cpp`/`Group.h` as a pure state machine, with
//! the `GroupHandler.cpp` packet shapes. Operations take
//! an online-status view and return [`GroupFx`] — packets
//! addressed by member guid low, `groups`/`group_member`
//! writes for the DB store, and the guid lows whose
//! `PLAYER_FLAGS_GROUP_LEADER` bit must be refreshed —
//! the world actor owns sessions, indexes, and entities.
//!
//! Deviations from the fork: battleground/original-group
//! duality does not exist (no battlegrounds yet), so the
//! `isBGGroup()` arms collapse to the normal-group path;
//! LFG/meeting-stone, group loot rolls, instance binds
//! (`group_instance`, homebind-on-leave, reset-on-join),
//! the offline-leader promotion timer, `BroadcastGroupUpdate`
//! value forcing (Rust visibility rebuilds each tick),
//! and the guild-bot invite/convert hooks (Slice E) are
//! not ported. Loot method/threshold/looter are stored
//! and broadcast exactly as C++ (loot itself is Phase 4
//! creature-loot scope, rolls land with it).

use mangos_db::group::{LoadedMember, StoredGroup};
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `MAX_GROUP_SIZE` (`Group.h:50`).
pub const MAX_GROUP_SIZE: usize = 5;
/// `MAX_RAID_SIZE` (`Group.h:51`).
pub const MAX_RAID_SIZE: usize = 40;
/// `MAX_RAID_SUBGROUPS` (`Group.h:52`).
pub const MAX_RAID_SUBGROUPS: u8 = 8;
/// `TARGET_ICON_COUNT` (`Group.h:65`).
pub const TARGET_ICON_COUNT: usize = 8;

/// `PartyOperation` (`WorldSession.h:65-69`).
pub mod party_op {
    pub const INVITE: u32 = 0;
    pub const LEAVE: u32 = 2;
}

/// `PartyResult` (`WorldSession.h:71-83`).
pub mod party_result {
    pub const OK: u32 = 0;
    pub const BAD_PLAYER_NAME_S: u32 = 1;
    pub const TARGET_NOT_IN_GROUP_S: u32 = 2;
    pub const GROUP_FULL: u32 = 3;
    pub const ALREADY_IN_GROUP_S: u32 = 4;
    pub const NOT_IN_GROUP: u32 = 5;
    pub const NOT_LEADER: u32 = 6;
    pub const PLAYER_WRONG_FACTION: u32 = 7;
    pub const IGNORING_YOU_S: u32 = 8;
}

/// `LootMethod` (`Group.h:67-74`).
pub mod loot_method {
    pub const FREE_FOR_ALL: u8 = 0;
    pub const ROUND_ROBIN: u8 = 1;
    pub const MASTER_LOOT: u8 = 2;
    pub const GROUP_LOOT: u8 = 3;
    pub const NEED_BEFORE_GREED: u8 = 4;
}

/// `RemoveMethod` (`Group.h:76-80`). The uninvite handler
/// path (`Player::RemoveFromGroup`) always passes
/// `GROUP_LEAVE`; `GROUP_KICK` only adds the
/// `SMSG_GROUP_UNINVITE` notice.
pub mod remove_method {
    pub const LEAVE: u8 = 0;
    pub const KICK: u8 = 1;
}

/// `GroupMemberStatus` bits (`Group.h:95-105`).
pub mod member_status {
    pub const OFFLINE: u8 = 0x00;
    pub const ONLINE: u8 = 0x01;
    pub const PVP: u8 = 0x02;
    pub const DEAD: u8 = 0x04;
    pub const GHOST: u8 = 0x08;
    pub const PVP_FFA: u8 = 0x10;
    pub const AFK: u8 = 0x40;
    pub const DND: u8 = 0x80;
}

/// `GroupType` (`Group.h:109-113`) as written in
/// `SMSG_GROUP_LIST`.
pub const GROUP_TYPE_NORMAL: u8 = 0;
pub const GROUP_TYPE_RAID: u8 = 1;

/// `ITEM_QUALITY_UNCOMMON`, the create-time loot
/// threshold.
const LOOT_THRESHOLD_DEFAULT: u8 = 2;

/// A `groups`/`group_member` write for the DB store,
/// mirroring one C++ `PExecute` statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GroupWrite {
    /// `Group::Create` store (id clear + `REPLACE`).
    Create(StoredGroup),
    /// `Group::Disband` deletes.
    DeleteGroup { group_id: u32 },
    /// `Group::_addMember` insert.
    InsertMember {
        group_id: u32,
        member_low: u32,
        assistant: bool,
        subgroup: u8,
    },
    /// `Group::_removeMember` delete (by member).
    DeleteMember { member_low: u32 },
    /// `Group::_setLeader` update.
    SetLeader { group_id: u32, leader_low: u32 },
    /// `Group::ConvertToRaid` update.
    SetRaid { group_id: u32 },
    /// `Group::_setMembersGroup` / swap update.
    SetSubgroup { member_low: u32, subgroup: u8 },
    /// `Group::_setAssistantFlag` update.
    SetAssistant { member_low: u32, assistant: bool },
}

/// Side effects of one group operation, in C++ emit
/// order within each list.
#[derive(Clone, Debug, Default)]
pub struct GroupFx {
    /// Packets by recipient guid low (actor drops the
    /// ones with no live session, as `BroadcastPacket`
    /// only walks online references).
    pub sends: Vec<(u32, WorldPacket)>,
    /// DB writes in statement order.
    pub writes: Vec<GroupWrite>,
    /// Guid lows whose group-leader update-field bit the
    /// actor must recompute (`_updateLeaderFlag`).
    pub flag_refresh: Vec<u32>,
}

/// Online view the actor supplies: the
/// `GetGroupMemberStatus` byte for an online member, or
/// `None` when offline (status `OFFLINE`, no packets).
pub type StatusView<'a> = dyn Fn(u32) -> Option<u8> + 'a;

/// One `Group::MemberSlot`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberSlot {
    pub guid_low: u32,
    pub name: String,
    pub subgroup: u8,
    pub assistant: bool,
}

/// Player object guid for a guid low.
#[must_use]
pub fn player_guid(guid_low: u32) -> ObjectGuid {
    ObjectGuid::new(HighGuid::PLAYER, guid_low)
}

/// The group state machine (`Group`). A group exists
/// first as an uncreated invite shell (`db_id == 0`,
/// leader + invitees, no members) exactly like the C++
/// pre-`Create` object.
#[derive(Clone, Debug)]
pub struct Group {
    db_id: u32,
    leader_low: u32,
    leader_name: String,
    is_raid: bool,
    loot_method: u8,
    loot_threshold: u8,
    looter_raw: u64,
    main_tank_low: u32,
    main_assistant_low: u32,
    members: Vec<MemberSlot>,
    invitees: Vec<u32>,
    target_icons: [u64; TARGET_ICON_COUNT],
}

impl Group {
    /// The invite shell for a leader starting a group
    /// (`new Group` + `AddLeaderInvite`): the leader is
    /// an invitee, there are no members yet.
    #[must_use]
    pub fn new_invite_shell(leader_low: u32, leader_name: String) -> Self {
        Self {
            db_id: 0,
            leader_low,
            leader_name,
            is_raid: false,
            loot_method: loot_method::FREE_FOR_ALL,
            loot_threshold: LOOT_THRESHOLD_DEFAULT,
            looter_raw: 0,
            main_tank_low: 0,
            main_assistant_low: 0,
            members: Vec::new(),
            invitees: vec![leader_low],
            target_icons: [0; TARGET_ICON_COUNT],
        }
    }

    /// Rebuilds one boot-loaded group (`LoadGroupFromDB` +
    /// `LoadMemberFromDB`).
    #[must_use]
    pub fn from_stored(
        stored: &StoredGroup,
        leader_name: String,
        members: &[LoadedMember],
    ) -> Self {
        Self {
            db_id: stored.group_id,
            leader_low: stored.leader_low,
            leader_name,
            is_raid: stored.is_raid,
            loot_method: stored.loot_method,
            #[allow(clippy::cast_possible_truncation)]
            loot_threshold: stored.loot_threshold as u8,
            looter_raw: player_guid(stored.looter_low).raw(),
            main_tank_low: stored.main_tank_low,
            main_assistant_low: stored.main_assistant_low,
            members: members
                .iter()
                .map(|m| MemberSlot {
                    guid_low: m.member_low,
                    name: m.name.clone(),
                    subgroup: m.subgroup,
                    assistant: m.assistant,
                })
                .collect(),
            invitees: Vec::new(),
            target_icons: stored.icons,
        }
    }

    /// The persisted group id (0 while uncreated).
    #[must_use]
    pub fn db_id(&self) -> u32 {
        self.db_id
    }

    /// `IsCreated`: at least one member slot.
    #[must_use]
    pub fn is_created(&self) -> bool {
        !self.members.is_empty()
    }

    #[must_use]
    pub fn leader_low(&self) -> u32 {
        self.leader_low
    }

    #[must_use]
    pub fn leader_name(&self) -> &str {
        &self.leader_name
    }

    #[must_use]
    pub fn is_raid(&self) -> bool {
        self.is_raid
    }

    /// `GetMembersCount`.
    #[must_use]
    pub fn members_count(&self) -> usize {
        self.members.len()
    }

    #[must_use]
    pub fn members(&self) -> &[MemberSlot] {
        &self.members
    }

    #[must_use]
    pub fn invitees(&self) -> &[u32] {
        &self.invitees
    }

    /// `IsFull` for the current group type.
    #[must_use]
    pub fn is_full(&self) -> bool {
        if self.is_raid {
            self.members.len() >= MAX_RAID_SIZE
        } else {
            self.members.len() >= MAX_GROUP_SIZE
        }
    }

    #[must_use]
    pub fn is_member(&self, guid_low: u32) -> bool {
        self.members.iter().any(|m| m.guid_low == guid_low)
    }

    #[must_use]
    pub fn is_leader(&self, guid_low: u32) -> bool {
        self.leader_low == guid_low
    }

    #[must_use]
    pub fn is_assistant(&self, guid_low: u32) -> bool {
        self.members
            .iter()
            .find(|m| m.guid_low == guid_low)
            .is_some_and(|m| m.assistant)
    }

    /// `GetMemberGuid(name)` over member slots.
    #[must_use]
    pub fn member_low_by_name(&self, name: &str) -> Option<u32> {
        self.members
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.guid_low)
    }

    /// `GetMemberGroup` (`MAX_RAID_SUBGROUPS + 1` when not
    /// a member).
    #[must_use]
    pub fn member_subgroup(&self, guid_low: u32) -> u8 {
        self.members
            .iter()
            .find(|m| m.guid_low == guid_low)
            .map_or(MAX_RAID_SUBGROUPS + 1, |m| m.subgroup)
    }

    /// `GetInvited(guid)` presence.
    #[must_use]
    pub fn is_invited(&self, guid_low: u32) -> bool {
        self.invitees.contains(&guid_low)
    }

    /// Members in one subgroup (derived counter; C++
    /// keeps `m_subGroupsCounts`).
    fn subgroup_count(&self, subgroup: u8) -> usize {
        self.members
            .iter()
            .filter(|m| m.subgroup == subgroup)
            .count()
    }

    /// `HasFreeSlotSubGroup`.
    #[must_use]
    pub fn has_free_slot_subgroup(&self, subgroup: u8) -> bool {
        self.is_raid && self.subgroup_count(subgroup) < MAX_GROUP_SIZE
    }

    /// `AddInvite`: the actor has already proven the
    /// player has no group and no other invite (the C++
    /// checks live on `Player`); the guard here keeps the
    /// invitee/member sets disjoint.
    pub fn add_invite(&mut self, guid_low: u32) -> bool {
        if self.is_invited(guid_low) || self.is_member(guid_low) {
            return false;
        }
        self.invitees.push(guid_low);
        true
    }

    /// `RemoveInvite`; returns the member count like C++.
    pub fn remove_invite(&mut self, guid_low: u32) -> usize {
        self.invitees.retain(|low| *low != guid_low);
        self.members.len()
    }

    /// `RemoveAllInvites`; returns the cleared invitees
    /// so the actor can fix its invite index.
    pub fn remove_all_invites(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.invitees)
    }

    /// `Create(leaderGuid, leaderName)` with the
    /// actor-assigned group id: stores the row, then
    /// `AddMember(leader)` (which emits the first
    /// `SendUpdate`). Returns `None` when the leader add
    /// fails, matching the C++ `false` return (cannot
    /// happen for an empty shell).
    pub fn create(&mut self, db_id: u32, view: &StatusView<'_>) -> Option<GroupFx> {
        self.db_id = db_id;
        self.is_raid = false;
        self.loot_method = loot_method::GROUP_LOOT;
        self.loot_threshold = LOOT_THRESHOLD_DEFAULT;
        self.looter_raw = player_guid(self.leader_low).raw();
        let mut fx = GroupFx {
            writes: vec![GroupWrite::Create(StoredGroup {
                group_id: db_id,
                leader_low: self.leader_low,
                main_tank_low: self.main_tank_low,
                main_assistant_low: self.main_assistant_low,
                loot_method: self.loot_method,
                looter_low: self.leader_low,
                loot_threshold: u16::from(self.loot_threshold),
                icons: self.target_icons,
                is_raid: false,
            })],
            ..GroupFx::default()
        };
        let leader_name = self.leader_name.clone();
        let add_fx = self.add_member(self.leader_low, &leader_name, view)?;
        fx.merge(add_fx);
        fx.flag_refresh.push(self.leader_low);
        Some(fx)
    }

    /// `AddMember` + `_addMember`: first non-full
    /// subgroup for raids, slot append, member insert,
    /// then `SendUpdate`. The actor removes any invite
    /// first (the accept handler does).
    pub fn add_member(
        &mut self,
        guid_low: u32,
        name: &str,
        view: &StatusView<'_>,
    ) -> Option<GroupFx> {
        if guid_low == 0 || self.is_full() || self.is_member(guid_low) {
            return None;
        }
        let subgroup = if self.is_raid {
            (0..MAX_RAID_SUBGROUPS).find(|sub| self.subgroup_count(*sub) < MAX_GROUP_SIZE)?
        } else {
            0
        };
        self.members.push(MemberSlot {
            guid_low,
            name: name.to_owned(),
            subgroup,
            assistant: false,
        });
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::InsertMember {
                group_id: self.db_id,
                member_low: guid_low,
                assistant: false,
                subgroup,
            });
        }
        fx.merge(self.send_update(view));
        Some(fx)
    }

    /// `RemoveMember(guid, method)`. Returns the effects,
    /// the member count after the removal (C++ return),
    /// and whether the group disbanded (slots cleared,
    /// actor must destroy it — `Player::RemoveFromGroup`
    /// deletes on `<= 1`).
    pub fn remove_member(
        &mut self,
        guid_low: u32,
        method: u8,
        view: &StatusView<'_>,
    ) -> (GroupFx, usize, bool) {
        if self.members.len() <= 2 {
            let fx = self.disband(true, view);
            return (fx, 0, true);
        }
        let Some(position) = self.members.iter().position(|m| m.guid_low == guid_low) else {
            return (GroupFx::default(), self.members.len(), false);
        };
        self.members.remove(position);
        let mut fx = GroupFx {
            flag_refresh: vec![guid_low],
            ..GroupFx::default()
        };
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::DeleteMember {
                member_low: guid_low,
            });
        }
        let leader_changed = self.leader_low == guid_low;
        if leader_changed {
            fx.merge(self.choose_leader(view));
        }
        if method == remove_method::KICK {
            fx.sends.push((guid_low, group_uninvite_packet()));
        }
        fx.sends.push((guid_low, empty_group_list_packet()));
        if leader_changed {
            let packet = group_set_leader_packet(&self.leader_name);
            fx.sends.extend(self.broadcast(&packet, None, None));
        }
        fx.merge(self.send_update(view));
        (fx, self.members.len(), false)
    }

    /// `Disband(hideDestroy)`: notify members, send the
    /// empty group list, clear slots and invites, delete
    /// the rows. Actor snapshots member/invitee lows
    /// before calling to fix its indexes.
    pub fn disband(&mut self, hide_destroy: bool, _view: &StatusView<'_>) -> GroupFx {
        let mut fx = GroupFx::default();
        for member in &self.members {
            if !hide_destroy {
                fx.sends.push((member.guid_low, group_destroyed_packet()));
            }
            fx.sends.push((member.guid_low, empty_group_list_packet()));
            fx.flag_refresh.push(member.guid_low);
        }
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::DeleteGroup {
                group_id: self.db_id,
            });
        }
        self.members.clear();
        self.invitees.clear();
        self.leader_low = 0;
        self.leader_name.clear();
        fx
    }

    /// `ChangeLeader`: set, broadcast the name, update.
    pub fn change_leader(&mut self, guid_low: u32, view: &StatusView<'_>) -> GroupFx {
        let Some(slot) = self.members.iter().find(|m| m.guid_low == guid_low) else {
            return GroupFx::default();
        };
        if guid_low == self.leader_low {
            return GroupFx::default();
        }
        let name = slot.name.clone();
        let mut fx = self.set_leader(guid_low, &name);
        let packet = group_set_leader_packet(&name);
        fx.sends.extend(self.broadcast(&packet, None, None));
        fx.merge(self.send_update(view));
        fx
    }

    /// `_setLeader` (instance-bind shuffling is Phase 6).
    fn set_leader(&mut self, guid_low: u32, name: &str) -> GroupFx {
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::SetLeader {
                group_id: self.db_id,
                leader_low: guid_low,
            });
        }
        fx.flag_refresh.push(self.leader_low);
        self.leader_low = guid_low;
        name.clone_into(&mut self.leader_name);
        fx.flag_refresh.push(guid_low);
        fx
    }

    /// `_chooseLeader(offline = false)`: online members
    /// first, raid assistants before plain members, then
    /// the first online member, then slot order.
    fn choose_leader(&mut self, view: &StatusView<'_>) -> GroupFx {
        if self.members.len() < 2 {
            return GroupFx::default();
        }
        let mut first: Option<u32> = None;
        let mut chosen: Option<u32> = None;
        for member in &self.members {
            if member.guid_low == self.leader_low || view(member.guid_low).is_none() {
                continue;
            }
            if self.is_raid && !member.assistant {
                first.get_or_insert(member.guid_low);
                continue;
            }
            chosen = Some(member.guid_low);
            break;
        }
        let chosen = chosen.or(first).unwrap_or_else(|| self.members[0].guid_low);
        let name = self
            .members
            .iter()
            .find(|m| m.guid_low == chosen)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        self.set_leader(chosen, &name)
    }

    /// `ConvertToRaid`.
    pub fn convert_to_raid(&mut self, view: &StatusView<'_>) -> GroupFx {
        self.is_raid = true;
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::SetRaid {
                group_id: self.db_id,
            });
        }
        fx.merge(self.send_update(view));
        fx
    }

    /// `SetLootMethod` + `SetLooterGuid` +
    /// `SetLootThreshold` + `SendUpdate` (handler-side
    /// validation lives in the actor). Loot settings are
    /// not persisted by C++ outside `Create`.
    pub fn set_loot(
        &mut self,
        method: u8,
        looter_raw: u64,
        threshold: u8,
        view: &StatusView<'_>,
    ) -> GroupFx {
        self.loot_method = method;
        self.looter_raw = looter_raw;
        self.loot_threshold = threshold;
        self.send_update(view)
    }

    /// `SetAssistant` (raid only).
    pub fn set_assistant(&mut self, guid_low: u32, state: bool, view: &StatusView<'_>) -> GroupFx {
        if !self.is_raid {
            return GroupFx::default();
        }
        let Some(slot) = self.members.iter_mut().find(|m| m.guid_low == guid_low) else {
            return GroupFx::default();
        };
        slot.assistant = state;
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::SetAssistant {
                member_low: guid_low,
                assistant: state,
            });
        }
        fx.merge(self.send_update(view));
        fx
    }

    /// `ChangeMembersGroup` (online and offline members
    /// take the same slot path here).
    pub fn change_member_subgroup(
        &mut self,
        guid_low: u32,
        subgroup: u8,
        view: &StatusView<'_>,
    ) -> GroupFx {
        if !self.is_raid || subgroup >= MAX_RAID_SUBGROUPS {
            return GroupFx::default();
        }
        if self.subgroup_count(subgroup) >= MAX_GROUP_SIZE {
            return GroupFx::default();
        }
        let Some(slot) = self.members.iter_mut().find(|m| m.guid_low == guid_low) else {
            return GroupFx::default();
        };
        if slot.subgroup == subgroup {
            return GroupFx::default();
        }
        slot.subgroup = subgroup;
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::SetSubgroup {
                member_low: guid_low,
                subgroup,
            });
        }
        fx.merge(self.send_update(view));
        fx
    }

    /// `SwapMembersGroup`.
    pub fn swap_member_subgroups(
        &mut self,
        first_low: u32,
        second_low: u32,
        view: &StatusView<'_>,
    ) -> GroupFx {
        if !self.is_raid {
            return GroupFx::default();
        }
        let Some(first_pos) = self.members.iter().position(|m| m.guid_low == first_low) else {
            return GroupFx::default();
        };
        let Some(second_pos) = self.members.iter().position(|m| m.guid_low == second_low) else {
            return GroupFx::default();
        };
        let first_subgroup = self.members[first_pos].subgroup;
        let second_subgroup = self.members[second_pos].subgroup;
        self.members[first_pos].subgroup = second_subgroup;
        self.members[second_pos].subgroup = first_subgroup;
        let mut fx = GroupFx::default();
        if self.db_id != 0 {
            fx.writes.push(GroupWrite::SetSubgroup {
                member_low: first_low,
                subgroup: second_subgroup,
            });
            fx.writes.push(GroupWrite::SetSubgroup {
                member_low: second_low,
                subgroup: first_subgroup,
            });
        }
        fx.merge(self.send_update(view));
        fx
    }

    /// `SetTargetIcon`: clears the same target from other
    /// icons first (each clear broadcasts its own delta,
    /// exactly like the recursive C++ calls), then sets
    /// and broadcasts the delta.
    pub fn set_target_icon(&mut self, icon: usize, target_raw: u64) -> GroupFx {
        if icon >= TARGET_ICON_COUNT {
            return GroupFx::default();
        }
        let mut fx = GroupFx::default();
        if target_raw != 0 {
            for other in 0..TARGET_ICON_COUNT {
                if self.target_icons[other] == target_raw {
                    fx.merge(self.set_target_icon(other, 0));
                }
            }
        }
        self.target_icons[icon] = target_raw;
        let packet = target_icon_delta_packet(icon, target_raw);
        fx.sends.extend(self.broadcast(&packet, None, None));
        fx
    }

    /// Raw guid carrying `icon` (`GetTargetWithIcon`),
    /// 0 when unassigned/out of range.
    #[must_use]
    pub fn target_with_icon(&self, icon: usize) -> u64 {
        self.target_icons.get(icon).copied().unwrap_or(0)
    }

    /// The full icon-list packet (`SendTargetIconList`
    /// and the non-raid `SendUpdate` resend).
    #[must_use]
    pub fn target_icon_list_packet(&self) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(1);
        for (icon, raw) in self.target_icons.iter().enumerate() {
            if *raw == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            writer.write_u8(icon as u8);
            writer.write_u64_le(*raw);
        }
        let mut packet = WorldPacket::new(opcode_id::MSG_RAID_TARGET_UPDATE);
        packet.take_writer(writer);
        packet
    }

    /// `SendUpdate`: one `SMSG_GROUP_LIST` per online
    /// member, plus the marked-targets resend for
    /// non-raid groups.
    pub fn send_update(&self, view: &StatusView<'_>) -> GroupFx {
        let marked = if !self.is_raid && self.target_icons.iter().any(|raw| *raw != 0) {
            Some(self.target_icon_list_packet())
        } else {
            None
        };
        let mut fx = GroupFx::default();
        for recipient in &self.members {
            if view(recipient.guid_low).is_none() {
                continue;
            }
            fx.sends
                .push((recipient.guid_low, self.group_list_packet(recipient, view)));
            if let Some(packet) = &marked {
                fx.sends.push((recipient.guid_low, packet.clone()));
            }
        }
        fx
    }

    /// One recipient's `SMSG_GROUP_LIST` (the recipient
    /// is excluded from the member rows).
    fn group_list_packet(&self, recipient: &MemberSlot, view: &StatusView<'_>) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(if self.is_raid {
            GROUP_TYPE_RAID
        } else {
            GROUP_TYPE_NORMAL
        });
        writer.write_u8(recipient.subgroup | if recipient.assistant { 0x80 } else { 0 });
        #[allow(clippy::cast_possible_truncation)]
        writer.write_u32_le(self.members.len().saturating_sub(1) as u32);
        for member in &self.members {
            if member.guid_low == recipient.guid_low {
                continue;
            }
            writer.write_cstring(&member.name);
            writer.write_u64_le(player_guid(member.guid_low).raw());
            writer.write_u8(view(member.guid_low).unwrap_or(member_status::OFFLINE));
            writer.write_u8(member.subgroup | if member.assistant { 0x80 } else { 0 });
        }
        writer.write_u64_le(player_guid(self.leader_low).raw());
        if self.members.len() > 1 {
            writer.write_u8(self.loot_method);
            writer.write_u64_le(if self.loot_method == loot_method::MASTER_LOOT {
                self.looter_raw
            } else {
                0
            });
            writer.write_u8(self.loot_threshold);
            writer.write_u8(0); // dungeon difficulty (1.10.2+)
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_GROUP_LIST);
        packet.take_writer(writer);
        packet
    }

    /// `BroadcastPacket` recipient expansion: every
    /// member (actor drops offline ones), optionally one
    /// subgroup only, optionally skipping one member.
    #[must_use]
    pub fn broadcast(
        &self,
        packet: &WorldPacket,
        subgroup: Option<u8>,
        ignore: Option<u32>,
    ) -> Vec<(u32, WorldPacket)> {
        self.members
            .iter()
            .filter(|m| subgroup.is_none_or(|sub| m.subgroup == sub))
            .filter(|m| ignore != Some(m.guid_low))
            .map(|m| (m.guid_low, packet.clone()))
            .collect()
    }
}

impl GroupFx {
    fn merge(&mut self, other: GroupFx) {
        self.sends.extend(other.sends);
        self.writes.extend(other.writes);
        self.flag_refresh.extend(other.flag_refresh);
    }
}

/// `WorldSession::SendPartyResult` packet.
#[must_use]
pub fn party_result_packet(operation: u32, member: &str, result: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(operation);
    writer.write_cstring(member);
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_PARTY_COMMAND_RESULT);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GROUP_INVITE` (inviter name).
#[must_use]
pub fn group_invite_packet(inviter_name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(inviter_name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GROUP_INVITE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GROUP_DECLINE` (decliner name).
#[must_use]
pub fn group_decline_packet(name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GROUP_DECLINE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GROUP_SET_LEADER` (new leader name).
#[must_use]
pub fn group_set_leader_packet(name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GROUP_SET_LEADER);
    packet.take_writer(writer);
    packet
}

/// The empty `SMSG_GROUP_LIST` (three zero u64s) sent
/// to a player who just left or whose group disbanded.
#[must_use]
pub fn empty_group_list_packet() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u64_le(0);
    writer.write_u64_le(0);
    writer.write_u64_le(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GROUP_LIST);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GROUP_DESTROYED` (empty body).
#[must_use]
pub fn group_destroyed_packet() -> WorldPacket {
    WorldPacket::new(opcode_id::SMSG_GROUP_DESTROYED)
}

/// `SMSG_GROUP_UNINVITE` (empty body; kick notice).
#[must_use]
pub fn group_uninvite_packet() -> WorldPacket {
    WorldPacket::new(opcode_id::SMSG_GROUP_UNINVITE)
}

/// One `MSG_RAID_TARGET_UPDATE` delta record.
fn target_icon_delta_packet(icon: usize, target_raw: u64) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(0);
    #[allow(clippy::cast_possible_truncation)]
    writer.write_u8(icon as u8);
    writer.write_u64_le(target_raw);
    let mut packet = WorldPacket::new(opcode_id::MSG_RAID_TARGET_UPDATE);
    packet.take_writer(writer);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)] // signature is StatusView
    fn all_online(_low: u32) -> Option<u8> {
        Some(member_status::ONLINE)
    }

    fn started_group() -> Group {
        let mut group = Group::new_invite_shell(2, "Groktar".to_owned());
        assert!(group.add_invite(3));
        group.remove_invite(2);
        group.create(7, &all_online).expect("create");
        group.remove_invite(3);
        group.add_member(3, "Zulra", &all_online).expect("join");
        group
    }

    #[test]
    fn invite_shell_create_and_join_writes_in_order() {
        let mut group = Group::new_invite_shell(2, "Groktar".to_owned());
        assert_eq!(group.invitees(), &[2]);
        assert!(!group.is_created());
        let fx = group.create(7, &all_online).expect("create");
        assert_eq!(group.db_id(), 7);
        assert!(group.is_created());
        assert!(
            matches!(fx.writes[0], GroupWrite::Create(ref g) if g.group_id == 7
            && g.loot_method == loot_method::GROUP_LOOT && g.loot_threshold == 2)
        );
        assert!(matches!(
            fx.writes[1],
            GroupWrite::InsertMember { member_low: 2, .. }
        ));
        // Leader's first list: count 0, no loot tail.
        let (to, packet) = &fx.sends[0];
        assert_eq!(*to, 2);
        assert_eq!(packet.opcode(), opcode_id::SMSG_GROUP_LIST);
        let mut expect = vec![0u8, 0, 0, 0, 0, 0];
        expect.extend_from_slice(&player_guid(2).raw().to_le_bytes());
        assert_eq!(packet.body(), expect.as_slice());
        assert_eq!(fx.flag_refresh.last(), Some(&2));
    }

    #[test]
    fn group_list_body_matches_send_update_shape() {
        let group = started_group();
        let fx = group.send_update(&all_online);
        assert_eq!(fx.sends.len(), 2);
        let zulra_view = fx
            .sends
            .iter()
            .find(|(to, _)| *to == 3)
            .map(|(_, p)| p)
            .expect("to zulra");
        let mut expect = vec![0u8, 0, 1, 0, 0, 0];
        expect.extend_from_slice(b"Groktar\0");
        expect.extend_from_slice(&player_guid(2).raw().to_le_bytes());
        expect.extend_from_slice(&[member_status::ONLINE, 0]);
        expect.extend_from_slice(&player_guid(2).raw().to_le_bytes());
        expect.extend_from_slice(&[loot_method::GROUP_LOOT]);
        expect.extend_from_slice(&0u64.to_le_bytes());
        expect.extend_from_slice(&[2, 0]);
        assert_eq!(zulra_view.body(), expect.as_slice());
    }

    #[test]
    fn remove_from_pair_disbands_silently() {
        let mut group = started_group();
        let (fx, remaining, destroyed) = group.remove_member(3, remove_method::LEAVE, &all_online);
        assert_eq!((remaining, destroyed), (0, true));
        assert!(matches!(
            fx.writes[..],
            [GroupWrite::DeleteGroup { group_id: 7 }]
        ));
        // hideDestroy: only the empty lists, no destroyed packet.
        assert_eq!(fx.sends.len(), 2);
        assert!(
            fx.sends
                .iter()
                .all(|(_, p)| p.opcode() == opcode_id::SMSG_GROUP_LIST && p.body().len() == 24)
        );
        assert_eq!(group.members_count(), 0);
        assert_eq!(group.leader_low(), 0);
    }

    #[test]
    fn leader_leave_from_trio_promotes_and_broadcasts() {
        let mut group = started_group();
        group.add_member(4, "Mokra", &all_online).expect("third");
        let (fx, remaining, destroyed) = group.remove_member(2, remove_method::LEAVE, &all_online);
        assert_eq!((remaining, destroyed), (2, false));
        assert_eq!(group.leader_low(), 3);
        assert!(matches!(
            fx.writes[0],
            GroupWrite::DeleteMember { member_low: 2 }
        ));
        assert!(matches!(
            fx.writes[1],
            GroupWrite::SetLeader {
                group_id: 7,
                leader_low: 3
            }
        ));
        assert!(fx.sends.iter().any(|(to, p)| {
            *to == 4 && p.opcode() == opcode_id::SMSG_GROUP_SET_LEADER && p.body() == b"Zulra\0"
        }));
    }

    #[test]
    fn party_full_at_five() {
        let mut group = started_group();
        for low in 4..=6 {
            group
                .add_member(low, &format!("P{low}"), &all_online)
                .expect("fill");
        }
        assert!(group.is_full());
        assert!(group.add_member(7, "Sixth", &all_online).is_none());
    }

    #[test]
    fn raid_subgroups_assistant_and_swap() {
        let mut group = started_group();
        group.convert_to_raid(&all_online);
        assert!(group.is_raid());
        let fx = group.change_member_subgroup(3, 2, &all_online);
        assert!(matches!(
            fx.writes[..],
            [GroupWrite::SetSubgroup {
                member_low: 3,
                subgroup: 2
            }]
        ));
        assert_eq!(group.member_subgroup(3), 2);
        let fx = group.set_assistant(3, true, &all_online);
        assert!(matches!(
            fx.writes[..],
            [GroupWrite::SetAssistant {
                member_low: 3,
                assistant: true
            }]
        ));
        group.swap_member_subgroups(2, 3, &all_online);
        assert_eq!(group.member_subgroup(2), 2);
        assert_eq!(group.member_subgroup(3), 0);
        assert_eq!(group.member_subgroup(99), MAX_RAID_SUBGROUPS + 1);
    }

    #[test]
    fn target_icon_clears_duplicate_first() {
        let mut group = started_group();
        let target = (0xF130_u64) << 48 | 0x2A;
        let fx = group.set_target_icon(7, target);
        assert_eq!(fx.sends.len(), 2); // delta to both members
        let fx = group.set_target_icon(0, target);
        // Clear-delta for icon 7, then set-delta for icon 0.
        assert_eq!(fx.sends.len(), 4);
        let list = group.target_icon_list_packet();
        assert_eq!(list.body(), &[1, 0, 42, 0, 0, 0, 0, 0, 0x30, 0xF1][..]);
    }
}
