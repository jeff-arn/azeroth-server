//! The Bot Host API (architecture doc §9): the
//! query/command surface behaviours run against.
//! `mangosd` implements it over the map + world
//! actor; tests implement it with fakes. The C++
//! behaviours reach this surface through
//! `CombatBotBaseAI` (`CanTryToCastSpell`,
//! `DoCastSpell`, `MotionMaster` calls on `me`,
//! group queries), which is exactly this split:
//! read-only views for scoring, verbs for executing.

use mangos_guid::ObjectGuid;

/// `SpellCastResult` reduced to what bot code
/// branches on (`SPELL_CAST_OK` vs anything else).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CastOutcome {
    Ok,
    Failed(u8),
    UnknownSpell,
}

/// Read-only view of one unit for scoring-time
/// queries beyond the situation snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "flat read-only snapshot of unit state for scoring"
)]
pub struct UnitView {
    pub guid: ObjectGuid,
    pub is_alive: bool,
    pub health_pct: f32,
    pub health: u32,
    pub max_health: u32,
    pub position: (f32, f32, f32),
    pub orientation: f32,
    pub in_combat: bool,
    pub is_player: bool,
    pub class_id: u8,
    /// Current victim (who this unit is attacking),
    /// if any.
    pub victim: Option<ObjectGuid>,
    /// `UNIT_FIELD_TARGET` (selection).
    pub target: Option<ObjectGuid>,
    pub is_casting: bool,
    /// `Powers` value (mana 0, rage 1, focus 2,
    /// energy 3).
    pub power_type: u8,
    /// Current power percent (innervate checks).
    pub power_pct: f32,
}

/// Motion generator classes the behaviours branch
/// on (`GetCurrentMovementGeneratorType` reduced;
/// the fork's DISTANCING maps to `Point` here —
/// our kite/flee uses point moves).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionKind {
    Idle,
    Chase,
    Point,
    Other,
}

/// Chat channels a bot can speak on (Slice E chat).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BotChatKind {
    Say,
    Yell,
    Party,
    Guild,
    Whisper(ObjectGuid),
}

/// The host verbs + views. One implementation is
/// bound to a single bot for the duration of a tick.
pub trait BotHost {
    /// The bot player this host acts for.
    fn me(&self) -> ObjectGuid;

    /// Unit view by guid (same map).
    fn unit(&self, guid: ObjectGuid) -> Option<UnitView>;

    /// Party/raid member guids including self
    /// (empty group returns just self).
    fn party_members(&self) -> Vec<ObjectGuid>;

    /// `CanTryToCastSpell`: pre-check without side
    /// effects (known spell, target validity,
    /// range/LoS/power/cooldown as the host can
    /// determine them).
    fn can_try_cast(&self, target: Option<ObjectGuid>, spell_id: u32) -> bool;

    /// `DoCastSpell` (non-triggered bot cast).
    fn cast_spell(&mut self, target: Option<ObjectGuid>, spell_id: u32) -> CastOutcome;

    /// `MoveChase(target, dist, angle)`.
    fn move_chase(&mut self, target: ObjectGuid, distance: f32, angle: f32);

    /// `MoveFollow(target, dist, angle)` — chase
    /// generator with follow semantics on the fork.
    fn move_follow(&mut self, target: ObjectGuid, distance: f32, angle: f32);

    /// `MovePoint` to an absolute position.
    fn move_point(&mut self, x: f32, y: f32, z: f32);

    /// `StopMoving`.
    fn stop_moving(&mut self);

    /// Sets the bot's target (selection).
    fn set_target(&mut self, target: Option<ObjectGuid>);

    /// Sends a chat line as the bot player.
    fn say(&mut self, kind: BotChatKind, text: &str);

    /// Whispers a player (`SendWhisperToPlayer`).
    fn whisper(&mut self, target: ObjectGuid, text: &str);

    /// Units currently melee-attacking `guid`
    /// (`Unit::GetAttackers` reduced to the
    /// melee-victim relation).
    fn attackers_of(&self, guid: ObjectGuid) -> Vec<ObjectGuid>;

    /// `HasAura(spellId)`.
    fn has_aura(&self, guid: ObjectGuid, spell_id: u32) -> bool;

    /// `HasAuraType(SPELL_AURA_PERIODIC_HEAL)`.
    fn has_periodic_heal_aura(&self, guid: ObjectGuid) -> bool;

    /// `Player::HasSpell` for the bot itself.
    fn knows_spell(&self, spell_id: u32) -> bool;

