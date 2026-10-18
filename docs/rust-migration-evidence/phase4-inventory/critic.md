# Phase 4 inventory — completeness critic

Scope: all eight digests in this directory were read fully (4,256 lines
total: ai-engines 432, content-ffi 402, entities-updates 765,
map-visibility 565, movement 691, rust-gaps 420, spawn-data 594,
spells-auras 387). No digest is missing. Oracle C++ at HEAD `1bec69f`;
production `2bae5f9` treated per §3. Spot-checks below opened the cited
C++ bodies (not search hits) except where a count is the claim, in which
case the count was re-executed.

## 1. Spot-checks (load-bearing claims, 12 checks / 8 digests)

| # | Digest claim | Anchor opened | Verdict |
|---|---|---|---|
| 1 | 64 grids, 533.33333 grid, 16 cells, world/grid typelists | GridDefines.h:38-65 | CONFIRMED |
| 2 | Map::Update double UpdatePlayers + third in `!Instanceable()` barrier | Map.cpp:933-995 body | CONFIRMED (barrier condition is `!Instanceable()`, digest's correction is right) |
| 3 | HaveRealPlayers has exactly one caller (BattleBotAI) | Map.cpp:1778-1784 + tree grep | CONFIRMED |
| 4 | MovementInfo::Read order, cos before sin, jump latch | Object.cpp:61-103 body | CONFIRMED |
| 5 | baseMoveSpeed 2.5/7.0/4.5/4.722222/2.5/3.141594 | Unit.cpp:66-73 | CONFIRMED |
| 6 | PLAYER_END real 0x502 vs header comment 0x4FC; CONTAINER NUM_SLOTS 0x30 vs 0x02A | UpdateFields_1_12_1.cpp:30,293 vs .h:34,301 | CONFIRMED — header-comment trap is real; codegen must use .cpp rows |
| 7 | TOTAL_SPELL_EFFECTS=134, table ends at 131 (132/133 null) | SpellDefines.h:347 + SpellEffects.cpp:55,180-188 | CONFIRMED |
| 8 | TOTAL_AURAS=193 | SpellAuraDefines.h:332 | CONFIRMED |
| 9 | 93 script commands (0-92 + MAX), 37 EventAI types (0-36 + END) | ScriptCommands.h:335-343, CreatureEventAI.h:38-79 | CONFIRMED |
| 10 | No SmartScript/SmartAI anywhere | re-executed grep over src+sql: 0 hits | CONFIRMED |
| 11 | 441 AddSC is grep artifact; 212 active calls; 749 `new Script` | ScriptLoader.cpp parsed: 212 calls in AddScripts(); 749 allocations | CONFIRMED — prior inventory's 441 stays refuted |
| 12 | creature_template loader = 78 cols, max(patch<=WowPatch) | ObjectMgr.cpp:1173-1195 SELECT (indices 0-77) | CONFIRMED |
| 13 | Rust: login sets has_player=true, in_world=false; only CHAR_ENUM/PLAYER_LOGIN branch, rest stub | world_actor.rs:680,752-758,825 | CONFIRMED |
| 14 | MapUpdateInterval capped to 50ms regardless of config | MapManager.h:115-122 | CONFIRMED |

No load-bearing check was REFUTED on its main anchor. The REFUTED items
in §2 are secondary counts where digests disagree with each other.

## 2. Contradictions between digests (resolved against source)

1. **Script-hook counts — all three digests differ, two are wrong.**
   Source (ScriptMgr.h:138-214): SpellScript = 14 hooks + dtor = 15
   virtuals; AuraScript = 15 hooks + dtor = 16 virtuals.
   - spells-auras: "14 hooks" SpellScript CONFIRMED; "13 hooks"
     AuraScript REFUTED (omits OnHolderInit, OnAuraInit,
     OnAuraValueCalculate, OnDurationCalculate, OnPeriodicTrigger,
     OnPeriodicDummy, OnAreaAuraCheckTarget from its enumeration).
   - content-ffi: "17 virtuals / 14 virtuals" REFUTED both.
   - ai-engines: "~14/~17" REFUTED (numbers appear swapped/inflated).
   Impact: low for sizing, but any FFI vtable shim built from the
   content-ffi/ai-engines counts would have wrong slot counts. Use
   14/15 hooks.
2. **Script record pointer count.** Source (ScriptMgr.h:216-251):
   20 plain + 5 factory = 25 function pointers. content-ffi
   CONFIRMED; ai-engines "21 function pointers" REFUTED (undercount).
3. **CreatureAI virtual count.** Source: 37 `virtual` incl. dtor
   (CreatureAI.h). content-ffi "~35" acceptable; ai-engines "~45"
   REFUTED (overcount). AI trait sizing should use 36 hooks.
4. **mangos-movement line count.** movement.md "2,140 lines" vs
   rust-gaps "1,673 lines" — both CONFIRMED once decomposed: 1,673
   src + 467 tests = 2,140. Not a contradiction, but neither digest
   states the other's basis in the same sentence; a reader summing
   crate sizes will double-count or under-count. State src/test split.
5. **Production-divergence confidence.** rust-gaps proves (token
   stream) Map/Object/Spell.cpp are formatting-only vs 2bae5f9, and
   movement.md proves the same for the MovementInfo codec. But
   spawn-data (ObjectMgr ~3,953 churn lines), ai-engines and
   content-ffi (ScriptMgr -w 80/120, 17/24 not inspected) explicitly
   make NO parity claim, and rust-gaps itself flags 431 uninspected
   files plus semantic commits 25ae6c39/6f082201 (formation/combat-AI)
   outside the sample. Digests that phrase HEAD as "the oracle"
   without the rust-gaps caveat (spells-auras header, ai-engines
   header) overstate by omission: HEAD is a proven oracle only for
   Map/Object/Spell logic + movement codec. Formation/AI-tuning areas
   are known-divergent until sampled — this directly affects the
   ai-engines and content-ffi (Deadmines) surfaces, whose testing
   commits are exactly 6f082201/25ae6c39's neighbourhood.
6. **UpdateData coverage.** map-visibility §f lists UpdateData wire
   format as unresolved/not opened, but entities-updates §A.6 did open
   UpdateData.cpp:43-243 (split, compression, out-of-range). The
   map digest's unresolved item is therefore already covered by a
   sibling; similarly its "spawn-table schema behind GetCellObjectGuids"
   is covered by spawn-data. Cross-references are missing — a planner
   reading only map-visibility will schedule duplicate work.

## 3. Cross-surface dependency / order errors

1. **AI slice 2 needs movement M4, not M3.** ai-engines slice step 2
   (BasicAI proximity aggro + melee) lists "chase generator +
   evade-to-home" as entry deps., but movement.md defers Chase/Follow
   to M4 ("combat/script coupling"), after M3 waypoint/point. BasicAI
   without chase can aggro but cannot close to melee — the slice as
   written is not completable in the movement order. Either promote a
   minimal chase into M3 or downgrade AI slice 2 to "aggro + stationary
   melee only" and say so.
2. **Unit::Update ownership gap (spells × map × movement).** Map tick
   calls ObjectUpdater → UpdateRealTime per object (map-visibility),
   inside which C++ runs spline tick (movement A5), cast timers and
   aura PeriodicTick (spells-auras), and AI UpdateAI (ai-engines a.8).
   No digest's smallest slice explicitly owns the *intra-object* order
   (spline vs cast-interrupt-by-movement vs aura tick vs AI). Spells
   slice says "cast timer in map update" without naming the slot;
   movement-interrupt (>0.5u, Spell.cpp:4128) is order-sensitive
   against spline/relocation in the same tick. Assign one owner
   (entities Unit update) and have the other three reference it.
3. **Movement M1 framing inverts its handler deps.** M1 is presented
   as codec+relay in mangos-movement, but its own entry deps require
   mover resolution, m_movementInfo storage, map coord bounds and
   session reject-time state (entities + map + rust-gaps steps 1-4).
   The pure codec can land first; the relay handler cannot. rust-gaps'
   order (handlers step 5, after object model/map/queues) is correct;
   M1 should be split M1a codec (now) / M1b handler (after map slice).
4. **Tick-rate reconciliation missing.** Rust world actor ticks at
   50ms (rust-gaps); C++ manager interval is capped to 50ms but
   UpdatePlayers/UpdateCells/packets sub-limit at 100ms
   (map-visibility §a, check #14). Content-ffi correctly warns all
   script/AI timers are diff-driven, and EventAI polls at 500ms
   accumulated diff (ai-engines). No digest states what `diff` a Rust
   port passes if it keeps the 50ms actor tick *and* ports the 100ms
   sub-limiters: naively doing both halves AI/spell-timer rates or
   doubles them depending on accumulation. This must be settled before
   the map slice, not after AI/spells land.
5. **Empty-inventory assumption vs real login.** entities slice defers
   Item/Bag ("self-create with empty inventory emits nothing") while
   its own §A.6 shows Player create emits owned items *before* the
   player block, and rust-gaps login completion requires the initial-
   packets train for real characters. The entities slice is valid only
   for fixture characters; the first real-DB login (spawn-data/rust-gaps
   slices) needs at least read-only Item creates. State fixture-only
   explicitly in the slice goal.
6. **Pooled-GO withholding vs "see static GOs" slices.** spawn-data:
   29,478/56,653 GO spawns are pool-managed and withheld from the grid
   index; its slice defers PoolManager spawning. entities/rust-gaps
   slices promise visible static gameobjects in a start area. In
   herb/ore-heavy cells the combined result is "creatures correct,
   nodes absent" — spawn-data does state this, the other two do not
   repeat the caveat. Acceptance tests must pick non-pooled GO fixtures
   or the slices will appear to fail each other.
7. **Visibility stub stacking.** map slice stubs IsVisibleForInState=true
   (stealth/GM/phase ignored) and entities slice grants GROUP_ONLY
   never / starts from "everyone sees everything" in its step 7 if taken
   literally alongside the map slice. Combined first slices would leak
   stealthed/private state that neither slice alone claims to leak.
   Harmless for a fixture map, but the two stubs must be retired
   together before any multi-client sniff test is treated as parity.
8. **ThreatHost has no non-test implementor (verified).** ai-engines
   and rust-gaps both assume mangos-threat is "ready, needs host impl";
   grep confirms the only impl is tests/behaviour.rs FakeHost. The
   12-method host (attackability, chase geometry, faction, reach) is
   unscoped work sitting between threat (done) and BasicAI (slice 2) —
   it belongs in the entities/Unit slice's entry deps, where neither
   digest lists it by name.
9. **Content-ffi "spells first" alternative contradicts its own deps.**
   Transpiling spells/ scripts first is suggested because hooks are
   regular, but the same section notes it needs live Spell/Aura objects
   — i.e. it is *last* among script graduations in dependency order,
   after the spells-auras slice. The suggested order at the end
   (Deadmines → instances → gossip → spells → raids → BG) is the
   correct one; the "spells first" bullet should be read as smallest,
   not earliest.
10. **Opcode-count bases differ silently.** movement: 41 StoreOpcode
    entries, 36 carrying MovementInfo; rust-gaps processing census:
    Movement class = 39. These count different things (handler entries
    vs registry processing class incl. worldport-ack World/Transfer),
    but no digest reconciles them. Handler-work estimates should use
    movement's 41/36; queue-design estimates rust-gaps' 39.

## 4. Per-digest completeness (smallest-slice / unresolved present?)

All eight digests contain both a smallest-slice (§e) and an unresolved
section (§f). None is missing either section. Quality notes:

- map-visibility: complete; two §f items already covered by siblings
  (§2.6 above) — add cross-refs.
- movement: complete; M1 split needed (§3.3); generator bodies honestly
  listed as not line-read.
- entities-updates: most complete digest; fixture-only caveat missing
  (§3.5); prod audit §f is partially superseded by rust-gaps token
  proof for Object.cpp — update to reference it.
- spells-auras: complete but thinnest (387 lines for 33k LOC surface);
  CheckCast/FillTargetMap/absorb-resist unread is the largest single
  unknown in the whole inventory — slice correctly routes around it.
  AuraScript hook count wrong (§2.1).
- ai-engines: complete; counts in §2.1-2.3 wrong; DB row-count triage
  for condition/command subsets is unresolved and blocks §e step 4
  sizing (needs one DB query pass, cf. spawn-data's executed counts).
- content-ffi: complete; strongest single refutation in the set (441).
  Hook counts wrong (§2.1). Deadmines respawn_delay sentinel finding
  (43199/43201/43202) is load-bearing and has no counterpart check in
  spawn-data — spawn slice must preserve those columns verbatim.
- spawn-data: complete; only digest with executed DB counts. Prod
  divergence honestly unclaimed; needs the token-check treatment
  rust-gaps applied to Map/Object/Spell before parity claims.
- rust-gaps: complete; token proof is the inventory's strongest
  divergence evidence but covers 3/434 files — its own §f says so.
  socket.rs BuildAddonPacket stale-comment question remains open and
  blocks trusting header comments elsewhere in mangosd.

## 5. Verdict

Inventory is usable for slice planning. Main anchors survived direct
body inspection (14/14 checks in §1). Fix before estimating from it:
(a) script/AI counts in §2.1-2.3, (b) AI↔movement chase ordering and
Unit::Update ownership (§3.1-3.2), (c) tick/diff model (§3.4),
(d) restrict "HEAD = production oracle" to the token-proven files
until the tree-wide token check and ObjectMgr/ScriptMgr samples run.

