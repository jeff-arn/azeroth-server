#!/usr/bin/env python3
"""
Generate C++ talent build definitions from DBC data.
Maps researched vanilla WoW talent builds to actual DBC talent IDs.
"""

import os

from dbc import get_string, read_dbc

DBC_PATH = "/home/wow/wow-private-server/vmangos/run/bin/5875/dbc"


# --- Parse TalentTab.dbc ---
tab_records, _ = read_dbc(os.path.join(DBC_PATH, "TalentTab.dbc"))

CLASS_NAMES = {
    1: "Warrior",
    2: "Paladin",
    3: "Hunter",
    4: "Rogue",
    5: "Priest",
    7: "Shaman",
    8: "Mage",
    9: "Warlock",
    11: "Druid",
}

tabs = {}  # tab_id -> {class_id, class_name, tab_page, tree_name}
for rec in tab_records:
    tab_id = rec[0]
    class_mask = rec[12]
    tab_page = rec[13]
    # Determine class from mask
    for class_id, class_name in CLASS_NAMES.items():
        if class_mask & (1 << (class_id - 1)):
            tabs[tab_id] = {
                "class_id": class_id,
                "class_name": class_name,
                "tab_page": tab_page,
            }
            break

# --- Parse Talent.dbc ---
talent_records, _ = read_dbc(os.path.join(DBC_PATH, "Talent.dbc"))

talents = {}  # talent_id -> {tab, row, col, ranks, rank_spells, depends_on, depends_rank}
for rec in talent_records:
    tid = rec[0]
    tab = rec[1]
    row = rec[2]
    col = rec[3]
    rank_spells = [rec[4 + i] for i in range(5)]
    num_ranks = sum(1 for s in rank_spells if s != 0)
    depends_on = rec[13]
    depends_rank = rec[16]

    talents[tid] = {
        "tab": tab,
        "row": row,
        "col": col,
        "num_ranks": num_ranks,
        "rank_spells": rank_spells[:num_ranks],
        "depends_on": depends_on if depends_on != 0 else None,
        "depends_rank": depends_rank,
    }

# --- Parse Spell.dbc for names ---
spell_records, spell_strings = read_dbc(os.path.join(DBC_PATH, "Spell.dbc"))

spell_names = {}
for rec in spell_records:
    spell_id = rec[0]
    name_offset = rec[120]  # enUS name at field 120
    if name_offset < len(spell_strings):
        spell_names[spell_id] = get_string(spell_strings, name_offset)

# --- Build name lookup: (class_name, spell_name_substring) -> talent_id ---
talent_by_name = {}  # (class_id, name) -> talent
for tid, t in talents.items():
    if t["tab"] not in tabs:
        continue
    tab_info = tabs[t["tab"]]
    rank1_spell = t["rank_spells"][0] if t["rank_spells"] else 0
    name = spell_names.get(rank1_spell, "???")
    key = (tab_info["class_id"], name)
    talent_by_name[key] = tid
    # Also store for lookup
    t["spell_name"] = name
    t["class_id"] = tab_info["class_id"]
    t["class_name"] = tab_info["class_name"]
    t["tab_page"] = tab_info["tab_page"]


def lookup(class_id, name):
    """Find talent ID by class and spell name."""
    key = (class_id, name)
    if key in talent_by_name:
        tid = talent_by_name[key]
        t = talents[tid]
        return tid, t["tab"], t["row"], t["col"], t["num_ranks"]
    # Try partial match
    for (cid, sname), tid in talent_by_name.items():
        if cid == class_id and name.lower() in sname.lower():
            t = talents[tid]
            return tid, t["tab"], t["row"], t["col"], t["num_ranks"]
    print(f"  WARNING: Could not find talent '{name}' for class {CLASS_NAMES.get(class_id, class_id)}")
    return None, None, None, None, None


# ============================================================================
# TALENT BUILDS - researched vanilla WoW PvE builds
# Format: (talent_name, ranks_to_take)
# Order matters — points are spent sequentially from level 10
# ============================================================================

