//! Chat channels (Slice B.2b): `Channel` + `ChannelMgr`
//! from `Channel.cpp`/`ChannelMgr.cpp`, as a pure state
//! machine. Operations take an acting-player view (and
//! actor-resolved target views) and return outbound
//! packets as [`ChannelOut`]; the world actor resolves
//! sessions, applies ignore filtering for `ToAll`, and
//! owns the two per-faction managers.
//!
//! Deviations from the fork: honor rank is a constant 0
//! (no honor system yet, so `WorldDefense` speech mutes
//! like a rank-0 player in C++), the account-flag
//! public-channel vanish mute and bot chat hooks are not
//! ported, channel-list visibility uses the security
//! gate only (no GM invisibility state), and the
//! channel-owner name resolves from online players only
//! (no DB name-by-guid lookup).

use std::collections::{BTreeMap, HashMap, HashSet};

use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;
use mangos_world_data::chat_channels::{ChatChannelDbc, dbc_flags};

use crate::security;

/// `ChatNotify` values (`Channel.h:35-71`).
pub mod notify {
    pub const JOINED: u8 = 0x00;
    pub const LEFT: u8 = 0x01;
    pub const YOU_JOINED: u8 = 0x02;
    pub const YOU_LEFT: u8 = 0x03;
    pub const WRONG_PASSWORD: u8 = 0x04;
    pub const NOT_MEMBER: u8 = 0x05;
    pub const NOT_MODERATOR: u8 = 0x06;
    pub const PASSWORD_CHANGED: u8 = 0x07;
    pub const OWNER_CHANGED: u8 = 0x08;
    pub const PLAYER_NOT_FOUND: u8 = 0x09;
    pub const NOT_OWNER: u8 = 0x0A;
    pub const CHANNEL_OWNER: u8 = 0x0B;
    pub const MODE_CHANGE: u8 = 0x0C;
    pub const ANNOUNCEMENTS_ON: u8 = 0x0D;
    pub const ANNOUNCEMENTS_OFF: u8 = 0x0E;
    pub const MODERATION_ON: u8 = 0x0F;
    pub const MODERATION_OFF: u8 = 0x10;
    pub const MUTED: u8 = 0x11;
    pub const PLAYER_KICKED: u8 = 0x12;
    pub const BANNED: u8 = 0x13;
    pub const PLAYER_BANNED: u8 = 0x14;
    pub const PLAYER_UNBANNED: u8 = 0x15;
    pub const PLAYER_NOT_BANNED: u8 = 0x16;
    pub const PLAYER_ALREADY_MEMBER: u8 = 0x17;
    pub const INVITE: u8 = 0x18;
    pub const INVITE_WRONG_FACTION: u8 = 0x19;
    pub const INVALID_NAME: u8 = 0x1B;
    pub const PLAYER_INVITED: u8 = 0x1D;
    pub const PLAYER_INVITE_BANNED: u8 = 0x1E;
}

/// `Channel::ChannelFlags` (`Channel.h:87-103`).
pub mod channel_flag {
    pub const CUSTOM: u8 = 0x01;
    pub const TRADE: u8 = 0x04;
    pub const NOT_LFG: u8 = 0x08;
    pub const GENERAL: u8 = 0x10;
    pub const CITY: u8 = 0x20;
    pub const LFG: u8 = 0x40;
}

/// `Channel::ChannelMemberFlags` (`Channel.h:119-130`).
pub mod member_flag {
    pub const OWNER: u8 = 0x01;
    pub const MODERATOR: u8 = 0x02;
    pub const VOICED: u8 = 0x04;
    pub const MUTED: u8 = 0x08;
}

/// `CHANNEL_ID_WORLD_DEFENSE` (`Channel.h:79`).
pub const CHANNEL_ID_WORLD_DEFENSE: u32 = 23;
/// `GuildRecruitment`'s exact flag set (`Channel.h:101`,
/// checked literally in `Channel::Join`).
const GUILD_RECRUITMENT_FLAGS: u8 = 0x38;

/// The world actor's view of one online player for
/// channel permission checks (`GetSession()->GetSecurity`,
/// `GetTeam`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelActor {
    pub guid_low: u32,
    pub security: u8,
    pub team_id: u32,
}

/// One outbound channel packet batch element.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelOut {
    /// `SendToOne`.
    ToOne { guid_low: u32, packet: WorldPacket },
    /// `SendToAll`: the member snapshot at emit time
    /// (join broadcasts precede the insert, kick
    /// broadcasts precede the erase), minus members
    /// ignoring `ignore_source` (when set), applied by
    /// the actor.
    ToAll {
        recipients: Vec<u32>,
        packet: WorldPacket,
        ignore_source: Option<u32>,
    },
}

