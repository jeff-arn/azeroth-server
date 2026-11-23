//! Finisher combo points (`Player::AddComboPoints` /
//! `ClearComboPoints` / `GetComboPoints`,
//! `Player.cpp:18829-18889` + `Player.h:1503-1507`):
//! the rogue/druid finisher resource. Points live on
//! the player, target one victim, clamp to 0-5, and
//! mirror into the update fields (`Player::SetComboPoints`,
//! same file) exactly as the fork does.
//!
//! Recorded gaps: `SPELL_AURA_RETAIN_COMBO_POINTS`
//! (148) has no Rust aura id yet, so its removal gate is
//! transcribed with the literal; the combo-holder list
//! (`AddComboPointHolder` / `RemoveComboPointHolder`,
//! `Unit.cpp:9468`) has no Rust counterpart — holders
//! only matter for the fork's target-died cleanup.

use mangos_guid::ObjectGuid;
use mangos_protocol::update_field_table::offsets;

use crate::map::Map;

/// `SPELL_AURA_RETAIN_COMBO_POINTS`
/// (`SpellAuraDefines.h:212`): no Rust define yet.
const AURA_RETAIN_COMBO_POINTS: u32 = 148;
/// `SPELL_ATTR_EX_FINISHING_MOVE_DAMAGE`
/// (`SpellDefines.h:837`).
const ATTR_EX_FINISHING_MOVE_DAMAGE: u32 = 0x0010_0000;
/// `SPELL_ATTR_EX_FINISHING_MOVE_DURATION`
/// (`SpellDefines.h:839`).
const ATTR_EX_FINISHING_MOVE_DURATION: u32 = 0x0040_0000;
/// `PLAYER_FIELD_BYTES` combo-points byte
/// (`Player.h:361`).
const COMBO_POINTS_BYTE: u8 = 1;
/// Finisher cap (`Player.cpp:18863`).
const MAX_COMBO_POINTS: i32 = 5;

impl Map {
    /// `Player::GetComboPoints` (`Player.h:1503`):
    /// players only, 0 otherwise.
    pub(crate) fn combo_points(&self, caster: ObjectGuid) -> u8 {
        self.players
            .get(&caster)
            .map_or(0, |entry| entry.player.combo_points)
    }

    /// `Player::GetComboTargetGuid` (`Player.h:1505`):
    /// players only, empty guid otherwise.
    pub(crate) fn combo_target(&self, caster: ObjectGuid) -> ObjectGuid {
        self.players
            .get(&caster)
            .map_or(ObjectGuid::EMPTY, |entry| entry.player.combo_target)
    }

    /// `Player::AddComboPoints`
    /// (`Player.cpp:18844-18877`): same-target
    /// accumulation, retarget reset, clamp 0-5, then the
    /// `SetComboPoints` field mirror.
    pub(crate) fn add_combo_points(&mut self, caster: ObjectGuid, target: ObjectGuid, count: i32) {
        if count == 0 || !caster.is_player() {
            return;
        }
        // Without combo points lost (duration checked in
        // aura).
        self.remove_spells_causing_aura(caster, AURA_RETAIN_COMBO_POINTS);
        if !self.players.contains_key(&caster) {
            return;
        }
        let base = if self
            .players
            .get(&caster)
            .is_some_and(|entry| entry.player.combo_target == target)
        {
            i32::from(
                self.players
                    .get(&caster)
                    .map_or(0, |entry| entry.player.combo_points),
            )
        } else {
            if let Some(entry) = self.players.get_mut(&caster) {
                entry.player.combo_target = target;
            }
            0
        };
        if let Some(entry) = self.players.get_mut(&caster) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let points = (base + count).clamp(0, MAX_COMBO_POINTS) as u8;
            entry.player.combo_points = points;
        }
        self.mirror_combo_points(caster);
    }

    /// `Player::ClearComboPoints`
    /// (`Player.cpp:18879-18891`): no-op without a
    /// target; zeroes the points, mirrors the fields
    /// while the target is still set (fork order), then
    /// clears the in-memory target.
    pub(crate) fn clear_combo_points(&mut self, caster: ObjectGuid) {
        if !caster.is_player() {
            return;
        }
        let target = self
            .players
            .get(&caster)
            .map_or(ObjectGuid::EMPTY, |entry| entry.player.combo_target);
        if target.is_empty() {
            return;
        }
        self.remove_spells_causing_aura(caster, AURA_RETAIN_COMBO_POINTS);
        if let Some(entry) = self.players.get_mut(&caster) {
            entry.player.combo_points = 0;
        }
        self.mirror_combo_points(caster);
        if let Some(entry) = self.players.get_mut(&caster) {
            entry.player.combo_target = ObjectGuid::EMPTY;
        }
    }

    /// `Unit::ClearComboPointHolders`
    /// (`Unit.cpp:9460-9474`): every player holding
    /// combo points on `target` drops them. The fork
    /// walks `m_ComboPointHolders`; this subset sweeps
    /// `combo_target` instead (the holder list has no
    /// Rust counterpart, and `add`/`clear` keep the
    /// target in sync — recorded in the module docs).
    pub(crate) fn clear_combo_point_holders(&mut self, target: ObjectGuid) {
        let holders: Vec<ObjectGuid> = self
            .players
            .iter()
            .filter(|(_, entry)| entry.player.combo_target == target)
            .map(|(guid, _)| *guid)
            .collect();
        for holder in holders {
            self.clear_combo_points(holder);
        }
    }

    /// `SpellEntry::NeedsComboPoints`
    /// (`SpellEntry.h:958`).
    pub(crate) fn spell_needs_combo_points(spell: &mangos_spells::entry::SpellEntry) -> bool {
        spell.attributes_ex & (ATTR_EX_FINISHING_MOVE_DAMAGE | ATTR_EX_FINISHING_MOVE_DURATION) != 0
    }

    /// `Spell::finish` combo tail (`Spell.cpp:4462-4482`):
    /// a finishing-move spell drops the caster's combo
    /// points when every non-caster target was hit
    /// (`needDrop` stays true for positive spells and
    /// when no non-caster target missed).
    pub(crate) fn clear_combo_on_spell_finish(
        &mut self,
        caster: ObjectGuid,
        spell: &mangos_spells::entry::SpellEntry,
        missed_non_caster: bool,
    ) {
        if !caster.is_player() || !Self::spell_needs_combo_points(spell) {
            return;
        }
        if !spell.is_positive_spell() && missed_non_caster {
            return;
        }
        self.clear_combo_points(caster);
    }

    /// `Player::SetComboPoints`
    /// (`Player.cpp:18829-18843`): writes
    /// `PLAYER_FIELD_COMBO_TARGET` + the combo byte when
    /// the victim is still on the map (fork
    /// `ObjectAccessor::GetUnit`).
    fn mirror_combo_points(&mut self, caster: ObjectGuid) {
        let Some((target, points)) = self
            .players
            .get(&caster)
            .map(|entry| (entry.player.combo_target, entry.player.combo_points))
        else {
            return;
        };
        if self.unit_base(target).is_none() {
            return;
        }
        if let Some(entry) = self.players.get_mut(&caster) {
            let values = entry.player.unit.world.object.values_mut();
            values.set_guid(offsets::PLAYER__FIELD_COMBO_TARGET, target);
            values.set_byte(offsets::PLAYER_FIELD_BYTES, COMBO_POINTS_BYTE, points);
        }
    }
}
