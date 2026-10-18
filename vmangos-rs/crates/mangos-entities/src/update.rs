//! Object-update block building, ported from `Object.cpp:236-940`
//! (`BuildCreateUpdateBlockForPlayer`, `BuildValuesUpdateBlockForPlayer`,
//! `BuildMovementUpdate`, `BuildValuesUpdate`, `DestroyForPlayer`,
//! `DirectSendPublicValueUpdate`) at the 1.12.1 shape.
//!
//! The C++ methods reach through `this` (Unit/Creature/GameObject)
//! and the target `Player` for dozens of facts. Here those facts
//! arrive as data: [`Viewer`] (the target side) and
//! [`ObjectView`] (the source side, incl. precomputed per-viewer
//! rewrite words for the creature/GO/corpse arms whose logic
//! belongs to those entity types). Everything decidable from the
//! values words alone — attack-time/stat float→u32 conversion, GM
//! field flags, health-as-percent, FFA strip condition, cast-speed
//! floor — is computed here, in the C++ arm order.
//!
//! Session sends (`SendCreateUpdateToPlayer`, the `Send` half of
//! `DirectSendPublicValueUpdate`, `DestroyForPlayer`'s send) stay
//! with the caller; builders only produce bytes/packets.

use mangos_guid::{ObjectGuid, TypeId};
use mangos_movement::move_spline::MoveSpline;
use mangos_movement::movement_info::{MovementFlags, MovementInfo, Position};
use mangos_movement::packet_builder;
use mangos_protocol::buffer::PacketWriter;
use mangos_protocol::update_data::{
    CompressionConfig, ObjectUpdateFlags, ObjectUpdateType, SMSG_DESTROY_OBJECT, UpdateData,
};
use mangos_protocol::update_field_table::offsets;
use mangos_protocol::update_fields::{TypeMask, UpdateFieldFlags, update_field_flags};
use mangos_protocol::update_mask::UpdateMask;
use mangos_protocol::world::WorldPacket;

use crate::object::ObjectBase;

/// `UNIT_FLAG_AURAS_VISIBLE` (`UnitDefines.h:568`).
pub const UNIT_FLAG_AURAS_VISIBLE: u32 = 0x0800_0000;
/// `UNIT_FLAG_NOT_SELECTABLE` (`UnitDefines.h:566`).
pub const UNIT_FLAG_NOT_SELECTABLE: u32 = 0x0200_0000;
/// `UNIT_DYNFLAG_SPECIALINFO` (`SharedDefines.h:1170`).
pub const UNIT_DYNFLAG_SPECIALINFO: u32 = 0x0010;
/// `UNIT_DYNFLAG_LOOTABLE` (`SharedDefines.h:1166`).
pub const UNIT_DYNFLAG_LOOTABLE: u32 = 0x0001;
/// `UNIT_DYNFLAG_TRACK_UNIT` (`SharedDefines.h:1167`).
pub const UNIT_DYNFLAG_TRACK_UNIT: u32 = 0x0002;
/// `UNIT_DYNFLAG_TAPPED` (`SharedDefines.h:1168`).
pub const UNIT_DYNFLAG_TAPPED: u32 = 0x0004;
/// `UNIT_DYNFLAG_TAPPED_BY_PLAYER` (`SharedDefines.h:1169`).
pub const UNIT_DYNFLAG_TAPPED_BY_PLAYER: u32 = 0x0008;
/// `CORPSE_DYNFLAG_LOOTABLE` (`SharedDefines.h:1176`).
pub const CORPSE_DYNFLAG_LOOTABLE: u32 = 0x0001;
/// `GO_DYNFLAG_LO_ACTIVATE` (`GameObjectDefines.h:108`).
pub const GO_DYNFLAG_LO_ACTIVATE: u16 = 0x01;
/// `PLAYER_FLAGS_FFA_PVP` (`Player.h:322`).
pub const PLAYER_FLAGS_FFA_PVP: u32 = 0x0000_0080;
/// `UNIT_NPC_FLAG_FLIGHTMASTER` (`UnitDefines.h:656`).
pub const UNIT_NPC_FLAG_FLIGHTMASTER: u32 = 0x0000_0008;
/// `UNIT_NPC_FLAG_TRAINER` (`UnitDefines.h:657`).
pub const UNIT_NPC_FLAG_TRAINER: u32 = 0x0000_0010;
/// `UNIT_NPC_FLAG_STABLEMASTER` (`UnitDefines.h:666`).
pub const UNIT_NPC_FLAG_STABLEMASTER: u32 = 0x0000_2000;

