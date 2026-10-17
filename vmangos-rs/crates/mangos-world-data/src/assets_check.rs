//! The boot asset gate: `MapManager::ExistMapAndVMap` over the
//! six race start areas (`World.cpp:1295-1302`). Missing or
//! wrong-version map/vmap files abort boot in C++ (`exit(1)`);
//! same here, with the failure identifying the file.

use std::io::Read;
use std::path::PathBuf;

use mangos_assets::gridmap::{SIZE_OF_GRIDS, gridmap_filename};

use crate::DataDir;

/// `CENTER_GRID_ID` (`GridDefines.h:41`).
const CENTER_GRID_ID: f64 = 32.0;
/// `CENTER_GRID_OFFSET` = `SIZE_OF_GRIDS / 2` as f32
/// (`GridDefines.h:43`).
const CENTER_GRID_OFFSET: f32 = SIZE_OF_GRIDS / 2.0;

/// `MAP_MAGIC` + `MAP_VERSION_MAGIC` (`GridMap.cpp:34-35`): the
/// first eight bytes of every `.map` file.
const MAP_HEADER_MAGIC: &[u8; 8] = b"MAPSz1.4";
/// `VMAP_MAGIC` (`VMapDefinitions.h:26`).
const VMAP_MAGIC: &[u8; 8] = b"VMAP_7.0";

/// The six start-area spots, verbatim (`World.cpp:1296`):
/// (map, x, y).
pub const START_AREA_SPOTS: [(u32, f32, f32); 6] = [
    (0, -6240.32, 331.033),
    (0, -8949.95, -132.493),
    (1, -618.518, -4251.67),
    (0, 1676.35, 1677.45),
    (1, 10311.3, 832.463),
    (1, -2917.58, -257.98),
];

/// Why one start-area check failed. Paths are absolute enough
/// for the operator log line the C++ produces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetFailure {
    MapMissing(PathBuf),
    MapBadMagic(PathBuf),
    VmapTreeMissing(PathBuf),
    VmapBadMagic(PathBuf),
    VmapTileMissing(PathBuf),
    VmapTileBadMagic(PathBuf),
}

impl std::fmt::Display for AssetFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetFailure::MapMissing(path) => {
                write!(
                    f,
                    "Check existing of map file '{}': not exist!",
                    path.display()
                )
            }
            AssetFailure::MapBadMagic(path) => write!(
                f,
                "Map file '{}' is non-compatible version (outdated?). Please, create new using ad.exe program.",
                path.display()
            ),
            AssetFailure::VmapTreeMissing(path)
            | AssetFailure::VmapTileMissing(path)
            | AssetFailure::VmapBadMagic(path)
            | AssetFailure::VmapTileBadMagic(path) => write!(
                f,
                "VMap file '{}' is missing or point to wrong version vmap file, redo vmaps with latest vmap_assembler.exe program",
                path.display()
            ),
        }
    }
}

impl std::error::Error for AssetFailure {}

/// The file-coordinate pair `ExistMapAndVMap` derives from a
/// world position, following the C++ swap chain exactly
/// (`MapManager.cpp:416-424`, `GridDefines.h:154-165`):
/// `Compute` returns (`y_val`, `x_val`), then gx = 63 - `y_val`,
/// gy = 63 - `x_val`. Returned as (gx, gy) in those C++ terms;
/// note the `.map` filename prints (gy, gx) while the `.vmtile`
/// filename prints (gx, gy).
#[must_use]
pub fn file_grid(x: f32, y: f32) -> (i32, i32) {
    // int() truncation toward zero, on f64 math over f32 inputs,
    // exactly like the C++ Compute template.
    let size = f64::from(SIZE_OF_GRIDS);
    let offset = f64::from(CENTER_GRID_OFFSET);
    let x_val = ((f64::from(x) - offset) / size + CENTER_GRID_ID + 0.5) as i32;
    let y_val = ((f64::from(y) - offset) / size + CENTER_GRID_ID + 0.5) as i32;
    (63 - y_val, 63 - x_val)
}

/// `GridMap::ExistMap` (`GridMap.cpp:629-656`): file exists and
/// starts with the map/version magics.
fn exist_map(dir: &DataDir, map_id: u32, gx: i32, gy: i32) -> Result<(), AssetFailure> {
    // C++ formats (mapid, gy, gx); gridmap_filename prints its
    // two coords in order, so gy goes first.
    let name = gridmap_filename(map_id, as_u32(gy), as_u32(gx));
    let path = dir.maps_dir().join(name);
    let mut file =
        std::fs::File::open(&path).map_err(|_| AssetFailure::MapMissing(path.clone()))?;
    let mut head = [0u8; 8];
    if file.read_exact(&mut head).is_err() || &head != MAP_HEADER_MAGIC {
        return Err(AssetFailure::MapBadMagic(path));
    }
    Ok(())
}

