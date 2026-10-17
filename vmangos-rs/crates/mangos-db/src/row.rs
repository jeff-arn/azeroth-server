//! Positional row access mirroring the C++ `Field` contract
//! (`Field.h:47-58`): values are stringly-typed, NULL reads as
//! 0 / "" / 0.0, and numeric conversion is `atol`/`atof`-style
//! prefix parsing. Used where result sets exceed the driver's
//! tuple arity (the 23-column char enum, the 58-column
//! character load) and where C++ reads positionally.

use mysql::Value;
use mysql::consts::ColumnType;

/// One result row as raw driver values.
pub struct Fields(Vec<Value>);

impl Fields {
    #[must_use]
    pub fn from_row(row: mysql::Row) -> Fields {
        Fields(row.unwrap())
    }

    fn value(&self, index: usize) -> Option<&Value> {
        self.0.get(index)
    }

    /// Text form, like `Field::GetString` returning the raw
    /// bytes as a C string (NULL → "").
    #[must_use]
    pub fn string(&self, index: usize) -> String {
        match self.value(index) {
            Some(Value::Bytes(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
            Some(Value::NULL) | None => String::new(),
            Some(other) => value_to_text(other),
        }
    }

    /// `GetUInt64`: `atol` prefix, NULL → 0, negatives wrap.
    #[must_use]
    pub fn u64(&self, index: usize) -> u64 {
        self.i64(index) as u64
    }

    /// `GetInt64`-family: `atol` semantics (leading whitespace,
    /// sign, digit prefix; no digits → 0).
    #[must_use]
    pub fn i64(&self, index: usize) -> i64 {
        match self.value(index) {
            Some(Value::Int(v)) => *v,
            Some(Value::UInt(v)) => *v as i64,
            Some(Value::Float(v)) => *v as i64,
            Some(Value::Double(v)) => *v as i64,
            Some(Value::Bytes(bytes)) => atol(bytes),
            _ => 0,
        }
    }

    #[must_use]
    pub fn u32(&self, index: usize) -> u32 {
        self.u64(index) as u32
    }

    #[must_use]
    pub fn i32(&self, index: usize) -> i32 {
        self.i64(index) as i32
    }

    #[must_use]
    pub fn u8(&self, index: usize) -> u8 {
        self.u64(index) as u8
    }

    #[must_use]
    pub fn i16(&self, index: usize) -> i16 {
        self.i64(index) as i16
    }

    #[must_use]
    pub fn u16(&self, index: usize) -> u16 {
        self.u64(index) as u16
    }

    /// `GetFloat`: `atof` prefix rounded to f32, NULL → 0.0.
    #[must_use]
    pub fn f32(&self, index: usize) -> f32 {
        match self.value(index) {
            Some(Value::Float(v)) => *v,
            Some(Value::Double(v)) => *v as f32,
            Some(Value::Int(v)) => *v as f32,
            Some(Value::UInt(v)) => *v as f32,
            Some(Value::Bytes(bytes)) => atof(bytes) as f32,
            _ => 0.0,
        }
    }

    /// Whether the column is SQL NULL (`fields[i].GetString()`
    /// null checks in C++).
    #[must_use]
    pub fn is_null(&self, index: usize) -> bool {
        matches!(self.value(index), Some(Value::NULL) | None)
    }
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        Value::Date(y, mo, d, h, mi, s, _) => {
            format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
        }
        Value::Time(_, d, h, mi, s, _) => format!("{d} {h:02}:{mi:02}:{s:02}"),
        Value::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::NULL => String::new(),
    }
}

/// C `atol`: skip ASCII whitespace, optional sign, digit prefix.
fn atol(bytes: &[u8]) -> i64 {
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let negative = i < bytes.len() && bytes[i] == b'-';
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        i += 1;
    }
    let mut value: i64 = 0;
    let mut any = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        any = true;
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(bytes[i] - b'0'));
        i += 1;
    }
    if !any {
        return 0;
    }
    if negative { -value } else { value }
}

/// C `atof`: longest valid floating prefix via Rust's parser over
/// successively shorter prefixes is overkill; the driver hands
/// us numeric columns as numbers, so text floats only arise
/// from string columns, where a trimmed full/prefix parse
/// suffices for the values this codebase stores.
fn atof(bytes: &[u8]) -> f64 {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    // Take the longest prefix the parser accepts.
    for end in (0..=text.len()).rev() {
        if let Ok(v) = text[..end].parse::<f64>() {
            return v;
        }
    }
    0.0
}

/// Marker so callers can name the column-type import the driver
/// pairs with rows (kept for parity with C++ field metadata).
pub type ColumnTypes = Vec<ColumnType>;

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(values: Vec<Value>) -> Fields {
        Fields(values)
    }

    #[test]
    fn field_coercion_semantics() {
        let f = fields(vec![
            Value::NULL,
            Value::Bytes(b"  -42xyz".to_vec()),
            Value::Bytes(b"1.5tail".to_vec()),
            Value::UInt(u64::MAX),
            Value::Int(-3),
            Value::Bytes("héllo".as_bytes().to_vec()),
        ]);
        assert_eq!(f.u32(0), 0);
        assert_eq!(f.string(0), "");
        assert!(f.is_null(0));
        assert_eq!(f.i64(1), -42);
        assert_eq!(f.i64(2), 1); // atol stops at '.'
        assert!((f.f32(2) - 1.5).abs() < f32::EPSILON);
        assert_eq!(f.u32(3), u32::MAX); // wrap via cast, like C++
        assert_eq!(f.u64(4), u64::MAX - 2);
        assert_eq!(f.string(5), "héllo");
        assert_eq!(f.u32(99), 0); // OOB reads as NULL-default
    }
}
