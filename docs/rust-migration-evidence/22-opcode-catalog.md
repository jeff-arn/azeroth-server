# Opcode payload catalogue skeleton (Phase 0, §3.5 / item #35)

Artifacts (all under `/tmp/muse-phase0/`):

- `gen_opcode_catalog.py` — the generator (re-runnable, read-only on repo).
- `22-opcode-catalog.csv` — one row per `StoreOpcode` registration (826 rows).
- This file — method, count reconciliation, manual checks, known limits.

## 1. Method

1. **Parse the registration table.** Regex over
   `vmangos/src/game/Server/Protocol/Opcodes.cpp:47-915`
   (`Opcodes::BuildOpcodeList`), capturing the `/*0xNNN*/` comment as the
   numeric value, enum name, string name, `SessionStatus`, `PacketProcessing`,
   handler name, and the innermost active `#if` guard per row. Two
   comment-tolerant fixes were needed (see §3).
2. **Resolve numeric values.** Cross-checked against the enum in
   `vmangos/src/game/Server/Protocol/Opcodes_1_12_1.h:1-830`. The header
   assigns every value explicitly (`MSG_NULL_ACTION = 0` … `NUM_MSG_TYPES`),
   with one deliberate gap: `0x33A` (826) is skipped and
   `SMSG_DEFENSE_MESSAGE = 827` (`Opcodes_1_12_1.h:828`), matching the table's
   jump from `/*0x339*/` to `/*0x33B*/` (`Opcodes.cpp:912-913`).
3. **Locate each handler.** Search `vmangos/src/game/**/*.cpp|*.h` for
   `void WorldSession::<Handler>(WorldPacket& <pkt>)`, preferring `.cpp`
   definitions over `WorldSession.h` declarations. File:line recorded per row.
4. **Extract the read sequence.** Brace-match the handler body from its
   opening `{`, then regex-scan in source order:
   `pktvar >> var` chains (each `>>` target one step, casts stripped,
   call-expressions kept opaque as `call(...)`), `read<T>` / `read_skip<T>` /
   `readPackGUID` / `ReadCString` / `ReadGuid` method reads, and
   `X.ReadForCaster(...)` target proxies. Variable types come from
   declaration statements in the same body (comments stripped first).
   Loop/conditional enclosure is flagged per read by mapping
   `for`/`while`/`if`/`switch` brace ranges (plus braceless single-statement
   bodies) onto read offsets.

## 2. Count reconciliation (census: 826 / 276 / 272 / 525)

| Measure | Census | Parser | Delta |
|---|---|---|---|
| `StoreOpcode` registrations | 826 | 826 | 0 |
| Distinct handlers | 276 | 276 | 0 |
| Non-stub handlers | 272 | 272 | 0 |
| `STATUS_NEVER` rows | 525 | 525 | 0 |
| Handlers with no definition found | — | 0 | — |
| Enum value mismatches | — | 0 | — |

Zero delta, but only after three parser fixes (§3) — a naive first pass gave
825 / 275 / 271 with 52 handlers "missing", all parser artifacts, not repo
facts. Supporting figures from this run: 96 rows carry a `#if
SUPPORTED_CLIENT_BUILD` guard, all true at the default build
(`SUPPORTED_CLIENT_BUILD = CLIENT_BUILD_1_12_1`,
`vmangos/cmake/options.cmake:11`, so the full 826 compile in); 524 rows map
to the 4 stub handlers (`Handle_NULL`, `Handle_ServerSide`,
`Handle_Deprecated`, `Handle_EarlyProccess`,
`vmangos/src/game/Server/WorldSession.h:436-439`); of the 302 non-stub
rows, 248 have ≥1 extracted read and 54 have none (handlers that ignore the
packet, e.g. `HandleAttackStopOpcode`, `CombatHandler.cpp:67`); 33 rows have
loop-enclosed reads, 264 conditional-enclosed reads; 47 rows contain a `?`
(unresolved-type) read step.

## 3. Parser fixes applied (why the naive pass undercounted)

