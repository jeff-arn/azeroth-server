# Parity long-tail inventory — review draft

Compiled 2026-11-23 at `ee21c139` (plus in-flight Warlock work) for
owner completeness review. Sources: spell-campaign artifacts
(`docs/rust-migration-evidence/spell-campaign/`), the canary waiver
list (`TIER4-CANARY-RUNBOOK.md`), session-evidence records
(`TIER4-PROGRESS.md`), and in-code remainder markers. Status keys:
**DONE** / **IN PROGRESS** / **QUEUED** / **SEAM** (needs an engine
or harness seam first) / **OWNER** (needs an owner decision or
credential). Sizes are rough: S < a day, M = days, L = a week+.

## 1. Spell fan-out remainder

Method per class: generated inventory (G1 trainable union + one
legal 51-point talent build), seed all ranks, cast first + last +
signature-change ranks over the real wire, verdict only on
observable outcomes (never `SPELL_GO` alone), fix shared roots
test-first. Counts below are cast rows / verdicts.

| Class | Build | State |
|---|---|---|
| Paladin | ret | DONE — 84 rows: 45 PASS, 11 PASSIVE-VERIFIED, 28 BLOCKED, 0 FAIL |
| Druid | balance | DONE — 63 rows: 40 PASS, 15 PASSIVE-VERIFIED, 8 BLOCKED, 0 FAIL |
| Warlock | smruin | DONE — 113 rows: 62 PASS, 14 PASSIVE-VERIFIED, 37 BLOCKED, 0 FAIL |
| Hunter | mm | DONE — 80 rows: 53 PASS, 13 PASSIVE-VERIFIED, 14 BLOCKED, 0 FAIL |
| Shaman | enhance | DONE — 93 rows: 27 PASS, 15 PASSIVE-VERIFIED, 51 BLOCKED, 0 FAIL (totem rows classify-only, JEF-20) |
| Warrior | fury | DONE — 85 rows: 54 PASS, 19 PASSIVE-VERIFIED, 12 BLOCKED, 0 FAIL |
| Rogue | combat | DONE — 59 rows: 34 PASS, 15 PASSIVE-VERIFIED, 10 BLOCKED, 0 FAIL |
| Priest | shadow | DONE — 113 rows: 88 PASS, 13 PASSIVE-VERIFIED, 12 BLOCKED, 0 FAIL |
| Mage | frost | DONE — 100 rows: 57 PASS, 16 PASSIVE-VERIFIED, 27 BLOCKED, 0 FAIL |

Warrior/Mage also have 2-row pilot coverage each; Rogue/Priest have
none yet. Per-class cost is dominated by real-time bench runs
(30–90 min each, several per class) and the fix loop.

**Second sweep (after all nine first passes):**

- *Build rotation* (M): each class currently audits one of 2–4
  builds. Talent spells unique to unselected trees are never cast
  (e.g. Paladin ran ret; holy/prot talent actives unseen). Rotate
  remaining builds per class, casting only newly unlocked actives.
- *Passive differential* (M): passive talents are verified as
  learned, not as working. Seed with/without and compare fields,
  procs, and stat outcomes.
- *Modifier isolation* (M): Improved-X talents run inside wide
  oracle bands; cast with/without to measure the delta itself.
- *Rank residuals* (M): interior same-signature ranks stay uncast
  by design; aura rank bounce/replacement and downranking
  (`SelectAuraRankForLevel`) need dedicated scenarios; per-rank
  mana costs are effectively untested (fixtures seed huge pools).
- *DoT depth* (S–M): full-duration tick counts/cadence, expiration
  cleanup, dispel interaction — partially covered from Warlock on
  via per-tick bands + rank-1 series; backfill Paladin/Druid DoTs.

## 2. Spell-engine seams (blockers behind today's BLOCKEDs)

- **RESURRECT three-layer** (M): **DONE** (JEF-17, deployed) —
  corpse cast targets, percent/flat effects, offer flow,
  `CMSG_RESURRECT_RESPONSE`; Resurrection verdict PASS in the
  Priest run.
- **Reagent gate** (M): **DONE** (JEF-18) — `CheckItems` reagent/
  totem arm + `TakeReagents` consumption in the cast path, harness
  `seed_reagents` grant; 42 blocked rows across five classes re-run
  to PASS in subsets. Landed alongside: directional-location cast
  arm (Portals), `TARGET_UNIT_RAID_AND_CLASS` (61) fill, and the
  percentage-cost create-pool fix.
- **Script-effect indexing (G4)** (L, SEAM): `SCRIPT_EFFECT` /
  script-named rows ride classify-only until each script body gets
  an observable oracle. Paladin 9, Druid small, Warlock 2 so far;
  grows with Priest/Mage script density.
- **Totem engine completion** (M–L): **DONE** (JEF-20) — slot
  summon/lifecycle, passive self-cast, active `TotemAI` subset;
  Shaman totem rows verdict 39/39 in subset
  (`results-Shaman-totems.jsonl`).
- **Party scenario** (S): `no-party` rows (party-target blessings,
  rituals) need a real invited party in the harness; two-client
  machinery exists from earlier probes.
- **Seal/aura exclusivity** (M): aura dedup is (spell, caster) only;
  same-family strongest-aura replacement and seal swapping are
  unmodeled (seals can stack; Judgement then consumes an arbitrary
  one). Affects Paladin correctness beyond single-seal casts.
