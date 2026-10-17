#!/usr/bin/env python3
"""Phase 3 scripted end-to-end login against the dev bench.

BENCH ONLY: this updates the bench JEFF account row (known
sessionkey, last_login=NOW(), last_ip=127.0.0.1) so the world
auth window passes, then drives the running Rust mangosd:
challenge -> auth -> AUTH_OK -> CHAR_ENUM (byte-compared to
the independent oracle) -> PLAYER_LOGIN -> verify-world /
account-data MD5 / friend+ignore lists / MOTD -> ping/pong.
A negative login (another account's guid) must end in a kick
with no packet train.

Expects mangosd already running on 127.0.0.1:18099 with the
bench conf (WowPatch=10). Prints SCRIPTED-LOGIN-PASS on success.
"""

import hashlib
import pathlib
import re
import socket
import struct
import subprocess
import sys
import zlib

HOST = "127.0.0.1"
PORT = 18099
ACCOUNT = "JEFF"
ACCOUNT_ID = 5
GUID = 1
OTHER_GUID = 2  # belongs to a different account in the snapshot
K_HEX = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08"
CLIENT_SEED = 0x11223344

SMSG_AUTH_CHALLENGE = 492
SMSG_AUTH_RESPONSE = 494
CMSG_AUTH_SESSION = 493
CMSG_PING = 476
SMSG_PONG = 477
CMSG_CHAR_ENUM = 55
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
SMSG_LOGIN_VERIFY_WORLD = 566
SMSG_ACCOUNT_DATA_MD5 = 521
SMSG_FRIEND_LIST = 103
SMSG_IGNORE_LIST = 107
SMSG_MESSAGECHAT = 150
SMSG_ADDON_INFO = 751


def bench_password() -> str:
    env = pathlib.Path("vmangos-rs/.env.test").read_text()
    return re.search(r"DEV_MYSQL_PASSWORD=(\S+)", env).group(1).strip("'\"")


def sql(db: str, stmt: str) -> None:
    r = subprocess.run(
        ["mysql", "-u", "mangos", f"-p{bench_password()}", db, "-e", stmt],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr)


def setup_account() -> None:
    sql(
        "realmd",
        f"UPDATE account SET sessionkey='{K_HEX}', last_ip='127.0.0.1', "
        f"last_login=NOW(), os='Win', platform='x86' WHERE id={ACCOUNT_ID}",
    )


