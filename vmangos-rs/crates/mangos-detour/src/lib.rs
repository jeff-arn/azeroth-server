//! Navigation-mesh loading and path queries over the vendored,
//! patched Recast/Detour build (`vmangos/dep/recastnavigation`,
//! `DT_POLYREF64` with the custom bit split — poly-refs are
//! u64 here), mirroring `MoveMap.cpp`'s tile loading and the
//! query/filter policy of `PathFinder.cpp`.
//!
//! This is the workspace's one sanctioned unsafe crate: all FFI
//! is confined to the `ffi` module behind a C shim
//! (`shim/detour_shim.cpp`), and the public API below is safe.
//! Scope of the spike: mesh/tile load, nearest-poly (with the
//! `FindWalkPoly` policy), poly-path and straight-path queries.
//! The full `PathInfo::calculate` state machine (shortcuts,
//! swim/fly paths, subpath reuse, smoothing) needs `Unit` and
//! lands with the Phase 3 world on top of this surface.
//!
//! Coordinates: the public API takes game `XYZ`; Detour works in
//! `YZX` — the swap happens at this boundary, as in `PathFinder`.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

use mangos_movement::Vec3;

#[allow(unsafe_code)]
mod ffi {
    use std::ffi::{c_int, c_uchar, c_void};

    unsafe extern "C" {
        pub fn mangos_tile_alloc(size: usize) -> *mut c_uchar;
        pub fn mangos_tile_free(ptr: *mut c_uchar);
        pub fn mangos_navmesh_create(params: *const c_uchar, params_len: usize) -> *mut c_void;
        pub fn mangos_navmesh_create_single_tile(data: *mut c_uchar, size: c_int) -> *mut c_void;
        pub fn mangos_navmesh_destroy(mesh: *mut c_void);
        pub fn mangos_navmesh_add_tile(
            mesh: *mut c_void,
            data: *mut c_uchar,
            size: c_int,
            out_tile_ref: *mut u64,
        ) -> u32;
        pub fn mangos_navmesh_remove_tile(mesh: *mut c_void, tile_ref: u64) -> u32;
        pub fn mangos_navmesh_calc_tile_loc(
            mesh: *const c_void,
            pos_yzx: *const f32,
            out_tx: *mut c_int,
            out_ty: *mut c_int,
        );
        pub fn mangos_navmesh_has_tile_at(mesh: *const c_void, tx: c_int, ty: c_int) -> c_int;
        pub fn mangos_query_create(mesh: *mut c_void, max_nodes: c_int) -> *mut c_void;
        pub fn mangos_query_destroy(query: *mut c_void);
        pub fn mangos_query_find_nearest_poly(
            query: *mut c_void,
            center: *const f32,
            half_extents: *const f32,
            include_flags: u16,
            exclude_flags: u16,
            out_ref: *mut u64,
            out_closest: *mut f32,
        ) -> u32;
        pub fn mangos_query_find_path(
            query: *mut c_void,
            start_ref: u64,
            end_ref: u64,
            start_pos: *const f32,
            end_pos: *const f32,
            include_flags: u16,
            exclude_flags: u16,
            out_path: *mut u64,
            out_count: *mut c_int,
            max_path: c_int,
        ) -> u32;
        pub fn mangos_query_find_straight_path(
            query: *mut c_void,
            start_pos: *const f32,
            end_pos: *const f32,
            path: *const u64,
            path_len: c_int,
            out_points: *mut f32,
            out_count: *mut c_int,
            max_points: c_int,
        ) -> u32;
        pub fn mangos_query_closest_point_on_poly(
            query: *mut c_void,
            poly_ref: u64,
            pos: *const f32,
            out_closest: *mut f32,
            out_over_poly: *mut c_int,
        ) -> u32;
        pub fn mangos_query_closest_point_on_poly_boundary(
            query: *mut c_void,
            poly_ref: u64,
            pos: *const f32,
            out_closest: *mut f32,
        ) -> u32;
    }
}

