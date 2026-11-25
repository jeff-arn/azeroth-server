//! Little-endian packet reading/writing with `ByteBuffer` semantics
//! (`vmangos/src/shared/ByteBuffer.h`): integers are little-endian on
//! the wire (the protocol targets little-endian hosts), strings are
//! NUL-terminated, and reads past the end are an error — where C++
//! throws `ByteBufferException`, these return [`ReadError`].

/// A read ran past the end of the packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadError {
    pub needed: usize,
    pub remaining: usize,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "packet read past end: needed {} bytes, {} remain",
            self.needed, self.remaining
        )
    }
}

impl std::error::Error for ReadError {}

/// Cursor-style reader over one packet payload.
#[derive(Clone, Debug)]
pub struct PacketReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PacketReader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// # Errors
    /// Returns [`ReadError`] if fewer than `len` bytes remain.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ReadError> {
        if self.remaining() < len {
            return Err(ReadError {
                needed: len,
                remaining: self.remaining(),
            });
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    /// # Errors
    /// Returns [`ReadError`] if fewer than N bytes remain.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.read_bytes(N)?);
        Ok(out)
    }

    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn read_u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.read_array::<1>()?[0])
    }

    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn read_u16_le(&mut self) -> Result<u16, ReadError> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn read_u32_le(&mut self) -> Result<u32, ReadError> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn read_u64_le(&mut self) -> Result<u64, ReadError> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn read_f32_le(&mut self) -> Result<f32, ReadError> {
        Ok(f32::from_le_bytes(self.read_array()?))
    }

    /// Reads a NUL-terminated string, consuming the terminator. Where
    /// C++ reads up to the buffer end when no NUL is present, this
    /// returns the remaining bytes; lossy UTF-8 like the C++ char*
    /// handling.
    pub fn read_cstring(&mut self) -> String {
        let start = self.pos;
        let rest = &self.data[start..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .map_or(self.data.len(), |i| start + i);
        let text = String::from_utf8_lossy(&self.data[start..end]).into_owned();
        self.pos = if end < self.data.len() { end + 1 } else { end };
        text
    }

    /// Skips `len` bytes.
    ///
    /// # Errors
    /// Returns [`ReadError`] on truncation.
    pub fn skip(&mut self, len: usize) -> Result<(), ReadError> {
        self.read_bytes(len).map(drop)
    }
}

/// Growable little-endian packet writer.
#[derive(Clone, Debug, Default)]
pub struct PacketWriter {
    buf: Vec<u8>,
}

impl PacketWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    pub fn write_u8(&mut self, value: u8) -> &mut Self {
        self.buf.push(value);
        self
    }

    pub fn write_u16_le(&mut self, value: u16) -> &mut Self {
        self.write_bytes(&value.to_le_bytes())
    }

    pub fn write_u32_le(&mut self, value: u32) -> &mut Self {
        self.write_bytes(&value.to_le_bytes())
    }

    pub fn write_u64_le(&mut self, value: u64) -> &mut Self {
        self.write_bytes(&value.to_le_bytes())
    }

    pub fn write_f32_le(&mut self, value: f32) -> &mut Self {
        self.write_bytes(&value.to_le_bytes())
    }

    pub fn write_cstring(&mut self, text: &str) -> &mut Self {
        self.buf.extend_from_slice(text.as_bytes());
        self.buf.push(0);
        self
    }

    /// Current write position (`ByteBuffer::wpos`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// `ByteBuffer::put<uint32>(pos, value)` — overwrite in place.
    pub fn put_u32(&mut self, pos: usize, value: u32) -> &mut Self {
        self.buf[pos..pos + 4].copy_from_slice(&value.to_le_bytes());
        self
    }

    /// `ByteBuffer::appendPackXYZ(float x, float y, float z)`:
    /// x in bits 0–10, y in bits 11–21, z in bits 22–31, each
    /// `lroundf(v * 4.f)` masked to two's-complement field width.
    pub fn write_packed_xyz(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        let packed = ((lround_pack(x) as u32) & 0x7FF)
            | (((lround_pack(y) as u32) & 0x7FF) << 11)
            | (((lround_pack(z) as u32) & 0x3FF) << 22);
        self.write_u32_le(packed)
    }
}

/// `lroundf(v * 4.f)` — round to nearest, ties away from zero
/// (Rust `f32::round` semantics).
fn lround_pack(v: f32) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    {
        (v * 4.0).round() as i32
    }
}
