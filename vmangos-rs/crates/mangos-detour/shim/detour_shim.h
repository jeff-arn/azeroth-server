// C ABI over the vendored Detour C++ classes (dtNavMesh / dtNavMeshQuery).
// The ONLY bridge between Rust and Detour; everything here is a thin
// forwarding wrapper so the Rust side never mirrors C++ class layouts
// (dtQueryFilter is constructed inside the shim from flag words).
#ifndef MANGOS_DETOUR_SHIM_H
#define MANGOS_DETOUR_SHIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// dtAlloc(DT_ALLOC_PERM) / dtFree for tile payload ownership.
unsigned char* mangos_tile_alloc(size_t size);
void mangos_tile_free(unsigned char* ptr);

// dtAllocNavMesh + init(dtNavMeshParams copied from `params` bytes).
// Returns null on failure.
void* mangos_navmesh_create(const unsigned char* params, size_t params_len);

// Single-tile mesh (gameobject models): init(data, size, DT_TILE_FREE_DATA).
// Takes ownership of `data` (must come from mangos_tile_alloc).
void* mangos_navmesh_create_single_tile(unsigned char* data, int size);

void mangos_navmesh_destroy(void* mesh);

// addTile(data, size, DT_TILE_FREE_DATA, 0, out_ref). On failure the caller
// still owns `data`. Returns dtStatus bits.
uint32_t mangos_navmesh_add_tile(void* mesh, unsigned char* data, int size, uint64_t* out_tile_ref);

// removeTile(ref, nullptr, nullptr). Returns dtStatus bits.
uint32_t mangos_navmesh_remove_tile(void* mesh, uint64_t tile_ref);

void mangos_navmesh_calc_tile_loc(const void* mesh, const float* pos_yzx, int* out_tx, int* out_ty);
int mangos_navmesh_has_tile_at(const void* mesh, int tx, int ty);

// dtAllocNavMeshQuery + init(mesh, max_nodes). Returns null on failure.
void* mangos_query_create(void* mesh, int max_nodes);
void mangos_query_destroy(void* query);

uint32_t mangos_query_find_nearest_poly(void* query, const float* center, const float* half_extents, uint16_t include_flags, uint16_t exclude_flags, uint64_t* out_ref, float* out_closest);

uint32_t mangos_query_find_path(void* query, uint64_t start_ref, uint64_t end_ref, const float* start_pos, const float* end_pos, uint16_t include_flags, uint16_t exclude_flags, uint64_t* out_path, int* out_count, int max_path);

uint32_t mangos_query_find_straight_path(void* query, const float* start_pos, const float* end_pos, const uint64_t* path, int path_len, float* out_points, int* out_count, int max_points);

uint32_t mangos_query_closest_point_on_poly(void* query, uint64_t ref, const float* pos, float* out_closest, int* out_over_poly);

uint32_t mangos_query_closest_point_on_poly_boundary(void* query, uint64_t ref, const float* pos, float* out_closest);

#ifdef __cplusplus
}
#endif

#endif