/// `MMAP_MAGIC` ('MMAP').
pub const MMAP_MAGIC: u32 = 0x4D4D_4150;
/// `MMAP_VERSION`.
pub const MMAP_VERSION: u32 = 6;
/// `DT_NAVMESH_VERSION` of the vendored build.
pub const DT_NAVMESH_VERSION: u32 = 7;
/// `sizeof(dtNavMeshParams)` (the `.mmap` file body).
pub const NAVMESH_PARAMS_SIZE: usize = 28;
/// `MAX_PATH_LENGTH` from `PathFinder.h`.
pub const MAX_PATH_LENGTH: usize = 256;
/// Default query node pool (`GetNavMeshQuery` inits with 2048).
pub const DEFAULT_MAX_NODES: i32 = 2048;

/// `NavTerrain` polygon flag bits (`MoveMapSharedDefines.h`).
pub mod nav_terrain {
    pub const EMPTY: u16 = 0x00;
    pub const GROUND: u16 = 0x01;
    pub const MAGMA: u16 = 0x02;
    pub const SLIME: u16 = 0x04;
    pub const WATER: u16 = 0x08;
    /// Slopes above player climb angle.
    pub const STEEP_SLOPES: u16 = 0x10;
}

/// `dtStatus` bitmask.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Status(pub u32);

impl Status {
    pub const FAILURE: u32 = 1 << 31;
    pub const SUCCESS: u32 = 1 << 30;
    pub const IN_PROGRESS: u32 = 1 << 29;
    pub const DETAIL_MASK: u32 = 0x0FFF_FFFF;
    pub const BUFFER_TOO_SMALL: u32 = 1 << 4;
    pub const PARTIAL_RESULT: u32 = 1 << 6;

    /// `dtStatusFailed`.
    #[must_use]
    pub const fn failed(self) -> bool {
        self.0 & Self::FAILURE != 0
    }

    /// `dtStatusSucceed`.
    #[must_use]
    pub const fn succeeded(self) -> bool {
        self.0 & Self::SUCCESS != 0
    }

    /// `dtStatusDetail`.
    #[must_use]
    pub const fn has_detail(self, detail: u32) -> bool {
        self.0 & detail != 0
    }
}

/// A polygon reference (`dtPolyRef`, 64-bit in this build).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct PolyRef(pub u64);

impl PolyRef {
    /// `INVALID_POLYREF`.
    pub const INVALID: Self = Self(0);

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// Game `XYZ` → Detour `YZX` (`PathFinder`'s constant warning).
const fn to_yzx(p: Vec3) -> [f32; 3] {
    [p.y, p.z, p.x]
}

/// Detour `YZX` → game `XYZ`.
const fn from_yzx(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[2], p[0], p[1])
}

/// Copy a tile payload into Detour-owned memory. On success the
/// pointer is owned by Detour (`DT_TILE_FREE_DATA`); the caller
/// frees it only when the hand-off did not happen.
#[allow(unsafe_code)]
fn detour_copy(payload: &[u8]) -> Option<*mut std::ffi::c_uchar> {
    // SAFETY: dtAlloc returns null on OOM; checked. The copy
    // length equals the allocation size.
    unsafe {
        let ptr = ffi::mangos_tile_alloc(payload.len());
        if ptr.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(payload.as_ptr(), ptr, payload.len());
        Some(ptr)
    }
}

/// `MmapTileHeader` (5 × u32, little-endian on disk).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileHeader {
    pub mmap_magic: u32,
    pub dt_version: u32,
    pub mmap_version: u32,
    pub size: u32,
    pub uses_liquids: u32,
}

impl TileHeader {
    pub const SIZE: usize = 20;

    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let head: &[u8; Self::SIZE] = bytes.get(..Self::SIZE)?.try_into().ok()?;
        let word = |i: usize| {
            u32::from_le_bytes([
                head[i * 4],
                head[i * 4 + 1],
                head[i * 4 + 2],
                head[i * 4 + 3],
            ])
        };
        Some(Self {
            mmap_magic: word(0),
            dt_version: word(1),
            mmap_version: word(2),
            size: word(3),
            uses_liquids: word(4),
        })
    }
}

