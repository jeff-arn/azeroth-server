//! Player mail (`MailHandler.cpp`, `MasterPlayer`
//! mail tracking): the pure pieces the world actor
//! drives over the per-player `DeferredLoad` mail
//! rows — constants, result/list packets, the
//! unread/next-delivery tracker, and the account
//! mail-spam ledger. Persistence leaves as
//! [`MailWrite`] intents plus synchronous store calls
//! where a verdict must precede the reply.
//!
//! Item attachments move as DB rows until the
//! runtime Item system lands (Phase 6), following
//! the petition charter pattern; the store check is
//! a first-free-backpack-slot verdict, so stack
//! merging and bag placement join with that phase.
//! Mail-template item generation (quest reward
//! mails) is not ported: template mails without
//! generated items show empty.

use std::collections::HashMap;

use mangos_db::character_load::MailRow;
use mangos_db::mail::MailUpdate;
use mangos_guid::{HighGuid, ObjectGuid};
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::opcode_table::opcode_id;
use mangos_protocol::world::WorldPacket;

/// Flat postage (`reqmoney = money + 30`).
pub const POSTAGE: u32 = 30;
/// Send-cap comparison (`mailsCount > 100`).
pub const MAILBOX_CAP: u8 = 100;
/// Client subject limit (drop when exceeded).
pub const SUBJECT_MAX_BYTES: usize = 64;
/// Client body limit (drop when exceeded).
pub const BODY_MAX_BYTES: usize = 500;
/// Client COD cap (anticheat drop above).
pub const COD_MAX: u32 = 100_000_000;
const DAY_SECS: i64 = 86_400;

/// `MailMessageType` (`Mail.h:54`).
pub mod message_type {
    pub const NORMAL: u8 = 0;
    pub const AUCTION: u8 = 2;
    pub const CREATURE: u8 = 3;
    pub const GAMEOBJECT: u8 = 4;
    pub const ITEM: u8 = 5;
}

/// `MailCheckMask` (`Mail.h:65`).
pub mod check_mask {
    pub const NONE: u32 = 0x00;
    pub const READ: u32 = 0x01;
    pub const RETURNED: u32 = 0x02;
    pub const COPIED: u32 = 0x04;
    pub const COD_PAYMENT: u32 = 0x08;
    pub const HAS_BODY: u32 = 0x10;
}

/// `MailStationery` (`Mail.h:78`).
pub mod stationery {
    pub const DEFAULT: u8 = 41;
}

/// `MailResponseType` (`SharedDefines.h:1706`).
pub mod response_type {
    pub const SEND: u32 = 0;
    pub const MONEY_TAKEN: u32 = 1;
    pub const ITEM_TAKEN: u32 = 2;
    pub const RETURNED_TO_SENDER: u32 = 3;
    pub const DELETED: u32 = 4;
    pub const MADE_PERMANENT: u32 = 5;
}

/// `MailResponseResult` (`SharedDefines.h:1716`).
pub mod response_result {
    pub const OK: u32 = 0;
    pub const EQUIP_ERROR: u32 = 1;
    pub const CANNOT_SEND_TO_SELF: u32 = 2;
    pub const NOT_ENOUGH_MONEY: u32 = 3;
    pub const RECIPIENT_NOT_FOUND: u32 = 4;
    pub const NOT_YOUR_TEAM: u32 = 5;
    pub const INTERNAL_ERROR: u32 = 6;
    pub const DISABLED_FOR_TRIAL_ACC: u32 = 14;
    pub const RECIPIENT_CAP_REACHED: u32 = 15;
}

/// Persistence intents emitted by the handlers
/// (each mirrors one `SaveMails`/handler statement
/// set; the fork batches them to logout, this port
/// writes per change like the social/group/guild
/// slices).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MailWrite {
    /// The `SaveMails` changed-mail UPDATE.
    Update(MailUpdate),
    /// The `SaveMails` deleted-mail block.
    DeleteFull {
        mail_id: u32,
        item_lows: Vec<u32>,
        item_text_id: u32,
        stationery: u8,
    },
    /// Return-to-sender pre-delete (rows only).
    DeleteRows { mail_id: u32 },
    /// One removed attachment's `mail_items` row.
    DeleteMailItem { item_guid: u32 },
    /// Sender inventory row out + instance re-owned.
    MoveItemToMail {
        owner_low: u32,
        item_low: u32,
        new_owner_low: u32,
    },
    /// Instance owner rewrite (returns).
    SetItemOwner { item_low: u32, new_owner_low: u32 },
    /// Destroy a dropped return item.
    DeleteItemInstance { item_low: u32 },
}