fn player_guid(guid_low: u32) -> ObjectGuid {
    ObjectGuid::with_entry(HighGuid::PLAYER, 0, guid_low)
}

/// `normalizePlayerName(name, 128)` for custom channel
/// names (`Channel::Channel`, `ObjectMgr.cpp:62-80`).
#[must_use]
pub fn normalize_channel_name(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if name.chars().count() > 128 {
        return None;
    }
    let mut normalized: String = first.to_uppercase().collect();
    normalized.extend(chars.flat_map(char::to_lowercase));
    Some(normalized)
}

/// `Channel` (`Channel.h:84-312`).
#[derive(Clone, Debug)]
pub struct Channel {
    announce: bool,
    moderate: bool,
    level_restricted: bool,
    name: String,
    password: String,
    flags: u8,
    security_level: u8,
    dbc_id: u32,
    owner_guid: Option<u32>,
    players: BTreeMap<u32, u8>,
    banned: HashSet<u32>,
}

impl Channel {
    /// `Channel::Channel(name)`: built-in detection via
    /// `ChatChannels.dbc`, else custom-channel rules
    /// (including the fork's `World`/`China` specials).
    #[must_use]
    pub fn new(name: &str, dbc: &ChatChannelDbc) -> Self {
        let mut channel = Self {
            announce: true,
            moderate: false,
            level_restricted: true,
            name: name.to_owned(),
            password: String::new(),
            flags: 0,
            security_level: 0,
            dbc_id: 0,
            owner_guid: None,
            players: BTreeMap::new(),
            banned: HashSet::new(),
        };
        if let Some(entry) = dbc.entry_for_name(name) {
            channel.dbc_id = entry.channel_id;
            channel.announce = false;
            channel.flags |= channel_flag::GENERAL;
            if entry.flags & dbc_flags::TRADE != 0 {
                channel.flags |= channel_flag::TRADE;
            }
            if entry.flags & dbc_flags::CITY_ONLY2 != 0 {
                channel.flags |= channel_flag::CITY;
            }
            if entry.flags & dbc_flags::LFG != 0 {
                channel.flags |= channel_flag::LFG;
            } else {
                channel.flags |= channel_flag::NOT_LFG;
            }
        } else {
            if let Some(normalized) = normalize_channel_name(name) {
                channel.name = normalized;
            } else {
                "INVALIDCHANNEL".clone_into(&mut channel.name);
                channel.announce = false;
            }
            if channel.name == "World" {
                channel.flags |= channel_flag::GENERAL;
                channel.announce = false;
            } else if channel.name == "China" || channel.name == "中国" {
                channel.flags |= channel_flag::CUSTOM;
                channel.announce = false;
            } else {
                channel.flags |= channel_flag::CUSTOM;
                channel.level_restricted = false;
            }
        }
        channel
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn channel_id(&self) -> u32 {
        self.dbc_id
    }

    #[must_use]
    pub fn is_constant(&self) -> bool {
        self.dbc_id != 0
    }

    #[must_use]
    pub fn is_level_restricted(&self) -> bool {
        self.level_restricted
    }

    #[must_use]
    pub fn has_flag(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    #[must_use]
    pub fn flags(&self) -> u8 {
        self.flags
    }

    #[must_use]
    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    #[must_use]
    pub fn security_level(&self) -> u8 {
        self.security_level
    }

    pub fn set_security_level(&mut self, security_level: u8) {
        self.security_level = security_level;
    }

    pub fn set_announce(&mut self, announce: bool) {
        self.announce = announce;
    }

    #[must_use]
    pub fn is_on(&self, guid_low: u32) -> bool {
        self.players.contains_key(&guid_low)
    }

    #[must_use]
    pub fn owner_guid(&self) -> Option<u32> {
        self.owner_guid
    }

    fn is_banned(&self, guid_low: u32) -> bool {
        self.banned.contains(&guid_low)
    }

    fn player_flags(&self, guid_low: u32) -> u8 {
        self.players.get(&guid_low).copied().unwrap_or(0)
    }

    fn is_moderator(&self, guid_low: u32) -> bool {
        self.player_flags(guid_low) & member_flag::MODERATOR != 0
    }

    fn is_muted(&self, guid_low: u32) -> bool {
        self.player_flags(guid_low) & member_flag::MUTED != 0
    }

    fn is_owner(&self, guid_low: u32) -> bool {
        self.player_flags(guid_low) & member_flag::OWNER != 0
    }

    fn announce_gate(&self, security: u8, silently_gm_join: bool) -> bool {
        self.announce && (security < security::GAMEMASTER || !silently_gm_join)
    }

    /// `Channel::Join`. `guild_id` drives the
    /// `GuildRecruitment` block (exact `0x38` flag set).
    pub fn join(
        &mut self,
        actor: ChannelActor,
        guild_id: u32,
        password: &str,
        silently_gm_join: bool,
    ) -> Vec<ChannelOut> {
        let guid = actor.guid_low;
        if self.is_on(guid) {
            if self.is_constant() {
                return Vec::new();
            }
            return vec![self.one(guid, notify::PLAYER_ALREADY_MEMBER, &[guid])];
        }
        if self.is_banned(guid) {
            return vec![self.one(guid, notify::BANNED, &[])];
        }
        if !self.password.is_empty() && password != self.password {
            return vec![self.one(guid, notify::WRONG_PASSWORD, &[])];
        }
        if self.security_level != 0 && actor.security < self.security_level {
            return vec![self.one(guid, notify::WRONG_PASSWORD, &[])];
        }
        if guild_id != 0 && self.flags == GUILD_RECRUITMENT_FLAGS {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.announce_gate(actor.security, silently_gm_join) {
            out.push(self.all(notify::JOINED, &[guid], None));
        }
        self.players.insert(guid, 0);
        out.push(ChannelOut::ToOne {
            guid_low: guid,
            packet: self.you_joined_packet(),
        });
        if self.has_flag(channel_flag::CUSTOM) && !self.is_constant() && self.owner_guid.is_none() {
            let exclaim = self.players.len() > 1;
            out.extend(self.set_owner_internal(guid, exclaim, Some(actor)));
            if let Some(flags) = self.players.get_mut(&guid) {
                *flags |= member_flag::MODERATOR;
            }
        }
        out
    }

    /// `Channel::Leave(guid, send)`.
    pub fn leave(
        &mut self,
        actor: ChannelActor,
        send: bool,
        silently_gm_join: bool,
    ) -> Vec<ChannelOut> {
        let guid = actor.guid_low;
        if !self.is_on(guid) {
            if send {
                return vec![self.one(guid, notify::NOT_MEMBER, &[])];
            }
            return Vec::new();
        }
        let mut out = Vec::new();
        if send {
            out.push(ChannelOut::ToOne {
                guid_low: guid,
                packet: self.notify_packet(notify::YOU_LEFT, &[]),
            });
        }
        let change_owner = self.is_owner(guid);
        self.players.remove(&guid);
        if self.announce_gate(actor.security, silently_gm_join) {
            out.push(self.all(notify::LEFT, &[guid], None));
        }
        if change_owner {
            let new_owner = self.players.keys().next().copied();
            // C++ `SetOwner(ObjectGuid())` no-ops on the
            // missing player, leaving the old owner guid.
            if let Some(new_owner) = new_owner {
                out.extend(self.set_owner_internal(new_owner, true, None));
            }
        }
        out
    }

    /// `Channel::Leave(guid, false)` for logout cleanup;
    /// announces still fire in C++ (the announce gate is
    /// outside the `send` condition) with the departing
    /// player's security unknown — treated as a plain
    /// player, matching `GetPlayer` during teardown.
    pub fn leave_cleanup(&mut self, guid_low: u32) -> Vec<ChannelOut> {
        self.leave(
            ChannelActor {
                guid_low,
                security: security::PLAYER,
                team_id: 0,
            },
            false,
            false,
        )
    }

    /// `Channel::SetOwner(guid, exclaim)`. `new_owner` is
    /// the actor view of `guid_low` when the caller has
    /// it; C++ returns early when the player can't be
    /// found, and refuses non-GM owners on GENERAL-flag
    /// channels. Ownership transfer triggered internally
    /// (leave/kick) passes `None` and the owner must be a
    /// current member to take effect.
    fn set_owner_internal(
        &mut self,
        guid_low: u32,
        exclaim: bool,
        new_owner: Option<ChannelActor>,
    ) -> Vec<ChannelOut> {
        let security = if let Some(actor) = new_owner {
            actor.security
        } else {
            if !self.is_on(guid_low) {
                return Vec::new();
            }
            // Internal succession: C++ resolves the
            // member's session security here; members
            // are online by construction, and the
            // GENERAL gate below uses plain-player
            // security unless the actor supplied it.
            security::PLAYER
        };
        if self.has_flag(channel_flag::GENERAL) && security < security::GAMEMASTER {
            return Vec::new();
        }
        if let Some(old) = self.owner_guid
            && let Some(flags) = self.players.get_mut(&old)
        {
            *flags &= !member_flag::OWNER;
        }
        self.owner_guid = Some(guid_low);
        let old_flags = self.player_flags(guid_low);
        if let Some(flags) = self.players.get_mut(&guid_low) {
            *flags |= member_flag::OWNER;
        }
        let mut out = vec![self.all_packet(self.mode_change_packet(guid_low, old_flags), None)];
        if exclaim {
            out.push(self.all(notify::OWNER_CHANGED, &[guid_low], None));
        }
        out
    }

    /// Shared member+moderator gate (`KickOrBan`, `UnBan`,
    /// `Password`, `Announce`, `Moderate` openings).
    fn require_moderator(&self, actor: ChannelActor) -> Result<(), Vec<ChannelOut>> {
        if !self.is_on(actor.guid_low) {
            return Err(vec![self.one(actor.guid_low, notify::NOT_MEMBER, &[])]);
        }
        if !self.is_moderator(actor.guid_low) && actor.security < security::GAMEMASTER {
            return Err(vec![self.one(actor.guid_low, notify::NOT_MODERATOR, &[])]);
        }
        Ok(())
    }

    fn not_found(&self, guid_low: u32, target_name: &str) -> Vec<ChannelOut> {
        vec![ChannelOut::ToOne {
            guid_low,
            packet: self.notify_packet_str(notify::PLAYER_NOT_FOUND, target_name),
        }]
    }

    /// `Channel::Say` with `skipCheck = true`
    /// (bot trade-chat posts; membership not required).
    #[must_use]
    pub fn say_skip_check(&self, speaker_low: u32, text: &str) -> Vec<ChannelOut> {
        let packet = crate::chat::build_channel_chat_packet(
            text,
            crate::chat::language::UNIVERSAL,
            0,
            player_guid(speaker_low),
            &self.name,
            0,
        );
        vec![self.all_packet(packet, Some(speaker_low))]
    }

    /// `Channel::KickOrBan`. `target` is the actor-resolved
    /// online target (`sObjectMgr.GetPlayer(name)`).
    pub fn kick_or_ban(
        &mut self,
        actor: ChannelActor,
        target: Option<ChannelActor>,
        target_name: &str,
        ban: bool,
    ) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        let Some(target) = target.filter(|t| self.is_on(t.guid_low)) else {
            return self.not_found(actor.guid_low, target_name);
        };
        let change_owner = self.owner_guid == Some(target.guid_low);
        if actor.security < security::GAMEMASTER
            && change_owner
            && self.owner_guid != Some(actor.guid_low)
        {
            return vec![self.one(actor.guid_low, notify::NOT_OWNER, &[])];
        }
        let packet = if ban && !self.is_banned(target.guid_low) {
            self.banned.insert(target.guid_low);
            self.notify_packet(notify::PLAYER_BANNED, &[target.guid_low, actor.guid_low])
        } else {
            self.notify_packet(notify::PLAYER_KICKED, &[target.guid_low, actor.guid_low])
        };
        let mut out = vec![self.all_packet(packet, None)];
        self.players.remove(&target.guid_low);
        if change_owner {
            out.extend(self.set_owner_internal(actor.guid_low, true, Some(actor)));
        }
        out
    }

    /// `Channel::UnBan`.
    pub fn unban(
        &mut self,
        actor: ChannelActor,
        target: Option<ChannelActor>,
        target_name: &str,
    ) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        let Some(target) = target else {
            return self.not_found(actor.guid_low, target_name);
        };
        if !self.is_banned(target.guid_low) {
            return vec![ChannelOut::ToOne {
                guid_low: actor.guid_low,
                packet: self.notify_packet_str(notify::PLAYER_NOT_BANNED, target_name),
            }];
        }
        self.banned.remove(&target.guid_low);
        vec![self.all(
            notify::PLAYER_UNBANNED,
            &[target.guid_low, actor.guid_low],
            None,
        )]
    }

    /// `Channel::Password`.
    pub fn password(&mut self, actor: ChannelActor, password: &str) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        password.clone_into(&mut self.password);
        vec![self.all(notify::PASSWORD_CHANGED, &[actor.guid_low], None)]
    }