BUILDS = {
    # ---- WARRIOR ----
    ("Warrior", "fury", "ROLE_MELEE_DPS"): [
        ("Cruelty", 5),
        ("Unbridled Wrath", 5),
        ("Piercing Howl", 1),
        ("Improved Battle Shout", 5),
        ("Enrage", 5),
        ("Flurry", 5),
        ("Bloodthirst", 1),
        ("Improved Heroic Strike", 3),
        ("Deflection", 5),
        ("Tactical Mastery", 5),
        ("Deep Wounds", 3),
        ("Impale", 2),
        ("Two-Handed Weapon Specialization", 5),
        ("Improved Berserker Rage", 1),  # 51
    ],
    ("Warrior", "prot", "ROLE_TANK"): [
        ("Shield Specialization", 5),
        ("Anticipation", 5),
        ("Improved Revenge", 3),
        ("Toughness", 5),
        ("Last Stand", 1),
        ("Improved Shield Block", 3),
        ("Defiance", 5),
        ("Concussion Blow", 1),
        ("Improved Sunder Armor", 3),
        ("One-Handed Weapon Specialization", 5),
        ("Shield Slam", 1),
        ("Cruelty", 5),
        ("Unbridled Wrath", 5),
        ("Improved Heroic Strike", 3),
        ("Improved Battle Shout", 1),  # 51
    ],
    ("Warrior", "arms", "ROLE_MELEE_DPS"): [
        ("Improved Heroic Strike", 3),
        ("Deflection", 5),
        ("Tactical Mastery", 5),
        ("Anger Management", 1),
        ("Deep Wounds", 3),
        ("Impale", 2),
        ("Two-Handed Weapon Specialization", 5),
        ("Axe Specialization", 5),
        ("Sweeping Strikes", 1),
        ("Sword Specialization", 5),
        ("Mortal Strike", 1),
        ("Cruelty", 5),
        ("Unbridled Wrath", 5),
        ("Piercing Howl", 1),
        ("Improved Battle Shout", 4),  # 51
    ],
    # ---- PALADIN ----
    ("Paladin", "holy", "ROLE_HEALER"): [
        ("Divine Intellect", 5),
        ("Spiritual Focus", 5),
        ("Healing Light", 3),
        ("Consecration", 1),
        ("Improved Lay on Hands", 2),
        ("Illumination", 5),
        ("Divine Favor", 1),
        ("Lasting Judgement", 3),
        ("Holy Power", 5),
        ("Holy Shock", 1),
        ("Improved Devotion Aura", 5),
        ("Guardian's Favor", 2),
        ("Precision", 3),
        ("Blessing of Kings", 1),
        ("Improved Blessing of Wisdom", 2),
        ("Improved Seal of Righteousness", 5),
        ("Divine Strength", 2),  # 51
    ],
    ("Paladin", "prot", "ROLE_TANK"): [
        ("Redoubt", 5),
        ("Precision", 3),
        ("Guardian's Favor", 2),
        ("Toughness", 5),
        ("Improved Hammer of Justice", 3),
        ("Blessing of Sanctuary", 1),
        ("Shield Specialization", 3),
        ("Reckoning", 5),
        ("One-Handed Weapon Specialization", 5),
        ("Holy Shield", 1),
        ("Improved Righteous Fury", 3),
        ("Divine Strength", 5),
        ("Divine Intellect", 5),
        ("Spiritual Focus", 5),
    ],
    ("Paladin", "ret", "ROLE_MELEE_DPS"): [
        ("Improved Blessing of Might", 5),
        ("Benediction", 5),
        ("Deflection", 5),
        ("Vindication", 3),
        ("Conviction", 5),
        ("Seal of Command", 1),
        ("Pursuit of Justice", 2),
        ("Two-Handed Weapon Specialization", 3),
        ("Sanctity Aura", 1),
        ("Vengeance", 5),
        ("Repentance", 1),
        ("Divine Strength", 5),
        ("Divine Intellect", 5),
        ("Spiritual Focus", 5),
    ],
    # ---- HUNTER ----
    ("Hunter", "mm", "ROLE_RANGE_DPS"): [
        ("Lethal Shots", 5),
        ("Efficiency", 5),
        ("Aimed Shot", 1),
        ("Improved Hunter's Mark", 5),
        ("Mortal Shots", 5),
        ("Scatter Shot", 1),
        ("Barrage", 3),
        ("Ranged Weapon Specialization", 5),
        ("Trueshot Aura", 1),
        ("Monster Slaying", 3),
        ("Humanoid Slaying", 3),
        ("Clever Traps", 2),
        ("Surefooted", 3),
        ("Killer Instinct", 3),
        ("Lightning Reflexes", 5),
        ("Improved Concussive Shot", 1),  # 51
    ],
    ("Hunter", "bm", "ROLE_RANGE_DPS"): [
        ("Endurance Training", 5),
        ("Thick Hide", 3),
        ("Improved Revive Pet", 2),
        ("Unleashed Fury", 5),
        ("Improved Mend Pet", 2),
        ("Ferocity", 5),
        ("Bestial Discipline", 2),
        ("Intimidation", 1),
        ("Frenzy", 5),
        ("Bestial Wrath", 1),
        ("Lethal Shots", 5),
        ("Efficiency", 5),
        ("Aimed Shot", 1),
        ("Improved Hunter's Mark", 5),
        ("Mortal Shots", 4),
    ],
    ("Hunter", "surv", "ROLE_RANGE_DPS"): [
        ("Monster Slaying", 3),
        ("Humanoid Slaying", 3),
        ("Deflection", 5),
        ("Savage Strikes", 2),
        ("Clever Traps", 2),
        ("Surefooted", 3),
        ("Trap Mastery", 2),
        ("Killer Instinct", 3),
        ("Counterattack", 1),
        ("Lightning Reflexes", 5),
        ("Wyvern Sting", 1),
        ("Lethal Shots", 5),
        ("Efficiency", 5),
        ("Aimed Shot", 1),
        ("Improved Hunter's Mark", 5),
        ("Mortal Shots", 5),
    ],
    # ---- ROGUE ----
    ("Rogue", "combat", "ROLE_MELEE_DPS"): [
        ("Improved Sinister Strike", 2),
        ("Lightning Reflexes", 3),
        ("Improved Gouge", 3),
        ("Precision", 5),
        ("Improved Slice and Dice", 3),
        ("Dual Wield Specialization", 5),
        ("Blade Flurry", 1),
        ("Sword Specialization", 5),
        ("Weapon Expertise", 2),
        ("Aggression", 3),
        ("Adrenaline Rush", 1),
        ("Malice", 5),
        ("Ruthlessness", 3),
        ("Murder", 2),
        ("Relentless Strikes", 1),
        ("Lethality", 5),
        ("Improved Eviscerate", 2),  # 51
    ],
    ("Rogue", "assassination", "ROLE_MELEE_DPS"): [
        ("Improved Eviscerate", 3),
        ("Malice", 5),
        ("Ruthlessness", 3),
        ("Murder", 2),
        ("Relentless Strikes", 1),
        ("Improved Slice and Dice", 3),
        ("Lethality", 5),
        ("Vile Poisons", 5),
        ("Improved Poisons", 5),
        ("Cold Blood", 1),
        ("Seal Fate", 5),
        ("Lightning Reflexes", 3),
        ("Precision", 5),
        ("Improved Sinister Strike", 2),
        ("Dual Wield Specialization", 3),  # 51
    ],
    # ---- PRIEST ----
    ("Priest", "holy", "ROLE_HEALER"): [
        ("Unbreakable Will", 5),
        ("Improved Power Word: Fortitude", 2),
        ("Improved Power Word: Shield", 3),
        ("Inner Focus", 1),
        ("Meditation", 3),
        ("Mental Agility", 3),
        ("Divine Spirit", 1),
        ("Mental Strength", 2),
        ("Power Infusion", 1),
        ("Healing Focus", 2),
        ("Holy Specialization", 5),
        ("Divine Fury", 5),
        ("Inspiration", 3),
        ("Holy Reach", 2),
        ("Improved Healing", 3),
        ("Spiritual Guidance", 5),
        ("Spiritual Healing", 5),  # 51
    ],
    ("Priest", "shadow", "ROLE_RANGE_DPS"): [
        ("Spirit Tap", 5),
        ("Shadow Affinity", 3),
        ("Improved Shadow Word: Pain", 2),
        ("Shadow Focus", 5),
        ("Mind Flay", 1),
        ("Improved Mind Blast", 5),
        ("Shadow Reach", 3),
        ("Shadow Weaving", 5),
        ("Silence", 1),
        ("Darkness", 5),
        ("Shadowform", 1),
        ("Vampiric Embrace", 1),
        ("Wand Specialization", 5),
        ("Improved Power Word: Fortitude", 2),
        ("Improved Power Word: Shield", 3),
        ("Inner Focus", 1),
        ("Meditation", 3),
    ],
    # ---- SHAMAN ----
    ("Shaman", "resto", "ROLE_HEALER"): [
        ("Improved Healing Wave", 5),
        ("Tidal Focus", 5),
        ("Totemic Focus", 5),
        ("Ancestral Healing", 3),
        ("Healing Focus", 3),
        ("Restorative Totems", 5),
        ("Tidal Mastery", 5),
        ("Nature's Swiftness", 1),
        ("Healing Way", 3),
        ("Purification", 5),
        ("Mana Tide Totem", 1),
        ("Concussion", 5),
        ("Call of Flame", 3),
        ("Reverberation", 2),
    ],
    ("Shaman", "enhance", "ROLE_MELEE_DPS"): [
        ("Ancestral Knowledge", 5),
        ("Shield Specialization", 5),
        ("Thundering Strikes", 5),
        ("Improved Ghost Wolf", 2),
        ("Flurry", 5),
        ("Elemental Weapons", 3),
        ("Parry", 1),
        ("Weapon Mastery", 5),
        ("Stormstrike", 1),
        ("Concussion", 5),
        ("Call of Flame", 3),
        ("Elemental Focus", 1),
        ("Call of Thunder", 5),
        ("Reverberation", 5),  # 51
    ],
    ("Shaman", "elemental", "ROLE_RANGE_DPS"): [
        ("Convection", 5),
        ("Concussion", 5),
        ("Call of Flame", 3),
        ("Call of Thunder", 5),
        ("Elemental Focus", 1),
        ("Reverberation", 5),
        ("Eye of the Storm", 3),
        ("Elemental Fury", 1),
        ("Lightning Mastery", 5),
        ("Elemental Mastery", 1),
        ("Improved Healing Wave", 5),
        ("Tidal Focus", 5),
        ("Totemic Focus", 4),
        ("Healing Focus", 3),  # 51
    ],
    # ---- MAGE ----
    ("Mage", "frost", "ROLE_RANGE_DPS"): [
        ("Improved Frostbolt", 5),
        ("Elemental Precision", 3),
        ("Ice Shards", 5),
        ("Piercing Ice", 3),
        ("Cold Snap", 1),
        ("Improved Blizzard", 3),
        ("Arctic Reach", 2),
        ("Frost Channeling", 3),
        ("Shatter", 5),
        ("Ice Block", 1),
        ("Improved Cone of Cold", 3),
        ("Winter's Chill", 5),
        ("Ice Barrier", 1),
        ("Arcane Subtlety", 2),
        ("Arcane Focus", 3),
        ("Arcane Concentration", 5),
        ("Arcane Meditation", 1),  # 51
    ],
    ("Mage", "fire", "ROLE_RANGE_DPS"): [
        ("Improved Fireball", 5),
        ("Ignite", 5),
        ("Flame Throwing", 2),
        ("Impact", 5),
        ("Improved Flamestrike", 3),
        ("Pyroblast", 1),
        ("Burning Soul", 2),
        ("Master of Elements", 3),
        ("Critical Mass", 3),
        ("Blast Wave", 1),
        ("Fire Power", 5),
        ("Combustion", 1),
        ("Arcane Subtlety", 2),
        ("Arcane Focus", 3),
        ("Arcane Concentration", 5),
        ("Arcane Meditation", 3),
        ("Arcane Mind", 2),
    ],
    ("Mage", "arcfrost", "ROLE_RANGE_DPS"): [
        ("Arcane Subtlety", 2),
        ("Arcane Focus", 5),
        ("Arcane Concentration", 5),
        ("Improved Arcane Missiles", 5),
        ("Arcane Meditation", 3),
        ("Presence of Mind", 1),
        ("Arcane Mind", 5),
        ("Arcane Instability", 3),
        ("Arcane Power", 1),
        ("Improved Frostbolt", 5),
        ("Elemental Precision", 3),
        ("Ice Shards", 5),
        ("Piercing Ice", 3),
        ("Frost Channeling", 3),
        ("Cold Snap", 1),
        ("Improved Frostbolt", 1),  # 51 (1 extra filler, total from above was 50)
    ],
    # ---- WARLOCK ----
    ("Warlock", "smruin", "ROLE_RANGE_DPS"): [
        ("Improved Corruption", 5),
        ("Suppression", 5),
        ("Improved Life Tap", 2),
        ("Improved Curse of Agony", 3),
        ("Amplify Curse", 1),
        ("Fel Concentration", 5),
        ("Nightfall", 2),
        ("Grim Reach", 2),
        ("Siphon Life", 1),
        ("Shadow Mastery", 5),
        ("Improved Shadow Bolt", 5),
        ("Cataclysm", 5),
        ("Bane", 5),
        ("Devastation", 4),
        ("Ruin", 1),  # 51
    ],
    ("Warlock", "dsruin", "ROLE_RANGE_DPS"): [
        ("Improved Healthstone", 2),
        ("Improved Imp", 3),
        ("Demonic Embrace", 5),
        ("Fel Intellect", 5),
        ("Fel Domination", 1),
        ("Fel Stamina", 3),
        ("Demonic Sacrifice", 1),
        ("Unholy Power", 1),
        ("Improved Shadow Bolt", 5),
        ("Cataclysm", 5),
        ("Bane", 5),
        ("Devastation", 5),
        ("Shadowburn", 1),
        ("Intensity", 2),
        ("Ruin", 1),
        ("Improved Corruption", 5),
        ("Suppression", 1),
    ],
    ("Warlock", "affliction", "ROLE_RANGE_DPS"): [
        ("Improved Corruption", 5),
        ("Suppression", 5),
        ("Improved Curse of Agony", 3),
        ("Improved Life Tap", 2),
        ("Amplify Curse", 1),
        ("Fel Concentration", 5),
        ("Nightfall", 2),
        ("Grim Reach", 2),
        ("Siphon Life", 1),
        ("Shadow Mastery", 5),
        ("Dark Pact", 1),
        ("Improved Drain Life", 2),
        ("Improved Shadow Bolt", 5),
        ("Cataclysm", 5),
        ("Bane", 5),
        ("Devastation", 2),  # 51
    ],
    # ---- DRUID ----
    ("Druid", "resto", "ROLE_HEALER"): [
        ("Improved Mark of the Wild", 5),
        ("Furor", 5),
        ("Improved Healing Touch", 5),
        ("Reflection", 3),
        ("Insect Swarm", 1),
        ("Subtlety", 5),
        ("Tranquil Spirit", 5),
        ("Nature's Swiftness", 1),
        ("Improved Rejuvenation", 3),
        ("Gift of Nature", 5),
        ("Improved Regrowth", 5),
        ("Swiftmend", 1),
        ("Nature's Grace", 1),
        ("Improved Nature's Grasp", 4),
        ("Natural Shapeshifter", 2),  # 51
    ],
    ("Druid", "feraltank", "ROLE_TANK"): [
        ("Ferocity", 5),
        ("Feral Aggression", 5),
        ("Thick Hide", 5),
        ("Feral Instinct", 5),
        ("Sharpened Claws", 3),
        ("Predatory Strikes", 3),
        ("Blood Frenzy", 2),
        ("Primal Fury", 2),
        ("Heart of the Wild", 5),
        ("Leader of the Pack", 1),
        ("Faerie Fire (Feral)", 1),
        ("Feral Charge", 1),
        ("Furor", 5),
        ("Improved Mark of the Wild", 5),
        ("Reflection", 3),
    ],
    ("Druid", "feralcat", "ROLE_MELEE_DPS"): [
        ("Ferocity", 5),
        ("Feral Aggression", 5),
        ("Feral Instinct", 5),
        ("Sharpened Claws", 3),
        ("Predatory Strikes", 3),
        ("Blood Frenzy", 2),
        ("Primal Fury", 2),
        ("Savage Fury", 2),
        ("Heart of the Wild", 5),
        ("Leader of the Pack", 1),
        ("Faerie Fire (Feral)", 1),
        ("Feral Charge", 1),
        ("Furor", 5),
        ("Improved Mark of the Wild", 5),
        ("Natural Shapeshifter", 3),
        ("Omen of Clarity", 1),
        ("Reflection", 2),
    ],
    ("Druid", "balance", "ROLE_RANGE_DPS"): [
        ("Improved Wrath", 5),
        ("Nature's Grasp", 1),
        ("Improved Nature's Grasp", 4),
        ("Improved Moonfire", 5),
        ("Natural Weapons", 5),
        ("Natural Shapeshifter", 3),
        ("Omen of Clarity", 1),
        ("Nature's Reach", 2),
        ("Vengeance", 5),
        ("Improved Starfire", 5),
        ("Nature's Grace", 1),
        ("Moonglow", 3),
        ("Moonfury", 5),
        ("Moonkin Form", 1),
        ("Improved Mark of the Wild", 5),  # 51
    ],
}

