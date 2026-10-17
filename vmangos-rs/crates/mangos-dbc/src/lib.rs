//! DBC (`WDBC`) parsing mirroring `vmangos/src/shared/Database/DBCFileLoader.*`
//! and the `DBCStorage` produce/index semantics of `DBCStore.h`.
//!
//! Raw layout (all little-endian): magic `WDBC`, then `recordCount`,
//! `fieldCount`, `recordSize`, `stringSize` as u32, then
//! `recordCount * recordSize` bytes of records and `stringSize` bytes of
//! NUL-terminated string block. Field offsets inside a record follow the
//! format string: byte fields (`b`/`X`) take 1 byte, everything else 4.
//! Trailing bytes past the declared sections are ignored, exactly like the
//! C++ `fread` of the computed length.
//!
//! Where the C++ asserts (bad field index, string offset past the block,
//! unknown format char, `FT_LOGIC`), this crate returns errors or `None`.

/// One format-string field kind (`FieldFormat`, `DBCFileLoader.h:28-42`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldFormat {
    /// `x` — ignored, 4 bytes in the file.
    Na,
    /// `X` — ignored, 1 byte in the file.
    NaByte,
    /// `F` — ignored float (unused by in-tree formats).
    NaFloat,
    /// `p` — pointer-sized default (unused by in-tree formats).
    NaPointer,
    /// `s` — string-block offset.
    String,
    /// `f` — f32.
    Float,
    /// `i` — u32.
    Int,
    /// `b` — u8.
    Byte,
    /// `d` — index (sort) field, excluded from produced data.
    Sort,
    /// `n` — index field, included in produced data.
    Index,
    /// `l` — boolean; the C++ asserts DBC files never use it.
    Logic,
    /// `L` — u64 (unused by in-tree formats).
    Int64,
}

impl FieldFormat {
    /// Parse one format character; `None` where C++ hits its
    /// "unknown format character" assert.
    #[must_use]
    pub const fn from_char(c: u8) -> Option<Self> {
        Some(match c {
            b'x' => Self::Na,
            b'X' => Self::NaByte,
            b'F' => Self::NaFloat,
            b'p' => Self::NaPointer,
            b's' => Self::String,
            b'f' => Self::Float,
            b'i' => Self::Int,
            b'b' => Self::Byte,
            b'd' => Self::Sort,
            b'n' => Self::Index,
            b'l' => Self::Logic,
            b'L' => Self::Int64,
            _ => return None,
        })
    }

    /// Byte width of this field inside a raw file record
    /// (`DBCFileLoader.cpp:94-101`: only `b`/`X` are 1 byte).
    #[must_use]
    pub const fn file_width(self) -> usize {
        match self {
            Self::Byte | Self::NaByte => 1,
            _ => 4,
        }
    }

    /// Whether the field survives into produced records
    /// (`AutoProduceData` skips `x`/`X`/`d`).
    #[must_use]
    pub const fn is_produced(self) -> bool {
        matches!(
            self,
            Self::Float | Self::Int | Self::Index | Self::Byte | Self::String
        )
    }

    /// Whether this field is the index key (`n`/`d`).
    #[must_use]
    pub const fn is_index(self) -> bool {
        matches!(self, Self::Sort | Self::Index)
    }
}

/// Parse a whole format string into field kinds.
///
/// # Errors
///
/// Returns [`DbcError::UnknownFormatChar`] for a character the C++
/// switch would reject, and [`DbcError::LogicField`] for `l`.
pub fn parse_format(fmt: &str) -> Result<Vec<FieldFormat>, DbcError> {
    fmt.bytes()
        .map(|c| match FieldFormat::from_char(c) {
            Some(FieldFormat::Logic) => Err(DbcError::LogicField),
            Some(f) => Ok(f),
            None => Err(DbcError::UnknownFormatChar(c as char)),
        })
        .collect()
}

/// Failure modes of DBC parsing/production.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DbcError {
    /// Input shorter than the 20-byte header or the declared sections.
    Truncated,
    /// Magic is not `WDBC`.
    BadMagic,
    /// Format length disagrees with the file's `fieldCount`
    /// (C++ `AutoProduce*` returns null).
    FormatLengthMismatch {
        /// Fields in the format string.
        format_fields: usize,
        /// Fields declared by the file header.
        file_fields: u32,
    },
    /// A character outside the C++ `FieldFormat` set.
    UnknownFormatChar(char),
    /// `l` (logic) field — asserted impossible by the C++.
    LogicField,
    /// Declared sizes overflow `usize` arithmetic.
    SizeOverflow,
}