class Crypt:
    """Client-side AuthCrypt (send 6 bytes, receive 4)."""

    def __init__(self, key_hex: str) -> None:
        k = int(key_hex, 16)
        self.key = k.to_bytes((k.bit_length() + 7) // 8, "little")
        self.si = self.sj = self.ri = self.rj = 0
        self.active = False

    def encrypt(self, data: bytearray) -> None:
        if not self.active:
            return
        for i in range(len(data)):
            self.si %= len(self.key)
            c = ((data[i] ^ self.key[self.si]) + self.sj) & 0xFF
            self.si += 1
            data[i] = c
            self.sj = c

    def decrypt(self, data: bytearray) -> None:
        if not self.active:
            return
        for i in range(len(data)):
            self.ri %= len(self.key)
            c = data[i]
            data[i] = ((c - self.rj) & 0xFF) ^ self.key[self.ri]
            self.ri += 1
            self.rj = c


class Client:
    def __init__(self) -> None:
        self.sock = socket.create_connection((HOST, PORT), timeout=5)
        self.crypt = Crypt(K_HEX)
        op, body = self.read_packet()
        assert op == SMSG_AUTH_CHALLENGE, op
        self.server_seed = struct.unpack("<I", body[:4])[0]

    def read_exact(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("closed")
            buf += chunk
        return buf

    def read_packet(self) -> tuple[int, bytes]:
        head = bytearray(self.read_exact(4))
        self.crypt.decrypt(head)
        size = struct.unpack(">H", head[:2])[0]
        op = struct.unpack("<H", head[2:])[0]
        return op, self.read_exact(size - 2)

    def send_packet(self, op: int, body: bytes) -> None:
        head = bytearray(struct.pack(">H", len(body) + 4) + struct.pack("<I", op))
        self.crypt.encrypt(head)
        self.sock.sendall(bytes(head) + body)

    def auth(self) -> None:
        k = int(K_HEX, 16)
        k_le = k.to_bytes((k.bit_length() + 7) // 8, "little")
        digest = hashlib.sha1(
            ACCOUNT.encode()
            + struct.pack("<I", 0)
            + struct.pack("<I", CLIENT_SEED)
            + struct.pack("<I", self.server_seed)
            + k_le
        ).digest()
        body = struct.pack("<II", 5875, 1) + ACCOUNT.encode() + b"\x00"
        body += struct.pack("<I", CLIENT_SEED) + digest
        # Addon tail: one custom + one Blizzard addon (real
        # clients always send this).
        plain = b"MyAddon\x00" + bytes([1]) + struct.pack("<II", 0, 0)
        plain += b"Blizzard_AuctionUI\x00" + bytes([1])
        plain += struct.pack("<II", 0x4C1C776D, 0)
        body += struct.pack("<I", len(plain)) + zlib.compress(plain)
        self.send_packet(CMSG_AUTH_SESSION, body)
        self.crypt.active = True
        # AUTH_OK (world actor) and ADDON_INFO (socket task) can
        # arrive in either order, like the C++ queue race.
        seen = {}
        for _ in range(2):
            op, body = self.read_packet()
            seen[op] = body
        assert seen[SMSG_AUTH_RESPONSE] == bytes([12] + [0] * 9), seen
        addon = seen[SMSG_ADDON_INFO]
        assert addon[:3] == bytes([1, 0, 0]), addon.hex()  # visible custom
        assert addon[3] == 2, addon.hex()  # hidden Blizzard
        print("AUTH-OK + ADDON-INFO")

    def expect_closed(self) -> None:
        try:
            data = self.sock.recv(64)
            assert data == b"", f"expected close, got {data.hex()}"
        except ConnectionResetError:
            pass


def main() -> None:
    setup_account()

    client = Client()
    client.auth()

    client.send_packet(CMSG_CHAR_ENUM, b"")
    op, body = client.read_packet()
    assert op == SMSG_CHAR_ENUM, op
    oracle = pathlib.Path("~/azeroth-dev-fixtures/char_enum_oracle.json").expanduser()
    expected_hex = re.search(r'"body_hex": "([0-9a-f]+)"', oracle.read_text()).group(1)
    assert body.hex() == expected_hex, "enum bytes differ from oracle"
    print("CHAR-ENUM-BYTE-EXACT")

    client.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", GUID))
    op, body = client.read_packet()
    assert op == SMSG_LOGIN_VERIFY_WORLD, op
    map_id, x, y, z, o = struct.unpack("<Iffff", body)
    print(f"VERIFY-WORLD map={map_id} pos=({x:.2f},{y:.2f},{z:.2f}) o={o:.2f}")

    op, body = client.read_packet()
    assert op == SMSG_ACCOUNT_DATA_MD5 and len(body) == 128, (op, len(body))
    print("ACCOUNT-DATA-MD5")

    op, body = client.read_packet()
    assert op == SMSG_FRIEND_LIST, op
    print(f"FRIEND-LIST count={body[0]}")

    op, body = client.read_packet()
    assert op == SMSG_IGNORE_LIST, op
    print(f"IGNORE-LIST count={body[0]}")

    for _ in range(2):
        op, body = client.read_packet()
        assert op == SMSG_MESSAGECHAT, op
    assert b"Patch 1.12: Drums of War is now live!" in body
    print("MOTD-OK")

    client.send_packet(CMSG_PING, struct.pack("<II", 0xDEADBEEF, 50))
    op, body = client.read_packet()
    assert op == SMSG_PONG and body == struct.pack("<I", 0xDEADBEEF)
    print("PING-PONG")

    # Negative: another account's guid must kick with no train.
    other = Client()
    other.auth()
    other.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", OTHER_GUID))
    other.expect_closed()
    print("FOREIGN-GUID-KICKED")

    print("SCRIPTED-LOGIN-PASS")


if __name__ == "__main__":
    sys.exit(main())