/// The `SaveMails` update for a live mail row.
#[must_use]
pub fn mail_update(mail: &MailRow) -> MailUpdate {
    MailUpdate {
        mail_id: mail.id,
        item_text_id: mail.item_text_id,
        has_items: mail.has_items != 0,
        expire_time: mail.expire_time,
        deliver_time: mail.deliver_time,
        money: mail.money.max(0) as u32,
        cod: mail.cod.max(0) as u32,
        checked: u32::from(mail.checked),
    }
}

/// Delivery + expiry times for a fresh mail
/// (`SendMailTo` defaults: COD expires in 3 days,
/// everything else in 30).
#[must_use]
pub fn mail_times(now: i64, cod: u32, deliver_delay_secs: u32) -> (i64, i64) {
    let deliver = now + i64::from(deliver_delay_secs);
    let expire = deliver + if cod > 0 { 3 * DAY_SECS } else { 30 * DAY_SECS };
    (deliver, expire)
}

/// Whether a mail shows in the mailbox list (and can
/// be acted on): delivered and not expired.
#[must_use]
pub fn is_visible(mail: &MailRow, now: i64) -> bool {
    mail.deliver_time <= now && now <= mail.expire_time
}

/// Unread/next-delivery tracking (`unReadMails`,
/// `m_nextMailDelivereTime`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MailTracker {
    pub unread: u32,
    /// 0 = nothing pending.
    pub next_deliver: i64,
}

impl MailTracker {
    /// `UpdateNextMailTimeAndUnreads`.
    #[must_use]
    pub fn recompute(mails: &[MailRow], now: i64) -> Self {
        let mut tracker = Self::default();
        for mail in mails {
            if mail.deliver_time > now {
                if tracker.next_deliver == 0 || tracker.next_deliver > mail.deliver_time {
                    tracker.next_deliver = mail.deliver_time;
                }
            } else if mail.checked & (check_mask::READ as u8) == 0 && now < mail.expire_time {
                tracker.unread += 1;
            }
        }
        tracker
    }

    /// `AddNewMailDeliverTime`; `true` = the mail is
    /// ready now and `SMSG_RECEIVED_MAIL` goes out.
    pub fn note_new_mail(&mut self, deliver_time: i64, now: i64) -> bool {
        if deliver_time <= now {
            self.unread += 1;
            true
        } else {
            if self.next_deliver == 0 || self.next_deliver > deliver_time {
                self.next_deliver = deliver_time;
            }
            false
        }
    }

    /// The `MasterPlayer::Update` due check; `true`
    /// fires `SMSG_RECEIVED_MAIL` once.
    pub fn poll_delivery(&mut self, now: i64) -> bool {
        if self.next_deliver != 0 && self.next_deliver <= now {
            self.unread += 1;
            self.next_deliver = 0;
            true
        } else {
            false
        }
    }

    /// `DecreaseUnreadMailsCount`.
    pub fn decrease_unread(&mut self) {
        self.unread = self.unread.saturating_sub(1);
    }

    /// `HasUnreadMail`.
    #[must_use]
    pub fn has_unread(&self) -> bool {
        self.unread > 0
    }
}

/// `AccountPersistentData` mail-spam ledger: first
/// mail time per (sender account, target account).
#[derive(Clone, Debug, Default)]
pub struct MailSpamLedger {
    sent: HashMap<u32, HashMap<u32, i64>>,
}

impl MailSpamLedger {
    /// `CanMail`: repeat targets always pass; new
    /// targets pass while the count of unexpired
    /// first-mail targets is under the cap.
    #[must_use]
    pub fn can_mail(
        &self,
        sender_account: u32,
        target_account: u32,
        now: i64,
        expire_secs: u32,
        max_mails: u32,
    ) -> bool {
        let Some(targets) = self.sent.get(&sender_account) else {
            return 0 < max_mails;
        };
        if targets.contains_key(&target_account) {
            return true;
        }
        let last_non_expired = now - i64::from(expire_secs);
        let score = targets
            .values()
            .filter(|time| **time >= last_non_expired)
            .count() as u32;
        score < max_mails
    }

    /// `JustMailed`: remember the first-mail time.
    pub fn just_mailed(&mut self, sender_account: u32, target_account: u32, now: i64) {
        self.sent
            .entry(sender_account)
            .or_default()
            .entry(target_account)
            .or_insert(now);
    }
}