impl std::fmt::Display for DbcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated DBC file"),
            Self::BadMagic => write!(f, "bad DBC magic (not WDBC)"),
            Self::FormatLengthMismatch {
                format_fields,
                file_fields,
            } => write!(
                f,
                "format has {format_fields} fields, file has {file_fields}"
            ),
            Self::UnknownFormatChar(c) => write!(f, "unknown DBC format character {c:?}"),
            Self::LogicField => write!(f, "DBC files cannot have logic fields"),
            Self::SizeOverflow => write!(f, "DBC declared sizes overflow"),
        }
    }
}

impl std::error::Error for DbcError {}

const HEADER_SIZE: usize = 20;

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
}

/// A parsed DBC file: header, raw records, and the string block.
/// Equivalent of a loaded `DBCFileLoader`.
#[derive(Clone, Debug)]
pub struct DbcFile {
    record_count: u32,
    field_count: u32,
    record_size: u32,
    string_size: u32,
    fields_offset: Vec<u32>,
    format: Vec<FieldFormat>,
    records: Vec<u8>,
    string_block: Vec<u8>,
}

impl DbcFile {
    /// Parse `bytes` under `fmt` (`DBCFileLoader::Load`).
    ///
    /// # Errors
    ///
    /// [`DbcError::BadMagic`]/[`DbcError::Truncated`]/
    /// [`DbcError::SizeOverflow`] for malformed input, format errors
    /// from [`parse_format`]. Note the C++ loader computes field
    /// offsets from the format's first `fieldCount` characters
    /// without a length check; the length check happens later in
    /// `AutoProduce*` ([`DbcFile::produce`] here). To stay memory-safe
    /// this requires `fmt` to name at least `fieldCount` fields.
    pub fn parse(bytes: &[u8], fmt: &str) -> Result<Self, DbcError> {
        let format = parse_format(fmt)?;
        if bytes.len() < HEADER_SIZE {
            return Err(DbcError::Truncated);
        }
        if &bytes[0..4] != b"WDBC" {
            return Err(DbcError::BadMagic);
        }
        let record_count = read_u32(bytes, 4).ok_or(DbcError::Truncated)?;
        let field_count = read_u32(bytes, 8).ok_or(DbcError::Truncated)?;
        let record_size = read_u32(bytes, 12).ok_or(DbcError::Truncated)?;
        let string_size = read_u32(bytes, 16).ok_or(DbcError::Truncated)?;

        if (field_count as usize) > format.len() {
            return Err(DbcError::FormatLengthMismatch {
                format_fields: format.len(),
                file_fields: field_count,
            });
        }
        let mut fields_offset = Vec::with_capacity(field_count as usize);
        let mut offset: u32 = 0;
        for field in format.iter().take(field_count as usize) {
            fields_offset.push(offset);
            offset = offset
                .checked_add(field.file_width() as u32)
                .ok_or(DbcError::SizeOverflow)?;
        }

        let records_len = (record_size as usize)
            .checked_mul(record_count as usize)
            .ok_or(DbcError::SizeOverflow)?;
        let total = records_len
            .checked_add(string_size as usize)
            .and_then(|n| n.checked_add(HEADER_SIZE))
            .ok_or(DbcError::SizeOverflow)?;
        if bytes.len() < total {
            return Err(DbcError::Truncated);
        }
        let records = bytes[HEADER_SIZE..HEADER_SIZE + records_len].to_vec();
        let string_block = bytes
            [HEADER_SIZE + records_len..HEADER_SIZE + records_len + string_size as usize]
            .to_vec();

        Ok(Self {
            record_count,
            field_count,
            record_size,
            string_size,
            fields_offset,
            format,
            records,
            string_block,
        })
    }

    /// `GetNumRows`.
    #[must_use]
    pub const fn record_count(&self) -> u32 {
        self.record_count
    }

