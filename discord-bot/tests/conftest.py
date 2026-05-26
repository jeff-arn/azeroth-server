"""Shared test setup for the discord-bot test suite.

Three things happen at collection time:

1. The bot module reads DB_PASSWORD and DISCORD_BOT_TOKEN from the
   environment at import time. Set dummies so tests can import without
   real credentials.

2. The discord-bot directory has a dash in its name, so we can't use a
   normal Python import. Load bot.py via importlib and register it as
   `discord_bot` for tests.

3. The bot's import-time code creates a discord.Client instance and
   registers event decorators. That's fine — no network call happens
   until client.run() is invoked (which we never call in tests).
"""

import importlib.util
import os
import sys
from pathlib import Path

os.environ.setdefault("DB_PASSWORD", "test-only-dummy")
os.environ.setdefault("DISCORD_BOT_TOKEN", "test-only-dummy")

if "discord_bot" not in sys.modules:
    _src = Path(__file__).resolve().parent.parent / "bot.py"
    _spec = importlib.util.spec_from_file_location("discord_bot", _src)
    _module = importlib.util.module_from_spec(_spec)
    sys.modules["discord_bot"] = _module
    _spec.loader.exec_module(_module)
