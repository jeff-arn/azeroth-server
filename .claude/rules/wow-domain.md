# WoW vanilla 1.12.1 domain rules

VMaNGOS targets the World of Warcraft 1.12.1 client (the final patch before The Burning Crusade). Mechanics you remember from retail, Wrath, or even TBC are often wrong here. This file lists the parts that bite — read it before "fixing" something that looks weird.

## Era and scope

- Client version: **1.12.1** (build 5875). No TBC, no Wrath, no Cata, no anything later.
- Level cap: **60**. Talent points: 51 at 60 (1 per level from 10–60, plus 1 from a quest at... no, just 51).
- Classes: 1–11 — Warrior, Paladin (Alliance only), Hunter, Rogue, Priest, Shaman (Horde only), Mage, Warlock, Druid. **No Death Knight, no Monk, no Demon Hunter, no Evoker.**
- Races: Alliance (Human, Dwarf, Night Elf, Gnome), Horde (Orc, Undead, Tauren, Troll). **No Blood Elf, no Draenei, no Worgen, etc.**
- Faction restrictions: Paladin = Alliance only; Shaman = Horde only. Don't write code that assumes paladins on the Horde or shamans on the Alliance — it crashes everything from talent application to spell teaching.
- No dual-spec. No LFG / Dungeon Finder. No achievements. No transmog. No void storage.

## Spell, talent, and skill systems

- Spell IDs come from `DBC` files (extracted client data). Don't invent spell IDs. Look them up in `Spell.dbc` (loaded by `SpellMgr`) or use `wowhead-classic` for cross-reference.
- `SpellEntry` is the data side; `SpellInfo` (where it exists) wraps it. Read the existing call sites before guessing.
- `TalentTab.dbc` defines the three trees per class. `Talent.dbc` defines ranks. The 26 PvE builds in the bot system are generated from these via `scripts/generate_talent_builds.py`.
- `SkillLineAbility.dbc` maps spells to skill lines. The bot spell-population logic iterates this — see `.claude/rules/bots.md`.
- Trainer spells: classes learn most non-talent spells from a trainer at specific levels. `LearnClassSpellsForLevel` simulates this for bots.
- Specs are emergent from talent point distribution. Don't bake "spec" into anywhere it doesn't already exist — the data model is talent points, not a spec enum.

## GUID system

`ObjectGuid` is a 64-bit value with two parts:
- High GUID — type tag (`HIGHGUID_PLAYER`, `HIGHGUID_UNIT`, `HIGHGUID_ITEM`, `HIGHGUID_GAMEOBJECT`, `HIGHGUID_CORPSE`, `HIGHGUID_GROUP`, etc.)
- Low GUID — counter within the type space

Implications:
- Player GUIDs and creature GUIDs are **not** interchangeable. Code that does `Unit* u = ObjectAccessor::FindUnit(guid)` is fine — the accessor disambiguates. Code that assumes "this is a player" because the number is small is wrong.
- `ObjectGuid` is a value type. Pass by value or `const&`. Cheap to copy.
- `ObjectGuid::IsEmpty()` is how you check for "no target". Don't compare against `0` directly.

## Opcodes and packets

- Opcodes (`SMSG_*` server-to-client, `CMSG_*` client-to-server) are defined in `Opcodes.h`. Handler tables in `Opcodes.cpp` map opcode → method on `WorldSession`.
- Packet structure is documented in code comments and in the long-dead-but-archived wiki references (Mangos wiki, old WoWDev). Never assume a packet shape — read the existing handler and the server-side write site.
- `WorldPacket` builds and parses packets. Big-endian and little-endian rules differ per field — copy the surrounding code's pattern.
- **Do not change packet shapes.** A handful of custom opcodes (e.g. addon telemetry) are okay, but anything the 1.12.1 client sends or receives has a fixed shape baked into the client binary.

## Combat and stats

- Defense, weapon skill, and resistance work via the **combat table** (miss / dodge / parry / block / glancing / crit / hit). Read `Unit::CalculateMeleeOutcome` before changing combat math.
- "Hit cap" is 9% for special attacks against a level-60 target with full 5-skill-difference. Hit chance is capped by the combat table — the cap is not configurable per spell in vanilla.
- Threat: `ThreatManager` and `HostileReference`. Tank threat is **5× damage** (warriors with Defensive Stance + Defiance get 1.3×). Healers generate threat split across the engaged mobs.
- Resilience does not exist in 1.12.1.
- Spell pushback exists and affects bot casters — be careful with cast-time math.