    /// `GetCols`.
    #[must_use]
    pub const fn field_count(&self) -> u32 {
        self.field_count
    }

    /// Raw record size from the header.
    #[must_use]
    pub const fn record_size(&self) -> u32 {
        self.record_size
    }

    /// String-block size from the header.
    #[must_use]
    pub const fn string_size(&self) -> u32 {
        self.string_size
    }

    /// The format this file was parsed under.
    #[must_use]
    pub fn format(&self) -> &[FieldFormat] {
        &self.format
    }

    /// `GetOffset(field)`.
    #[must_use]
    pub fn field_offset(&self, field: usize) -> Option<u32> {
        self.fields_offset.get(field).copied()
    }

    /// `getRecord(id)` — `None` past the last row.
    #[must_use]
    pub fn record(&self, id: u32) -> Option<DbcRecord<'_>> {
        if id >= self.record_count {
            return None;
        }
        let start = id as usize * self.record_size as usize;
        Some(DbcRecord {
            file: self,
            offset: start,
        })
    }

    /// The raw record-section bytes (all rows concatenated).
    #[must_use]
    pub fn records_bytes(&self) -> &[u8] {
        &self.records
    }

    /// The raw string block bytes.
    #[must_use]
    pub fn string_block(&self) -> &[u8] {
        &self.string_block
    }

    /// Resolve a string-block offset to its NUL-terminated bytes
    /// (the bytes behind `Record::getString`; `None` where the C++
    /// assert `stringOffset < stringSize` would fire or no NUL
    /// terminates the string inside the block).
    #[must_use]
    pub fn string_at(&self, offset: u32) -> Option<&[u8]> {
        let block = self.string_block.get(offset as usize..)?;
        let end = block.iter().position(|&b| b == 0)?;
        Some(&block[..end])
    }

    /// `AutoProduceData` + first `AutoProduceStrings`: build the
    /// indexed store of produced records (see [`DbcStore`]).
    ///
    /// # Errors
    ///
    /// [`DbcError::FormatLengthMismatch`] when the format length and
    /// the file's field count disagree (true equality, as in C++).
    pub fn produce(&self, fmt: &str) -> Result<DbcStore, DbcError> {
        let format = parse_format(fmt)?;
        if format.len() != self.field_count as usize {
            return Err(DbcError::FormatLengthMismatch {
                format_fields: format.len(),
                file_fields: self.field_count,
            });
        }
        let index_pos = format.iter().position(|f| f.is_index());

        let mut rows: Vec<Vec<FieldValue>> = Vec::with_capacity(self.record_count as usize);
        for id in 0..self.record_count {
            let Some(record) = self.record(id) else {
                continue;
            };
            let mut row = Vec::new();
            for (x, field) in format.iter().enumerate() {
                match field {
                    FieldFormat::Float => {
                        row.push(FieldValue::Float(record.f32(x).unwrap_or(0.0)));
                    }
                    FieldFormat::Int | FieldFormat::Index => {
                        row.push(FieldValue::UInt(record.u32(x).unwrap_or(0)));
                    }
                    FieldFormat::Byte => row.push(FieldValue::Byte(record.u8(x).unwrap_or(0))),
                    FieldFormat::String => {
                        let text = record
                            .string_bytes(x)
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                            .unwrap_or_default();
                        row.push(FieldValue::Text(text));
                    }
                    _ => {}
                }
            }
            rows.push(row);
        }

        let (row_count, index) = match index_pos {
            Some(pos) => {
                let mut max_index: u32 = 0;
                for id in 0..self.record_count {
                    if let Some(v) = self.record(id).and_then(|r| r.u32(pos))
                        && v > max_index
                    {
                        max_index = v;
                    }
                }
                let count = max_index + 1;
                let mut index = vec![None; count as usize];
                for (row_idx, id) in (0..self.record_count).enumerate() {
                    if let Some(v) = self.record(id).and_then(|r| r.u32(pos)) {
                        index[v as usize] = Some(row_idx);
                    }
                }
                (count, index)
            }
            None => (
                self.record_count,
                (0..self.record_count as usize).map(Some).collect(),
            ),
        };

        Ok(DbcStore {
            format,
            rows,
            index,
            row_count,
        })
    }
}

