//! Guild service (Slice B.4): `Guild` from `Guild.cpp`
//! as a pure state machine, with the `GuildHandler.cpp`
//! packet shapes. Operations return [`GuildFx`] —
//! packets by member guid low, `guild`-family DB writes,
//! and `PLAYER_GUILDID`/`PLAYER_GUILDRANK` field
//! updates for online members — the world actor owns
//! sessions, the guild/member/invite indexes, and
//! entities.
//!
//! Deviations from the fork: charter petitions live
//! in `crate::petition` (B.4b) — `CMSG_GUILD_CREATE`
//! is `STATUS_NEVER` in the fork, so petitions are
//! the only live creation path; roster and leader-
//! succession iteration is guid-low order where C++
//! walks an unordered map (the client sorts the roster
//! itself); broken-zone member repair at load keeps
//! zone 0 instead of recomputing from coordinates;
//! emblem save skips the tabard-designer NPC proximity
//! check (no NPC-flag interaction surface in the actor
//! yet) but keeps the guild/leader/money gates; trial
//! restrictions read from the billing flags; created
//! date uses UTC civil time where C++ uses `localtime`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use mangos_db::guild::{LoadedGuild, StoredGuild, StoredGuildEvent};
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// `GUILD_RANKS_MIN_COUNT` / `GUILD_RANKS_MAX_COUNT`.
pub const RANKS_MIN_COUNT: usize = 5;
pub const RANKS_MAX_COUNT: usize = 10;
/// `GUILD_EVENTLOG_MAX_RECORDS`.
pub const EVENT_LOG_MAX_RECORDS: usize = 100;
/// Text limits from `Guild.h:38-45` (handler-enforced).
pub const NAME_MAX_LENGTH: usize = 24;
pub const RANK_NAME_MAX_LENGTH: usize = 15;
pub const NOTE_MAX_LENGTH: usize = 31;
pub const INFO_MAX_LENGTH: usize = 500;
pub const MOTD_MAX_LENGTH: usize = 128;
/// `GUILD_ROSTER_MAX_LENGTH`.
const ROSTER_MAX_LENGTH: i32 = 0x8000 - 4;

/// Default rank ids (`GuildDefaultRanks`).
pub const RANK_GUILDMASTER: u32 = 0;
pub const RANK_OFFICER: u32 = 1;

/// `GuildRankRights` values.
pub mod rights {
    pub const EMPTY: u32 = 0x0000_0040;
    pub const GCHATLISTEN: u32 = 0x0000_0041;
    pub const GCHATSPEAK: u32 = 0x0000_0042;
    pub const OFFCHATLISTEN: u32 = 0x0000_0044;
    pub const OFFCHATSPEAK: u32 = 0x0000_0048;
    pub const PROMOTE: u32 = 0x0000_00C0;
    pub const DEMOTE: u32 = 0x0000_0140;
    pub const INVITE: u32 = 0x0000_0050;
    pub const REMOVE: u32 = 0x0000_0060;
    pub const SETMOTD: u32 = 0x0000_1040;
    pub const EPNOTE: u32 = 0x0000_2040;
    pub const VIEWOFFNOTE: u32 = 0x0000_4040;
    pub const EOFFNOTE: u32 = 0x0000_8040;
    pub const MODIFY_GUILD_INFO: u32 = 0x0001_0040;
    pub const ALL: u32 = 0x000F_F1FF;
}

/// `Typecommand` for `SMSG_GUILD_COMMAND_RESULT`.
pub mod type_command {
    pub const CREATE: u32 = 0x00;
    pub const INVITE: u32 = 0x01;
    pub const QUIT: u32 = 0x03;
}

/// `CommandErrors` for `SMSG_GUILD_COMMAND_RESULT`.
pub mod command_error {
    pub const PLAYER_NO_MORE_IN_GUILD: u32 = 0x00;
    pub const ALREADY_IN_GUILD_S: u32 = 0x03;
    pub const ALREADY_INVITED_TO_GUILD_S: u32 = 0x05;
    pub const NAME_INVALID: u32 = 0x06;
    pub const NAME_EXISTS_S: u32 = 0x07;
    pub const LEADER_LEAVE: u32 = 0x08;
    pub const PERMISSIONS: u32 = 0x08;
    pub const PLAYER_NOT_IN_GUILD: u32 = 0x09;
    pub const PLAYER_NOT_IN_GUILD_S: u32 = 0x0A;
    pub const PLAYER_NOT_FOUND_S: u32 = 0x0B;
    pub const NOT_ALLIED: u32 = 0x0C;
    pub const RANK_TOO_HIGH_S: u32 = 0x0D;
    pub const RANK_TOO_LOW_S: u32 = 0x0E;
    pub const IGNORING_YOU_S: u32 = 0x13;
}

/// `GuildEvents` for `SMSG_GUILD_EVENT`.
pub mod guild_event {
    pub const PROMOTION: u8 = 0x00;
    pub const DEMOTION: u8 = 0x01;
    pub const MOTD: u8 = 0x02;
    pub const JOINED: u8 = 0x03;
    pub const LEFT: u8 = 0x04;
    pub const REMOVED: u8 = 0x05;
    pub const LEADER_CHANGED: u8 = 0x07;
    pub const DISBANDED: u8 = 0x08;
    pub const SIGNED_ON: u8 = 0x0C;
    pub const SIGNED_OFF: u8 = 0x0D;
}