    /// `Channel::SetMode` (moderator/mute set/clear).
    pub fn set_mode(
        &mut self,
        actor: ChannelActor,
        target: Option<ChannelActor>,
        target_name: &str,
        moderator: bool,
        set: bool,
        allow_two_side_channel: bool,
    ) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        let Some(target) = target else {
            return self.not_found(actor.guid_low, target_name);
        };
        if moderator
            && Some(actor.guid_low) == self.owner_guid
            && Some(target.guid_low) == self.owner_guid
        {
            return Vec::new();
        }
        if !self.is_on(target.guid_low) {
            return self.not_found(actor.guid_low, target_name);
        }
        if (actor.security < security::GAMEMASTER || target.security < security::GAMEMASTER)
            && actor.team_id != target.team_id
            && !allow_two_side_channel
        {
            return self.not_found(actor.guid_low, target_name);
        }
        if self.owner_guid == Some(target.guid_low) && self.owner_guid != Some(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_OWNER, &[])];
        }
        if moderator {
            if self.has_flag(channel_flag::GENERAL) && target.security < security::GAMEMASTER {
                return Vec::new();
            }
            self.set_member_flag(target.guid_low, member_flag::MODERATOR, set)
        } else {
            self.set_member_flag(target.guid_low, member_flag::MUTED, set)
        }
    }

    /// `Channel::SetModerator`/`SetMute` (`Channel.h:268`):
    /// mode-change broadcast only on an actual change.
    fn set_member_flag(&mut self, guid_low: u32, flag: u8, set: bool) -> Vec<ChannelOut> {
        let old_flags = self.player_flags(guid_low);
        if (old_flags & flag != 0) == set {
            return Vec::new();
        }
        if let Some(flags) = self.players.get_mut(&guid_low) {
            if set {
                *flags |= flag;
            } else {
                *flags &= !flag;
            }
        }
        vec![self.all_packet(self.mode_change_packet(guid_low, old_flags), None)]
    }

    /// `Channel::SetOwner(guid, newname)`.
    pub fn set_owner_named(
        &mut self,
        actor: ChannelActor,
        target: Option<ChannelActor>,
        target_name: &str,
        allow_two_side_channel: bool,
    ) -> Vec<ChannelOut> {
        if !self.is_on(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_MEMBER, &[])];
        }
        if actor.security < security::GAMEMASTER && self.owner_guid != Some(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_OWNER, &[])];
        }
        let Some(target) = target.filter(|t| self.is_on(t.guid_low)) else {
            return self.not_found(actor.guid_low, target_name);
        };
        if target.team_id != actor.team_id && !allow_two_side_channel {
            return self.not_found(actor.guid_low, target_name);
        }
        if self.has_flag(channel_flag::GENERAL) && target.security < security::GAMEMASTER {
            return Vec::new();
        }
        if let Some(flags) = self.players.get_mut(&target.guid_low) {
            *flags |= member_flag::MODERATOR;
        }
        self.set_owner_internal(target.guid_low, true, Some(target))
    }

    /// `Channel::SendWhoOwner`. `owner_name` is the
    /// actor-resolved online name of the owner guid.
    #[must_use]
    pub fn send_who_owner(&self, actor: ChannelActor, owner_name: Option<&str>) -> Vec<ChannelOut> {
        if !self.is_on(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_MEMBER, &[])];
        }
        let name = if self.is_constant() || self.owner_guid.is_none() {
            "Nobody"
        } else {
            owner_name.unwrap_or("PLAYER_NOT_FOUND")
        };
        vec![ChannelOut::ToOne {
            guid_low: actor.guid_low,
            packet: self.notify_packet_str(notify::CHANNEL_OWNER, name),
        }]
    }

    /// `Channel::Announce` / `Channel::Moderate` toggles.
    pub fn toggle_announce(&mut self, actor: ChannelActor) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        self.announce = !self.announce;
        let ty = if self.announce {
            notify::ANNOUNCEMENTS_ON
        } else {
            notify::ANNOUNCEMENTS_OFF
        };
        vec![self.all(ty, &[actor.guid_low], None)]
    }

    /// `Channel::Moderate`.
    pub fn toggle_moderate(&mut self, actor: ChannelActor) -> Vec<ChannelOut> {
        if let Err(out) = self.require_moderator(actor) {
            return out;
        }
        self.moderate = !self.moderate;
        let ty = if self.moderate {
            notify::MODERATION_ON
        } else {
            notify::MODERATION_OFF
        };
        vec![self.all(ty, &[actor.guid_low], None)]
    }

    /// `Channel::List`: member enumeration filtered by
    /// `visible` (online + who-list security gate,
    /// resolved by the actor).
    pub fn list(&self, actor: ChannelActor, visible: &dyn Fn(u32) -> bool) -> Vec<ChannelOut> {
        if !self.is_on(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_MEMBER, &[])];
        }
        let mut writer = PacketWriter::new();
        writer.write_cstring(&self.name);
        writer.write_u8(self.flags);
        let members: Vec<(u32, u8)> = self
            .players
            .iter()
            .filter(|(guid, _)| visible(**guid))
            .map(|(guid, flags)| (*guid, *flags))
            .collect();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        writer.write_u32_le(members.len() as u32);
        for (guid, flags) in members {
            writer.write_u64_le(player_guid(guid).raw());
            writer.write_u8(flags);
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_LIST);
        packet.take_writer(writer);
        vec![ChannelOut::ToOne {
            guid_low: actor.guid_low,
            packet,
        }]
    }

    /// `Channel::Say(guid, text, lang)` (skipCheck=false
    /// path). `honor_rank` is 0 until the honor system
    /// lands.
    #[must_use]
    pub fn say(
        &self,
        actor: ChannelActor,
        text: &str,
        lang: u32,
        chat_tag: u8,
        honor_rank: u8,
        allow_two_side_channel: bool,
    ) -> Vec<ChannelOut> {
        let guid = actor.guid_low;
        if !self.is_on(guid) {
            return vec![self.one(guid, notify::NOT_MEMBER, &[])];
        }
        if self.is_muted(guid) || (self.dbc_id == CHANNEL_ID_WORLD_DEFENSE && honor_rank < 15) {
            return vec![self.one(guid, notify::MUTED, &[])];
        }
        if self.moderate && !self.is_moderator(guid) && actor.security < security::GAMEMASTER {
            return vec![self.one(guid, notify::NOT_MODERATOR, &[])];
        }
        let lang = if allow_two_side_channel {
            crate::chat::language::UNIVERSAL
        } else {
            lang
        };
        let packet = crate::chat::build_channel_chat_packet(
            text,
            lang,
            chat_tag,
            player_guid(guid),
            &self.name,
            u32::from(honor_rank),
        );
        let ignore_source = if self.is_moderator(guid) {
            None
        } else {
            Some(guid)
        };
        vec![self.all_packet(packet, ignore_source)]
    }

    /// `Channel::Invite`. `target_ignores_actor` mirrors
    /// `GetSocial()->HasIgnore`.
    #[must_use]
    pub fn invite(
        &self,
        actor: ChannelActor,
        target: Option<ChannelActor>,
        target_name: &str,
        allow_two_side_channel: bool,
        target_ignores_actor: bool,
    ) -> Vec<ChannelOut> {
        if !self.is_on(actor.guid_low) {
            return vec![self.one(actor.guid_low, notify::NOT_MEMBER, &[])];
        }
        let Some(target) = target else {
            return self.not_found(actor.guid_low, target_name);
        };
        if self.is_on(target.guid_low) {
            return vec![self.one(
                actor.guid_low,
                notify::PLAYER_ALREADY_MEMBER,
                &[target.guid_low],
            )];
        }
        if self.is_banned(target.guid_low) {
            return vec![ChannelOut::ToOne {
                guid_low: actor.guid_low,
                packet: self.notify_packet_str(notify::PLAYER_INVITE_BANNED, target_name),
            }];
        }
        if target.team_id != actor.team_id && !allow_two_side_channel {
            return vec![self.one(actor.guid_low, notify::INVITE_WRONG_FACTION, &[])];
        }
        let mut out = Vec::new();
        if !target_ignores_actor {
            out.push(self.one(target.guid_low, notify::INVITE, &[actor.guid_low]));
        }
        out.push(ChannelOut::ToOne {
            guid_low: actor.guid_low,
            packet: self.notify_packet_str(notify::PLAYER_INVITED, target_name),
        });
        out
    }

    fn one(&self, guid_low: u32, notify_type: u8, guids: &[u32]) -> ChannelOut {
        ChannelOut::ToOne {
            guid_low,
            packet: self.notify_packet(notify_type, guids),
        }
    }

    fn all(&self, notify_type: u8, guids: &[u32], ignore_source: Option<u32>) -> ChannelOut {
        ChannelOut::ToAll {
            recipients: self.players.keys().copied().collect(),
            packet: self.notify_packet(notify_type, guids),
            ignore_source,
        }
    }

    fn all_packet(&self, packet: WorldPacket, ignore_source: Option<u32>) -> ChannelOut {
        ChannelOut::ToAll {
            recipients: self.players.keys().copied().collect(),
            packet,
            ignore_source,
        }
    }

    /// `MakeYouJoined` (0x02): notify header + flags +
    /// zero instance number.
    fn you_joined_packet(&self) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(notify::YOU_JOINED);
        writer.write_cstring(&self.name);
        writer.write_u32_le(u32::from(self.flags));
        writer.write_u32_le(0);
        let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
        packet.take_writer(writer);
        packet
    }

    /// `MakeModeChange` (0x0C).
    fn mode_change_packet(&self, guid_low: u32, old_flags: u8) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(notify::MODE_CHANGE);
        writer.write_cstring(&self.name);
        writer.write_u64_le(player_guid(guid_low).raw());
        writer.write_u8(old_flags);
        writer.write_u8(self.player_flags(guid_low));
        let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
        packet.take_writer(writer);
        packet
    }

    /// `MakeNotifyPacket` + trailing guids.
    fn notify_packet(&self, notify_type: u8, guids: &[u32]) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(notify_type);
        writer.write_cstring(&self.name);
        for guid in guids {
            writer.write_u64_le(player_guid(*guid).raw());
        }
        let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
        packet.take_writer(writer);
        packet
    }

    /// `MakeNotifyPacket` + trailing name string.
    fn notify_packet_str(&self, notify_type: u8, name: &str) -> WorldPacket {
        let mut writer = PacketWriter::new();
        writer.write_u8(notify_type);
        writer.write_cstring(&self.name);
        writer.write_cstring(name);
        let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
        packet.take_writer(writer);
        packet
    }
}