/// Tile load failures, mirroring `MMapManager::loadMap` checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileError {
    /// Fewer bytes than the header (or than header + payload).
    Truncated,
    /// `mmapMagic != MMAP_MAGIC`.
    BadMagic,
    /// `mmapVersion != MMAP_VERSION`.
    BadVersion(u32),
    /// Detour allocation failed.
    OutOfMemory,
    /// `addTile` rejected the payload.
    AddTileFailed(Status),
}

/// An initialized navmesh (`dtNavMesh` + tile bookkeeping
/// happens in the caller, as with `MMapManager`).
pub struct NavMesh {
    ptr: NonNull<c_void>,
    // Detour objects are not internally synchronized (production
    // guards tile mutation with locks); keep the wrapper !Sync.
    _not_sync: PhantomData<*mut c_void>,
}

// SAFETY: moving the owning handle between threads is fine as
// long as use stays exclusive, which &mut / non-Sync enforces.
#[allow(unsafe_code)]
unsafe impl Send for NavMesh {}

impl Drop for NavMesh {
    fn drop(&mut self) {
        // SAFETY: ptr came from mangos_navmesh_create* and is
        // destroyed exactly once here.
        #[allow(unsafe_code)]
        unsafe {
            ffi::mangos_navmesh_destroy(self.ptr.as_ptr());
        }
    }
}

impl NavMesh {
    fn from_ptr(ptr: *mut c_void) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self {
            ptr,
            _not_sync: PhantomData,
        })
    }

    /// `loadMapData`: build from a `NNN.mmap` file's
    /// `dtNavMeshParams` bytes.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn from_mmap_params(params: &[u8]) -> Option<Self> {
        // SAFETY: the shim bounds-checks the length and copies the
        // params struct before init.
        unsafe { Self::from_ptr(ffi::mangos_navmesh_create(params.as_ptr(), params.len())) }
    }

    /// `loadGameObject`: a whole tile payload as its own mesh.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn single_tile(payload: &[u8]) -> Option<Self> {
        let data = detour_copy(payload)?;
        // SAFETY: on success Detour owns `data`; on failure the
        // shim frees the mesh but not a failed-init payload —
        // init(DT_TILE_FREE_DATA) takes it over only on success
        // in this build, so free on null return.
        #[allow(unsafe_code)]
        unsafe {
            let mesh = Self::from_ptr(ffi::mangos_navmesh_create_single_tile(
                data,
                payload.len() as i32,
            ));
            if mesh.is_none() {
                ffi::mangos_tile_free(data);
            }
            mesh
        }
    }

    /// `loadMap` tile path: add one `.mmtile` payload (the bytes
    /// after the file header). Returns the tile reference.
    ///
    /// # Errors
    ///
    /// [`TileError::OutOfMemory`] or [`TileError::AddTileFailed`].
    #[allow(unsafe_code)]
    #[allow(clippy::cast_possible_wrap)]
    pub fn add_tile_payload(&mut self, payload: &[u8]) -> Result<u64, TileError> {
        let data = detour_copy(payload).ok_or(TileError::OutOfMemory)?;
        let mut tile_ref = 0u64;
        // SAFETY: `data` is a fresh Detour allocation of
        // payload.len(); on success Detour owns it, on failure we
        // free it (the C++ does the same).
        let status = unsafe {
            Status(ffi::mangos_navmesh_add_tile(
                self.ptr.as_ptr(),
                data,
                payload.len() as i32,
                &raw mut tile_ref,
            ))
        };
        if status.succeeded() {
            Ok(tile_ref)
        } else {
            // SAFETY: addTile failed before taking ownership.
            unsafe { ffi::mangos_tile_free(data) };
            Err(TileError::AddTileFailed(status))
        }
    }

    /// Load one complete `.mmtile` file image (header + payload),
    /// with `loadMap`'s header validation.
    ///
    /// # Errors
    ///
    /// Header problems as [`TileError`] variants.
    pub fn load_tile_file(&mut self, bytes: &[u8]) -> Result<u64, TileError> {
        let header = TileHeader::parse(bytes).ok_or(TileError::Truncated)?;
        if header.mmap_magic != MMAP_MAGIC {
            return Err(TileError::BadMagic);
        }
        if header.mmap_version != MMAP_VERSION {
            return Err(TileError::BadVersion(header.mmap_version));
        }
        let end = TileHeader::SIZE + header.size as usize;
        if bytes.len() < end {
            return Err(TileError::Truncated);
        }
        self.add_tile_payload(&bytes[TileHeader::SIZE..end])
    }

    /// `unloadMap(map, x, y)` tile removal by reference.
    #[allow(unsafe_code)]
    pub fn remove_tile(&mut self, tile_ref: u64) -> Status {
        // SAFETY: self.ptr is a live mesh (owned, !Sync).
        unsafe { Status(ffi::mangos_navmesh_remove_tile(self.ptr.as_ptr(), tile_ref)) }
    }

    /// `HaveTiles(p)`: is a tile loaded at this position (XYZ)?
    #[must_use]
    #[allow(unsafe_code)]
    pub fn has_tile_at_position(&self, pos: Vec3) -> bool {
        let (mut tx, mut ty) = (0i32, 0i32);
        // SAFETY: out pointers are valid stack slots.
        unsafe {
            ffi::mangos_navmesh_calc_tile_loc(
                self.ptr.as_ptr(),
                to_yzx(pos).as_ptr(),
                &raw mut tx,
                &raw mut ty,
            );
            ffi::mangos_navmesh_has_tile_at(self.ptr.as_ptr(), tx, ty) != 0
        }
    }
}

