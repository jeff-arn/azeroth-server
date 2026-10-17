//! `GridMap` (`.map` tiles): parser and terrain queries mirroring
//! `vmangos/src/game/Maps/GridMap.cpp` + `GridMapDefines.h`.
//!
//! File layout (little-endian throughout): a 40-byte
//! `GridMapFileHeader` (magic `MAPS`, version `z1.4`, then
//! offset/size pairs for the area, height, liquid, and holes
//! sections). Height planes come in four flavours selected by the
//! height header flags: flat, float, uint16, uint8 (production data
//! uses float/flat; the integer paths are ported and unit-tested
//! synthetically).
//!
//! Height sampling reproduces the C++ float arithmetic bit-for-bit
//! (same operation order, `f32` throughout), including the quirks:
//! only the float path consults the holes bitmap, and
//! `getLiquidLevel` crosses its X/Y offsets exactly like the C++.
//! Where the C++ ignores short `fread`s, this parser errors.

/// Grid size in world units (`GridDefines.h`).
pub const SIZE_OF_GRIDS: f32 = 533.3333;
/// Heightmap cells per grid edge.
pub const MAP_RESOLUTION: usize = 128;
const MAP_RESOLUTION_F: f32 = 128.0;
const MAP_RESOLUTION_I: i32 = 128;
/// Sentinel for "no height" checks (`GridMap.h`).
pub const INVALID_HEIGHT: f32 = -100_000.0;
/// Returned where the height is unknown.
pub const INVALID_HEIGHT_VALUE: f32 = -200_000.0;
/// "Can be used for find ground height at surface".
pub const MAX_HEIGHT: f32 = 100_000.0;

const HOLETAB_H: [u16; 4] = [0x1111, 0x2222, 0x4444, 0x8888];
const HOLETAB_V: [u16; 4] = [0x000F, 0x00F0, 0x0F00, 0xF000];

/// Failure modes of `.map` parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GridMapError {
    /// Fewer bytes than a header/section requires.
    Truncated,
    /// File magic or version is not `MAPS`/`z1.4`.
    BadMagic,
    /// A section fourcc (`AREA`/`MHGT`/`MLIQ`) does not match.
    BadSectionMagic(&'static str),
    /// A section offset points outside the file.
    OffsetOutOfRange(&'static str),
}

impl std::fmt::Display for GridMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated map file"),
            Self::BadMagic => write!(f, "bad map magic or version (want MAPS/z1.4)"),
            Self::BadSectionMagic(s) => write!(f, "bad {s} section magic"),
            Self::OffsetOutOfRange(s) => write!(f, "{s} section offset out of range"),
        }
    }
}

impl std::error::Error for GridMapError {}

/// Little-endian cursor helpers over the section bytes.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn at(data: &'a [u8], offset: u32, what: &'static str) -> Result<Self, GridMapError> {
        let pos = offset as usize;
        if pos > data.len() {
            return Err(GridMapError::OffsetOutOfRange(what));
        }
        Ok(Self { data, pos })
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], GridMapError> {
        let out = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or(GridMapError::Truncated)?;
        self.pos += n;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, GridMapError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, GridMapError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| GridMapError::Truncated)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, GridMapError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| GridMapError::Truncated)?,
        ))
    }

    fn f32(&mut self) -> Result<f32, GridMapError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn fourcc(&mut self, want: [u8; 4], what: &'static str) -> Result<(), GridMapError> {
        if self.take(4)? == want.as_slice() {
            Ok(())
        } else {
            Err(GridMapError::BadSectionMagic(what))
        }
    }

    fn u16s(&mut self, n: usize) -> Result<Vec<u16>, GridMapError> {
        let bytes = self.take(n * 2)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect())
    }

    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, GridMapError> {
        let bytes = self.take(n * 4)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }
}

/// Height-plane storage selected by the height header flags.
#[derive(Clone, Debug)]
enum Heights {
    Flat,
    Float { v9: Vec<f32>, v8: Vec<f32> },
    Uint16 { v9: Vec<u16>, v8: Vec<u16> },
    Uint8 { v9: Vec<u8>, v8: Vec<u8> },
}