/// Item block data for one mail-list row.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MailListItem {
    pub entry: u32,
    pub perm_enchant: u32,
    pub random_property_id: i32,
    pub count: u8,
    pub charges: u32,
    pub max_durability: u32,
    pub durability: u32,
}

/// First enchantment id (permanent slot) from the
/// `item_instance.enchantments` text.
#[must_use]
pub fn perm_enchant_id(enchantments: &str) -> u32 {
    enchantments
        .split_whitespace()
        .next()
        .and_then(|word| word.parse::<u32>().ok())
        .unwrap_or(0)
}

/// `Item::GetSpellCharges` (first charge counter).
#[must_use]
pub fn spell_charges(charges: &str) -> u32 {
    charges
        .split_whitespace()
        .next()
        .and_then(|word| word.parse::<i32>().ok())
        .map_or(0, |value| value as u32)
}

/// `SMSG_SEND_MAIL_RESULT`.
#[must_use]
pub fn mail_result_packet(
    mail_id: u32,
    action: u32,
    error: u32,
    equip_error: u32,
    item_id: u32,
    item_count: u32,
) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(mail_id);
    writer.write_u32_le(action);
    writer.write_u32_le(error);
    if error == response_result::EQUIP_ERROR {
        writer.write_u32_le(equip_error);
    } else if action == response_type::ITEM_TAKEN {
        writer.write_u32_le(item_id);
        writer.write_u32_le(item_count);
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_SEND_MAIL_RESULT);
    packet.take_writer(writer);
    packet
}

/// `SMSG_RECEIVED_MAIL` (`SendNewMail`).
#[must_use]
pub fn received_mail_packet() -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u32_le(0);
    let mut packet = WorldPacket::new(opcode_id::SMSG_RECEIVED_MAIL);
    packet.take_writer(writer);
    packet
}

/// `MSG_QUERY_NEXT_MAIL_TIME` answer.
#[must_use]
pub fn next_mail_time_packet(has_unread: bool) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_f32_le(if has_unread { 0.0 } else { -86_400.0 });
    let mut packet = WorldPacket::new(opcode_id::MSG_QUERY_NEXT_MAIL_TIME);
    packet.take_writer(writer);
    packet
}