/// A navmesh query (`dtNavMeshQuery`); one per thread in
/// production (queries are not thread-safe). Borrows its mesh.
pub struct NavMeshQuery<'m> {
    ptr: NonNull<c_void>,
    _mesh: PhantomData<&'m NavMesh>,
    _not_sync: PhantomData<*mut c_void>,
}

impl Drop for NavMeshQuery<'_> {
    fn drop(&mut self) {
        // SAFETY: created in `new`, destroyed once here.
        #[allow(unsafe_code)]
        unsafe {
            ffi::mangos_query_destroy(self.ptr.as_ptr());
        }
    }
}

impl<'m> NavMeshQuery<'m> {
    /// `GetNavMeshQuery` allocation (`max_nodes` 2048 in
    /// production; see [`DEFAULT_MAX_NODES`]).
    #[must_use]
    #[allow(unsafe_code)]
    pub fn new(mesh: &'m NavMesh, max_nodes: i32) -> Option<Self> {
        // SAFETY: mesh outlives the query via the borrow.
        let ptr = unsafe { ffi::mangos_query_create(mesh.ptr.as_ptr(), max_nodes) };
        NonNull::new(ptr).map(|ptr| Self {
            ptr,
            _mesh: PhantomData,
            _not_sync: PhantomData,
        })
    }

    /// `FindWalkPoly` + `getPolyByLocation`: nearest walkable
    /// poly to `pos` within `{5, z_search, 5}` (YZX extents),
    /// rejecting polys whose closest point lies more than 3 yd
    /// above the query point. Returns the ref, the closest point
    /// (XYZ), and its distance (`dtVdist`).
    #[must_use]
    #[allow(unsafe_code)]
    pub fn find_walk_poly(
        &self,
        pos: Vec3,
        include_flags: u16,
        exclude_flags: u16,
        z_search_dist: f32,
    ) -> Option<(PolyRef, Vec3, f32)> {
        let center = to_yzx(pos);
        let extents = [5.0f32, z_search_dist, 5.0f32];
        let mut closest = [0.0f32; 3];
        let mut poly = 0u64;
        // SAFETY: all buffers are valid for the call; the query
        // writes through out_ref/out_closest only.
        let status = unsafe {
            Status(ffi::mangos_query_find_nearest_poly(
                self.ptr.as_ptr(),
                center.as_ptr(),
                extents.as_ptr(),
                include_flags,
                exclude_flags,
                &raw mut poly,
                closest.as_mut_ptr(),
            ))
        };
        if status.failed() || poly == PolyRef::INVALID.0 {
            return None;
        }
        // Do not select points over player pos (YZX[1] is WoW Z).
        if closest[1] > center[1] + 3.0 {
            return None;
        }
        let closest_xyz = from_yzx(closest);
        let dist = (closest_xyz - pos).length();
        Some((PolyRef(poly), closest_xyz, dist))
    }