- **Holder script state / GDR** (M): aura-holder state storage for
  scripted auras deferred earlier (periodic behavior regression
  risk); blocks a tail of script auras.
- **Selected-target accessor, stealth/invisibility detection, GO
  visibility, holder cast-item GUID** (S–M each): recorded
  spell-seam remainders from the Tier 4 inventory; surface as
  individual class/content rows demand them.
- **Script target bounds filtering** (S): `spell_script_target`
  bounds noted unmodeled in target collection.
- **Effect-arm discoveries per class** (ongoing): each class so far
  found 1–2 (HEAL_MAX_HEALTH, combo gate, channel cancel). Budget
  assumes similar per remaining class; they cannot be enumerated
  in advance — that is the point of the campaign.

## 3. Script/content helper remainders

- **1,195 `NEEDS-MAP-HELPER` references** (grep census 2026-11-23;
  1,158 in `content.rs` alone, rest in split content modules,
  trainer, area triggers). Many are interim equivalents rather
  than dead code; burn down in ranked clusters, L total.
  Named clusters queued first: Execute-Dummy `OnCast` route,
  `SummonGameObject`, GO respawn scheduling, visibility
  remainders, script gossip `PrepareQuestMenu` sites.
- **Instance/content lanes not yet extracted** from the monolith
  modules (see §5): behavior ports ride along as lanes are split.

## 4. Systems and admin surfaces

- **GM tickets** (M): surface reduced vs fork; needs table
  validation + handlers. Was migration-gated; no longer blocked.
- **Movement anticheat completeness** (M): movement detectors
  exist; non-movement detectors and the ban-bit policy were left
  at "reassess later."
- **Warden** (L, OWNER): not implemented; module loading is a
  security-surface decision, not just parity work.
- **RA / SOAP / console remote admin** (M each, OWNER): never
  ported. Network listeners need an explicit owner go (security).
- **`DynamicRespawn`** (S): off unless droplet config enables it;
  becomes a blocker, not a waiver, if ever enabled.
- **Fork-parity loot holes** (S, OWNER): autostore does not
  re-check loot permission; repeat roll votes count per call.
  Replicated verbatim for parity — keep, or approve the
  safe-direction deviation.
- **Config-key parity tail** (M): droplet non-defaults in the
  "behavior behind fork-default constants" class (remaining
  `Rate.*`, creature-AI scalars, chat flood/min-levels,
  durability, environmental/mirror timers, corpse/bones timers,
  visibility modes, `Spell.EffectDelay`) need a final diff pass
  now that config changes are unblocked.

## 5. Tooling, generated artifacts, code health

- **Lost extractors** (M): `commands_table.rs` and
  `class_spells_table.rs` generators were lost; both files are
  frozen until extractors are recreated and committed. Caps GM
  command and class-spell data updates.
- **`content.rs` decomposition remainder** (L): still ~100k lines
  after the Tier 4 splits; remaining content lanes and
  `world_actor` handler clusters to extract, behavior-neutral,
  one seam per commit.
- **Comment/provenance cleanup remainder** (S): the refactor pass
  covered the worst C++-referencing comments; a tail remains in
  unextracted modules.
- **G3 `IsSpellValid` full replay** (S): generator caveat; cast
  sets rely on `spell_disabled` + derivation asserts instead.
- **G5 trainer equivalence** (S): trainer spell data verified
  bench-only; production trainer rows never diffed.

## 6. Ops and owner-held items

- **Deploy cadence** (ongoing): per-class/per-slice Ubuntu builds
  + droplet swaps now pre-authorized; rollback pairs preserved
  per swap (`mangosd.a8c3-bak` etc.), C++ `.cpp-bak` pair intact.
- **C++ fallback retirement** (OWNER): formal cutover decision —
  when to delete the C++ binaries/build tree and stop treating
  rollback-to-C++ as the safety net.
- **Discord bot token** (OWNER): `azeroth-discord-bot` is in a
  long-standing crash loop (`LoginFailure: Improper token`,
  ~298k restarts observed 2026-11-23). Needs a fresh token.
- **Credential rotation** (OWNER): droplet DB password appeared
  in an earlier session transcript; bench DB password was printed
  into workflow tool output three times during the Paladin
  campaign. Both should rotate.
- **Real-client smoke depth** (ongoing): wire probes cover the
  protocol; periodic real-client passes (group loot roll,
  persistence) remain owner-side evidence per the runbook gates.

## 7. Close-out (goal exit criteria)

1. Requirement-by-requirement audit of this inventory plus the
   recorded backlog (`TIER4-PROGRESS.md`): every item DONE with
   evidence, or explicitly carried as a named remainder with an
   owner or an unblocker.
2. Full gates on the final tree: `ci.sh` (fmt, clippy `-D
   warnings`, workspace tests), live suites under their env gates,
   both spell self-tests, pilot + every class gate green.
3. Docs/runbook/memory updated to the final state; droplet on the
   final binaries with hashes recorded; work committed and pushed.

## Decisions needed from the owner

1. Network admin surfaces (RA/SOAP/console, Warden module
   loading): build, or formally waive?
2. Fork-parity loot holes: keep verbatim or fix in the safe
   direction?
3. C++ fallback retirement timing.
4. Discord bot token + DB credential rotations (owner-issued
   secrets; I can apply them once provided, never print them).
5. Anything on this list that should be explicitly *dropped*
   from parity scope.