/// `GuildEventLogTypes`.
pub mod event_log {
    pub const INVITE_PLAYER: u8 = 1;
    pub const JOIN_GUILD: u8 = 2;
    pub const PROMOTE_PLAYER: u8 = 3;
    pub const DEMOTE_PLAYER: u8 = 4;
    pub const UNINVITE_PLAYER: u8 = 5;
    pub const LEAVE_GUILD: u8 = 6;
}

/// `GuildEmblem` results for `MSG_SAVE_GUILD_EMBLEM`.
pub mod emblem_result {
    pub const SUCCESS: u32 = 0;
    pub const NOGUILD: u32 = 2;
    pub const NOTGUILDMASTER: u32 = 3;
    pub const NOTENOUGHMONEY: u32 = 4;
    pub const FAIL_NO_MESSAGE: u32 = 5;
}

/// One `guild`-family DB write, mirroring a C++
/// statement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuildWrite {
    CreateGuild(StoredGuild),
    DeleteGuild {
        guild_id: u32,
    },
    RenameGuild {
        guild_id: u32,
        name: String,
    },
    SetLeader {
        guild_id: u32,
        leader_low: u32,
    },
    SetMotd {
        guild_id: u32,
        motd: String,
    },
    SetInfo {
        guild_id: u32,
        info: String,
    },
    SetEmblem {
        guild_id: u32,
        emblem: [i32; 5],
    },
    InsertMember {
        guild_id: u32,
        member_low: u32,
        rank_id: u32,
    },
    DeleteMember {
        member_low: u32,
    },
    SetMemberRank {
        member_low: u32,
        rank_id: u32,
    },
    SetMemberPnote {
        member_low: u32,
        note: String,
    },
    SetMemberOffnote {
        member_low: u32,
        note: String,
    },
    InsertRank {
        guild_id: u32,
        rank_id: u32,
        name: String,
        rights: u32,
    },
    DeleteAllRanks {
        guild_id: u32,
    },
    DeleteRanksFrom {
        guild_id: u32,
        rank_id: u32,
    },
    SetRankName {
        guild_id: u32,
        rank_id: u32,
        name: String,
    },
    SetRankRights {
        guild_id: u32,
        rank_id: u32,
        rights: u32,
    },
    ReplaceEventLog {
        guild_id: u32,
        event: StoredGuildEvent,
    },
}

/// `SetInGuild` + `SetRank` for one online member's
/// update fields (guild id 0 clears membership).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuildFieldUpdate {
    pub guid_low: u32,
    pub guild_id: u32,
    pub rank_id: u32,
}

/// Side effects of one guild operation, in C++ emit
/// order within each list.
#[derive(Clone, Debug, Default)]
pub struct GuildFx {
    pub sends: Vec<(u32, WorldPacket)>,
    pub writes: Vec<GuildWrite>,
    pub fields: Vec<GuildFieldUpdate>,
}

impl GuildFx {
    fn merge(&mut self, other: GuildFx) {
        self.sends.extend(other.sends);
        self.writes.extend(other.writes);
        self.fields.extend(other.fields);
    }
}

/// One `MemberSlot`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuildMember {
    pub guid_low: u32,
    pub account_id: u32,
    pub name: String,
    pub rank_id: u32,
    pub level: u8,
    pub class: u8,
    pub zone_id: u32,
    pub logout_time: u64,
    pub pnote: String,
    pub offnote: String,
}

/// Character stats for a joining member (online player
/// or the `characters` cache row, as in `AddMember`).
#[derive(Clone, Debug)]
pub struct NewMemberStats {
    pub guid_low: u32,
    pub account_id: u32,
    pub name: String,
    pub level: u8,
    pub class: u8,
    pub zone_id: u32,
}

/// Live stats for one online member in the roster.
#[derive(Clone, Copy, Debug)]
pub struct RosterOnline {
    pub level: u8,
    pub class: u8,
    pub zone_id: u32,
    pub afk: bool,
    pub dnd: bool,
}

/// Player object guid for a guid low.
#[must_use]
pub fn player_guid(guid_low: u32) -> ObjectGuid {
    ObjectGuid::new(HighGuid::PLAYER, guid_low)
}

/// Civil (year, month, day) for a unix timestamp in
/// UTC; C++ uses `localtime` (see module deviation).
#[must_use]
pub fn civil_from_unix(secs: u64) -> (u32, u32, u32) {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (
        u32::try_from(year).unwrap_or(0),
        u32::try_from(m).unwrap_or(0),
        u32::try_from(d).unwrap_or(0),
    )
}

/// The guild state machine (`Guild`).
#[derive(Clone, Debug)]
pub struct Guild {
    id: u32,
    name: String,
    leader_low: u32,
    motd: String,
    ginfo: String,
    created: (u32, u32, u32),
    emblem: [i32; 5],
    ranks: Vec<(String, u32)>,
    members: BTreeMap<u32, GuildMember>,
    event_log: VecDeque<StoredGuildEvent>,
    event_log_next_guid: u32,
}

/// The five `CreateDefaultGuildRanks` rights rows.
const DEFAULT_RANK_RIGHTS: [u32; 5] = [
    rights::ALL,
    rights::ALL,
    rights::GCHATLISTEN | rights::GCHATSPEAK,
    rights::GCHATLISTEN | rights::GCHATSPEAK,
    rights::GCHATLISTEN | rights::GCHATSPEAK,
];