    /// `findPath`: poly corridor between two polys/positions
    /// (positions in XYZ).
    #[must_use]
    #[allow(unsafe_code, clippy::too_many_arguments)]
    #[allow(clippy::cast_possible_wrap)]
    pub fn find_path(
        &self,
        start_ref: PolyRef,
        end_ref: PolyRef,
        start: Vec3,
        end: Vec3,
        include_flags: u16,
        exclude_flags: u16,
        max_path: usize,
    ) -> (Status, Vec<PolyRef>) {
        let mut path = vec![0u64; max_path];
        let mut count = 0i32;
        // SAFETY: `path` has room for max_path refs; count returns
        // the number written (clamped below for safety).
        let status = unsafe {
            Status(ffi::mangos_query_find_path(
                self.ptr.as_ptr(),
                start_ref.0,
                end_ref.0,
                to_yzx(start).as_ptr(),
                to_yzx(end).as_ptr(),
                include_flags,
                exclude_flags,
                path.as_mut_ptr(),
                &raw mut count,
                max_path as i32,
            ))
        };
        let n = (count.max(0) as usize).min(max_path);
        path.truncate(n);
        (status, path.into_iter().map(PolyRef).collect())
    }

    /// `findStraightPath` (options 0, no flags/refs out, as
    /// `BuildPointPath` calls it). Points come back in XYZ.
    #[must_use]
    #[allow(unsafe_code)]
    #[allow(clippy::cast_possible_wrap)]
    pub fn find_straight_path(
        &self,
        start: Vec3,
        end: Vec3,
        poly_path: &[PolyRef],
        max_points: usize,
    ) -> (Status, Vec<Vec3>) {
        let mut points = vec![0.0f32; max_points * 3];
        let mut count = 0i32;
        // SAFETY: buffers sized by max_points/path length; count
        // clamped below.
        let status = unsafe {
            Status(ffi::mangos_query_find_straight_path(
                self.ptr.as_ptr(),
                to_yzx(start).as_ptr(),
                to_yzx(end).as_ptr(),
                poly_path.as_ptr().cast::<u64>(),
                poly_path.len() as i32,
                points.as_mut_ptr(),
                &raw mut count,
                max_points as i32,
            ))
        };
        let n = (count.max(0) as usize).min(max_points);
        let out = (0..n)
            .map(|i| from_yzx([points[i * 3], points[i * 3 + 1], points[i * 3 + 2]]))
            .collect();
        (status, out)
    }

    /// `closestPointOnPoly`.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn closest_point_on_poly(&self, poly: PolyRef, pos: Vec3) -> (Status, Vec3, bool) {
        let mut closest = [0.0f32; 3];
        let mut over = 0i32;
        // SAFETY: stack out-params, valid for the call.
        let status = unsafe {
            Status(ffi::mangos_query_closest_point_on_poly(
                self.ptr.as_ptr(),
                poly.0,
                to_yzx(pos).as_ptr(),
                closest.as_mut_ptr(),
                &raw mut over,
            ))
        };
        (status, from_yzx(closest), over != 0)
    }

    /// `closestPointOnPolyBoundary`.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn closest_point_on_poly_boundary(&self, poly: PolyRef, pos: Vec3) -> (Status, Vec3) {
        let mut closest = [0.0f32; 3];
        // SAFETY: stack out-param, valid for the call.
        let status = unsafe {
            Status(ffi::mangos_query_closest_point_on_poly_boundary(
                self.ptr.as_ptr(),
                poly.0,
                to_yzx(pos).as_ptr(),
                closest.as_mut_ptr(),
            ))
        };
        (status, from_yzx(closest))
    }
}