/// `Channel::MakeNotOnPacket` for a channel that does
/// not exist (`GetChannel` with `pkt=true`).
#[must_use]
pub fn not_on_packet(name: &str) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(notify::NOT_MEMBER);
    writer.write_cstring(name);
    let mut packet = WorldPacket::new(opcode_id::SMSG_CHANNEL_NOTIFY);
    packet.take_writer(writer);
    packet
}

/// `ChannelMgr` (`ChannelMgr.cpp:52-129`): per-faction
/// channel map keyed by lowercased name.
#[derive(Clone, Debug, Default)]
pub struct ChannelMgr {
    channels: HashMap<String, Channel>,
}

impl ChannelMgr {
    /// Constructor + `CreateDefaultChannels`: the fork's
    /// staff monitoring channels, announce off.
    #[must_use]
    pub fn new_with_defaults(dbc: &ChatChannelDbc) -> Self {
        let mut mgr = Self::default();
        for (name, level) in [
            ("Warden", security::GAMEMASTER),
            ("Anticrash", security::GAMEMASTER),
            ("Antiflood", security::GAMEMASTER),
            ("ItemsCheck", security::GAMEMASTER),
            ("GoldDupe", security::GAMEMASTER),
            ("SAC", security::GAMEMASTER),
            ("MailsAC", security::GAMEMASTER),
            ("BotsDetector", security::GAMEMASTER),
            ("ChatSpam", security::MODERATOR),
            ("LowLevelBots", security::GAMEMASTER),
        ] {
            mgr.get_join_channel(name, dbc).set_security_level(level);
        }
        for channel in mgr.channels.values_mut() {
            channel.set_announce(false);
        }
        mgr
    }