/// The facts `GetUpdateFieldFlagsForTarget` + the rewrite arms
/// need about the **target player** relative to the source object
/// (`Object.cpp:974-1033,643-837`). Fields default to the
/// no-privilege stranger.
#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)] // field-for-field port of the C++ target-side predicates
pub struct Viewer {
    /// `target == this`.
    pub is_self: bool,
    /// `target->IsGameMaster()`.
    pub is_game_master: bool,
    /// `target->HasCheatOption(PLAYER_CHEAT_DEBUG_TARGET_INFO)`.
    pub has_debug_target_cheat: bool,
    /// Item case: the item's owner is the target.
    pub item_owner_is_target: bool,
    /// Unit case: owner or charmer is the target.
    pub owner_or_charmer_is_target: bool,
    /// Unit case: `target->CanSeeSpecialInfoOf(this)` (only read
    /// when the stored dynflags carry SPECIALINFO).
    pub can_see_special_info: bool,
    /// Unit case: charmer-or-owner-player-or-self exists and is in
    /// the same raid as the target.
    pub group_only_granted: bool,
    /// `target->CanSeeHealthOf(this)`.
    pub can_see_health: bool,
    /// `ShowHealthValues` world config (default false).
    pub show_health_values: bool,
    /// `PLAYER_FLAGS` arm: owner exists, is not the target, and is
    /// in the same raid (`Object.cpp:768-769`).
    pub strip_ffa_pvp: bool,
}

/// `GetUpdateFieldFlagsForTarget` flag computation
/// (`Object.cpp:974-1033`); the per-type flags array comes from
/// [`update_field_flags`]. `has_special_info_dynflag` is the
/// stored `UNIT_DYNAMIC_FLAGS & UNIT_DYNFLAG_SPECIALINFO` test.
#[must_use]
pub fn visible_flags_for(type_id: TypeId, viewer: &Viewer, has_special_info_dynflag: bool) -> u16 {
    let mut visible = UpdateFieldFlags::PUBLIC | UpdateFieldFlags::DYNAMIC;
    if viewer.is_self {
        visible |= UpdateFieldFlags::PRIVATE;
    }
    match type_id {
        TypeId::Item | TypeId::Container => {
            if viewer.item_owner_is_target {
                visible |= UpdateFieldFlags::OWNER_ONLY | UpdateFieldFlags::UNK2;
            }
        }
        TypeId::Unit | TypeId::Player => {
            if viewer.has_debug_target_cheat {
                visible |= UpdateFieldFlags::OWNER_ONLY
                    | UpdateFieldFlags::SPECIAL_INFO
                    | UpdateFieldFlags::GROUP_ONLY;
            } else {
                if viewer.owner_or_charmer_is_target {
                    visible |= UpdateFieldFlags::OWNER_ONLY;
                }
                if has_special_info_dynflag && viewer.can_see_special_info {
                    visible |= UpdateFieldFlags::SPECIAL_INFO;
                }
                if viewer.group_only_granted {
                    visible |= UpdateFieldFlags::GROUP_ONLY;
                }
            }
        }
        // GameObject/DynamicObject/Corpse owner arms are commented
        // out in C++ (Object.cpp:1012-1029): no owner-only grants.
        _ => {}
    }
    visible
}

/// Per-(source, viewer) rewrite words that only the concrete
/// entity types can compute. `None` = arm not applicable, the
/// stored word is used (this matches the player/plain-unit paths).
#[derive(Clone, Copy, Debug, Default)]
pub struct RewriteInputs {
    /// Creature `UNIT_NPC_FLAGS` after trainer/stable/flight
    /// stripping (`Object.cpp:643-691`).
    pub npc_flags: Option<u32>,
    /// `UNIT_DYNAMIC_FLAGS` after track/tapped/lootable logic
    /// (`Object.cpp:706-742`); the debug-cheat SPECIALINFO bit is
    /// OR-ed in by the builder afterwards, as in C++.
    pub dynamic_flags: Option<u32>,
    /// Cross-faction raid replacement faction template
    /// (`Object.cpp:744-764`).
    pub faction_template: Option<u32>,
    /// Creature casting-target override for `UNIT_FIELD_TARGET`
    /// (`m_castingTargetGuid`, `Object.cpp:805-828`); empty = none.
    pub casting_target_guid: ObjectGuid,
    /// Corpse dynflags after looted/friendly logic
    /// (`Object.cpp:886-898`).
    pub corpse_dynamic_flags: Option<u32>,
    /// `GameObject` `ActivateToQuest(target)` (`Object.cpp:593,616`);
    /// the builder ORs in the viewer's GM state and records the
    /// result through the quest-activation sink.
    pub go_activate_to_quest: bool,
    /// `GameObject` type id for the dyn-flags switch
    /// (`enum GameobjectTypes`, `GameObjectDefines.h:25-…`).
    pub go_type: u32,
}

