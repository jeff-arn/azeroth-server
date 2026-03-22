#!/usr/bin/env python3
"""
Post patch notes to Discord from the command line.
Usage: python3 post_patch_notes.py "Your patch notes here"
       python3 post_patch_notes.py --file /path/to/notes.txt
"""
import discord
import asyncio
import sys
import os
from datetime import datetime, timezone

TOKEN = os.environ.get("DISCORD_BOT_TOKEN", "MTQ4NTMyMzgzMTY5MzQxNDQ2MQ.Gx6Utz.TfLhHb6RSzK2YOaCqsSlml1z_6WqAzGZlnlRV4")
CHANNEL_PATCH_NOTES = 1485324341062402239

async def post(content):
    intents = discord.Intents.default()
    client = discord.Client(intents=intents)

    @client.event
    async def on_ready():
        channel = client.get_channel(CHANNEL_PATCH_NOTES)
        if channel:
            embed = discord.Embed(
                title="📋 Patch Notes",
                description=content,
                color=0xFFD100,
                timestamp=datetime.now(timezone.utc),
            )
            embed.set_footer(text="Azeroth Private Server")
            await channel.send(embed=embed)
            print("✅ Patch notes posted!")
        else:
            print("❌ Could not find patch notes channel")
        await client.close()

    await client.start(TOKEN)

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 post_patch_notes.py \"Your patch notes here\"")
        print("       python3 post_patch_notes.py --file /path/to/notes.txt")
        sys.exit(1)

    if sys.argv[1] == "--file":
        with open(sys.argv[2], "r") as f:
            content = f.read()
    else:
        content = " ".join(sys.argv[1:])

    asyncio.run(post(content))