impl Guild {
    /// `Guild::Create(leader, gname)` with the
    /// actor-assigned id: store the row, create the
    /// default ranks, and add the leader as guildmaster.
    #[must_use]
    pub fn create_fresh(
        id: u32,
        name: String,
        leader: NewMemberStats,
        now_unix: u64,
        default_rank_names: &[String; 5],
    ) -> (Guild, GuildFx) {
        let mut guild = Guild {
            id,
            name: name.clone(),
            leader_low: leader.guid_low,
            motd: "No message set.".to_owned(),
            ginfo: String::new(),
            created: civil_from_unix(now_unix),
            emblem: [-1; 5],
            ranks: Vec::new(),
            members: BTreeMap::new(),
            event_log: VecDeque::new(),
            event_log_next_guid: 0,
        };
        let mut fx = GuildFx {
            writes: vec![GuildWrite::CreateGuild(StoredGuild {
                guild_id: id,
                name,
                leader_low: leader.guid_low,
                emblem_style: -1,
                emblem_color: -1,
                border_style: -1,
                border_color: -1,
                background_color: -1,
                info: String::new(),
                motd: guild.motd.clone(),
                create_date: now_unix,
            })],
            ..GuildFx::default()
        };
        fx.merge(guild.create_default_ranks(default_rank_names));
        if let Some(add_fx) = guild.add_member(leader, RANK_GUILDMASTER, now_unix) {
            fx.merge(add_fx);
        }
        (guild, fx)
    }

    /// Boot rebuild (`LoadGuildFromDB`, `LoadRanksFromDB`,
    /// `LoadMembersFromDB`, `CheckGuildStructure`).
    /// `None` means the guild disbands during load; the
    /// returned effects carry the cleanup writes either
    /// way. Sends are always empty at boot (no sessions).
    #[must_use]
    pub fn from_stored(
        loaded: &LoadedGuild,
        default_rank_names: &[String; 5],
    ) -> (Option<Guild>, GuildFx) {
        let stored = &loaded.guild;
        let mut fx = GuildFx::default();
        let mut guild = Guild {
            id: stored.guild_id,
            name: stored.name.clone(),
            leader_low: stored.leader_low,
            motd: stored.motd.clone(),
            ginfo: stored.info.clone(),
            created: civil_from_unix(stored.create_date),
            emblem: [
                stored.emblem_style,
                stored.emblem_color,
                stored.border_style,
                stored.border_color,
                stored.background_color,
            ],
            ranks: Vec::new(),
            members: BTreeMap::new(),
            event_log: VecDeque::new(),
            event_log_next_guid: loaded.events.first().map_or(0, |event| event.log_guid),
        };
        fx.merge(guild.load_ranks_repair(loaded, default_rank_names));
        let lowest = guild.lowest_rank();
        for member in &loaded.members {
            guild.members.insert(
                member.guid_low,
                GuildMember {
                    guid_low: member.guid_low,
                    account_id: member.account_id,
                    name: member.name.clone(),
                    rank_id: if member.rank_id > lowest {
                        lowest
                    } else {
                        member.rank_id
                    },
                    level: member.level,
                    class: member.class,
                    zone_id: member.zone_id,
                    logout_time: member.logout_time,
                    pnote: member.pnote.clone(),
                    offnote: member.offnote.clone(),
                },
            );
        }
        guild.event_log = loaded.events.iter().rev().copied().collect();
        if guild.members.is_empty() {
            fx.writes
                .push(GuildWrite::DeleteGuild { guild_id: guild.id });
            return (None, fx);
        }
        // CheckGuildStructure.
        match guild.rank_of(guild.leader_low) {
            None => {
                let (del_fx, need_disband) = guild.del_member(guild.leader_low, false);
                fx.merge(del_fx);
                if need_disband {
                    fx.writes
                        .push(GuildWrite::DeleteGuild { guild_id: guild.id });
                    return (None, fx);
                }
            }
            Some(rank) if rank != RANK_GUILDMASTER => {
                let leader = guild.leader_low;
                fx.merge(guild.set_leader(leader));
            }
            _ => {}
        }
        let leader = guild.leader_low;
        let extra_masters: Vec<u32> = guild
            .members
            .values()
            .filter(|member| member.rank_id == RANK_GUILDMASTER && member.guid_low != leader)
            .map(|member| member.guid_low)
            .collect();
        for low in extra_masters {
            fx.merge(guild.change_rank(low, RANK_OFFICER));
        }
        (Some(guild), fx)
    }

    /// `LoadRanksFromDB` + its repairs: sequence check,
    /// too-few rebuild to defaults, broken-sequence
    /// rewrite with sequential ids.
    fn load_ranks_repair(
        &mut self,
        loaded: &LoadedGuild,
        default_rank_names: &[String; 5],
    ) -> GuildFx {
        let mut fx = GuildFx::default();
        let broken_sequence = loaded
            .ranks
            .iter()
            .enumerate()
            .any(|(index, rank)| rank.rank_id != u32::try_from(index).unwrap_or(u32::MAX));
        for (index, rank) in loaded.ranks.iter().enumerate() {
            let row_rights = if index == 0 {
                rank.rights | rights::ALL
            } else {
                rank.rights
            };
            self.ranks.push((rank.name.clone(), row_rights));
        }
        if self.ranks.len() < RANKS_MIN_COUNT {
            self.ranks.clear();
            fx.merge(self.create_default_ranks(default_rank_names));
        } else if broken_sequence {
            fx.writes
                .push(GuildWrite::DeleteAllRanks { guild_id: self.id });
            for (index, (name, row_rights)) in self.ranks.iter().enumerate() {
                fx.writes.push(GuildWrite::InsertRank {
                    guild_id: self.id,
                    rank_id: u32::try_from(index).unwrap_or(0),
                    name: name.clone(),
                    rights: *row_rights,
                });
            }
        }
        fx
    }