/// GO types whose dyn-flags word becomes
/// `GO_DYNFLAG_LO_ACTIVATE` when quest-activated
/// (`Object.cpp:859-863`): QUESTGIVER 2, CHEST 3, GENERIC 5,
/// `SPELL_FOCUS` 8, `GOOBER` 10.
const GO_TYPES_LO_ACTIVATE: [u32; 5] = [2, 3, 5, 8, 10];

/// Movement-section inputs for `BuildMovementUpdate`.
#[derive(Clone, Copy, Debug)]
pub enum MovementSection<'a> {
    /// Neither `LIVING` nor `HAS_POSITION` (items).
    None,
    /// `HAS_POSITION` floats (`Object.cpp:485-504`): stationary
    /// position for transports, world position otherwise — the
    /// caller picks, as the C++ branches on the same flag pair.
    HasPosition(Position),
    /// LIVING (`Object.cpp:452-482`).
    Living {
        /// Seeded copy of the unit's movement info (C++ copies
        /// `m_movementInfo` before adjusting it).
        info: MovementInfo,
        /// Authoritative current position, used when
        /// `info.client_time == 0` (server-side synthesis).
        position: Position,
        /// `WALK`, `RUN`, `RUN_BACK`, `SWIM`, `SWIM_BACK`, `TURN_RATE`.
        speeds: [f32; 6],
        /// `MOVEFLAG_ROOT` is stripped for creatures (`:463-464`).
        is_creature: bool,
        /// Active spline, written when the info carries
        /// `SPLINE_ENABLED`.
        spline: Option<&'a MoveSpline>,
    },
}

/// Everything the block builders need about the source object
/// beyond its [`ObjectBase`].
#[derive(Clone, Copy)]
pub struct ObjectView<'a> {
    pub movement: MovementSection<'a>,
    pub rewrites: RewriteInputs,
    /// Melee victim for the `MELEE_ATTACKING` create flag + packed
    /// victim in the movement section; `None` = not attacking.
    pub melee_victim: Option<ObjectGuid>,
    /// Transport path progress for transport GOs; `None` falls
    /// back to current server mstime (`Object.cpp:534-538`).
    pub transport_path_progress: Option<u32>,
    /// `IsTransport()` for the GO forced-bit arms.
    pub is_transport: bool,
}

impl Default for ObjectView<'_> {
    fn default() -> Self {
        Self {
            movement: MovementSection::None,
            rewrites: RewriteInputs::default(),
            melee_victim: None,
            transport_path_progress: None,
            is_transport: false,
        }
    }
}

