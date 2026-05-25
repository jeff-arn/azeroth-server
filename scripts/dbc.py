"""DBC (Database Client) binary file reader for WoW 1.12.1 client data."""

import struct


def read_dbc(filepath, field_count_override=None):
    """Read a DBC file and return (records, string_block).

    records is a list of tuples of uint32 fields. string_block is the
    raw bytes of the null-terminated string section that follows the
    fixed-width records.
    """
    with open(filepath, "rb") as f:
        magic = f.read(4)
        assert magic == b"WDBC", f"Not a DBC file: {filepath}"
        record_count, field_count, record_size, string_block_size = struct.unpack("<4I", f.read(16))

        fields_per_record = record_size // 4
        records = []
        for _ in range(record_count):
            raw = f.read(record_size)
            fields = struct.unpack(f"<{fields_per_record}I", raw)
            records.append(fields)

        string_block = f.read(string_block_size)
    return records, string_block


def get_string(string_block, offset):
    """Extract null-terminated string from string block at the given offset."""
    end = string_block.index(b"\x00", offset)
    return string_block[offset:end].decode("utf-8", errors="replace")
