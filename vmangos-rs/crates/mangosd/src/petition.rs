//! Guild charter petitions (`GuildMgr` petition map +
//! `PetitionsHandler.cpp`): the pure petition state
//! machine the world actor drives. All persistence
//! leaves as [`PetitionWrite`] intents; charter item
//! rows are created/destroyed through synchronous
//! store calls in the actor because the buy handler
//! needs the free-slot verdict before it charges.
//!
//! Faithful quirks kept: `IsComplete` compares with
//! `==` against `MinPetitionSigns`, and
//! `RemovePetitionsAndSigns` deletes the owned
//! petition's database row while leaving the
//! in-memory petition object registered (only other
//! players' signatures are swept from memory).

use std::collections::BTreeMap;

use mangos_db::petition::LoadedPetitions;
use mangos_guid::{HighGuid, ObjectGuid};

/// `GUILD_CHARTER_COST` (`PetitionsHandler.cpp:39`).
pub const CHARTER_COST: u32 = 1000;
/// `CHARTER_DISPLAY_ID` (`PetitionsHandler.cpp:40`).
pub const CHARTER_DISPLAY_ID: u32 = 16161;
/// `MAX_CHARTER_NAME` (`ObjectMgr.h:398`).
pub const CHARTER_NAME_MAX_CHARS: usize = 24;
/// Client hard limit checked in the sign handler.
pub const SIGNATURES_HARD_LIMIT: usize = 9;

/// `PetitionSigns` result codes (`Guild.h:146-154`).
pub mod sign_result {
    pub const OK: u32 = 0;
    pub const ALREADY_SIGNED: u32 = 1;
    pub const ALREADY_IN_GUILD: u32 = 2;
    pub const CANT_SIGN_OWN: u32 = 3;
    pub const NEED_MORE: u32 = 4;
}

/// One signature (`PetitionSignature`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PetitionSignature {
    pub player_low: u32,
    pub account_id: u32,
}

/// One live petition (`Petition`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Petition {
    pub id: u32,
    pub owner_low: u32,
    pub charter_low: u32,
    pub name: String,
    /// Owner team (`m_team == HORDE`).
    pub horde: bool,
    pub signatures: Vec<PetitionSignature>,
}

impl Petition {
    /// Full owner guid.
    #[must_use]
    pub fn owner_guid(&self) -> ObjectGuid {
        ObjectGuid::new(HighGuid::PLAYER, self.owner_low)
    }

    /// Full charter item guid.
    #[must_use]
    pub fn charter_guid(&self) -> ObjectGuid {
        ObjectGuid::new(HighGuid::ITEM, self.charter_low)
    }

    /// `GetSignatureCount`.
    #[must_use]
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }

    /// `IsComplete`: signature count equals the
    /// configured minimum (C++ uses `==`).
    #[must_use]
    pub fn is_complete(&self, min_signatures: u32) -> bool {
        self.signatures.len() == min_signatures as usize
    }

    /// `GetSignatureForPlayer`: account match first,
    /// then character match.
    #[must_use]
    pub fn signature_for_player(&self, player_low: u32, account_id: u32) -> bool {
        self.signatures
            .iter()
            .any(|sign| sign.account_id == account_id || sign.player_low == player_low)
    }
}

/// Persistence intents emitted by the service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PetitionWrite {
    InsertPetition {
        petition_id: u32,
        owner_low: u32,
        charter_low: u32,
        name: String,
    },
    InsertSign {
        owner_low: u32,
        petition_id: u32,
        player_low: u32,
        account_id: u32,
    },
    Rename {
        petition_id: u32,
        name: String,
    },
    Delete {
        petition_id: u32,
    },
    /// `Player::RemovePetitionsAndSigns` statements:
    /// the player's owned `petition` rows plus every
    /// `petition_sign` row they own or signed.
    PurgePlayer {
        player_low: u32,
    },
}

/// Horde races (`TeamForRace`, `SharedDefines.h`):
/// Orc, Undead, Tauren, Troll, Blood Elf.
#[must_use]
pub fn is_horde_race(race: u8) -> bool {
    matches!(race, 2 | 5 | 6 | 8 | 10)
}

/// The live petition map (`GuildMgr::m_petitionMap`).
#[derive(Clone, Debug, Default)]
pub struct PetitionService {
    by_id: BTreeMap<u32, Petition>,
    next_id: u32,
}