/// `BuildMovementUpdate` (`Object.cpp:447-539`, 1.12.1 path).
/// `now_ms` replaces `WorldTimer::getMSTime()` / `sWorld` time:
/// server-side infos are stamped `now + 1000` and repositioned to
/// the authoritative position, and non-transport objects under
/// `UPDATEFLAG_TRANSPORT` emit `now`.
fn build_movement_update(
    view: &ObjectView<'_>,
    buf: &mut PacketWriter,
    flags: ObjectUpdateFlags,
    now_ms: u32,
) {
    buf.write_u8(flags.raw());

    if flags.has(ObjectUpdateFlags::LIVING) {
        // C++ asserts a Unit here; a non-living view is a caller
        // bug, treated as the all-1.0f fallback's empty movement
        // (defensive: writes only the six speeds).
        if let MovementSection::Living {
            info,
            position,
            speeds,
            is_creature,
            spline,
        } = view.movement
        {
            let mut m = info;
            if m.client_time == 0 {
                m.server_time = now_ms.wrapping_add(1000);
                m.change_position(position);
            }
            if is_creature {
                m.flags.remove(MovementFlags::ROOT);
            }
            m.write(buf);
            for speed in speeds {
                buf.write_f32_le(speed);
            }
            if info.flags.has(MovementFlags::SPLINE_ENABLED) {
                if let Some(spline) = spline {
                    packet_builder::write_create(buf, spline);
                }
            }
        } else {
            for _ in 0..6 {
                buf.write_f32_le(1.0);
            }
        }
    } else if flags.has(ObjectUpdateFlags::HAS_POSITION) {
        if let MovementSection::HasPosition(p) = view.movement {
            buf.write_f32_le(p.x)
                .write_f32_le(p.y)
                .write_f32_le(p.z)
                .write_f32_le(p.o);
        }
    }

    if flags.has(ObjectUpdateFlags::HIGH_GUID) {
        buf.write_u32_le(0);
    }
    if flags.has(ObjectUpdateFlags::ALL) {
        buf.write_u32_le(1);
    }
    if flags.has(ObjectUpdateFlags::MELEE_ATTACKING) {
        match view.melee_victim {
            Some(victim) => {
                let mut packed = Vec::with_capacity(9);
                victim.write_packed(&mut packed);
                buf.write_bytes(&packed);
            }
            None => {
                buf.write_u8(0);
            }
        }
    }
    if flags.has(ObjectUpdateFlags::TRANSPORT) {
        buf.write_u32_le(view.transport_path_progress.unwrap_or(now_ms));
    }
}

/// Forced mask bits + GO quest-activation resolution shared by the
/// create/values branches of `BuildValuesUpdate`
/// (`Object.cpp:583-629`). Returns `IsActivateToQuest`.
fn prepare_values_mask(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    update_type: ObjectUpdateType,
    mask: &mut UpdateMask,
    quest_sink: &mut Vec<(ObjectGuid, bool)>,
) -> bool {
    let mut activate = false;
    let is_create = matches!(
        update_type,
        ObjectUpdateType::CreateObject | ObjectUpdateType::CreateObject2
    );
    if base.is_type(TypeMask::GAMEOBJECT) && !view.is_transport {
        if view.rewrites.go_activate_to_quest || viewer.is_game_master {
            activate = true;
        }
        if is_create {
            mask.set_bit(u32::from(offsets::GAMEOBJECT_DYN_FLAGS));
        } else if update_type == ObjectUpdateType::Values {
            mask.set_bit(u32::from(offsets::GAMEOBJECT_DYN_FLAGS));
            mask.set_bit(u32::from(offsets::GAMEOBJECT_ANIMPROGRESS));
        }
    } else if is_create && base.is_type(TypeMask::UNIT) && viewer.has_debug_target_cheat {
        mask.set_bit(u32::from(offsets::UNIT_DYNAMIC_FLAGS));
    }

    if base.is_type(TypeMask::GAMEOBJECT) {
        quest_sink.push((base.object_guid(), activate));
    }
    activate
}

/// `BuildValuesUpdate` (`Object.cpp:576-940`): mask header, then
/// per-word serialization with the unit/GO/corpse rewrite arms in
/// C++ priority order.
#[allow(clippy::too_many_arguments)]
fn build_values_update(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    update_type: ObjectUpdateType,
    buf: &mut PacketWriter,
    mask: &mut UpdateMask,
    quest_sink: &mut Vec<(ObjectGuid, bool)>,
) {
    let activate_to_quest = prepare_values_mask(base, view, viewer, update_type, mask, quest_sink);

    debug_assert_eq!(mask.count(), u32::from(base.values().count()));
    buf.write_u8(mask.block_count() as u8);
    for &block in mask.blocks() {
        buf.write_u32_le(block);
    }

    let values = base.values();
    if base.is_type(TypeMask::UNIT) {
        build_unit_values(values.words(), view, viewer, mask, buf);
    } else if base.is_type(TypeMask::GAMEOBJECT) {
        for (index, &word) in values.words().iter().enumerate() {
            if !mask.get_bit(index as u32) {
                continue;
            }
            if index as u16 == offsets::GAMEOBJECT_DYN_FLAGS {
                if activate_to_quest && GO_TYPES_LO_ACTIVATE.contains(&view.rewrites.go_type) {
                    buf.write_u16_le(GO_DYNFLAG_LO_ACTIVATE).write_u16_le(0);
                } else {
                    buf.write_u32_le(0);
                }
            } else {
                buf.write_u32_le(word);
            }
        }
    } else if base.is_type(TypeMask::CORPSE) {
        for (index, &word) in values.words().iter().enumerate() {
            if !mask.get_bit(index as u32) {
                continue;
            }
            if index as u16 == offsets::CORPSE_FIELD_DYNAMIC_FLAGS {
                buf.write_u32_le(view.rewrites.corpse_dynamic_flags.unwrap_or(word));
            } else {
                buf.write_u32_le(word);
            }
        }
    } else {
        for (index, &word) in values.words().iter().enumerate() {
            if mask.get_bit(index as u32) {
                buf.write_u32_le(word);
            }
        }
    }
}