1. `Opcodes.cpp:654` embeds a `/*COD*/` comment between the processing arg
   and `&WorldSession::HandleMailTakeItem` — the 826th row. Regex now skips
   `/*…*/` there.
2. ~52 handlers declare the packet param as commented-out
   (`WorldPacket& /*recv_data*/`, e.g. `CharacterHandler.cpp:164`,
   `CombatHandler.cpp:67`), plus one double-star variant (`WorldPacket&
   /**recv_data*/`, `MailHandler.cpp:923`). Definition regex now accepts an
   absent or `/*…*/`-commented name with a common-name fallback.
3. Bare `>> var` targets were mis-split by a two-group cast pattern
   (`name` → type `nam`, var `e`); bare identifiers now go straight to
   declaration lookup. `ReadForCaster` was double-counted (once as a `>>`
   call target, once as a proxy); the proxy entry is now skipped when
   subsumed. `//` comments inside multi-line declarations leaked into
   inferred types; declarations are comment-stripped first.

## 4. Manual end-to-end checks (all agree)

1. **`CMSG_CHAR_CREATE → HandleCharCreateOpcode`**
   (`CharacterHandler.cpp:181`). Body reads: `name; race_; class_;`
   `gender >> skin >> face;` `hairStyle >> hairColor >> facialHair >>
   outfitId;` (lines 185-190). CSV: 10 reads —
   `std::string name | uint8 race_ | uint8 class_ | uint8 gender |
   uint8 skin | uint8 face | uint8 hairStyle | uint8 hairColor |
   uint8 facialHair | uint8 outfitId`. **Agree** (order, count, types).
2. **`CMSG_MESSAGECHAT → HandleMessagechatOpcode`**
   (`ChatHandler.cpp:149`). Body reads in order: `type`, `lang`
   (152-153), then per-`switch(type)`: `channel` + `ReadCString(msg,msgLen)`
   (CHAT_MSG_CHANNEL), `to` (WHISPER), `ReadCString(msg,msgLen)` (say/yell/…),
   `ReadCString(msg,msgLen)` (AFK/DND). CSV: 7 reads, last 5 flagged
   `[cond]`. **Agree** (count, order, conditional flags).
3. **`CMSG_CAST_SPELL → HandleCastSpellOpcode`**
   (`SpellHandler.cpp:286`). Body: `recvPacket >> spellId;` (288) then
   `recvPacket >> targets.ReadForCaster(_player);` (306) after validation
   with the `rpos(wpos())` ignore-packet idiom. CSV: 2 reads —
   `uint32 spellId | call(targets.ReadForCaster(_player))`,
   `rpos_skip=True`. **Agree**, including the early-return/skip note.

## 5. Known extraction limits (what the skeleton does NOT yet capture)

- **Reads hidden in helpers.** Anything behind `targets.ReadForCaster`,
  movement-packet `ReadMovementInfo`-style helpers, or `ByteBuffer`
  template helpers is one opaque `call(...)` step — field-level order inside
  is not expanded. This is the largest gap for spells, movement, and trade.
- **Template/container reads.** `packet >> myVector` / `>> myMap`
  (`ByteBuffer.h:504-556`) deserialize N elements with no visible loop; the
  row shows one step with no `[loop]` flag.
- **Early-return control flow.** `body_notes` counts bare `return;` and the
  `rpos(wpos())` skip idiom, but does not record *which* reads are guarded
  by *which* validation check — a Rust port must re-derive validation order
  by reading the bodies.
- **`read_skip<T>` / padding** is recorded as a step but carries no byte
  width beyond the `<T>` parameter.
- **47 rows contain `?`-typed steps** (targets of `>>` with no visible
  declaration — typically member accesses, macro-produced lvalues, or
  re-reads of globals). Each needs a human look before codegen.
- **54 non-stub handlers with zero reads** are genuine no-payload handlers
  (verified by sampling), not extraction failures — but the remaining
  `?`-typed rows should be triaged the same way before trusting any row as
  a complete payload spec.
