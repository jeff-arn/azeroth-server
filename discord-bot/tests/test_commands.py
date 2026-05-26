"""Tests for the !status and !patchnotes command handlers.

The handlers live in on_message() in bot.py. Discord.py is fully
mocked here — no network, no event loop driving real events. We call
the handler directly with a constructed mock message and verify the
embed it sends back via channel.send (AsyncMock).
"""

from unittest.mock import AsyncMock, MagicMock, PropertyMock, patch

import pytest
from discord_bot import on_message


def make_message(content, channel_id=1485324341062402239):
    """Build a mock discord.Message with the bits the handler reads."""
    msg = MagicMock()
    msg.content = content
    msg.author = MagicMock()  # any non-client object — not equal to client.user
    msg.channel = MagicMock()
    msg.channel.id = channel_id
    msg.channel.send = AsyncMock()
    return msg


@pytest.fixture(autouse=True)
def stub_client_user():
    """Make message.author != client.user so the handler doesn't early-return.

    discord.Client.user is a read-only property; patch.object with
    `new_callable=PropertyMock` works on it.
    """
    import discord

    sentinel = MagicMock(name="bot-user-sentinel")
    with patch.object(discord.Client, "user", new_callable=lambda: PropertyMock(return_value=sentinel)):
        yield sentinel


@pytest.fixture
def mock_subprocess_running():
    """pgrep mangosd returns 0 (running) by default."""
    with patch("discord_bot.subprocess.run") as run:
        run.return_value.returncode = 0
        yield run


@pytest.fixture
def mock_subprocess_down():
    with patch("discord_bot.subprocess.run") as run:
        run.return_value.returncode = 1
        yield run


@pytest.fixture
def no_maintenance(monkeypatch):
    monkeypatch.setattr("discord_bot.os.path.exists", lambda p: False)


# ---- !status --------------------------------------------------------------


@pytest.mark.asyncio
async def test_status_command_offline(no_maintenance, mock_subprocess_down):
    msg = make_message("!status")
    with patch("discord_bot.get_online_players", return_value=[]):
        await on_message(msg)
    msg.channel.send.assert_awaited_once()
    embed = msg.channel.send.call_args.kwargs["embed"]
    assert "Offline" in embed.title
    assert embed.color.value == 0xFF0000


@pytest.mark.asyncio
async def test_status_command_online_with_players(no_maintenance, mock_subprocess_running):
    msg = make_message("!status")
    players = [
        {"guid": 1, "name": "Thrall", "level": 60, "race": 2, "cls": 7, "online": 1, "username": "x"},
        {"guid": 2, "name": "Jaina", "level": 55, "race": 1, "cls": 8, "online": 1, "username": "y"},
        {"guid": 3, "name": "Offliner", "level": 30, "race": 1, "cls": 1, "online": 0, "username": "z"},
    ]
    with patch("discord_bot.get_online_players", return_value=players):
        await on_message(msg)
    msg.channel.send.assert_awaited_once()
    embed = msg.channel.send.call_args.kwargs["embed"]
    assert "Online" in embed.title
    assert embed.color.value == 0x00FF00
    # Two online (1 and 2); offline (3) excluded.
    assert "Players online:** 2" in embed.description  # bold-wrapped "2"
    assert "Thrall" in embed.description
    assert "Jaina" in embed.description
    assert "Offliner" not in embed.description


@pytest.mark.asyncio
async def test_status_command_in_maintenance(monkeypatch):
    monkeypatch.setattr("discord_bot.os.path.exists", lambda p: True)
    msg = make_message("!status")
    await on_message(msg)
    msg.channel.send.assert_awaited_once()
    embed = msg.channel.send.call_args.kwargs["embed"]
    assert "Maintenance" in embed.title


@pytest.mark.asyncio
async def test_unknown_command_does_nothing(no_maintenance):
    msg = make_message("hello there")
    await on_message(msg)
    msg.channel.send.assert_not_awaited()


@pytest.mark.asyncio
async def test_message_from_self_is_ignored(stub_client_user):
    msg = make_message("!status")
    msg.author = stub_client_user  # author == bot
    await on_message(msg)
    msg.channel.send.assert_not_awaited()


# ---- !patchnotes ----------------------------------------------------------


@pytest.mark.asyncio
async def test_patchnotes_command_without_content_shows_usage(no_maintenance):
    msg = make_message("!patchnotes")
    await on_message(msg)
    # Usage hint goes back to the calling channel, not the patch-notes channel.
    msg.channel.send.assert_awaited_once()
    text = msg.channel.send.call_args.args[0]
    assert "Usage" in text


@pytest.mark.asyncio
async def test_patchnotes_command_with_content_posts_to_patch_channel(no_maintenance):
    import discord_bot

    msg = make_message("!patchnotes v0.5.0 — fixed warlock pet AI", channel_id=999)
    patch_channel = MagicMock()
    patch_channel.id = discord_bot.CHANNEL_PATCH_NOTES
    patch_channel.send = AsyncMock()
    with patch.object(discord_bot.client, "get_channel", return_value=patch_channel):
        await on_message(msg)
    patch_channel.send.assert_awaited_once()
    embed = patch_channel.send.call_args.kwargs["embed"]
    assert "Patch Notes" in embed.title
    assert "v0.5.0" in embed.description
    # And a confirmation goes back to the source channel (since it wasn't
    # the patch-notes channel itself).
    msg.channel.send.assert_awaited_with("✅ Patch notes posted!")


@pytest.mark.asyncio
async def test_patchnotes_in_target_channel_no_confirmation(no_maintenance):
    import discord_bot

    msg = make_message("!patchnotes v0.6.0", channel_id=discord_bot.CHANNEL_PATCH_NOTES)
    patch_channel = MagicMock()
    patch_channel.send = AsyncMock()
    with patch.object(discord_bot.client, "get_channel", return_value=patch_channel):
        await on_message(msg)
    patch_channel.send.assert_awaited_once()
    # No second "✅" confirmation when posting from the patch-notes channel.
    msg.channel.send.assert_not_awaited()


# ---- is_server_running / is_maintenance_mode ------------------------------


def test_is_server_running_true_when_pgrep_returns_zero(mock_subprocess_running):
    from discord_bot import is_server_running

    assert is_server_running() is True


def test_is_server_running_false_when_pgrep_returns_nonzero(mock_subprocess_down):
    from discord_bot import is_server_running

    assert is_server_running() is False


def test_is_server_running_false_when_pgrep_raises():
    with patch("discord_bot.subprocess.run", side_effect=OSError("no pgrep")):
        from discord_bot import is_server_running

        assert is_server_running() is False


def test_is_maintenance_mode_reflects_flag_file(monkeypatch):
    from discord_bot import is_maintenance_mode

    monkeypatch.setattr("discord_bot.os.path.exists", lambda p: True)
    assert is_maintenance_mode() is True

    monkeypatch.setattr("discord_bot.os.path.exists", lambda p: False)
    assert is_maintenance_mode() is False