/// The `TYPEMASK_UNIT` serialization loop (`Object.cpp:637-845`),
/// arms in C++ priority order.
fn build_unit_values(
    words: &[u32],
    view: &ObjectView<'_>,
    viewer: &Viewer,
    mask: &UpdateMask,
    buf: &mut PacketWriter,
) {
    for (index, &word) in words.iter().enumerate() {
        if !mask.get_bit(index as u32) {
            continue;
        }
        let index = index as u16;
        let out = if index == offsets::UNIT_NPC_FLAGS {
            view.rewrites.npc_flags.unwrap_or(word)
        } else if (offsets::UNIT_FIELD_BASEATTACKTIME..=offsets::UNIT_FIELD_RANGEDATTACKTIME)
            .contains(&index)
        {
            // Stored float, sent as uint32 (Object.cpp:693-697).
            let f = f32::from_bits(word);
            if f < 0.0 { 0 } else { f as u32 }
        } else if is_stat_buff_range(index) {
            f32::from_bits(word) as u32
        } else if index == offsets::UNIT_FIELD_FLAGS && viewer.is_game_master {
            (word | UNIT_FLAG_AURAS_VISIBLE) & !UNIT_FLAG_NOT_SELECTABLE
        } else if index == offsets::UNIT_DYNAMIC_FLAGS {
            let mut dynflags = view.rewrites.dynamic_flags.unwrap_or(word);
            if viewer.has_debug_target_cheat {
                dynflags |= UNIT_DYNFLAG_SPECIALINFO;
            }
            dynflags
        } else if index == offsets::UNIT_FIELD_FACTIONTEMPLATE {
            view.rewrites.faction_template.unwrap_or(word)
        } else if index == offsets::PLAYER_FLAGS && word & PLAYER_FLAGS_FFA_PVP != 0 {
            if viewer.strip_ffa_pvp {
                word & !PLAYER_FLAGS_FFA_PVP
            } else {
                word
            }
        } else if !viewer.show_health_values
            && (index == offsets::UNIT_FIELD_HEALTH || index == offsets::UNIT_FIELD_MAXHEALTH)
        {
            health_word(words, index, viewer.can_see_health, word)
        } else if index == offsets::UNIT_FIELD_TARGET
            && !view.rewrites.casting_target_guid.is_empty()
        {
            view.rewrites.casting_target_guid.raw() as u32
        } else if index == offsets::UNIT_FIELD_TARGET + 1
            && !view.rewrites.casting_target_guid.is_empty()
        {
            (view.rewrites.casting_target_guid.raw() >> 32) as u32
        } else if index == offsets::UNIT_MOD_CAST_SPEED {
            let f = f32::from_bits(word);
            if f < 0.001 { 0.0f32.to_bits() } else { word }
        } else {
            word
        };
        buf.write_u32_le(out);
    }
}

/// Stat/resist-buff float ranges sent as `uint32(f)`
/// (`Object.cpp:700-701`).
fn is_stat_buff_range(index: u16) -> bool {
    (offsets::PLAYER_FIELD_NEGSTAT0..=offsets::PLAYER_FIELD_NEGSTAT0 + 4).contains(&index)
        || (offsets::PLAYER_FIELD_POSSTAT0..=offsets::PLAYER_FIELD_POSSTAT0 + 4).contains(&index)
        || (offsets::PLAYER_FIELD_RESISTANCEBUFFMODSPOSITIVE
            ..=offsets::PLAYER_FIELD_RESISTANCEBUFFMODSPOSITIVE + 6)
            .contains(&index)
        || (offsets::PLAYER_FIELD_RESISTANCEBUFFMODSNEGATIVE
            ..=offsets::PLAYER_FIELD_RESISTANCEBUFFMODSNEGATIVE + 6)
            .contains(&index)
}