/// `SMSG_MAIL_LIST_RESULT` over already-filtered
/// (visible, newest-first) mails with their item
/// blocks (`None` = the all-zero item block).
#[must_use]
#[allow(clippy::cast_precision_loss)] // mirrors the C++ float math
pub fn mail_list_packet(mails: &[(&MailRow, Option<MailListItem>)], now: i64) -> WorldPacket {
    let mut writer = PacketWriter::new();
    writer.write_u8(0);
    let mut count = 0u32;
    for (mail, item) in mails {
        if count >= 254 {
            break;
        }
        writer.write_u32_le(mail.id);
        writer.write_u8(mail.message_type);
        match mail.message_type {
            message_type::NORMAL => {
                writer.write_u64_le(ObjectGuid::new(HighGuid::PLAYER, mail.sender_guid).raw());
            }
            message_type::CREATURE | message_type::GAMEOBJECT | message_type::AUCTION => {
                writer.write_u32_le(mail.sender_guid);
            }
            _ => {}
        }
        writer.write_cstring(&mail.subject);
        writer.write_u32_le(mail.item_text_id);
        writer.write_u32_le(0);
        writer.write_u32_le(u32::from(mail.stationery));
        let item = item.unwrap_or_default();
        writer.write_u32_le(item.entry);
        writer.write_u32_le(item.perm_enchant);
        writer.write_u32_le(item.random_property_id as u32);
        writer.write_u32_le(0); // suffix factor (see module note)
        writer.write_u8(item.count);
        writer.write_u32_le(item.charges);
        writer.write_u32_le(item.max_durability);
        writer.write_u32_le(item.durability);
        writer.write_u32_le(mail.money.max(0) as u32);
        writer.write_u32_le(mail.cod.max(0) as u32);
        writer.write_u32_le(u32::from(mail.checked));
        writer.write_f32_le((mail.expire_time - now) as f32 / DAY_SECS as f32);
        writer.write_u32_le(mail.mail_template_id);
        count += 1;
    }
    let mut packet = WorldPacket::new(opcode_id::SMSG_MAIL_LIST_RESULT);
    packet.take_writer(writer);
    if let Some(first) = packet.body_mut().first_mut() {
        *first = count as u8;
    }
    packet
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn mail(id: u32, deliver: i64, expire: i64, checked: u8) -> MailRow {
        MailRow {
            id,
            message_type: 0,
            sender_guid: 9,
            receiver_guid: 7,
            subject: "hi".into(),
            item_text_id: 0,
            expire_time: expire,
            deliver_time: deliver,
            money: 0,
            cod: 0,
            checked,
            stationery: 41,
            mail_template_id: 0,
            has_items: 0,
        }
    }

    #[test]
    fn tracker_recompute_and_poll() {
        let mails = vec![
            mail(1, 50, 500, 0),  // delivered unread
            mail(2, 50, 500, 1),  // delivered read
            mail(3, 300, 900, 0), // pending
            mail(4, 200, 900, 0), // pending sooner
            mail(5, 10, 90, 0),   // already expired
        ];
        let mut tracker = MailTracker::recompute(&mails, 100);
        assert_eq!(tracker.unread, 1);
        assert_eq!(tracker.next_deliver, 200);
        assert!(!tracker.poll_delivery(199));
        assert!(tracker.poll_delivery(200));
        assert_eq!(tracker.unread, 2);
        assert_eq!(tracker.next_deliver, 0);
        assert!(!tracker.poll_delivery(300));
        assert!(!tracker.note_new_mail(250, 240));
        assert_eq!(tracker.next_deliver, 250);
        assert!(tracker.note_new_mail(240, 240));
        assert_eq!(tracker.unread, 3);
        tracker.decrease_unread();
        assert_eq!(tracker.unread, 2);
    }

    #[test]
    fn times_and_visibility() {
        assert_eq!(mail_times(1000, 0, 3600), (4600, 4600 + 30 * 86_400));
        assert_eq!(mail_times(1000, 5, 0), (1000, 1000 + 3 * 86_400));
        assert!(is_visible(&mail(1, 100, 200, 0), 100));
        assert!(is_visible(&mail(1, 100, 200, 0), 200));
        assert!(!is_visible(&mail(1, 100, 200, 0), 99));
        assert!(!is_visible(&mail(1, 100, 200, 0), 201));
    }

    #[test]
    fn spam_ledger_defaults() {
        let mut ledger = MailSpamLedger::default();
        // Defaults: expire 0 (only same-second counts), max 2.
        assert!(ledger.can_mail(1, 2, 100, 0, 2));
        ledger.just_mailed(1, 2, 100);
        assert!(ledger.can_mail(1, 2, 100, 0, 2)); // repeat target
        assert!(ledger.can_mail(1, 3, 100, 0, 2));
        ledger.just_mailed(1, 3, 100);
        assert!(!ledger.can_mail(1, 4, 100, 0, 2)); // two at t=100
        assert!(ledger.can_mail(1, 4, 101, 0, 2)); // expired a second later
    }

    #[test]
    fn result_packet_tails() {
        let ok = mail_result_packet(7, response_type::SEND, response_result::OK, 0, 0, 0);
        assert_eq!(ok.body().len(), 12);
        let eq = mail_result_packet(
            7,
            response_type::ITEM_TAKEN,
            response_result::EQUIP_ERROR,
            50,
            0,
            0,
        );
        assert_eq!(eq.body().len(), 16);
        assert_eq!(&eq.body()[12..], &50u32.to_le_bytes());
        let taken = mail_result_packet(7, response_type::ITEM_TAKEN, response_result::OK, 0, 25, 3);
        assert_eq!(taken.body().len(), 20);
        assert_eq!(&taken.body()[12..16], &25u32.to_le_bytes());
        assert_eq!(&taken.body()[16..], &3u32.to_le_bytes());
    }

    #[test]
    fn list_packet_shape() {
        let m = mail(3, 10, 100 + 86_400, 0x10);
        let item = MailListItem {
            entry: 25,
            perm_enchant: 7,
            random_property_id: -3,
            count: 2,
            charges: 1,
            max_durability: 40,
            durability: 39,
        };
        let packet = mail_list_packet(&[(&m, Some(item))], 100);
        let body = packet.body();
        assert_eq!(body[0], 1);
        assert_eq!(u32::from_le_bytes(body[1..5].try_into().unwrap()), 3);
        assert_eq!(body[5], 0); // NORMAL
        // sender player guid full 8 bytes at 6..14
        assert_eq!(u64::from_le_bytes(body[6..14].try_into().unwrap()), 9);
        assert_eq!(&body[14..17], b"hi\0");
        // expiry float == 1.0 day
        let tail = &body[body.len() - 8..];
        assert_eq!(
            f32::from_le_bytes(tail[..4].try_into().unwrap()).to_bits(),
            1.0f32.to_bits()
        );
    }
}