/// Size of the packed C++ struct a format produces, with the index
/// field position (`DBCFileLoader::GetFormatRecordSize`). String
/// fields count as an 8-byte pointer, as on the 64-bit platforms the
/// C++ targets here. Kept for parity checks against `DBCStructure.h`
/// struct sizes; the Rust store itself holds values, not packed rows.
///
/// # Errors
///
/// Same format errors as [`parse_format`].
pub fn format_record_size(fmt: &str) -> Result<(usize, Option<usize>), DbcError> {
    let format = parse_format(fmt)?;
    let mut size = 0usize;
    let mut index_pos = None;
    for (x, field) in format.iter().enumerate() {
        match field {
            FieldFormat::Float | FieldFormat::Int => size += 4,
            FieldFormat::String => size += 8,
            FieldFormat::Sort => index_pos = Some(x),
            FieldFormat::Index => {
                index_pos = Some(x);
                size += 4;
            }
            FieldFormat::Byte => size += 1,
            _ => {}
        }
    }
    Ok((size, index_pos))
}

/// One row of a [`DbcFile`] (`DBCFileLoader::Record`).
#[derive(Clone, Copy, Debug)]
pub struct DbcRecord<'a> {
    file: &'a DbcFile,
    offset: usize,
}

impl DbcRecord<'_> {
    fn field_bytes(&self, field: usize, width: usize) -> Option<&[u8]> {
        let offset = self.file.field_offset(field)? as usize;
        self.file
            .records
            .get(self.offset + offset..self.offset + offset + width)
    }

    /// `getUInt`.
    #[must_use]
    pub fn u32(&self, field: usize) -> Option<u32> {
        self.field_bytes(field, 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    }

    /// `getFloat` (bit-exact LE f32).
    #[must_use]
    pub fn f32(&self, field: usize) -> Option<f32> {
        self.u32(field).map(f32::from_bits)
    }

    /// `getUInt8`.
    #[must_use]
    pub fn u8(&self, field: usize) -> Option<u8> {
        self.field_bytes(field, 1).map(|b| b[0])
    }

    /// `getString`, as raw bytes (DBC strings are locale-encoded;
    /// the C++ treats them as `char const*`).
    #[must_use]
    pub fn string_bytes(&self, field: usize) -> Option<&[u8]> {
        self.file.string_at(self.u32(field)?)
    }
}

/// One produced field value (`AutoProduceData` output cell).
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// `i`/`n` field.
    UInt(u32),
    /// `f` field.
    Float(f32),
    /// `b` field.
    Byte(u8),
    /// `s` field, resolved from the string block (lossy UTF-8).
    Text(String),
}

/// Produced, indexed DBC contents (`DBCStorage` + its `indexTable`).
#[derive(Clone, Debug)]
pub struct DbcStore {
    format: Vec<FieldFormat>,
    rows: Vec<Vec<FieldValue>>,
    index: Vec<Option<usize>>,
    row_count: u32,
}

impl DbcStore {
    /// `GetNumRows` — max index + 1 when the format has an index
    /// field, else the file record count.
    #[must_use]
    pub const fn row_count(&self) -> u32 {
        self.row_count
    }

    /// Number of records actually present.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no records are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `LookupEntry(id)`: the produced record for an index id.
    #[must_use]
    pub fn lookup(&self, id: u32) -> Option<&[FieldValue]> {
        self.index
            .get(id as usize)
            .copied()
            .flatten()
            .and_then(|row| self.rows.get(row))
            .map(Vec::as_slice)
    }

    /// All produced records in file order.
    #[must_use]
    pub fn rows(&self) -> &[Vec<FieldValue>] {
        &self.rows
    }

