"""Tests for the pure lookup helpers in discord-bot/bot.py.

These feed every user-visible Discord embed (faction emoji, class
emoji, race/class name). They're stable enums in vanilla 1.12 — if
they change, the bug is silent (wrong emoji on level-up message), so
parametrized coverage is cheap insurance.
"""

import pytest
from discord_bot import (
    get_class_emoji,
    get_class_emoji_str,
    get_class_name,
    get_faction_emoji,
    get_faction_emoji_str,
    get_race_name,
)

# ---- race lookups ---------------------------------------------------------


@pytest.mark.parametrize(
    "race_id,expected",
    [
        (1, "Human"),
        (2, "Orc"),
        (3, "Dwarf"),
        (4, "Night Elf"),
        (5, "Undead"),
        (6, "Tauren"),
        (7, "Gnome"),
        (8, "Troll"),
    ],
)
def test_get_race_name_covers_all_vanilla_races(race_id, expected):
    assert get_race_name(race_id) == expected


def test_get_race_name_unknown_id():
    assert get_race_name(99) == "Unknown"
    assert get_race_name(0) == "Unknown"


def test_get_race_name_excludes_tbc_races():
    # No Draenei (11) or Blood Elf (10) in 1.12.
    assert get_race_name(10) == "Unknown"
    assert get_race_name(11) == "Unknown"


# ---- class lookups --------------------------------------------------------


@pytest.mark.parametrize(
    "class_id,expected",
    [
        (1, "Warrior"),
        (2, "Paladin"),
        (3, "Hunter"),
        (4, "Rogue"),
        (5, "Priest"),
        (7, "Shaman"),
        (8, "Mage"),
        (9, "Warlock"),
        (11, "Druid"),
    ],
)
def test_get_class_name_covers_all_vanilla_classes(class_id, expected):
    assert get_class_name(class_id) == expected


def test_get_class_name_skips_unused_slots():
    # Class IDs 6 and 10 are unused in vanilla.
    assert get_class_name(6) == "Unknown"
    assert get_class_name(10) == "Unknown"


def test_get_class_name_excludes_post_vanilla_classes():
    # No DK, Monk, DH, Evoker.
    assert get_class_name(12) == "Unknown"


# ---- class emoji ----------------------------------------------------------


@pytest.mark.parametrize(
    "class_id,expected",
    [
        (1, "⚔️"),
        (2, "🛡️"),
        (3, "🏹"),
        (4, "🗡️"),
        (5, "✨"),
        (7, "🌩️"),
        (8, "🔥"),
        (9, "😈"),
        (11, "🌿"),
    ],
)
def test_get_class_emoji(class_id, expected):
    assert get_class_emoji(class_id) == expected


def test_get_class_emoji_unknown():
    assert get_class_emoji(99) == "❓"


def test_get_class_emoji_str_round_trips_through_levelup_format():
    # The levelup log uses "c1", "c2", etc. — the _str variant should
    # produce the same emoji as the int variant.
    for n in (1, 2, 3, 4, 5, 7, 8, 9, 11):
        assert get_class_emoji_str(f"c{n}") == get_class_emoji(n)


def test_get_class_emoji_str_unknown_returns_question_mark():
    assert get_class_emoji_str("cZ") == "❓"
    assert get_class_emoji_str("") == "❓"


# ---- faction emoji --------------------------------------------------------


@pytest.mark.parametrize(
    "race_id,expected",
    [
        (1, "🔵"),  # Human  -> Alliance
        (3, "🔵"),  # Dwarf  -> Alliance
        (4, "🔵"),  # NElf   -> Alliance
        (7, "🔵"),  # Gnome  -> Alliance
        (2, "🔴"),  # Orc    -> Horde
        (5, "🔴"),  # Undead -> Horde
        (6, "🔴"),  # Tauren -> Horde
        (8, "🔴"),  # Troll  -> Horde
    ],
)
def test_get_faction_emoji_assignment(race_id, expected):
    assert get_faction_emoji(race_id) == expected


def test_get_faction_emoji_unknown_race_defaults_to_horde():
    # Defensive: unknown race id falls into the not-alliance branch.
    # If this ever needs to change, the test makes the intent explicit.
    assert get_faction_emoji(99) == "🔴"


def test_get_faction_emoji_str_matches_int_variant():
    # Alliance races by string form should produce blue, all others red.
    alliance_strs = {"r1", "r3", "r4", "r7"}
    horde_strs = {"r2", "r5", "r6", "r8"}
    for rs in alliance_strs:
        assert get_faction_emoji_str(rs) == "🔵"
    for rs in horde_strs:
        assert get_faction_emoji_str(rs) == "🔴"
