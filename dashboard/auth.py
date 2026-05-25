"""SRP6 password verification for WoW 1.12 realmd accounts.

Pure crypto — no DB access, no Flask coupling. The realmd database
stores SRP6 salt (s) and verifier (v) per account; this module checks a
plaintext password against those values.

The implementation must match the C++ flow in vmangos:
SRP6::CalculateVerifier + AccountMgr::CalculateShaPassHash. A
regression here is silent (login starts failing for everyone) so the
verifier is covered by test vectors in dashboard/tests/test_auth.py.
"""

import hashlib

# WoW 1.12 SRP6 constants
SRP6_N = int("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7", 16)
SRP6_g = 7


def _verify_srp6_password(username, password, s_hex, v_hex):
    """Verify password against stored SRP6 salt and verifier.

    Matches the C++ flow in SRP6::CalculateVerifier + AccountMgr::CalculateShaPassHash:
    1. rI = hex(SHA1("USERNAME:PASSWORD"))
    2. BigNumber I from rI hex -> BN_bn2bin (big-endian) -> AsByteArray reverses to LE
    3. Copy into mDigest[20], then std::reverse(mDigest) back to big-endian
    4. s: SetHexStr (big-endian) -> AsByteArray (reversed to little-endian)
    5. x = SHA1(s_LE || mDigest_BE)  -- A is zero/empty, contributes nothing
    6. v = g^x mod N
    """
    username = username.upper()
    password = password.upper()

    # Step 1: CalculateShaPassHash -> SHA1("USERNAME:PASSWORD") -> hex
    identity_hash = hashlib.sha1(f"{username}:{password}".encode()).digest()
    ri_hex = identity_hash.hex().upper()

    # Step 2: BigNumber I from hex. BN_bn2bin = big-endian, AsByteArray reverses to LE
    ri_int = int(ri_hex, 16)
    num_bytes = (ri_int.bit_length() + 7) // 8
    ri_be = ri_int.to_bytes(max(num_bytes, 1), byteorder="big")
    ri_le = ri_be[::-1]

    # Step 3: memset mDigest[20] = 0, memcpy LE bytes in, then std::reverse
    m_digest = bytearray(20)
    copy_len = min(len(ri_le), 20)
    m_digest[:copy_len] = ri_le[:copy_len]
    m_digest.reverse()  # back to big-endian

    # Step 4: salt from hex (big-endian) -> AsByteArray reverses to little-endian
    s_int = int(s_hex, 16)
    s_be = s_int.to_bytes(32, byteorder="big")
    s_le = s_be[::-1]

    # Step 5: x = SHA1(s_LE || mDigest_BE) -- A is empty, feeds 0 bytes
    x_hash = hashlib.sha1(s_le + bytes(m_digest)).digest()
    x = int.from_bytes(x_hash, byteorder="little")  # SetBinary = little-endian

    # Step 6: v = g^x mod N
    computed_v = pow(SRP6_g, x, SRP6_N)

    # ProofVerifier compares AsHexStr() = big-endian hex
    stored_v = int(v_hex, 16)
    return computed_v == stored_v