    /// Hostile units within `radius` of `center`
    /// (`GetEnemyCountInRadiusAround`).
    fn hostile_count_around(&self, center: ObjectGuid, radius: f32) -> u8;

    /// `GetIncomingdamage`: sum of reachable
    /// attackers' average weapon damage.
    fn incoming_damage(&self, guid: ObjectGuid) -> i32;

    /// `FindPartyTank` (explicit role assignment,
    /// then bot role, then tanking-form/shield
    /// heuristics — host-side).
    fn party_tank(&self) -> Option<ObjectGuid>;

    /// Group leader (`GetPartyLeader` reduced to
    /// the actor group leader / bot master).
    fn party_leader(&self) -> Option<ObjectGuid>;

    /// Threat `who` holds on `enemy`
    /// (`ThreatManager::getThreat`); 0 when absent.
    fn threat_of(&self, enemy: ObjectGuid, who: ObjectGuid) -> f32;

    /// Current motion generator class of the bot.
    fn motion_kind(&self) -> MotionKind;

    /// `AttackStop` (clears melee victim).
    fn attack_stop(&mut self);

    /// `CanReachWithMeleeAutoAttack` (combat-reach
    /// distance check, host-side).
    fn can_reach_melee(&self, target: ObjectGuid) -> bool;

    /// First party member with a dispellable aura
    /// for `spell_id` (`SelectDispelTarget` +
    /// `IsValidDispelTarget`, friendly arm; the
    /// dispel-mask math lives host-side with the
    /// spell store).
    fn dispel_target(&self, spell_id: u32) -> Option<ObjectGuid>;

    /// Raid icons this bot must CC (`m_marksToCC`).
    fn cc_marks(&self) -> Vec<u8>;

    /// Unit carrying raid icon `icon`, if any.
    fn marked_target(&self, icon: u8) -> Option<ObjectGuid>;

    /// Whether `guid` carries any raid icon.
    fn is_marked(&self, guid: ObjectGuid) -> bool;

    /// `AutoMarkForCC`: claim a free CC icon for
    /// `target` and register it in `cc_marks`.
    fn auto_mark_cc(&mut self, target: ObjectGuid) -> bool;

    /// `UNIT_STATE_CAN_NOT_REACT_OR_LOST_CONTROL`
    /// (stunned/rooted/feared etc., host's unit
    /// state).
    fn is_ccd(&self, guid: ObjectGuid) -> bool;

    /// `me->ClearTarget()` after friendly casts.
    fn clear_target(&mut self);

    /// `HasAuraType`.
    fn has_aura_type(&self, guid: ObjectGuid, aura_type: u32) -> bool;

    /// `RemoveAurasDueToSpellByCancel`.
    fn cancel_aura(&mut self, spell_id: u32);

    /// `RemoveSpellsCausingAura(MOD_SHAPESHIFT)`.
    fn remove_shapeshift(&mut self);

    /// Current pet, if any (`GetPet`).
    fn pet(&self) -> Option<ObjectGuid>;

    /// Orders the pet to attack (`PetAI::AttackStart`).
    fn pet_attack(&mut self, target: ObjectGuid);

    /// `SummonPetIfNeeded` (hunter call/revive/tame,
    /// warlock random summon) — world-side verb.
    fn summon_pet_if_needed(&mut self);

    /// `CastWeaponBuff` (temp-enchant cast on the
    /// equipped weapon); `false` until item-target
    /// casts exist host-side.
    fn cast_weapon_buff(&mut self, spell_id: u32, offhand: bool) -> bool;

    /// `GetTotem(slot)` presence (0 fire, 1 earth,
    /// 2 water, 3 air).
    fn totem_present(&self, slot: u32) -> bool;

    /// Records a totem summon for `totem_present`
    /// (host bookkeeping until real totem tracking).
    fn note_totem_cast(&mut self, slot: u32);

    /// `SetFacingToObject`.
    fn face(&mut self, target: ObjectGuid);

    /// Random index `<len` (warlock summon pick).
    fn pick_index(&mut self, len: usize) -> usize;
}

/// `SPELL_AURA_MOD_STEALTH`.
pub const AURA_MOD_STEALTH: u32 = 16;
/// `SPELL_AURA_MOD_INVISIBILITY`.
pub const AURA_MOD_INVISIBILITY: u32 = 18;
/// `SPELL_AURA_MOD_SHAPESHIFT`.
pub const AURA_MOD_SHAPESHIFT: u32 = 36;