    /// `AutoProduceStrings` locale merge (`LoadStringsFrom`): for
    /// each record (aligned by file order, as in C++), string fields
    /// that are currently empty take the other file's string.
    /// `other` must have been parsed with a compatible format; rows
    /// or fields missing there are skipped.
    pub fn merge_strings_from(&mut self, other: &DbcFile) {
        let string_fields: Vec<usize> = self
            .format
            .iter()
            .enumerate()
            .filter(|(_, f)| **f == FieldFormat::String)
            .map(|(x, _)| x)
            .collect();
        if string_fields.is_empty() {
            return;
        }
        // Produced-row position of each format field.
        let mut produced_pos: Vec<Option<usize>> = Vec::with_capacity(self.format.len());
        let mut next = 0usize;
        for field in &self.format {
            if field.is_produced() {
                produced_pos.push(Some(next));
                next += 1;
            } else {
                produced_pos.push(None);
            }
        }
        for (row_idx, row) in self.rows.iter_mut().enumerate() {
            let Some(other_record) = other.record(row_idx as u32) else {
                continue;
            };
            for &x in &string_fields {
                let Some(pos) = produced_pos[x] else { continue };
                if let Some(FieldValue::Text(current)) = row.get(pos)
                    && !current.is_empty()
                {
                    continue;
                }
                if let Some(bytes) = other_record.string_bytes(x) {
                    row[pos] = FieldValue::Text(String::from_utf8_lossy(bytes).into_owned());
                }
            }
        }
    }
}

pub mod formats;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal WDBC byte string: records are raw u32 tuples.
    fn make_dbc(records: &[[u32; 3]], strings: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(records.len() as u32).to_le_bytes());
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&12u32.to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for record in records {
            for value in record {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        out
    }

    #[test]
    fn parse_header_and_fields() {
        let bytes = make_dbc(&[[1, 2, 0], [7, 8, 4]], b"one\0two\0");
        let dbc = DbcFile::parse(&bytes, "nis").expect("parse");
        assert_eq!(dbc.record_count(), 2);
        assert_eq!(dbc.field_count(), 3);
        let row = dbc.record(1).expect("row 1");
        assert_eq!(row.u32(0), Some(7));
        assert_eq!(row.string_bytes(2), Some(b"two".as_slice()));
        assert!(dbc.record(2).is_none());
    }

    #[test]
    fn produce_indexes_by_n_field() {
        let bytes = make_dbc(&[[5, 2, 0], [9, 8, 4]], b"one\0two\0");
        let store = DbcFile::parse(&bytes, "nis")
            .expect("parse")
            .produce("nis")
            .expect("produce");
        assert_eq!(store.row_count(), 10);
        assert_eq!(
            store.lookup(5).map(|r| &r[2]),
            Some(&FieldValue::Text("one".into()))
        );
        assert_eq!(store.lookup(9).map(|r| &r[1]), Some(&FieldValue::UInt(8)));
        assert_eq!(store.lookup(6), None);
    }

    #[test]
    fn merge_strings_fills_only_empty() {
        let bytes = make_dbc(&[[1, 2, 0], [2, 3, 1]], b"\0two\0");
        let mut store = DbcFile::parse(&bytes, "nis")
            .expect("parse")
            .produce("nis")
            .expect("produce");
        let locale = DbcFile::parse(&make_dbc(&[[1, 2, 0], [2, 3, 0]], b"un\0"), "nis")
            .expect("locale parse");
        store.merge_strings_from(&locale);
        assert_eq!(
            store.lookup(1).map(|r| &r[2]),
            Some(&FieldValue::Text("un".into()))
        );
        assert_eq!(
            store.lookup(2).map(|r| &r[2]),
            Some(&FieldValue::Text("two".into()))
        );
    }

    #[test]
    fn rejects_bad_magic_truncation_and_length_mismatch() {
        assert_eq!(
            DbcFile::parse(b"NOPE", "ni").unwrap_err(),
            DbcError::Truncated
        );
        assert_eq!(
            DbcFile::parse(b"NOPE1234567890123456", "ni").unwrap_err(),
            DbcError::BadMagic
        );
        let bytes = make_dbc(&[[1, 2, 0]], b"\0");
        let dbc = DbcFile::parse(&bytes, "nisx").expect("loader tolerates longer fmt");
        assert!(matches!(
            dbc.produce("nisx"),
            Err(DbcError::FormatLengthMismatch { .. })
        ));
    }

    #[test]
    fn byte_fields_offset_by_one() {
        // fmt "nb": field 1 is a single byte after the u32 index.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&42u32.to_le_bytes());
        bytes.push(7);
        let dbc = DbcFile::parse(&bytes, "nb").expect("parse");
        let row = dbc.record(0).expect("row");
        assert_eq!(row.u32(0), Some(42));
        assert_eq!(row.u8(1), Some(7));
    }
}