/// Health hiding (`Object.cpp:775-797`): percent with at-least-1
/// rounding, f32 arithmetic as in C++.
#[allow(clippy::cast_precision_loss)] // C++ computes in float; big-HP rounding is the oracle behavior
fn health_word(words: &[u32], index: u16, can_see: bool, word: u32) -> u32 {
    if can_see {
        return word;
    }
    if index == offsets::UNIT_FIELD_MAXHEALTH {
        return 100;
    }
    let health = words[offsets::UNIT_FIELD_HEALTH as usize];
    if health == 0 {
        return 0;
    }
    let mut pct =
        ((health as f32 * 100.0) / words[offsets::UNIT_FIELD_MAXHEALTH as usize] as f32) as u32;
    if pct > 100 {
        pct = 100;
    }
    if pct == 0 {
        pct = 1;
    }
    pct
}

/// Side effects of block building that belong to the target
/// player/map layers.
#[derive(Clone, Debug, Default)]
pub struct BuildEffects {
    /// `target->m_visibleGobjQuestActivated[guid] = activate`
    /// writes (`Object.cpp:625-629`), in build order.
    pub go_quest_activated: Vec<(ObjectGuid, bool)>,
}

fn flags_and_visible(base: &ObjectBase, viewer: &Viewer) -> (&'static [u16], u16) {
    // C++ asserts the flags array exists (GetUpdateFieldFlagsArray
    // returns null only for TYPEID_OBJECT, which never builds
    // blocks); mirror with an expectation at the same point.
    let flags = update_field_flags(base.type_id()).expect("flags array for block-building type");
    let has_special = base.values().count() > offsets::UNIT_DYNAMIC_FLAGS
        && base
            .values()
            .has_flag(offsets::UNIT_DYNAMIC_FLAGS, UNIT_DYNFLAG_SPECIALINFO);
    (
        flags,
        visible_flags_for(base.type_id(), viewer, has_special),
    )
}

/// `BuildCreateUpdateBlockForPlayer` (`Object.cpp:260-320`).
pub fn build_create_block(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    data: &mut UpdateData,
    now_ms: u32,
    effects: &mut BuildEffects,
) {
    let mut update_flags = base.update_flag();
    let update_type = if base.is_new_object() {
        ObjectUpdateType::CreateObject2
    } else {
        ObjectUpdateType::CreateObject
    };
    if viewer.is_self {
        update_flags.add(ObjectUpdateFlags::SELF);
    }
    if base.is_type(TypeMask::UNIT) && view.melee_victim.is_some() {
        update_flags.add(ObjectUpdateFlags::MELEE_ATTACKING);
    }

    let (flags, visible) = flags_and_visible(base, viewer);
    let mut mask = base.values().create_bits(flags, visible);

    let buf = data.add_update_block();
    buf.write_u8(update_type as u8)
        .write_bytes(base.pack_guid())
        .write_u8(base.type_id() as u8);
    build_movement_update(view, buf, update_flags, now_ms);
    build_values_update(
        base,
        view,
        viewer,
        update_type,
        buf,
        &mut mask,
        &mut effects.go_quest_activated,
    );
}

/// `BuildValuesUpdateBlockForPlayer(data, target)`
/// (`Object.cpp:394-402`): emits nothing when the dirty mask is
/// empty for this viewer.
pub fn build_values_block(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    data: &mut UpdateData,
    effects: &mut BuildEffects,
) {
    let (flags, visible) = flags_and_visible(base, viewer);
    let mask = base.values().update_bits(flags, visible);
    if mask.has_data() {
        build_values_block_with_mask(base, view, viewer, data, mask, effects);
    }
}

/// `BuildValuesUpdateBlockForPlayerWithFlags` (`Object.cpp:404-411`).
pub fn build_values_block_with_flags(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    data: &mut UpdateData,
    flag: u16,
    including_empty: bool,
    effects: &mut BuildEffects,
) {
    let (flags, _) = flags_and_visible(base, viewer);
    let mut mask = UpdateMask::new();
    mask.set_count(u32::from(base.values().count()));
    base.values()
        .mark_fields_with_flag(&mut mask, flags, flag, including_empty);
    if mask.has_data() {
        build_values_block_with_mask(base, view, viewer, data, mask, effects);
    }
}

