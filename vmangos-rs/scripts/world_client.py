#!/usr/bin/env python3
"""Shadow-run world client: Rust realmd logon -> C++ mangosd char list.

Uses /tmp/logon_client.py for the SRP logon, then speaks just enough
1.12.1 world protocol (AuthCrypt header crypto, CMSG_AUTH_SESSION,
CMSG_CHAR_ENUM/CREATE, CMSG_PLAYER_LOGIN) to prove the session key
written by the Rust realmd is accepted by the C++ world server.
"""

import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import socket

import logon_client as lc

SMSG_AUTH_CHALLENGE = 492
CMSG_AUTH_SESSION = 493
CMSG_CHAR_CREATE = 54
CMSG_CHAR_ENUM = 55
SMSG_CHAR_CREATE = 58
SMSG_CHAR_ENUM = 59
CMSG_PLAYER_LOGIN = 61
SMSG_LOGIN_VERIFY_WORLD = 566


class Crypt:
    """Mirror of the client side of AuthCrypt.cpp (header-only)."""

    def __init__(self, key):
        self.key = key or b"\x00"
        self.send_i = self.send_j = 0
        self.recv_i = self.recv_j = 0

    def encrypt_send(self, data: bytearray):
        for t in range(6):
            self.send_i %= len(self.key)
            x = ((data[t] ^ self.key[self.send_i]) + self.send_j) & 0xFF
            self.send_i += 1
            data[t] = self.send_j = x

    def decrypt_recv(self, data: bytearray):
        for t in range(4):
            self.recv_i %= len(self.key)
            x = ((data[t] - self.recv_j) & 0xFF) ^ self.key[self.recv_i]
            self.recv_i += 1
            self.recv_j = data[t]
            data[t] = x


class WorldConn:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=15)
        self.buf = b""
        self.crypt = None

    def _fill(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError("world socket closed")
            self.buf += chunk

    def read_exact(self, n):
        self._fill(n)
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send_packet(self, opcode, body=b""):
        header = bytearray(struct.pack(">H", len(body) + 4) + struct.pack("<I", opcode))
        if self.crypt:
            self.crypt.encrypt_send(header)
        self.sock.sendall(bytes(header) + body)

    def read_packet(self, timeout=15.0):
        self.sock.settimeout(timeout)
        header = bytearray(self.read_exact(4))
        if self.crypt:
            self.crypt.decrypt_recv(header)
        size = struct.unpack(">H", header[0:2])[0] - 2
        opcode = struct.unpack("<H", header[2:4])[0]
        body = self.read_exact(size) if size > 0 else b""
        return opcode, body

    def close(self):
        self.sock.close()


def expect_packet(wc, wanted, timeout=20.0):
    """Read packets until `wanted` arrives; return its body. Fails on SMSG_AUTH_RESPONSE."""
    deadline = time.time() + timeout
    seen = []
    while time.time() < deadline:
        opcode, body = wc.read_packet(timeout=max(1.0, deadline - time.time()))
        seen.append(opcode)
        if opcode == 494:  # SMSG_AUTH_RESPONSE; 0x0C == AUTH_OK
            if body and body[0] == 0x0C:
                print("  SMSG_AUTH_RESPONSE: AUTH_OK")
                continue
            raise AssertionError(f"AUTH_RESPONSE failure code {body.hex()} (seen {seen})")
        if opcode == wanted:
            return body, seen
    raise AssertionError(f"timeout waiting for opcode {wanted}; seen {seen}")


def parse_char_enum(body):
    count = body[0]
    chars = []
    off = 1
    for _ in range(count):
        guid = struct.unpack_from("<Q", body, off)[0]
        off += 8
        end = body.index(b"\x00", off)
        name = body[off:end].decode()
        off = end + 1
        race, cls = body[off], body[off + 1]
        chars.append((guid, name, race, cls))
        # Tail per char (1.12.1): gender..level 7B + zone/map 8B + xyz 12B +
        # guild 4B + flags 4B + firstlogin 1B + pet 12B + 19*(4+1) equip.
        # Skip via next-char search is unsafe; use the fixed size after name
        # minus the two bytes already consumed conceptually below.
        off += 7 + 8 + 12 + 4 + 4 + 1 + 12 + 19 * 5
    return chars


def main():
    realmd_port = int(sys.argv[1]) if len(sys.argv) > 1 else 37241
    world_port = int(sys.argv[2]) if len(sys.argv) > 2 else 18085
    username = "SHADOWTEST"
    password = "shadowpass1"

    print("== step 1: SRP logon via Rust realmd")
    _, K = lc.full_logon("127.0.0.1", realmd_port, username, password)
    k_int = int.from_bytes(K, "little")
    k_feed = lc.le_min(k_int)
    print(f"  session key established, feed {len(k_feed)} bytes")

    print("== step 2: world AUTH_SESSION vs oracle mangosd")
    wc = WorldConn("127.0.0.1", world_port)
    opcode, body = wc.read_packet()
    assert opcode == SMSG_AUTH_CHALLENGE, f"first world packet {opcode}"
    server_seed = struct.unpack("<I", body[:4])[0]
    client_seed = 0x11223344
    digest = lc.sha1(
        username.encode(),
        struct.pack("<I", 0),
        struct.pack("<I", client_seed),
        struct.pack("<I", server_seed),
        k_feed,
    )
    auth_body = (
        struct.pack("<I", 5875)
        + struct.pack("<I", 1)
        + username.encode()
        + b"\x00"
        + struct.pack("<I", client_seed)
        + digest
    )
    wc.send_packet(CMSG_AUTH_SESSION, auth_body)
    wc.crypt = Crypt(k_feed)
    print("  AUTH_SESSION sent, header crypt initialized")

    print("== step 3: character enum (expect empty)")
    wc.send_packet(CMSG_CHAR_ENUM)
    body, seen = expect_packet(wc, SMSG_CHAR_ENUM)
    chars = parse_char_enum(body)
    print(f"  opcodes seen {seen}; chars={chars}")

    if not chars:
        print("== step 4: create character Rustshadow")
        create_body = (
            b"Rustshadow\x00"
            + bytes([1, 1, 0, 0, 0, 0, 0, 0, 0])  # race,class,gender,skin,face,hair,haircolor,facial,outfit
        )
        wc.send_packet(CMSG_CHAR_CREATE, create_body)
        body, seen = expect_packet(wc, SMSG_CHAR_CREATE)
        print(f"  create result byte=0x{body[0]:02x} (seen {seen})")
        wc.send_packet(CMSG_CHAR_ENUM)
        body, seen = expect_packet(wc, SMSG_CHAR_ENUM)
        chars = parse_char_enum(body)
        print(f"  chars after create={chars}")
        assert chars and chars[0][1] == "Rustshadow", chars

    print("== step 5: player login -> LOGIN_VERIFY_WORLD")
    guid = chars[0][0]
    wc.send_packet(CMSG_PLAYER_LOGIN, struct.pack("<Q", guid))
    body, seen = expect_packet(wc, SMSG_LOGIN_VERIFY_WORLD, timeout=30.0)
    map_id = struct.unpack_from("<I", body, 0)[0]
    x, y, z, o = struct.unpack_from("<ffff", body, 4)
    print(f"  verify world: map={map_id} pos=({x:.1f},{y:.1f},{z:.1f}) o={o:.2f} (opcodes {seen})")
    wc.close()
    print("SHADOW RUN PASS")


if __name__ == "__main__":
    main()