    fn key(name: &str) -> String {
        name.to_lowercase()
    }

    /// `GetJoinChannel`: find or create.
    pub fn get_join_channel(&mut self, name: &str, dbc: &ChatChannelDbc) -> &mut Channel {
        self.channels
            .entry(Self::key(name))
            .or_insert_with(|| Channel::new(name, dbc))
    }

    /// `GetChannel` lookup (notify behavior lives in the
    /// actor).
    pub fn get_channel(&mut self, name: &str) -> Option<&mut Channel> {
        self.channels.get_mut(&Self::key(name))
    }

    /// Shared `GetChannel` lookup.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(&Self::key(name))
    }

    #[must_use]
    pub fn has_channel(&self, name: &str) -> bool {
        self.channels.contains_key(&Self::key(name))
    }

    /// `LeftChannel`: erase empty non-constant channels
    /// without a security level.
    pub fn left_channel(&mut self, name: &str) {
        let key = Self::key(name);
        let erase = self.channels.get(&key).is_some_and(|channel| {
            channel.num_players() == 0 && !channel.is_constant() && channel.security_level() == 0
        });
        if erase {
            self.channels.remove(&key);
        }
    }

    /// `Player::CleanupChannels` for one departing
    /// player: silent leave everywhere, then sweep.
    pub fn cleanup_player(&mut self, guid_low: u32) -> Vec<ChannelOut> {
        let names: Vec<String> = self
            .channels
            .values()
            .filter(|channel| channel.is_on(guid_low))
            .map(|channel| channel.name().to_owned())
            .collect();
        let mut out = Vec::new();
        for name in names {
            if let Some(channel) = self.get_channel(&name) {
                out.extend(channel.leave_cleanup(guid_low));
            }
            self.left_channel(&name);
        }
        out
    }

    /// Member guid lows of a channel (delivery + tests).
    #[must_use]
    pub fn member_guids(&self, name: &str) -> Vec<u32> {
        self.channels
            .get(&Self::key(name))
            .map(|channel| channel.players.keys().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangos_world_data::chat_channels::ChatChannelEntry;

    fn dbc() -> ChatChannelDbc {
        ChatChannelDbc {
            entries: vec![
                ChatChannelEntry {
                    channel_id: 1,
                    flags: 0x3,
                    patterns: vec!["General - %s".into()],
                },
                ChatChannelEntry {
                    channel_id: 23,
                    flags: 0x1_0004,
                    patterns: vec!["WorldDefense".into()],
                },
            ],
        }
    }

    fn actor(guid_low: u32) -> ChannelActor {
        ChannelActor {
            guid_low,
            security: security::PLAYER,
            team_id: 67,
        }
    }

    fn types(outs: &[ChannelOut]) -> Vec<u8> {
        outs.iter()
            .map(|out| match out {
                ChannelOut::ToOne { packet, .. } | ChannelOut::ToAll { packet, .. } => {
                    packet.body().first().copied().unwrap_or(0xFF)
                }
            })
            .collect()
    }

    #[test]
    fn custom_channel_lifecycle() {
        let dbc = dbc();
        let mut mgr = ChannelMgr::default();
        let channel = mgr.get_join_channel("mychan", &dbc);
        assert_eq!(channel.name(), "Mychan");
        assert_eq!(channel.flags(), channel_flag::CUSTOM);

        // First joiner: JOINED broadcast to the empty
        // member set, YouJoined, then the owner mode
        // change; becomes owner + moderator.
        let outs = channel.join(actor(7), 0, "", false);
        assert_eq!(
            types(&outs),
            [notify::JOINED, notify::YOU_JOINED, notify::MODE_CHANGE]
        );
        let ChannelOut::ToAll { recipients, .. } = &outs[0] else {
            panic!("joined must broadcast");
        };
        assert!(recipients.is_empty());
        assert_eq!(channel.owner_guid(), Some(7));
        assert!(channel.is_moderator(7));

        // Second joiner: JOINED broadcast to the existing
        // member snapshot, then YouJoined.
        let outs = channel.join(actor(8), 0, "", false);
        assert_eq!(types(&outs), [notify::JOINED, notify::YOU_JOINED]);
        let ChannelOut::ToAll { recipients, .. } = &outs[0] else {
            panic!("joined must broadcast");
        };
        assert_eq!(recipients, &[7]);

        // Say: both members, sender as ignore source
        // (non-moderator path uses moderator=8? 8 is not
        // moderator; 7 is, so use 8 to exercise it).
        let outs = channel.say(actor(8), "hello", 7, 0, 0, false);
        let ChannelOut::ToAll {
            recipients,
            ignore_source,
            ..
        } = &outs[0]
        else {
            panic!("say must broadcast");
        };
        assert_eq!(recipients, &[7, 8]);
        assert_eq!(*ignore_source, Some(8));

        // Owner mutes 8; 8's say returns MUTED to self.
        let outs = channel.set_mode(actor(7), Some(actor(8)), "Bee", false, true, false);
        assert_eq!(types(&outs), [notify::MODE_CHANGE]);
        let outs = channel.say(actor(8), "hello?", 7, 0, 0, false);
        assert_eq!(types(&outs), [notify::MUTED]);

        // Kick broadcasts before the erase (target still
        // in the recipient snapshot).
        let outs = channel.kick_or_ban(actor(7), Some(actor(8)), "Bee", false);
        assert_eq!(types(&outs), [notify::PLAYER_KICKED]);
        let ChannelOut::ToAll { recipients, .. } = &outs[0] else {
            panic!("kick must broadcast");
        };
        assert_eq!(recipients, &[7, 8]);
        assert!(!channel.is_on(8));
    }

    #[test]
    fn builtin_channel_is_constant_and_silent() {
        let dbc = dbc();
        let mut mgr = ChannelMgr::default();
        let channel = mgr.get_join_channel("General - Elwynn Forest", &dbc);
        assert!(channel.is_constant());
        assert_eq!(
            channel.flags(),
            channel_flag::GENERAL | channel_flag::NOT_LFG
        );
        let outs = channel.join(actor(7), 0, "", false);
        assert_eq!(types(&outs), [notify::YOU_JOINED]);
        // No JOINED announce for built-in channels.
        let outs = channel.join(actor(8), 0, "", false);
        assert_eq!(types(&outs), [notify::YOU_JOINED]);
        assert_eq!(channel.owner_guid(), None);

        // WorldDefense mutes honor rank < 15.
        let channel = mgr.get_join_channel("WorldDefense", &dbc);
        channel.join(actor(7), 0, "", false);
        let outs = channel.say(actor(7), "defend!", 0, 0, 0, false);
        assert_eq!(types(&outs), [notify::MUTED]);
    }

    #[test]
    fn password_and_cleanup() {
        let dbc = dbc();
        let mut mgr = ChannelMgr::default();
        let channel = mgr.get_join_channel("sekrit", &dbc);
        channel.join(actor(7), 0, "", false);
        let outs = channel.password(actor(7), "hunter2");
        assert_eq!(types(&outs), [notify::PASSWORD_CHANGED]);
        let outs = channel.join(actor(8), 0, "wrong", false);
        assert_eq!(types(&outs), [notify::WRONG_PASSWORD]);
        let outs = channel.join(actor(8), 0, "hunter2", false);
        assert_eq!(types(&outs), [notify::JOINED, notify::YOU_JOINED]);

        // Cleanup erases the emptied custom channel but
        // keeps built-ins and security channels.
        mgr.cleanup_player(7);
        mgr.cleanup_player(8);
        assert!(!mgr.has_channel("Sekrit"));
        let mgr = ChannelMgr::new_with_defaults(&dbc);
        assert!(mgr.has_channel("Warden"));
    }
}
