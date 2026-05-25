"""Tests for the pure formatting helpers in dashboard.py.

These outputs are user-visible (player money in /api/players, server
uptime in /api/status). The format is part of the dashboard's
shape-stable JSON API surface in production phase — these tests lock it
against unintended changes.
"""

import pytest
from dashboard_app import format_money, format_uptime

# ---- format_money ---------------------------------------------------------


@pytest.mark.parametrize(
    "copper,expected",
    [
        (0, "0g 0s 0c"),
        (1, "0g 0s 1c"),
        (99, "0g 0s 99c"),
        (100, "0g 1s 0c"),
        (9999, "0g 99s 99c"),
        (10000, "1g 0s 0c"),
        (12345, "1g 23s 45c"),
        (1_000_000, "100g 0s 0c"),
        (2_147_483_647, "214748g 36s 47c"),  # near int32 max — vanilla cap
    ],
)
def test_format_money(copper, expected):
    assert format_money(copper) == expected


# ---- format_uptime --------------------------------------------------------


@pytest.mark.parametrize(
    "seconds,expected",
    [
        (0, "0h 0m"),
        (59, "0h 0m"),
        (60, "0h 1m"),
        (3599, "0h 59m"),
        (3600, "1h 0m"),
        (3660, "1h 1m"),
        (86399, "23h 59m"),
        (86400, "1d 0h 0m"),
        (90061, "1d 1h 1m"),
        (172800, "2d 0h 0m"),
        (864000, "10d 0h 0m"),
    ],
)
def test_format_uptime(seconds, expected):
    assert format_uptime(seconds) == expected


def test_format_uptime_boundary_at_24h():
    # Just below 1 day stays in hh-mm form; at 1 day flips to d-h-m form.
    assert format_uptime(86399) == "23h 59m"
    assert format_uptime(86400) == "1d 0h 0m"