impl PetitionService {
    /// Build from the boot load (`LoadPetitions`).
    #[must_use]
    pub fn from_loaded(loaded: &LoadedPetitions) -> Self {
        let mut by_id: BTreeMap<u32, Petition> = BTreeMap::new();
        for stored in &loaded.petitions {
            by_id.insert(
                stored.petition_id,
                Petition {
                    id: stored.petition_id,
                    owner_low: stored.owner_low,
                    charter_low: stored.charter_low,
                    name: stored.name.clone(),
                    horde: is_horde_race(stored.owner_race),
                    signatures: Vec::new(),
                },
            );
        }
        for sign in &loaded.signs {
            if let Some(petition) = by_id.get_mut(&sign.petition_id) {
                petition.signatures.push(PetitionSignature {
                    player_low: sign.player_low,
                    account_id: sign.account_id,
                });
            }
        }
        Self {
            by_id,
            next_id: loaded.next_petition_id.max(1),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// `GetPetitionById`.
    #[must_use]
    pub fn by_id(&self, petition_id: u32) -> Option<&Petition> {
        self.by_id.get(&petition_id)
    }

    /// `GetPetitionByOwnerGuid`.
    #[must_use]
    pub fn by_owner(&self, owner_low: u32) -> Option<&Petition> {
        self.by_id.values().find(|p| p.owner_low == owner_low)
    }

    /// `GetPetitionByCharterGuid`.
    #[must_use]
    pub fn by_charter(&self, charter_low: u32) -> Option<&Petition> {
        self.by_id.values().find(|p| p.charter_low == charter_low)
    }

    /// The id the next buy will consume
    /// (`ObjectMgr::GeneratePetitionID` peek: the
    /// actor needs it before the charter rows exist).
    #[must_use]
    pub fn peek_next_id(&self) -> u32 {
        self.next_id
    }

    /// `GuildMgr::CreatePetition`: register a freshly
    /// bought petition and emit its insert.
    pub fn create(
        &mut self,
        petition_id: u32,
        owner_low: u32,
        charter_low: u32,
        name: String,
        owner_horde: bool,
    ) -> PetitionWrite {
        self.next_id = self.next_id.max(petition_id.saturating_add(1));
        self.by_id.insert(
            petition_id,
            Petition {
                id: petition_id,
                owner_low,
                charter_low,
                name: name.clone(),
                horde: owner_horde,
                signatures: Vec::new(),
            },
        );
        PetitionWrite::InsertPetition {
            petition_id,
            owner_low,
            charter_low,
            name,
        }
    }

    /// `Petition::AddNewSignature`. `None` when the
    /// petition is already complete (or unknown).
    pub fn add_signature(
        &mut self,
        petition_id: u32,
        player_low: u32,
        account_id: u32,
        min_signatures: u32,
    ) -> Option<PetitionWrite> {
        let petition = self.by_id.get_mut(&petition_id)?;
        if petition.is_complete(min_signatures) {
            return None;
        }
        petition.signatures.push(PetitionSignature {
            player_low,
            account_id,
        });
        Some(PetitionWrite::InsertSign {
            owner_low: petition.owner_low,
            petition_id,
            player_low,
            account_id,
        })
    }

    /// `Petition::Rename`.
    pub fn rename(&mut self, petition_id: u32, name: String) -> Option<PetitionWrite> {
        let petition = self.by_id.get_mut(&petition_id)?;
        petition.name.clone_from(&name);
        Some(PetitionWrite::Rename { petition_id, name })
    }

    /// `GuildMgr::DeletePetition`.
    pub fn delete(&mut self, petition_id: u32) -> Option<PetitionWrite> {
        self.by_id.remove(&petition_id)?;
        Some(PetitionWrite::Delete { petition_id })
    }

    /// `GuildMgr::DeletePetitionSignaturesByPlayer`:
    /// drop the player's in-memory signature from
    /// every petition except `except_id` (0 = none).
    /// Memory-only, exactly like the C++ destructor
    /// path; see the module note.
    pub fn delete_signatures_by_player(&mut self, player_low: u32, except_id: u32) {
        for petition in self.by_id.values_mut() {
            if petition.id == except_id {
                continue;
            }
            if let Some(index) = petition
                .signatures
                .iter()
                .position(|sign| sign.player_low == player_low)
            {
                petition.signatures.remove(index);
            }
        }
    }
}

/// `isBasicLatinCharacter` (`Util.h`).
fn is_basic_latin_character(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// `isExtendedLatinCharacter` (`Util.h:191-210`).
fn is_extended_latin_character(c: char) -> bool {
    if is_basic_latin_character(c) {
        return true;
    }
    let code = c as u32;
    (0x00C0..=0x00D6).contains(&code)
        || (0x00D8..=0x00DF).contains(&code)
        || (0x00E0..=0x00F6).contains(&code)
        || (0x00F8..=0x00FE).contains(&code)
        || (0x0100..=0x012F).contains(&code)
        || code == 0x1E9E
}

/// `isCyrillicCharacter` (`Util.h:212-219`).
fn is_cyrillic_character(c: char) -> bool {
    let code = c as u32;
    (0x0410..=0x044F).contains(&code) || code == 0x0401 || code == 0x0451
}

/// `isEastAsianCharacter` (`Util.h:221-243`).
fn is_east_asian_character(c: char) -> bool {
    let code = c as u32;
    (0x1100..=0x11F9).contains(&code)
        || (0x3041..=0x30FF).contains(&code)
        || (0x3131..=0x318E).contains(&code)
        || (0x31F0..=0x31FF).contains(&code)
        || (0x3400..=0x4DB5).contains(&code)
        || (0x4E00..=0x9FC3).contains(&code)
        || (0xAC00..=0xD7A3).contains(&code)
        || (0xFF01..=0xFFEE).contains(&code)
}

fn is_numeric_or_space(c: char) -> bool {
    c.is_ascii_digit() || c == ' '
}

fn all_match(name: &str, class: fn(char) -> bool) -> bool {
    name.chars().all(|c| class(c) || is_numeric_or_space(c))
}

/// `ObjectMgr::IsValidCharterName`, name length
/// measured in chars (the C++ wide-string size). `strict_mask` follows
/// `isValidString`: 0 = any single language class,
/// bit 0 = basic Latin allowed, bit 1 = realm-zone
/// class allowed (the Rust server runs an
/// any-language realm zone, so bit 1 accepts all
/// three classes like `LT_ANY`).
#[must_use]
pub fn is_valid_charter_name(name: &str, min_chars: u32, strict_mask: u32) -> bool {
    let count = name.chars().count();
    if count > CHARTER_NAME_MAX_CHARS || count < min_chars as usize {
        return false;
    }
    if strict_mask == 0 {
        return all_match(name, is_extended_latin_character)
            || all_match(name, is_cyrillic_character)
            || all_match(name, is_east_asian_character);
    }
    if strict_mask & 0x2 != 0
        && (all_match(name, is_extended_latin_character)
            || all_match(name, is_cyrillic_character)
            || all_match(name, is_east_asian_character))
    {
        return true;
    }
    if strict_mask & 0x1 != 0 && all_match(name, is_basic_latin_character) {
        return true;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use mangos_db::petition::{LoadedPetitions, StoredPetition, StoredPetitionSign};

    fn loaded() -> LoadedPetitions {
        LoadedPetitions {
            petitions: vec![StoredPetition {
                petition_id: 7,
                owner_low: 3,
                charter_low: 900,
                name: "Bench Wardens".to_string(),
                owner_race: 8,
            }],
            signs: vec![StoredPetitionSign {
                petition_id: 7,
                player_low: 4,
                account_id: 44,
            }],
            next_petition_id: 8,
        }
    }

    #[test]
    fn boot_load_indexes_and_team() {
        let service = PetitionService::from_loaded(&loaded());
        let petition = service.by_id(7).expect("petition");
        assert!(petition.horde);
        assert_eq!(petition.signature_count(), 1);
        assert_eq!(service.by_owner(3).map(|p| p.id), Some(7));
        assert_eq!(service.by_charter(900).map(|p| p.id), Some(7));
        assert_eq!(service.peek_next_id(), 8);
        assert!(petition.signature_for_player(99, 44));
        assert!(petition.signature_for_player(4, 99));
        assert!(!petition.signature_for_player(99, 99));
        assert!(!petition.is_complete(9));
    }

    #[test]
    fn create_sign_rename_delete_flow() {
        let mut service = PetitionService::from_loaded(&loaded());
        let write = service.create(8, 10, 901, "New Guild".to_string(), false);
        assert!(matches!(write, PetitionWrite::InsertPetition { .. }));
        assert_eq!(service.peek_next_id(), 9);
        for i in 0..2u32 {
            assert!(service.add_signature(8, 20 + i, 60 + i, 2).is_some());
        }
        // Complete at exactly the configured minimum.
        assert!(service.by_id(8).expect("p").is_complete(2));
        assert!(service.add_signature(8, 30, 70, 2).is_none());
        let write = service.rename(8, "Renamed".to_string());
        assert!(matches!(write, Some(PetitionWrite::Rename { .. })));
        assert_eq!(service.by_id(8).expect("p").name, "Renamed");
        assert!(service.delete(8).is_some());
        assert!(service.by_id(8).is_none());
    }

    #[test]
    fn signature_cleanup_respects_exception() {
        let mut service = PetitionService::from_loaded(&loaded());
        service.delete_signatures_by_player(4, 7);
        assert_eq!(service.by_id(7).expect("p").signature_count(), 1);
        service.delete_signatures_by_player(4, 0);
        assert_eq!(service.by_id(7).expect("p").signature_count(), 0);
    }

    #[test]
    fn charter_name_validation() {
        assert!(is_valid_charter_name("Bench Wardens 2", 2, 0));
        assert!(!is_valid_charter_name("A", 2, 0));
        assert!(!is_valid_charter_name(&"A".repeat(25), 2, 0));
        assert!(!is_valid_charter_name("Bad!Name", 2, 0));
        assert!(is_valid_charter_name("Гильдия", 2, 0));
        assert!(!is_valid_charter_name("MixГил", 2, 0));
        assert!(!is_valid_charter_name("Ärzte", 2, 1));
        assert!(is_valid_charter_name("Doctors", 2, 1));
        assert!(is_valid_charter_name("Ärzte", 2, 2));
    }
}
