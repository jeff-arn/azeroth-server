"""Tests for the SRP6 password verifier in dashboard/auth.py.

The vectors below were generated against the working implementation
that has been verifying real player logins. If a future change to the
SRP6 flow (encoding, hashing order, endianness) breaks compatibility
with the realmd C++ verifier generation, these tests will fail.
"""

import pytest
from auth import _verify_srp6_password

# Each tuple: (username, password, salt_hex, expected_verifier_hex)
# Salts are 32 bytes (64 hex chars), verifiers are <= 32 bytes (varies).
VECTORS = [
    (
        "TESTUSER",
        "password123",
        "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
        "68EA0AA0AFC9E06942699D0EB823713252D1BD8B30A1806A3FEFD23B77DB7E34",
    ),
    (
        "AdminGM",
        "shortpw",
        "DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF",
        "29F1EDE47B46D96CADD6C14437895F6B726F694E04100DD2622ECD9A30B89CCD",
    ),
    (
        "USER3",
        "xxxxxxxxxxxxxxxx",
        "0000000000000000000000000000000000000000000000000000000000000001",
        "88188611247BB484DEB46509E0915195D1601C59064487D03DDFA659B1879BE1",
    ),
]


@pytest.mark.parametrize("username,password,s_hex,v_hex", VECTORS)
def test_correct_password_verifies(username, password, s_hex, v_hex):
    assert _verify_srp6_password(username, password, s_hex, v_hex) is True


@pytest.mark.parametrize("username,password,s_hex,v_hex", VECTORS)
def test_wrong_password_rejects(username, password, s_hex, v_hex):
    assert _verify_srp6_password(username, password + "_wrong", s_hex, v_hex) is False


@pytest.mark.parametrize("username,password,s_hex,v_hex", VECTORS)
def test_empty_password_rejects(username, password, s_hex, v_hex):
    assert _verify_srp6_password(username, "", s_hex, v_hex) is False


def test_username_is_case_insensitive():
    # The realmd C++ side uppercases both fields before SHA1 — verify that
    # both lowercase and mixed-case usernames produce the same result.
    u, p, s, v = VECTORS[0]
    assert _verify_srp6_password(u.lower(), p, s, v) is True
    assert _verify_srp6_password(u.title(), p, s, v) is True


def test_password_is_case_insensitive():
    # The vanilla client also uppercases the password before SHA1.
    u, p, s, v = VECTORS[0]
    assert _verify_srp6_password(u, p.upper(), s, v) is True


def test_wrong_salt_rejects():
    u, p, s, v = VECTORS[0]
    # Flip the leading hex digit of the salt and verify it now fails.
    wrong_s = ("F" if s[0] != "F" else "0") + s[1:]
    assert _verify_srp6_password(u, p, wrong_s, v) is False


def test_wrong_verifier_rejects():
    u, p, s, v = VECTORS[0]
    # Flip the trailing hex digit of the verifier.
    wrong_v = v[:-1] + ("F" if v[-1] != "F" else "0")
    assert _verify_srp6_password(u, p, s, wrong_v) is False
