#!/usr/bin/env python3
"""Independent 1.12.1 logon client for realmd differential testing.

Implements client-side SRP6 from the C++ semantics (BigNumber minimal-LE
feeds) using only the stdlib. Self-validates against the golden
TESTUSER verifier before any network I/O, so a client bug cannot
masquerade as a server bug.
"""

import hashlib
import os
import socket
import struct
import sys

N = int("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7", 16)
G = 7
VC = bytes([0xBA, 0xA3, 0x1E, 0x99, 0xA0, 0x0B, 0x21, 0x57, 0xFC, 0x37, 0x3F, 0xB3, 0x69, 0xCD, 0xD2, 0xF1])


def sha1(*parts):
    h = hashlib.sha1()
    for p in parts:
        h.update(p)
    return h.digest()


def le_min(n):
    if n == 0:
        return b""
    return n.to_bytes((n.bit_length() + 7) // 8, "little")


def le_pad(n, size):
    return n.to_bytes(size, "little")


def derive_x(username, password, salt_int):
    digest = sha1(f"{username.upper()}:{password.upper()}".encode())
    ident = int.from_bytes(digest, "big")  # bn_from_hex of the hex digest
    i_bytes = le_min(ident)
    m_digest = bytearray(20)
    m_digest[: len(i_bytes)] = i_bytes
    m_digest.reverse()
    return int.from_bytes(sha1(b"", le_min(salt_int), bytes(m_digest)), "little")


def verifier_hex(username, password, salt_hex):
    x = derive_x(username, password, int(salt_hex, 16))
    return format(pow(G, x, N), "X")


def hash_session_key(s_value):
    t = le_pad(s_value, 32)
    he = sha1(t[0::2][:16])
    ho = sha1(t[1::2][:16])
    k = bytearray(40)
    for i in range(20):
        k[i * 2] = he[i]
        k[i * 2 + 1] = ho[i]
    return bytes(k)


# Golden self-check (dashboard/tests/test_auth.py + srp_vectors.json).
assert (
    verifier_hex(
        "TESTUSER",
        "password123",
        "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
    )
    == "68EA0AA0AFC9E06942699D0EB823713252D1BD8B30A1806A3FEFD23B77DB7E34"
), "golden TESTUSER"
assert (
    verifier_hex(
        "AdminGM",
        "shortpw",
        "DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF",
    )
    == "29F1EDE47B46D96CADD6C14437895F6B726F694E04100DD2622ECD9A30B89CCD"
), "golden AdminGM"
print("client SRP self-check: golden verifiers match")


class Conn:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=10)
        self.buf = b""

    def send(self, data):
        self.sock.sendall(data)

    def read_exact(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError(f"closed with {len(self.buf)}/{n} bytes: {self.buf.hex()}")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def read_to_idle(self, idle=0.4):
        self.sock.settimeout(idle)
        try:
            while True:
                chunk = self.sock.recv(65536)
                if not chunk:
                    break
                self.buf += chunk
        except (socket.timeout, TimeoutError):
            pass
        finally:
            self.sock.settimeout(10)
        out, self.buf = self.buf, b""
        return out

    def close(self):
        self.sock.close()


def challenge_packet(cmd, username, build=5875):
    user = username.encode()
    body = b"WoW\x00" + bytes([1, 12, 1]) + struct.pack("<H", build)
    body += b"68x\x00"  # platform wire (server reverses -> x86)
    body += b"niW\x00"  # os wire (server reverses -> Win)
    body += b"SUne"  # country wire (server reverses -> enUS)
    body += struct.pack("<II", 0, 0x7F000001)
    body += bytes([len(user)]) + user
    return bytes([cmd, 0]) + struct.pack("<H", len(body)) + body


def do_challenge(conn, username):
    conn.send(challenge_packet(0x00, username))
    head = conn.read_exact(3)
    assert head[0] == 0x00 and head[1] == 0x00, f"challenge prefix {head.hex()}"
    result = head[2]
    if result != 0:
        return {"result": result}
    b_bytes = conn.read_exact(32)
    glen = conn.read_exact(1)[0]
    g_bytes = conn.read_exact(glen)
    nlen = conn.read_exact(1)[0]
    n_bytes = conn.read_exact(nlen)
    tail = conn.read_to_idle()
    salt = tail[:-17]
    vc = tail[-17:-1]
    secflags = tail[-1]
    assert vc == VC, "version challenge mismatch"
    return {
        "result": 0,
        "B": int.from_bytes(b_bytes, "little"),
        "g": int.from_bytes(g_bytes, "little"),
        "N": int.from_bytes(n_bytes, "little"),
        "salt": int.from_bytes(salt, "little"),
        "secflags": secflags,
    }


def client_proof(username, password, ch):
    a = int.from_bytes(os.urandom(32), "little")
    A = pow(G, a, N)
    x = derive_x(username, password, ch["salt"])
    u = int.from_bytes(sha1(le_min(A), le_min(ch["B"])), "little")
    s_value = pow((ch["B"] - 3 * pow(G, x, N)) % N, a + u * x, N)
    K = hash_session_key(s_value)
    k_int = int.from_bytes(K, "little")
    h_n = sha1(le_min(N))
    h_g = sha1(le_min(G))
    t3 = int.from_bytes(bytes(x ^ y for x, y in zip(h_n, h_g)), "little")
    m1 = sha1(le_min(t3), sha1(username.encode()), le_min(ch["salt"]), le_min(A), le_min(ch["B"]), le_min(k_int))
    m1_int = int.from_bytes(m1, "little")
    m2_expected = sha1(le_min(A), le_min(m1_int), le_min(k_int))
    return A, m1, m2_expected, K


def parse_realm_list(body):
    off = 0
    (unk,) = struct.unpack_from("<I", body, off)
    off += 4
    count = body[off]
    off += 1
    realms = []
    for _ in range(count):
        (icon,) = struct.unpack_from("<I", body, off)
        off += 4
        flags = body[off]
        off += 1

        def cstr():
            nonlocal off
            end = body.index(b"\x00", off)
            s = body[off:end].decode()
            off = end + 1
            return s

        name = cstr()
        addr = cstr()
        (pop,) = struct.unpack_from("<f", body, off)
        off += 4
        numchars = body[off]
        cat = body[off + 1]
        off += 3
        realms.append(dict(icon=icon, flags=flags, name=name, addr=addr, pop=pop, numchars=numchars, cat=cat))
    (tail,) = struct.unpack_from("<H", body, off)
    return realms, tail


def realm_list(conn):
    conn.send(b"\x10\x00\x00\x00\x00")
    head = conn.read_exact(3)
    assert head[0] == 0x10, f"realm list cmd {head.hex()}"
    (size,) = struct.unpack("<H", head[1:3])
    body = conn.read_exact(size)
    return parse_realm_list(body)


def full_logon(host, port, username, password, expect_proof_error=None):
    conn = Conn(host, port)
    ch = do_challenge(conn, username)
    print(f"  challenge result={ch['result']}")
    if ch["result"] != 0:
        conn.close()
        return ch, None
    assert ch["g"] == 7 and ch["N"] == N and ch["secflags"] == 0
    A, m1, m2_expected, K = client_proof(username, password, ch)
    proof = b"\x01" + le_pad(A, 32) + m1 + b"\x00" * 20 + b"\x00\x00"
    conn.send(proof)
    resp_head = conn.read_exact(2)
    assert resp_head[0] == 0x01, f"proof cmd {resp_head.hex()}"
    error = resp_head[1]
    print(f"  proof error={error}")
    if expect_proof_error is not None:
        assert error == expect_proof_error, f"expected proof error {expect_proof_error}, got {error}"
        conn.close()
        return ch, None
    assert error == 0, f"logon failed with {error}"
    m2 = conn.read_exact(20)
    conn.read_exact(4)  # surveyId
    assert m2 == m2_expected, "M2 mismatch: server session key differs from client"
    print("  M2 verified: client/server session keys identical")
    realms, tail = realm_list(conn)
    print(f"  realms: {realms} tail=0x{tail:04x}")
    conn.close()
    return ch, K


def reconnect(host, port, username, K):
    conn = Conn(host, port)
    conn.send(challenge_packet(0x02, username))
    resp = conn.read_exact(34)
    assert resp[0] == 0x02 and resp[1] == 0x00, f"reconnect challenge {resp[:2].hex()}"
    proof_bytes = resp[2:18]
    assert resp[18:34] == VC
    proof_int = int.from_bytes(proof_bytes, "little")
    k_int = int.from_bytes(K, "little")
    r1 = bytes(range(16, 32))
    t1 = int.from_bytes(r1, "little")
    r2 = sha1(username.encode(), le_min(t1), le_min(proof_int), le_min(k_int))
    r3 = sha1(r1, b"\x00" * 20)
    conn.send(b"\x03" + r1 + r2 + r3 + b"\x00")
    resp2 = conn.read_exact(2)
    assert resp2 == b"\x03\x00", f"reconnect proof resp {resp2.hex()}"
    print("  reconnect proof accepted")
    realms, tail = realm_list(conn)
    print(f"  realms after reconnect: {[r['name'] for r in realms]}")
    conn.close()


def main():
    host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 37241
    print(f"== unknown account ({host}:{port})")
    ch, _ = full_logon(host, port, "NOSUCHUSER", "whatever")
    assert ch["result"] == 0x04, ch
    print("  unknown account -> WOW_FAIL_UNKNOWN_ACCOUNT ok")

    print("== wrong password")
    full_logon(host, port, "TESTUSER", "wrongpass", expect_proof_error=0x04)
    print("  wrong password -> proof UNKNOWN_ACCOUNT ok")

    print("== successful logon + realm list")
    _, K = full_logon(host, port, "TESTUSER", "password123")
    print(f"SESSIONKEY_HEX={format(int.from_bytes(K, 'little'), 'X')}")

    print("== reconnect")
    reconnect(host, port, "TESTUSER", K)
    print("ALL SCENARIOS PASS")


if __name__ == "__main__":
    main()