/// `BuildValuesUpdateBlockForPlayer(data, updateMask, target)`
/// (`Object.cpp:413-425`).
pub fn build_values_block_with_mask(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    viewer: &Viewer,
    data: &mut UpdateData,
    mut mask: UpdateMask,
    effects: &mut BuildEffects,
) {
    let buf = data.add_update_block();
    buf.write_u8(ObjectUpdateType::Values as u8)
        .write_bytes(base.pack_guid());
    build_values_update(
        base,
        view,
        viewer,
        ObjectUpdateType::Values,
        buf,
        &mut mask,
        &mut effects.go_quest_activated,
    );
}

/// `BuildMovementUpdateBlock` (`Object.cpp:250-258`): MOVEMENT
/// type + **unpacked** GUID + movement section.
pub fn build_movement_block(
    base: &ObjectBase,
    view: &ObjectView<'_>,
    data: &mut UpdateData,
    flags: ObjectUpdateFlags,
    now_ms: u32,
) {
    let buf = data.add_update_block();
    buf.write_u8(ObjectUpdateType::Movement as u8)
        .write_u64_le(base.object_guid().raw());
    build_movement_update(view, buf, flags, now_ms);
}

/// `BuildOutOfRangeUpdateBlock` (`Object.cpp:427`).
pub fn build_out_of_range(base: &ObjectBase, data: &mut UpdateData) {
    data.add_out_of_range_guid(base.object_guid());
}

/// `DestroyForPlayer` packet (`Object.cpp:438-445`); the caller
/// sends it on the target's session.
#[must_use]
pub fn destroy_packet(base: &ObjectBase) -> WorldPacket {
    WorldPacket::with_body(
        SMSG_DESTROY_OBJECT,
        base.object_guid().raw().to_le_bytes().to_vec(),
    )
}

/// `DirectSendPublicValueUpdate(index, count)`
/// (`Object.cpp:331-353`): returns the broadcast packet, or `None`
/// when nothing in the range actually changed. The mirror is
/// updated inline exactly as in C++ (range primed first, then the
/// mask build re-mirrors the same words; the priming write is
/// folded into the final sync here, which reaches the same
/// mirror state).
pub fn direct_public_value_update_range(
    base: &mut ObjectBase,
    index: u32,
    count: u32,
    compression: CompressionConfig,
) -> Option<WorldPacket> {
    let mut abort = true;
    for i in 0..count {
        let idx = (index + i) as usize;
        if base.values().mirror()[idx] != base.values().words()[idx] {
            abort = false;
        }
    }
    if abort {
        return None;
    }
    let mut mask = UpdateMask::new();
    mask.set_count(u32::from(base.values().count()));
    for i in 0..count {
        mask.set_bit(index + i);
    }
    direct_public_value_update(base, &mask, compression)
}

/// `DirectSendPublicValueUpdate(indexes)` (`Object.cpp:355-363`):
/// no mirror priming — the mask build mirrors each sent word.
pub fn direct_public_value_update_indexes(
    base: &mut ObjectBase,
    indexes: &[u32],
    compression: CompressionConfig,
) -> Option<WorldPacket> {
    let mut mask = UpdateMask::new();
    mask.set_count(u32::from(base.values().count()));
    for &index in indexes {
        mask.set_bit(index);
    }
    direct_public_value_update(base, &mask, compression)
}

/// `DirectSendPublicValueUpdate(UpdateMask&)`
/// (`Object.cpp:365-392`): raw words (no per-viewer rewrites),
/// mirror synced per sent word.
fn direct_public_value_update(
    base: &mut ObjectBase,
    mask: &UpdateMask,
    compression: CompressionConfig,
) -> Option<WorldPacket> {
    let mut data = UpdateData::new();
    {
        let buf = data.add_update_block();
        buf.write_u8(ObjectUpdateType::Values as u8)
            .write_bytes(base.pack_guid())
            .write_u8(mask.block_count() as u8);
        for &block in mask.blocks() {
            buf.write_u32_le(block);
        }
        let words: Vec<u32> = base.values().words().to_vec();
        for (index, &word) in words.iter().enumerate() {
            if mask.get_bit(index as u32) {
                buf.write_u32_le(word);
            }
        }
    }
    // Mirror sync (C++ does it inline per written word).
    for index in 0..base.values().count() {
        if mask.get_bit(u32::from(index)) {
            base.values_mut().sync_mirror_word(index);
        }
    }
    data.build_first_packet(false, compression)
}