    /// `CreateDefaultGuildRanks`: clear the rank table,
    /// then the five localized defaults.
    fn create_default_ranks(&mut self, names: &[String; 5]) -> GuildFx {
        let mut fx = GuildFx {
            writes: vec![GuildWrite::DeleteAllRanks { guild_id: self.id }],
            ..GuildFx::default()
        };
        for (index, (name, row_rights)) in names.iter().zip(DEFAULT_RANK_RIGHTS).enumerate() {
            self.ranks.push((name.clone(), row_rights));
            fx.writes.push(GuildWrite::InsertRank {
                guild_id: self.id,
                rank_id: u32::try_from(index).unwrap_or(0),
                name: name.clone(),
                rights: row_rights,
            });
        }
        fx
    }

    #[must_use]
    pub fn id(&self) -> u32 {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn leader_low(&self) -> u32 {
        self.leader_low
    }

    #[must_use]
    pub fn motd(&self) -> &str {
        &self.motd
    }

    #[must_use]
    pub fn created(&self) -> (u32, u32, u32) {
        self.created
    }

    /// `GetMemberSize`.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    #[must_use]
    pub fn members(&self) -> &BTreeMap<u32, GuildMember> {
        &self.members
    }

    /// `GetLowestRank` (highest rank id).
    #[must_use]
    pub fn lowest_rank(&self) -> u32 {
        u32::try_from(self.ranks.len().saturating_sub(1)).unwrap_or(0)
    }

    #[must_use]
    pub fn ranks_len(&self) -> usize {
        self.ranks.len()
    }

    /// `GetRank` (-1 in C++ when not a member).
    #[must_use]
    pub fn rank_of(&self, guid_low: u32) -> Option<u32> {
        self.members.get(&guid_low).map(|member| member.rank_id)
    }

    #[must_use]
    pub fn is_member(&self, guid_low: u32) -> bool {
        self.members.contains_key(&guid_low)
    }

    /// `GetMemberSlot(name)`: actor passes the normalized
    /// name; stored names are canonical.
    #[must_use]
    pub fn member_low_by_name(&self, name: &str) -> Option<u32> {
        self.members
            .values()
            .find(|member| member.name == name)
            .map(|member| member.guid_low)
    }

    /// `GetRankName` (`<unknown>` out of range).
    #[must_use]
    pub fn rank_name(&self, rank_id: u32) -> String {
        self.ranks
            .get(rank_id as usize)
            .map_or_else(|| "<unknown>".to_owned(), |(name, _)| name.clone())
    }

    /// `GetRankRights` (0 out of range).
    #[must_use]
    pub fn rank_rights(&self, rank_id: u32) -> u32 {
        self.ranks
            .get(rank_id as usize)
            .map_or(0, |(_, row_rights)| *row_rights)
    }

    /// `HasRankRight`, literally: the stored rights
    /// masked by the right constant (which itself
    /// carries the 0x40 base bit) must exceed that base.
    #[must_use]
    pub fn has_rank_right(&self, rank_id: u32, right: u32) -> bool {
        (self.rank_rights(rank_id) & right) != rights::EMPTY
    }

    /// `GetAccountsNumber` (distinct member accounts).
    #[must_use]
    pub fn accounts_number(&self) -> u32 {
        let accounts: HashSet<u32> = self
            .members
            .values()
            .map(|member| member.account_id)
            .collect();
        u32::try_from(accounts.len()).unwrap_or(0)
    }

    /// `AddMember` (the petition signature purge is
    /// an actor-side effect; see
    /// `World::remove_petitions_and_signs`). `None` =
    /// already a member.
    pub fn add_member(
        &mut self,
        stats: NewMemberStats,
        rank_id: u32,
        now_unix: u64,
    ) -> Option<GuildFx> {
        if self.members.contains_key(&stats.guid_low) {
            return None;
        }
        self.members.insert(
            stats.guid_low,
            GuildMember {
                guid_low: stats.guid_low,
                account_id: stats.account_id,
                name: stats.name,
                rank_id,
                level: stats.level,
                class: stats.class,
                zone_id: stats.zone_id,
                logout_time: now_unix,
                pnote: String::new(),
                offnote: String::new(),
            },
        );
        Some(GuildFx {
            writes: vec![GuildWrite::InsertMember {
                guild_id: self.id,
                member_low: stats.guid_low,
                rank_id,
            }],
            fields: vec![GuildFieldUpdate {
                guid_low: stats.guid_low,
                guild_id: self.id,
                rank_id,
            }],
            ..GuildFx::default()
        })
    }

    /// `DelMember`. The bool mirrors the C++ return:
    /// true when the guild must disband (emptied, or the
    /// leader left with no successor).
    pub fn del_member(&mut self, guid_low: u32, is_disbanding: bool) -> (GuildFx, bool) {
        let mut fx = GuildFx::default();
        if self.leader_low == guid_low && !is_disbanding {
            let old_leader_name = self
                .members
                .get(&guid_low)
                .map(|member| member.name.clone());
            let best = self
                .members
                .values()
                .filter(|member| member.guid_low != guid_low)
                .min_by_key(|member| member.rank_id)
                .map(|member| (member.guid_low, member.name.clone()));
            let Some((new_leader, new_leader_name)) = best else {
                return (fx, true);
            };
            fx.merge(self.set_leader(new_leader));
            if let Some(old_name) = old_leader_name {
                let changed = guild_event_packet(
                    guild_event::LEADER_CHANGED,
                    None,
                    &[&old_name, &new_leader_name],
                );
                fx.sends.extend(self.broadcast(&changed));
                let left = guild_event_packet(guild_event::LEFT, Some(guid_low), &[&old_name]);
                fx.sends.extend(self.broadcast(&left));
            }
        }
        self.members.remove(&guid_low);
        fx.writes.push(GuildWrite::DeleteMember {
            member_low: guid_low,
        });
        fx.fields.push(GuildFieldUpdate {
            guid_low,
            guild_id: 0,
            rank_id: 0,
        });
        (fx, self.members.is_empty())
    }

    /// `Disband`: event, remove every member, delete the
    /// guild rows. The actor destroys the guild after.
    pub fn disband(&mut self) -> GuildFx {
        let mut fx = GuildFx::default();
        let packet = guild_event_packet(guild_event::DISBANDED, None, &[]);
        fx.sends.extend(self.broadcast(&packet));
        while let Some(low) = self.members.keys().next().copied() {
            let (del_fx, _) = self.del_member(low, true);
            fx.merge(del_fx);
        }
        fx.writes
            .push(GuildWrite::DeleteGuild { guild_id: self.id });
        fx
    }

    /// `SetLeader`: promote the slot to guildmaster and
    /// store the new leader.
    pub fn set_leader(&mut self, guid_low: u32) -> GuildFx {
        if !self.members.contains_key(&guid_low) {
            return GuildFx::default();
        }
        self.leader_low = guid_low;
        let mut fx = self.change_rank(guid_low, RANK_GUILDMASTER);
        fx.writes.push(GuildWrite::SetLeader {
            guild_id: self.id,
            leader_low: guid_low,
        });
        fx
    }

    /// `MemberSlot::ChangeRank`.
    pub fn change_rank(&mut self, guid_low: u32, new_rank: u32) -> GuildFx {
        let Some(member) = self.members.get_mut(&guid_low) else {
            return GuildFx::default();
        };
        member.rank_id = new_rank;
        GuildFx {
            writes: vec![GuildWrite::SetMemberRank {
                member_low: guid_low,
                rank_id: new_rank,
            }],
            fields: vec![GuildFieldUpdate {
                guid_low,
                guild_id: self.id,
                rank_id: new_rank,
            }],
            ..GuildFx::default()
        }
    }

    /// `MemberSlot::SetMemberStats` + `UpdateLogoutTime`
    /// at logout.
    pub fn note_logout(&mut self, stats: &RosterOnline, guid_low: u32, now_unix: u64) {
        if let Some(member) = self.members.get_mut(&guid_low) {
            member.level = stats.level;
            member.class = stats.class;
            member.zone_id = stats.zone_id;
            member.logout_time = now_unix;
        }
    }

    /// `SetMOTD`.
    pub fn set_motd(&mut self, motd: String) -> GuildFx {
        self.motd.clone_from(&motd);
        GuildFx {
            writes: vec![GuildWrite::SetMotd {
                guild_id: self.id,
                motd,
            }],
            ..GuildFx::default()
        }
    }

    /// `SetGINFO`.
    pub fn set_ginfo(&mut self, info: String) -> GuildFx {
        self.ginfo.clone_from(&info);
        GuildFx {
            writes: vec![GuildWrite::SetInfo {
                guild_id: self.id,
                info,
            }],
            ..GuildFx::default()
        }
    }

    /// `SetEmblem`.
    pub fn set_emblem(&mut self, emblem: [i32; 5]) -> GuildFx {
        self.emblem = emblem;
        GuildFx {
            writes: vec![GuildWrite::SetEmblem {
                guild_id: self.id,
                emblem,
            }],
            ..GuildFx::default()
        }
    }

    /// Roster-slot level refresh (guild-bot level sync).
    pub fn set_member_level(&mut self, guid_low: u32, level: u8) {
        if let Some(member) = self.members.get_mut(&guid_low) {
            member.level = level;
        }
    }

    /// `MemberSlot::SetPNOTE`.
    pub fn set_pnote(&mut self, guid_low: u32, note: String) -> GuildFx {
        let Some(member) = self.members.get_mut(&guid_low) else {
            return GuildFx::default();
        };
        member.pnote.clone_from(&note);
        GuildFx {
            writes: vec![GuildWrite::SetMemberPnote {
                member_low: guid_low,
                note,
            }],
            ..GuildFx::default()
        }
    }

    /// `MemberSlot::SetOFFNOTE`.
    pub fn set_offnote(&mut self, guid_low: u32, note: String) -> GuildFx {
        let Some(member) = self.members.get_mut(&guid_low) else {
            return GuildFx::default();
        };
        member.offnote.clone_from(&note);
        GuildFx {
            writes: vec![GuildWrite::SetMemberOffnote {
                member_low: guid_low,
                note,
            }],
            ..GuildFx::default()
        }
    }

    /// `CreateRank` (capped at ten).
    pub fn create_rank(&mut self, name: String, row_rights: u32) -> GuildFx {
        if self.ranks.len() >= RANKS_MAX_COUNT {
            return GuildFx::default();
        }
        let rank_id = u32::try_from(self.ranks.len()).unwrap_or(0);
        self.ranks.push((name.clone(), row_rights));
        GuildFx {
            writes: vec![GuildWrite::InsertRank {
                guild_id: self.id,
                rank_id,
                name,
                rights: row_rights,
            }],
            ..GuildFx::default()
        }
    }

    /// `DelRank` (never below five; members on the
    /// deleted rank keep the stale id, as in C++).
    pub fn del_rank(&mut self) -> GuildFx {
        if self.ranks.len() <= RANKS_MIN_COUNT {
            return GuildFx::default();
        }
        let rank_id = self.lowest_rank();
        self.ranks.pop();
        GuildFx {
            writes: vec![GuildWrite::DeleteRanksFrom {
                guild_id: self.id,
                rank_id,
            }],
            ..GuildFx::default()
        }
    }

    /// `SetRankName`.
    pub fn set_rank_name(&mut self, rank_id: u32, name: String) -> GuildFx {
        let Some(rank) = self.ranks.get_mut(rank_id as usize) else {
            return GuildFx::default();
        };
        rank.0.clone_from(&name);
        GuildFx {
            writes: vec![GuildWrite::SetRankName {
                guild_id: self.id,
                rank_id,
                name,
            }],
            ..GuildFx::default()
        }
    }

    /// `SetRankRights`.
    pub fn set_rank_rights(&mut self, rank_id: u32, row_rights: u32) -> GuildFx {
        let Some(rank) = self.ranks.get_mut(rank_id as usize) else {
            return GuildFx::default();
        };
        rank.1 = row_rights;
        GuildFx {
            writes: vec![GuildWrite::SetRankRights {
                guild_id: self.id,
                rank_id,
                rights: row_rights,
            }],
            ..GuildFx::default()
        }
    }

    /// `LogGuildEvent`: ring-buffer append (oldest
    /// dropped at 100) plus the replace write keyed by
    /// the cycling log guid.
    pub fn log_event(
        &mut self,
        event_type: u8,
        guid1: u32,
        guid2: u32,
        new_rank: u8,
        now_unix: u64,
        event_log_count: u32,
    ) -> GuildFx {
        self.event_log_next_guid = (self.event_log_next_guid + 1) % event_log_count.max(1);
        let event = StoredGuildEvent {
            log_guid: self.event_log_next_guid,
            event_type,
            player_guid1: guid1,
            player_guid2: guid2,
            new_rank,
            timestamp: now_unix,
        };
        if self.event_log.len() >= EVENT_LOG_MAX_RECORDS {
            self.event_log.pop_front();
        }
        self.event_log.push_back(event);
        GuildFx {
            writes: vec![GuildWrite::ReplaceEventLog {
                guild_id: self.id,
                event,
            }],
            ..GuildFx::default()
        }
    }

    /// `GetGuildInviter`: latest matching invite-log
    /// search is front-to-back in C++ (oldest first);
    /// kept literal.
    #[must_use]
    pub fn inviter_of(&self, guid_low: u32) -> Option<u32> {
        self.event_log
            .iter()
            .find(|event| {
                event.event_type == event_log::INVITE_PLAYER && event.player_guid2 == guid_low
            })
            .map(|event| event.player_guid1)
    }

    /// `BroadcastPacket` recipient expansion (actor
    /// drops offline members).
    #[must_use]
    pub fn broadcast(&self, packet: &WorldPacket) -> Vec<(u32, WorldPacket)> {
        self.members
            .keys()
            .map(|low| (*low, packet.clone()))
            .collect()
    }

    /// `BroadcastEvent` to every member.
    #[must_use]
    pub fn broadcast_event(&self, event: u8, guid_low: Option<u32>, strs: &[&str]) -> GuildFx {
        let packet = guild_event_packet(event, guid_low, strs);
        GuildFx {
            sends: self.broadcast(&packet),
            ..GuildFx::default()
        }
    }

    /// `Query` response packet.
    #[must_use]
    pub fn query_packet(&self) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u32_le(self.id);
        writer.write_cstring(&self.name);
        for index in 0..RANKS_MAX_COUNT {
            match self.ranks.get(index) {
                Some((rank_name, _)) => writer.write_cstring(rank_name),
                None => writer.write_u8(0),
            };
        }
        for value in self.emblem {
            #[allow(clippy::cast_sign_loss)]
            writer.write_u32_le(value as u32);
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_QUERY_RESPONSE);
        packet.take_writer(writer);
        packet
    }