/// `GridMap::ExistVMap` → `StaticMapTree::CanLoadMap`
/// (`GridMap.cpp:659-678`, `MapTree.cpp:289-323`): the
/// `<map>.vmtree` magic + tiled flag, then the tile magic when
/// tiled.
fn exist_vmap(dir: &DataDir, map_id: u32, gx: i32, gy: i32) -> Result<(), AssetFailure> {
    let tree_path = dir.vmaps_dir().join(format!("{map_id:03}.vmtree"));
    let mut tree = std::fs::File::open(&tree_path)
        .map_err(|_| AssetFailure::VmapTreeMissing(tree_path.clone()))?;
    let mut head = [0u8; 9];
    if tree.read_exact(&mut head).is_err() || &head[..8] != VMAP_MAGIC {
        return Err(AssetFailure::VmapBadMagic(tree_path));
    }
    if head[8] != 0 {
        let tile_path = dir
            .vmaps_dir()
            .join(format!("{map_id:03}_{gx:02}_{gy:02}.vmtile"));
        let mut tile = std::fs::File::open(&tile_path)
            .map_err(|_| AssetFailure::VmapTileMissing(tile_path.clone()))?;
        let mut magic = [0u8; 8];
        if tile.read_exact(&mut magic).is_err() || &magic != VMAP_MAGIC {
            return Err(AssetFailure::VmapTileBadMagic(tile_path));
        }
    }
    Ok(())
}

/// `MapManager::ExistMapAndVMap` for one position.
///
/// # Errors
///
/// Returns the first failing file check, in C++ order
/// (map before vmap).
pub fn exist_map_and_vmap(dir: &DataDir, map_id: u32, x: f32, y: f32) -> Result<(), AssetFailure> {
    let (gx, gy) = file_grid(x, y);
    exist_map(dir, map_id, gx, gy)?;
    exist_vmap(dir, map_id, gx, gy)
}

/// All six start-area spots, short-circuiting on the first
/// failure like the C++ `||` chain.
///
/// # Errors
///
/// Returns the first [`AssetFailure`].
pub fn check_start_area_assets(dir: &DataDir) -> Result<(), AssetFailure> {
    for (map_id, x, y) in START_AREA_SPOTS {
        exist_map_and_vmap(dir, map_id, x, y)?;
    }
    Ok(())
}

/// C++ formats grid coords as unsigned; negatives would only
/// come from out-of-world positions, which the fixed spot list
/// never contains. Wrap like the C++ `%02u` of a negative int
/// would not — instead clamp through the same cast so a bad
/// input produces a deterministic (missing) filename.
const fn as_u32(v: i32) -> u32 {
    v as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_grid_matches_fixture_names() {
        // Validated against the production fixture tree: these
        // .map/.vmtile files exist under datadir/{maps,vmaps}.
        let cases = [
            (
                (0u32, -6240.32f32, 331.033f32),
                "0004331.map",
                "000_31_43.vmtile",
            ),
            ((0, -8949.95, -132.493), "0004832.map", "000_32_48.vmtile"),
            ((1, -618.518, -4251.67), "0013339.map", "001_39_33.vmtile"),
            ((0, 1676.35, 1677.45), "0002828.map", "000_28_28.vmtile"),
            ((1, 10311.3, 832.463), "0011230.map", "001_30_12.vmtile"),
            ((1, -2917.58, -257.98), "0013732.map", "001_32_37.vmtile"),
        ];
        for ((map_id, x, y), map_name, tile_name) in cases {
            let (gx, gy) = file_grid(x, y);
            assert_eq!(gridmap_filename(map_id, gy as u32, gx as u32), map_name);
            assert_eq!(format!("{map_id:03}_{gx:02}_{gy:02}.vmtile"), tile_name);
        }
    }

    fn temp_datadir() -> (std::path::PathBuf, DataDir) {
        let root = std::env::temp_dir().join(format!("mangos-wd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("maps")).expect("maps");
        std::fs::create_dir_all(root.join("vmaps")).expect("vmaps");
        let dir = DataDir::new(root.clone());
        (root, dir)
    }

    fn write(path: &std::path::Path, bytes: &[u8]) {
        std::fs::File::create(path)
            .expect("create")
            .write_all(bytes)
            .expect("write");
    }

    #[test]
    fn missing_and_bad_magic_failures() {
        let (root, dir) = temp_datadir();
        let (map_id, x, y) = START_AREA_SPOTS[0];
        assert!(matches!(
            exist_map_and_vmap(&dir, map_id, x, y),
            Err(AssetFailure::MapMissing(_))
        ));

        write(&root.join("maps/0004331.map"), b"BADMAGIC!");
        assert!(matches!(
            exist_map_and_vmap(&dir, map_id, x, y),
            Err(AssetFailure::MapBadMagic(_))
        ));

        write(&root.join("maps/0004331.map"), b"MAPSz1.4rest");
        assert!(matches!(
            exist_map_and_vmap(&dir, map_id, x, y),
            Err(AssetFailure::VmapTreeMissing(_))
        ));

        // Untiled tree: magic + zero flag passes with no tile.
        write(&root.join("vmaps/000.vmtree"), b"VMAP_7.0\x00");
        assert!(exist_map_and_vmap(&dir, map_id, x, y).is_ok());

        // Tiled tree without the tile fails; with it, passes.
        write(&root.join("vmaps/000.vmtree"), b"VMAP_7.0\x01");
        assert!(matches!(
            exist_map_and_vmap(&dir, map_id, x, y),
            Err(AssetFailure::VmapTileMissing(_))
        ));
        write(&root.join("vmaps/000_31_43.vmtile"), b"VMAP_7.0til");
        assert!(exist_map_and_vmap(&dir, map_id, x, y).is_ok());

        let _ = std::fs::remove_dir_all(&root);
    }
}