# ============================================================================
# Now generate the C++ output
# ============================================================================

CLASS_IDS = {
    "Warrior": 1,
    "Paladin": 2,
    "Hunter": 3,
    "Rogue": 4,
    "Priest": 5,
    "Shaman": 7,
    "Mage": 8,
    "Warlock": 9,
    "Druid": 11,
}

print("// ============================================================================")
print("// Auto-generated talent build definitions")
print("// Generated by scripts/generate_talent_builds.py")
print("// ============================================================================")
print()

errors = []
total_builds = 0

for (class_name, spec_name, role), talent_list in BUILDS.items():
    class_id = CLASS_IDS[class_name]
    total_builds += 1

    print(f"// {class_name} - {spec_name} ({role})")
    print(f"static const BotTalentPoint s_{class_name}_{spec_name}_talents[] = {{")

    total_points = 0
    for talent_name, ranks in talent_list:
        tid, tab, row, col, max_ranks = lookup(class_id, talent_name)
        if tid is None:
            errors.append(f"{class_name}/{spec_name}: '{talent_name}' not found")
            print(f"    // ERROR: '{talent_name}' not found!")
            continue

        if ranks > max_ranks:
            errors.append(f"{class_name}/{spec_name}: '{talent_name}' wants {ranks} ranks but max is {max_ranks}")
            ranks = max_ranks

        total_points += ranks
        print(f"    {{ {tid:4d}, {ranks} }},  // {talent_name} ({ranks}/{max_ranks}) [row {row}, col {col}]")

    print("};")
    print(f"// Total points: {total_points}")
    print()

if errors:
    print("// ============================================================================")
    print("// ERRORS:")
    for e in errors:
        print(f"//   {e}")
    print("// ============================================================================")

print(f"// Generated {total_builds} builds")