    /// `Roster` packet. `online` resolves live member
    /// stats; `can_view_offnote` mirrors the requesting
    /// session right (false for the broadcast form —
    /// C++ passes the null session there and hides
    /// officer notes from everyone).
    #[must_use]
    pub fn roster_packet(
        &self,
        online: &HashMap<u32, RosterOnline>,
        can_view_offnote: bool,
        now_unix: u64,
    ) -> WorldPacket {
        let mut writer = PacketWriter::new();
        // Count first (patched semantics computed ahead):
        // walk once to know how many fit the size cap.
        let mut space_left = ROSTER_MAX_LENGTH;
        space_left -= 4; // count
        space_left -= i32::try_from(self.motd.len() + 1).unwrap_or(i32::MAX);
        space_left -= i32::try_from(self.ginfo.len() + 1).unwrap_or(i32::MAX);
        space_left -= 4; // rank count
        space_left -= i32::try_from(self.ranks.len() * 4).unwrap_or(i32::MAX);
        let mut included: Vec<&GuildMember> = Vec::new();
        for member in self.members.values() {
            let needed = 8
                + 1
                + i32::try_from(member.name.len() + 1).unwrap_or(i32::MAX)
                + 4
                + 1
                + 1
                + 4
                + 4
                + i32::try_from(member.pnote.len() + 1).unwrap_or(i32::MAX)
                + i32::try_from(member.offnote.len() + 1).unwrap_or(i32::MAX);
            space_left -= needed;
            if space_left <= 0 {
                break;
            }
            included.push(member);
        }
        writer.write_u32_le(u32::try_from(included.len()).unwrap_or(0));
        writer.write_cstring(&self.motd);
        writer.write_cstring(&self.ginfo);
        writer.write_u32_le(u32::try_from(self.ranks.len()).unwrap_or(0));
        for (_, row_rights) in &self.ranks {
            writer.write_u32_le(*row_rights);
        }
        for member in included {
            writer.write_u64_le(player_guid(member.guid_low).raw());
            if let Some(live) = online.get(&member.guid_low) {
                let mut flags = 0x01u8;
                if live.afk {
                    flags |= 0x02;
                }
                if live.dnd {
                    flags |= 0x04;
                }
                writer.write_u8(flags);
                writer.write_cstring(&member.name);
                writer.write_u32_le(member.rank_id);
                writer.write_u8(live.level);
                writer.write_u8(live.class);
                writer.write_u32_le(live.zone_id);
            } else {
                writer.write_u8(0);
                writer.write_cstring(&member.name);
                writer.write_u32_le(member.rank_id);
                writer.write_u8(member.level);
                writer.write_u8(member.class);
                writer.write_u32_le(member.zone_id);
                #[allow(clippy::cast_precision_loss)]
                let days = now_unix.saturating_sub(member.logout_time) as f32 / 86_400.0;
                writer.write_f32_le(days);
            }
            writer.write_cstring(&member.pnote);
            writer.write_cstring(if can_view_offnote {
                &member.offnote
            } else {
                ""
            });
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_ROSTER);
        packet.take_writer(writer);
        packet
    }
}