/// One loaded `.map` tile (`GridMap`).
#[derive(Clone, Debug)]
pub struct GridMap {
    holes: [u16; 256],
    grid_area: u16,
    area_map: Option<Vec<u16>>,
    grid_height: f32,
    int_height_multiplier: f32,
    heights: Heights,
    liquid_global_entry: u16,
    liquid_global_flags: u8,
    liquid_off_x: u8,
    liquid_off_y: u8,
    liquid_width: u8,
    liquid_height: u8,
    liquid_level: f32,
    liquid_entry: Option<Vec<u16>>,
    liquid_flags: Option<Vec<u8>>,
    liquid_map: Option<Vec<f32>>,
}

impl Default for GridMap {
    fn default() -> Self {
        Self::empty()
    }
}

impl GridMap {
    /// A tile with no data (C++ default-constructed `GridMap`:
    /// flat at [`INVALID_HEIGHT_VALUE`]).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            holes: [0; 256],
            grid_area: 0,
            area_map: None,
            grid_height: INVALID_HEIGHT_VALUE,
            int_height_multiplier: 0.0,
            heights: Heights::Flat,
            liquid_global_entry: 0,
            liquid_global_flags: 0,
            liquid_off_x: 0,
            liquid_off_y: 0,
            liquid_width: 0,
            liquid_height: 0,
            liquid_level: 0.0,
            liquid_entry: None,
            liquid_flags: None,
            liquid_map: None,
        }
    }

    /// Parse one `.map` file (`GridMap::loadData`).
    ///
    /// # Errors
    ///
    /// [`GridMapError`] on bad magic/version, bad section magic, or
    /// truncated/out-of-range sections.
    pub fn parse(data: &[u8]) -> Result<Self, GridMapError> {
        let mut map = Self::empty();
        let mut head = Reader::at(data, 0, "header")?;
        if head.take(4)? != b"MAPS" || head.take(4)? != b"z1.4" {
            return Err(GridMapError::BadMagic);
        }
        let area_offset = head.u32()?;
        let _area_size = head.u32()?;
        let height_offset = head.u32()?;
        let _height_size = head.u32()?;
        let liquid_offset = head.u32()?;
        let _liquid_size = head.u32()?;
        let holes_offset = head.u32()?;
        let _holes_size = head.u32()?;

        if area_offset != 0 {
            map.load_area(data, area_offset)?;
        }
        if holes_offset != 0 {
            let mut r = Reader::at(data, holes_offset, "holes")?;
            let holes = r.u16s(256)?;
            map.holes.copy_from_slice(&holes);
        }
        if height_offset != 0 {
            map.load_heights(data, height_offset)?;
        }
        if liquid_offset != 0 {
            map.load_liquid(data, liquid_offset)?;
        }
        Ok(map)
    }

    fn load_area(&mut self, data: &[u8], offset: u32) -> Result<(), GridMapError> {
        let mut r = Reader::at(data, offset, "area")?;
        r.fourcc(*b"AREA", "area")?;
        let flags = r.u16()?;
        self.grid_area = r.u16()?;
        if flags & 0x0001 == 0 {
            self.area_map = Some(r.u16s(256)?);
        }
        Ok(())
    }

    fn load_heights(&mut self, data: &[u8], offset: u32) -> Result<(), GridMapError> {
        let mut r = Reader::at(data, offset, "height")?;
        r.fourcc(*b"MHGT", "height")?;
        let flags = r.u32()?;
        let grid_height = r.f32()?;
        let grid_max_height = r.f32()?;
        self.grid_height = grid_height;
        if flags & 0x0001 == 0 {
            if flags & 0x0002 != 0 {
                let v9 = r.u16s(129 * 129)?;
                let v8 = r.u16s(128 * 128)?;
                self.int_height_multiplier = (grid_max_height - grid_height) / 65535.0;
                self.heights = Heights::Uint16 { v9, v8 };
            } else if flags & 0x0004 != 0 {
                let v9 = r.take(129 * 129)?.to_vec();
                let v8 = r.take(128 * 128)?.to_vec();
                self.int_height_multiplier = (grid_max_height - grid_height) / 255.0;
                self.heights = Heights::Uint8 { v9, v8 };
            } else {
                let v9 = r.f32s(129 * 129)?;
                let v8 = r.f32s(128 * 128)?;
                self.heights = Heights::Float { v9, v8 };
            }
        }
        Ok(())
    }

    fn load_liquid(&mut self, data: &[u8], offset: u32) -> Result<(), GridMapError> {
        let mut r = Reader::at(data, offset, "liquid")?;
        r.fourcc(*b"MLIQ", "liquid")?;
        let flags = r.u8()?;
        self.liquid_global_flags = r.u8()?;
        self.liquid_global_entry = r.u16()?;
        self.liquid_off_x = r.u8()?;
        self.liquid_off_y = r.u8()?;
        self.liquid_width = r.u8()?;
        self.liquid_height = r.u8()?;
        self.liquid_level = r.f32()?;
        if flags & 0x01 == 0 {
            self.liquid_entry = Some(r.u16s(256)?);
            self.liquid_flags = Some(r.take(256)?.to_vec());
        }
        if flags & 0x02 == 0 {
            let n = self.liquid_width as usize * self.liquid_height as usize;
            self.liquid_map = Some(r.f32s(n)?);
        }
        Ok(())
    }

    fn is_hole(&self, row: i32, col: i32) -> bool {
        let cell_row = row / 8;
        let cell_col = col / 8;
        let hole_row = (row % 8) / 2;
        let hole_col = (col - cell_col * 8) / 2;
        let hole = self.holes[(cell_row * 16 + cell_col) as usize];
        hole & HOLETAB_H[hole_col as usize] & HOLETAB_V[hole_row as usize] != 0
    }

    /// `getArea`.
    #[must_use]
    pub fn area(&self, x: f32, y: f32) -> u16 {
        let Some(area_map) = &self.area_map else {
            return self.grid_area;
        };
        let lx = (16.0 * (32.0 - x / SIZE_OF_GRIDS)) as i32 & 15;
        let ly = (16.0 * (32.0 - y / SIZE_OF_GRIDS)) as i32 & 15;
        area_map[(lx * 16 + ly) as usize]
    }

    /// `getHeight` (dispatches on the loaded height flavour).
    #[must_use]
    pub fn height(&self, x: f32, y: f32) -> f32 {
        match &self.heights {
            Heights::Flat => self.grid_height,
            Heights::Float { v9, v8 } => self.height_float(x, y, v9, v8),
            Heights::Uint16 { v9, v8 } => {
                self.height_int(x, y, &|i| i32::from(v9[i]), &|i| i32::from(v8[i]))
            }
            Heights::Uint8 { v9, v8 } => {
                self.height_int(x, y, &|i| i32::from(v9[i]), &|i| i32::from(v8[i]))
            }
        }
    }

    #[allow(
        clippy::many_single_char_names,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    fn height_float(&self, x: f32, y: f32, v9: &[f32], v8: &[f32]) -> f32 {
        let mut x = MAP_RESOLUTION_F * (32.0 - x / SIZE_OF_GRIDS);
        let mut y = MAP_RESOLUTION_F * (32.0 - y / SIZE_OF_GRIDS);
        let x_int_full = x as i32;
        let y_int_full = y as i32;
        x -= x_int_full as f32;
        y -= y_int_full as f32;
        let x_int = x_int_full & (MAP_RESOLUTION_I - 1);
        let y_int = y_int_full & (MAP_RESOLUTION_I - 1);
        if self.is_hole(x_int, y_int) {
            return INVALID_HEIGHT_VALUE;
        }
        let (xi, yi) = (x_int as usize, y_int as usize);
        let (a, b, c);
        if x + y < 1.0 {
            if x > y {
                let h1 = v9[xi * 129 + yi];
                let h2 = v9[(xi + 1) * 129 + yi];
                let h5 = 2.0 * v8[xi * 128 + yi];
                a = h2 - h1;
                b = h5 - h1 - h2;
                c = h1;
            } else {
                let h1 = v9[xi * 129 + yi];
                let h3 = v9[xi * 129 + yi + 1];
                let h5 = 2.0 * v8[xi * 128 + yi];
                a = h5 - h1 - h3;
                b = h3 - h1;
                c = h1;
            }
        } else if x > y {
            let h2 = v9[(xi + 1) * 129 + yi];
            let h4 = v9[(xi + 1) * 129 + yi + 1];
            let h5 = 2.0 * v8[xi * 128 + yi];
            a = h2 + h4 - h5;
            b = h4 - h2;
            c = h5 - h4;
        } else {
            let h3 = v9[xi * 129 + yi + 1];
            let h4 = v9[(xi + 1) * 129 + yi + 1];
            let h5 = 2.0 * v8[xi * 128 + yi];
            a = h4 - h3;
            b = h3 + h4 - h5;
            c = h5 - h4;
        }
        a * x + b * y + c
    }

    #[allow(
        clippy::many_single_char_names,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    fn height_int(
        &self,
        x: f32,
        y: f32,
        v9: &dyn Fn(usize) -> i32,
        v8: &dyn Fn(usize) -> i32,
    ) -> f32 {
        let mut x = MAP_RESOLUTION_F * (32.0 - x / SIZE_OF_GRIDS);
        let mut y = MAP_RESOLUTION_F * (32.0 - y / SIZE_OF_GRIDS);
        let x_int_full = x as i32;
        let y_int_full = y as i32;
        x -= x_int_full as f32;
        y -= y_int_full as f32;
        let x_int = x_int_full & (MAP_RESOLUTION_I - 1);
        let y_int = y_int_full & (MAP_RESOLUTION_I - 1);
        // The C++ int paths do NOT consult the holes bitmap.
        let base = (x_int * 128 + x_int + y_int) as usize;
        let (a, b, c);
        if x + y < 1.0 {
            if x > y {
                let h1 = v9(base);
                let h2 = v9(base + 129);
                let h5 = 2 * v8((x_int * 128 + y_int) as usize);
                a = h2 - h1;
                b = h5 - h1 - h2;
                c = h1;
            } else {
                let h1 = v9(base);
                let h3 = v9(base + 1);
                let h5 = 2 * v8((x_int * 128 + y_int) as usize);
                a = h5 - h1 - h3;
                b = h3 - h1;
                c = h1;
            }
        } else if x > y {
            let h2 = v9(base + 129);
            let h4 = v9(base + 130);
            let h5 = 2 * v8((x_int * 128 + y_int) as usize);
            a = h2 + h4 - h5;
            b = h4 - h2;
            c = h5 - h4;
        } else {
            let h3 = v9(base + 1);
            let h4 = v9(base + 130);
            let h5 = 2 * v8((x_int * 128 + y_int) as usize);
            a = h4 - h3;
            b = h3 + h4 - h5;
            c = h5 - h4;
        }
        (a as f32 * x + b as f32 * y + c as f32) * self.int_height_multiplier + self.grid_height
    }

    /// `getLiquidLevel` (note the C++ crosses offsetX/offsetY here —
    /// reproduced verbatim).
    #[must_use]
    #[allow(clippy::similar_names)]
    pub fn liquid_level(&self, x: f32, y: f32) -> f32 {
        let Some(liquid_map) = &self.liquid_map else {
            return self.liquid_level;
        };
        let x = MAP_RESOLUTION_F * (32.0 - x / SIZE_OF_GRIDS);
        let y = MAP_RESOLUTION_F * (32.0 - y / SIZE_OF_GRIDS);
        let cx_int = ((x as i32) & (MAP_RESOLUTION_I - 1)) - i32::from(self.liquid_off_y);
        let cy_int = ((y as i32) & (MAP_RESOLUTION_I - 1)) - i32::from(self.liquid_off_x);
        if cx_int < 0 || cx_int >= i32::from(self.liquid_height) {
            return INVALID_HEIGHT_VALUE;
        }
        if cy_int < 0 || cy_int >= i32::from(self.liquid_width) {
            return INVALID_HEIGHT_VALUE;
        }
        liquid_map[(cx_int * i32::from(self.liquid_width) + cy_int) as usize]
    }

    /// `getTerrainType`.
    #[must_use]
    pub fn terrain_type(&self, x: f32, y: f32) -> u8 {
        let Some(flags) = &self.liquid_flags else {
            return self.liquid_global_flags;
        };
        let lx = (16.0 * (32.0 - x / SIZE_OF_GRIDS)) as i32 & 15;
        let ly = (16.0 * (32.0 - y / SIZE_OF_GRIDS)) as i32 & 15;
        flags[(lx * 16 + ly) as usize]
    }

    /// The tile's base (minimum) height.
    #[must_use]
    pub const fn grid_height(&self) -> f32 {
        self.grid_height
    }

    /// Whether any height plane is loaded (vs flat).
    #[must_use]
    pub const fn has_height_plane(&self) -> bool {
        !matches!(self.heights, Heights::Flat)
    }
}

