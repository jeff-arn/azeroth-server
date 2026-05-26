"""Tests for scripts/dbc.py — the binary DBC reader."""

import struct

import pytest
from dbc import get_string, read_dbc


def make_dbc(records, string_block, field_count=None):
    """Build a minimal DBC binary blob in memory.

    records: list of tuples of uint32 values (all rows same width)
    string_block: raw bytes for the string section
    """
    if not records:
        record_size = 0
        fields_per_record = 0
    else:
        fields_per_record = len(records[0])
        record_size = fields_per_record * 4
    record_count = len(records)
    fc = field_count if field_count is not None else fields_per_record
    header = b"WDBC" + struct.pack("<4I", record_count, fc, record_size, len(string_block))
    body = b"".join(struct.pack(f"<{fields_per_record}I", *r) for r in records)
    return header + body + string_block


# ---- read_dbc -------------------------------------------------------------


def test_read_dbc_rejects_non_dbc_file(tmp_path):
    bad = tmp_path / "bad.dbc"
    bad.write_bytes(b"XXXX" + b"\x00" * 16)
    with pytest.raises(AssertionError, match="Not a DBC file"):
        read_dbc(str(bad))


def test_read_dbc_empty_record_set(tmp_path):
    f = tmp_path / "empty.dbc"
    # 0 records, but include field_count + record_size = 4 so header is valid
    header = b"WDBC" + struct.pack("<4I", 0, 1, 4, 0)
    f.write_bytes(header)
    records, string_block = read_dbc(str(f))
    assert records == []
    assert string_block == b""


def test_read_dbc_single_record_single_field(tmp_path):
    f = tmp_path / "single.dbc"
    f.write_bytes(make_dbc([(42,)], b""))
    records, string_block = read_dbc(str(f))
    assert records == [(42,)]
    assert string_block == b""


def test_read_dbc_multiple_records_multiple_fields(tmp_path):
    f = tmp_path / "multi.dbc"
    payload = [
        (1, 100, 200, 300),
        (2, 400, 500, 600),
        (3, 700, 800, 900),
    ]
    f.write_bytes(make_dbc(payload, b""))
    records, _ = read_dbc(str(f))
    assert records == payload


def test_read_dbc_preserves_string_block(tmp_path):
    f = tmp_path / "withstrings.dbc"
    sb = b"\x00Hello\x00World\x00"
    f.write_bytes(make_dbc([(1, 1), (2, 7)], sb))
    records, string_block = read_dbc(str(f))
    assert records == [(1, 1), (2, 7)]
    assert string_block == sb


def test_read_dbc_handles_max_uint32(tmp_path):
    f = tmp_path / "maxuint.dbc"
    max_u32 = 0xFFFFFFFF
    f.write_bytes(make_dbc([(max_u32, 0, max_u32)], b""))
    records, _ = read_dbc(str(f))
    assert records == [(max_u32, 0, max_u32)]


# ---- get_string -----------------------------------------------------------


def test_get_string_at_offset_zero():
    block = b"hello\x00world\x00"
    assert get_string(block, 0) == "hello"


def test_get_string_at_nonzero_offset():
    block = b"hello\x00world\x00"
    assert get_string(block, 6) == "world"


def test_get_string_empty_string():
    block = b"\x00rest\x00"
    assert get_string(block, 0) == ""


def test_get_string_utf8_decoding():
    block = "café\x00".encode("utf-8")
    assert get_string(block, 0) == "café"


def test_get_string_invalid_utf8_does_not_crash():
    # 0xFF is not valid UTF-8; replace strategy should produce U+FFFD
    block = b"abc\xff\x00"
    result = get_string(block, 0)
    assert result.startswith("abc")
    # remaining char is the replacement character
    assert len(result) == 4


def test_get_string_missing_terminator_raises():
    block = b"unterminated"
    with pytest.raises(ValueError):
        get_string(block, 0)
