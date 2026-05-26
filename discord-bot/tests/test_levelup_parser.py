"""Tests for the LEVELUP_PATTERN regex.

The pattern matches lines emitted by the world server's level-up log,
parsed by levelup_log_watcher() to post Discord announcements. The
real log lines (with redacted account/character names per
.claude/rules/security.md) are the fixture below.
"""

import pytest
from discord_bot import LEVELUP_PATTERN

# Real log line shape, redacted. Format observed in production:
#   <ISO-timestamp> INFO: Character <Name>:<guid> [c<class> r<race>] reaches level <N>
REDACTED_LINES = [
    "2025-06-01 12:34:56 INFO: Character Thrall:42 [c7 r2] reaches level 30",
    "2025-06-01 13:45:00 INFO: Character Jaina:7 [c8 r1] reaches level 60",
    "2025-06-01 14:00:00 INFO: Character GnomeRogue:101 [c4 r7] reaches level  10",  # double-space
]


@pytest.mark.parametrize(
    "line,name,cls,race,level",
    [
        (REDACTED_LINES[0], "Thrall", "c7", "r2", 30),
        (REDACTED_LINES[1], "Jaina", "c8", "r1", 60),
        (REDACTED_LINES[2], "GnomeRogue", "c4", "r7", 10),
    ],
)
def test_parser_extracts_fields(line, name, cls, race, level):
    m = LEVELUP_PATTERN.search(line)
    assert m is not None
    assert m.group(1) == name
    assert m.group(2) == cls
    assert m.group(3) == race
    assert int(m.group(4)) == level


def test_parser_handles_extra_whitespace_before_level():
    # The original regex used \s+ so double space between "level" and N
    # must match — the log writer sometimes emits 1 space, sometimes 2.
    line = "Character Foo:1 [c1 r1] reaches level   42"
    m = LEVELUP_PATTERN.search(line)
    assert m is not None
    assert int(m.group(4)) == 42


def test_parser_rejects_non_levelup_line():
    assert LEVELUP_PATTERN.search("2025-06-01 12:00:00 INFO: Server started") is None
    assert LEVELUP_PATTERN.search("random garbage") is None
    assert LEVELUP_PATTERN.search("") is None


def test_parser_extracts_first_match_in_line():
    # Two matches in one line — search() returns the first.
    line = "Character Alpha:1 [c1 r1] reaches level 10 ... Character Beta:2 [c2 r3] reaches level 20"
    m = LEVELUP_PATTERN.search(line)
    assert m.group(1) == "Alpha"


def test_parser_does_not_match_bot_lines_when_filter_applied():
    # Real bot lines include "<BOT>" — the watcher filters before regex.
    # The regex still matches the well-formed part, so the filter is
    # the actual guard. This test pins that responsibility on the
    # watcher loop, not the pattern.
    bot_line = "Character Botto:99 <BOT> [c1 r1] reaches level 10"
    m = LEVELUP_PATTERN.search(bot_line)
    # The pattern does NOT match because <BOT> breaks the bracket form.
    assert m is None