/// `.map` file name for a tile (`TerrainInfo::Load` `"%03u%02u%02u.map"`).
#[must_use]
pub fn gridmap_filename(map_id: u32, gx: u32, gy: u32) -> String {
    format!("{map_id:03}{gx:02}{gy:02}.map")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(area: u32, height: u32, liquid: u32, holes: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"MAPS");
        v.extend_from_slice(b"z1.4");
        for (off, size) in [(area, 0u32), (height, 0), (liquid, 0), (holes, 0)] {
            v.extend_from_slice(&off.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
        }
        v
    }

    #[test]
    fn flat_tile_returns_grid_height() {
        let mut data = header(0, 40, 0, 0);
        data.extend_from_slice(b"MHGT");
        data.extend_from_slice(&1u32.to_le_bytes()); // MAP_HEIGHT_NO_HEIGHT
        data.extend_from_slice(&12.5f32.to_le_bytes());
        data.extend_from_slice(&12.5f32.to_le_bytes());
        let map = GridMap::parse(&data).expect("parse");
        assert!(!map.has_height_plane());
        assert_eq!(map.height(100.0, -50.0).to_bits(), 12.5f32.to_bits());
    }

    #[test]
    fn uint16_plane_sampling() {
        // Synthetic ramp: V9 all 100, V8 all 100 -> height = base + 100*mult.
        let mut data = header(0, 40, 0, 0);
        data.extend_from_slice(b"MHGT");
        data.extend_from_slice(&2u32.to_le_bytes()); // AS_INT16
        data.extend_from_slice(&10.0f32.to_le_bytes());
        data.extend_from_slice(&20.0f32.to_le_bytes());
        for _ in 0..(129 * 129 + 128 * 128) {
            data.extend_from_slice(&100u16.to_le_bytes());
        }
        let map = GridMap::parse(&data).expect("parse");
        let expected = 100.0f32 * ((20.0 - 10.0) / 65535.0) + 10.0;
        let got = map.height(-8950.0, -132.0);
        assert_eq!(got.to_bits(), expected.to_bits());
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert_eq!(GridMap::parse(b"NOP").unwrap_err(), GridMapError::Truncated);
        assert_eq!(GridMap::parse(b"NOPE").unwrap_err(), GridMapError::BadMagic);
        let data = header(0, 0, 0, 0);
        let mut bad = data.clone();
        bad[0] = b'X';
        assert_eq!(GridMap::parse(&bad).unwrap_err(), GridMapError::BadMagic);
        assert!(GridMap::parse(&data).is_ok());
    }
}