/// `createFilter` include flags for the *moving* unit.
#[must_use]
pub fn mover_filter_include(can_walk: bool, can_swim: bool, is_player: bool) -> u16 {
    let mut include = 0u16;
    if can_walk {
        include |= nav_terrain::GROUND;
    }
    if can_swim {
        if is_player {
            include |= nav_terrain::WATER;
        } else {
            // creatures don't take environmental damage
            include |= nav_terrain::WATER | nav_terrain::MAGMA | nav_terrain::SLIME;
        }
    }
    include
}

/// `FillTargetAllowedFlags`: extra flags OR'd into the
/// destination lookup so a target standing somewhere the mover
/// cannot go still resolves to a poly.
#[must_use]
pub fn target_allowed_flags(can_swim: bool, can_walk: bool, is_player: bool) -> u16 {
    let mut flags = 0u16;
    if can_swim {
        flags |= nav_terrain::WATER | nav_terrain::SLIME | nav_terrain::MAGMA;
    }
    if can_walk {
        flags |= nav_terrain::GROUND;
    }
    if !is_player {
        flags |= nav_terrain::STEEP_SLOPES;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_parse_and_rejects() {
        let mut bytes = vec![0u8; 20];
        bytes[0..4].copy_from_slice(&MMAP_MAGIC.to_le_bytes());
        bytes[8..12].copy_from_slice(&MMAP_VERSION.to_le_bytes());
        let h = TileHeader::parse(&bytes).unwrap();
        assert_eq!(h.mmap_magic, MMAP_MAGIC);
        assert_eq!(h.mmap_version, MMAP_VERSION);
        assert!(TileHeader::parse(&bytes[..19]).is_none());
    }

    #[test]
    fn filter_flag_policies_match_cpp() {
        use nav_terrain as n;
        assert_eq!(mover_filter_include(true, false, false), n::GROUND);
        assert_eq!(
            mover_filter_include(true, true, false),
            n::GROUND | n::WATER | n::MAGMA | n::SLIME
        );
        assert_eq!(mover_filter_include(true, true, true), n::GROUND | n::WATER);
        assert_eq!(mover_filter_include(false, false, false), 0);
        assert_eq!(
            target_allowed_flags(true, true, false),
            n::WATER | n::SLIME | n::MAGMA | n::GROUND | n::STEEP_SLOPES
        );
        assert_eq!(target_allowed_flags(false, true, true), n::GROUND);
    }

    #[test]
    fn mesh_create_rejects_short_params() {
        assert!(NavMesh::from_mmap_params(&[0u8; 10]).is_none());
    }

    #[test]
    fn tile_file_header_validation() {
        // A params block of zeros is not a valid mesh, so use the
        // header checks against a tiny synthetic mesh instead:
        // construct params oversized-enough for one tile.
        let mut params = vec![0u8; NAVMESH_PARAMS_SIZE];
        // orig (0,0,0), tileWidth/Height 533.3333, maxTiles 32, maxPolys 4096
        params[12..16].copy_from_slice(&533.333_3f32.to_le_bytes());
        params[16..20].copy_from_slice(&533.333_3f32.to_le_bytes());
        params[20..24].copy_from_slice(&32i32.to_le_bytes());
        params[24..28].copy_from_slice(&4096i32.to_le_bytes());
        let mut mesh = NavMesh::from_mmap_params(&params).expect("params accepted");

        assert_eq!(mesh.load_tile_file(&[0u8; 10]), Err(TileError::Truncated));

        let mut bad = vec![0u8; 20];
        bad[8..12].copy_from_slice(&MMAP_VERSION.to_le_bytes());
        assert_eq!(mesh.load_tile_file(&bad), Err(TileError::BadMagic));

        bad[0..4].copy_from_slice(&MMAP_MAGIC.to_le_bytes());
        bad[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(mesh.load_tile_file(&bad), Err(TileError::BadVersion(99)));

        // Valid header, declared payload longer than the file.
        bad[8..12].copy_from_slice(&MMAP_VERSION.to_le_bytes());
        bad[12..16].copy_from_slice(&64u32.to_le_bytes());
        assert_eq!(mesh.load_tile_file(&bad), Err(TileError::Truncated));
    }
}