## Items and inventory

- Item slots: see `EquipmentSlots` enum. 19 equipment slots (head through trinket-2 plus ranged plus tabard). No relics slot beyond the libram/totem/idol equipped in the ranged slot for paladin/shaman/druid.
- Item quality tiers: poor (gray) through legendary (orange). Heirlooms do not exist.
- `Item::SetEnchantment()` applies an enchant by index. `Item::AddEnchantment` is the legacy name in some forks — match the surrounding code.
- Bots equip items via standard `character_inventory` / `item_instance` tables. The guild bot persistence relies on this.

## Auction House

- One AH per faction per major city — Stormwind, Ironforge, Darnassus, Orgrimmar, Undercity, Thunder Bluff. Plus the neutral Booty Bay / Gadgetzan / Everlook AHs.
- 24h / 8h / 2h posting durations (no 48h in vanilla). Deposit scales with duration and item quality.
- AH bot simulation (`AuctionHouseBotMgr`) uses the world AH and 12 fake seller identities under account 99. See [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md#auction-house-bot).

## Common retail-vs-vanilla mistakes

- **Dual wielding rangers**: warriors and rogues only (and hunters from melee weapons). No fury-stance dual wield bonus rework — Titan's Grip doesn't exist.
- **Paladins**: Alliance only. No Crusader Strike (that's TBC). Seal-twisting / weapon-swap is the DPS rotation.
- **Druids**: Bear Form is the tank form (no Dire Bear pre-TBC — well, there is Dire Bear at 40 via talent). Cat Form is the DPS form. Moonkin Form is talented in Balance.
- **Hunters**: pet system uses `Pet` class. Feed Pet exists. Pet talent trees do **not** — that's WotLK.
- **Mounts**: trained at level 40 (60% ground) and 60 (100% ground). No flying mounts. No cross-faction mounts.
- **Honor / PvP**: ranks and the grind system (Sergeant, Knight, etc.). No Arena, no Battlegrounds rated. WSG, AB, and AV exist.

## Sources of truth

When you need to verify a mechanic, in priority order:

1. **Official Blizzard patch notes** — archived at WoW BlueTracker, Wayback Machine snapshots of the official WoW site
2. **`Spell.dbc` / `Talent.dbc` / etc.** — the client-side data. If the client has it, it's authoritative.
3. **wowhead.com/classic** — content was scraped from 1.12 data; comments often reference pre-TBC behavior
4. **Vanilla WoW Wiki** (gamepedia archive) — usually accurate for mechanics
5. **YouTube vanilla gameplay footage** — useful for "did this animation play?" and similar
6. **Wayback Machine on old fansites (Thottbot, Allakhazam)** — often the only record of an obscure quest or item bug

**Never trust private-server forum threads** for game mechanics. They mix retail, TBC, and vanilla freely and pre-suppose whatever the forum's server happened to implement. The `vmangos/CONTRIBUTING.md` file says the same thing — follow it.

## Things to never assume

- The client is forgiving about packet ordering, but it's not. Many opcodes have implicit state transitions (e.g. `CMSG_AUTH_SESSION` → session is authenticated → next opcodes are valid). Don't reorder packet sends without testing the client behavior.
- DBC strings are encoded as the locale at extraction time. Don't compare client-supplied strings to DBC strings without normalizing.
- Item link parsing: the 1.12 client uses a slightly different item link format than retail. The colors and IDs match, but the suffix encoding (random enchants, e.g. `of the Bear`) lives in `item_enchantment_template`.

## Anti-patterns

- Importing TBC+ data structures (e.g. relic slot logic, dual-spec tables, glyph slots) "for future-proofing"
- Hardcoding spell IDs in C++ without a comment naming the spell
- Assuming a class has a spell — every spell lookup should go through `BotSpellKit` (for bot AI) or a DBC lookup (for game logic)
- Calling private-server forum claims "the canonical behavior" without a primary source