/// `SMSG_GUILD_EVENT` packet (`BroadcastEvent` body).
#[must_use]
pub fn guild_event_packet(event: u8, guid_low: Option<u32>, strs: &[&str]) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(event);
    writer.write_u8(u8::try_from(strs.len()).unwrap_or(0));
    for text in strs {
        writer.write_cstring(text);
    }
    if let Some(low) = guid_low {
        writer.write_u64_le(player_guid(low).raw());
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_EVENT);
    packet.take_writer(writer);
    packet
}

/// `SendGuildCommandResult` packet.
#[must_use]
pub fn command_result_packet(type_command: u32, text: &str, result: u32) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(type_command);
    writer.write_cstring(text);
    writer.write_u32_le(result);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_COMMAND_RESULT);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GUILD_INVITE` (inviter name, guild name).
#[must_use]
pub fn guild_invite_packet(inviter_name: &str, guild_name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(inviter_name);
    writer.write_cstring(guild_name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_INVITE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GUILD_DECLINE` (decliner name).
#[must_use]
pub fn guild_decline_packet(name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_cstring(name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_DECLINE);
    packet.take_writer(writer);
    packet
}

/// `SMSG_GUILD_INFO` (`HandleGuildInfoOpcode`).
#[must_use]
pub fn guild_info_packet(guild: &Guild) -> WorldPacket {
    let (year, month, day) = guild.created();
    let mut writer = PacketWriter::new();
    writer.write_cstring(guild.name());
    writer.write_u32_le(day);
    writer.write_u32_le(month);
    writer.write_u32_le(year);
    writer.write_u32_le(u32::try_from(guild.member_count()).unwrap_or(0));
    writer.write_u32_le(guild.accounts_number());
    let mut packet = WorldPacket::new(opcode_id::SMSG_GUILD_INFO);
    packet.take_writer(writer);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_db::guild::{LoadedGuild, LoadedGuildMember, StoredGuildRank};

    fn rank_names() -> [String; 5] {
        [
            "Guild Master".to_owned(),
            "Officer".to_owned(),
            "Veteran".to_owned(),
            "Member".to_owned(),
            "Initiate".to_owned(),
        ]
    }

    fn stats(low: u32, name: &str) -> NewMemberStats {
        NewMemberStats {
            guid_low: low,
            account_id: 100 + low,
            name: name.to_owned(),
            level: 35,
            class: 1,
            zone_id: 215,
        }
    }

    #[test]
    fn create_writes_in_order_and_defaults() {
        let (guild, fx) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            1_700_000_000,
            &rank_names(),
        );
        assert_eq!(guild.leader_low(), 2);
        assert_eq!(guild.motd(), "No message set.");
        assert_eq!(guild.ranks_len(), 5);
        assert!(matches!(fx.writes[0], GuildWrite::CreateGuild(ref g) if g.guild_id == 9));
        assert!(matches!(
            fx.writes[1],
            GuildWrite::DeleteAllRanks { guild_id: 9 }
        ));
        assert!(matches!(
            fx.writes[2],
            GuildWrite::InsertRank {
                rank_id: 0,
                rights: rights::ALL,
                ..
            }
        ));
        assert!(matches!(
            fx.writes[7],
            GuildWrite::InsertMember {
                member_low: 2,
                rank_id: 0,
                ..
            }
        ));
        assert_eq!(
            fx.fields,
            vec![GuildFieldUpdate {
                guid_low: 2,
                guild_id: 9,
                rank_id: 0
            }]
        );
    }

    #[test]
    fn rank_right_base_bit_semantics() {
        let (guild, _) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            0,
            &rank_names(),
        );
        // Initiate has listen|speak (0x41|0x42) — both true.
        assert!(guild.has_rank_right(4, rights::GCHATLISTEN));
        assert!(guild.has_rank_right(4, rights::GCHATSPEAK));
        assert!(!guild.has_rank_right(4, rights::INVITE));
        assert!(guild.has_rank_right(0, rights::MODIFY_GUILD_INFO));
    }

    #[test]
    fn leader_leave_promotes_lowest_rank() {
        let (mut guild, _) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            0,
            &rank_names(),
        );
        guild.add_member(stats(3, "Zulrath"), 4, 10).unwrap();
        guild.add_member(stats(4, "Morka"), 1, 10).unwrap();
        let (fx, disband) = guild.del_member(2, false);
        assert!(!disband);
        assert_eq!(guild.leader_low(), 4);
        assert!(
            fx.writes
                .iter()
                .any(|w| matches!(w, GuildWrite::SetLeader { leader_low: 4, .. }))
        );
        assert!(
            fx.sends
                .iter()
                .any(|(_, p)| p.body().starts_with(&[guild_event::LEADER_CHANGED]))
        );
        assert!(!guild.is_member(2));
    }

    #[test]
    fn last_member_out_needs_disband() {
        let (mut guild, _) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            0,
            &rank_names(),
        );
        let (_, disband) = guild.del_member(2, false);
        assert!(disband);
    }

    #[test]
    fn roster_packet_shape_online_and_offline() {
        let (mut guild, _) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            0,
            &rank_names(),
        );
        guild.add_member(stats(3, "Zulrath"), 4, 86_400).unwrap();
        let mut online = HashMap::new();
        online.insert(
            2,
            RosterOnline {
                level: 35,
                class: 1,
                zone_id: 215,
                afk: true,
                dnd: false,
            },
        );
        let packet = guild.roster_packet(&online, false, 172_800);
        let body = packet.body();
        assert_eq!(&body[..4], &2u32.to_le_bytes());
        let mut pos = 4;
        let read_str = |body: &[u8], pos: &mut usize| {
            let end = body[*pos..].iter().position(|b| *b == 0).unwrap() + *pos;
            let s = String::from_utf8_lossy(&body[*pos..end]).into_owned();
            *pos = end + 1;
            s
        };
        assert_eq!(read_str(body, &mut pos), "No message set.");
        assert_eq!(read_str(body, &mut pos), "");
        assert_eq!(&body[pos..pos + 4], &5u32.to_le_bytes());
        pos += 4 + 20; // rank rights
        // Groktar online: flags 1|AFK(2).
        pos += 8; // guid
        assert_eq!(body[pos], 0x01 | 0x02);
        pos += 1;
        assert_eq!(read_str(body, &mut pos), "Groktar");
        pos += 4 + 1 + 1 + 4; // rank, level, class, zone
        assert_eq!(read_str(body, &mut pos), ""); // pnote
        assert_eq!(read_str(body, &mut pos), ""); // hidden offnote
        // Zulrath offline one day.
        pos += 8;
        assert_eq!(body[pos], 0);
        pos += 1;
        assert_eq!(read_str(body, &mut pos), "Zulrath");
        pos += 4 + 1 + 1 + 4;
        let days = f32::from_le_bytes(body[pos..pos + 4].try_into().unwrap());
        assert!((days - 1.0).abs() < 0.01, "{days}");
    }

    #[test]
    fn boot_rebuilds_missing_ranks() {
        let loaded = LoadedGuild {
            guild: StoredGuild {
                guild_id: 5,
                name: "Old".to_owned(),
                leader_low: 2,
                emblem_style: -1,
                emblem_color: -1,
                border_style: -1,
                border_color: -1,
                background_color: -1,
                info: String::new(),
                motd: "hi".to_owned(),
                create_date: 0,
            },
            ranks: vec![StoredGuildRank {
                rank_id: 0,
                name: "Boss".to_owned(),
                rights: 0,
            }],
            members: vec![LoadedGuildMember {
                guid_low: 2,
                rank_id: 7,
                pnote: String::new(),
                offnote: String::new(),
                name: "Groktar".to_owned(),
                level: 35,
                class: 1,
                zone_id: 215,
                logout_time: 0,
                account_id: 1,
            }],
            events: Vec::new(),
        };
        let (guild, fx) = Guild::from_stored(&loaded, &rank_names());
        let guild = guild.expect("live guild");
        assert_eq!(guild.ranks_len(), 5);
        // Rank 7 clamped to 4, then leader promoted to 0.
        assert_eq!(guild.rank_of(2), Some(0));
        assert!(
            fx.writes
                .iter()
                .any(|w| matches!(w, GuildWrite::DeleteAllRanks { .. }))
        );
    }

    #[test]
    fn event_log_guid_cycles() {
        let (mut guild, _) = Guild::create_fresh(
            9,
            "Test Guild".to_owned(),
            stats(2, "Groktar"),
            0,
            &rank_names(),
        );
        let fx = guild.log_event(event_log::JOIN_GUILD, 0, 3, 0, 10, 100);
        assert!(matches!(
            fx.writes[0],
            GuildWrite::ReplaceEventLog { event, .. } if event.log_guid == 1
        ));
    }
}