- **Brace-matching approximation.** The extractor does not preprocess
  `#if` blocks inside bodies or multi-line `for` headers beyond a blanking
  regex; exotic formatting could misattribute a `[loop]`/`[cond]` flag.
  Offsets were sanity-checked on the 3 manual handlers only.

## 6. Reuse for the Rust port

Rerun: `python3 /tmp/muse-phase0/gen_opcode_catalog.py [out.csv]`
(read-only; no repo writes). Suggested next slice (not done here): expand
the ~10 highest-traffic `call(...)` helpers (movement, spell targets) into
field lists, and hand-clear the 47 `?` rows — that turns this skeleton into
a codegen-ready payload spec.


## 7. Phase 3 triage of the 47 `?` rows (2026-10-17)

All 47 rows resolved to concrete field lists by reading the
handler bodies; no row was an extraction mystery:

- **Movement family (36 rows)** — `HandleMovementOpcodes` (22),
  `HandleForceSpeedChangeAckOpcodes` (6),
  `HandleMovementFlagChangeToggleAck` (3), `HandleMoveRootAck`
  (2), `HandleMoveTeleportAckOpcode`, `HandleMoveKnockBackAck`,
  `HandleMoveSplineDoneOpcode` (1 each): each `?` is one
  `MovementInfo` read (`recvData >> movementInfo`,
  `Object.cpp:61-103`): `moveFlags u32, ctime u32, pos.x/y/z/o
  f32`, then conditional blocks — ONTRANSPORT: `t_guid u64,
  t_pos.x/y/z/o f32`; SWIMMING: `s_pitch f32`; `fallTime u32`;
  JUMPING: `zspeed, cosAngle, sinAngle, xyspeed f32` (cos is
  read before sin); SPLINE_ELEVATION: `splineElevation f32`.
  SplineDone appends `splineId u32` + `Unused<float>`. This
  parser is NOT yet ported (Phase 2's `mangos-movement` covers
  spline math/packet building, not `MovementInfo`); it is the
  first item of Phase 4 movement ingestion, now with the exact
  field list.
- **`CMSG_WHO`** — task members are all `uint32`
  (`MiscHandler.cpp:74`): levelMin, levelMax, raceMask,
  classMask, zonesCount (loop of u32 zone ids, client cap 10),
  strCount (loop, cap 4), plus the two plain strings.
- **`CMSG_PET_SET_ACTION`** — `petGuid` then a size-derived
  count (packet size 24 → 2, else 1) of (`position u32`,
  `data u32`) pairs; the `?` was the size-derived count itself.
- **`CMSG_ACTIVATE_TAXI`** — guid + `nodes[0..2]` u32 pair
  (vector element lvalues).
- **`CMSG_TRAINER_BUY_SPELL`** — guid + `spellId u32`
  (initialized local shadowed the extractor's declaration scan).
- **`CMSG_SEND_MAIL`** — mailbox guid, three `req->` member
  strings (receiver/subject/body), two u32 unknowns,
  `req->itemGuid u32`, `req->money u32`, `req->COD u32`
  (`Mail.h:297,334,336`), then u64 + u8 consts.
- **`CMSG_BATTLEFIELD_LIST` / `CMSG_LEAVE_BATTLEFIELD`** —
  single `mapId u32` each.
- **`CMSG_BATTLEFIELD_PORT`** — at >1.8.4: `mapId u32,
  action u8`; the catalog's double `action` read is the
  `#else` branch conflated by the non-preprocessing extractor.
- **`CMSG_AUCTION_LIST_ITEMS`** — `AuctionListItemsTask`
  (`AuctionHouseMgr.h:96-104`): listfrom u32, name string,
  levelmin u8, levelmax u8, slot/main/sub/quality u32 x4,
  usable u8.
- **`CMSG_BUYBACK_ITEM`** — vendor guid + `slot u32`.
- **`CMSG_SET_WATCHED_FACTION`** — `repId i32` (signed; -1
  clears the watched faction).

The CSV itself is unchanged (generator output); this appendix
is the hand-clearing the catalog asked for.
