#include "detour_shim.h"

#include <string.h>

#include "DetourAlloc.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshQuery.h"

static dtQueryFilter makeFilter(uint16_t include_flags, uint16_t exclude_flags)
{
    dtQueryFilter filter;
    filter.setIncludeFlags(include_flags);
    filter.setExcludeFlags(exclude_flags);
    return filter;
}

extern "C" {

unsigned char* mangos_tile_alloc(size_t size) { return (unsigned char*)dtAlloc(size, DT_ALLOC_PERM); }

void mangos_tile_free(unsigned char* ptr) { dtFree(ptr); }

void* mangos_navmesh_create(const unsigned char* params, size_t params_len)
{
    if (!params || params_len < sizeof(dtNavMeshParams))
        return nullptr;
    dtNavMeshParams p;
    memcpy(&p, params, sizeof(dtNavMeshParams));
    dtNavMesh* mesh = dtAllocNavMesh();
    if (!mesh)
        return nullptr;
    if (dtStatusFailed(mesh->init(&p)))
    {
        dtFreeNavMesh(mesh);
        return nullptr;
    }
    return mesh;
}

void* mangos_navmesh_create_single_tile(unsigned char* data, int size)
{
    dtNavMesh* mesh = dtAllocNavMesh();
    if (!mesh)
        return nullptr;
    if (dtStatusFailed(mesh->init(data, size, DT_TILE_FREE_DATA)))
    {
        dtFreeNavMesh(mesh);
        return nullptr;
    }
    return mesh;
}

void mangos_navmesh_destroy(void* mesh) { dtFreeNavMesh((dtNavMesh*)mesh); }

uint32_t mangos_navmesh_add_tile(void* mesh, unsigned char* data, int size, uint64_t* out_tile_ref)
{
    dtTileRef ref = 0;
    dtStatus st = ((dtNavMesh*)mesh)->addTile(data, size, DT_TILE_FREE_DATA, 0, &ref);
    if (out_tile_ref)
        *out_tile_ref = (uint64_t)ref;
    return (uint32_t)st;
}

uint32_t mangos_navmesh_remove_tile(void* mesh, uint64_t tile_ref) { return (uint32_t)((dtNavMesh*)mesh)->removeTile((dtTileRef)tile_ref, nullptr, nullptr); }

void mangos_navmesh_calc_tile_loc(const void* mesh, const float* pos_yzx, int* out_tx, int* out_ty) { ((const dtNavMesh*)mesh)->calcTileLoc(pos_yzx, out_tx, out_ty); }

int mangos_navmesh_has_tile_at(const void* mesh, int tx, int ty) { return ((const dtNavMesh*)mesh)->getTileAt(tx, ty, 0) != nullptr ? 1 : 0; }

void* mangos_query_create(void* mesh, int max_nodes)
{
    dtNavMeshQuery* query = dtAllocNavMeshQuery();
    if (!query)
        return nullptr;
    if (dtStatusFailed(query->init((dtNavMesh*)mesh, max_nodes)))
    {
        dtFreeNavMeshQuery(query);
        return nullptr;
    }
    return query;
}

void mangos_query_destroy(void* query) { dtFreeNavMeshQuery((dtNavMeshQuery*)query); }

uint32_t mangos_query_find_nearest_poly(void* query, const float* center, const float* half_extents, uint16_t include_flags, uint16_t exclude_flags, uint64_t* out_ref, float* out_closest)
{
    dtQueryFilter filter = makeFilter(include_flags, exclude_flags);
    dtPolyRef ref = 0;
    dtStatus st = ((dtNavMeshQuery*)query)->findNearestPoly(center, half_extents, &filter, &ref, out_closest);
    if (out_ref)
        *out_ref = (uint64_t)ref;
    return (uint32_t)st;
}

uint32_t mangos_query_find_path(void* query, uint64_t start_ref, uint64_t end_ref, const float* start_pos, const float* end_pos, uint16_t include_flags, uint16_t exclude_flags, uint64_t* out_path, int* out_count, int max_path)
{
    dtQueryFilter filter = makeFilter(include_flags, exclude_flags);
    // dtPolyRef is uint64_t in this vendored build (DT_POLYREF64), so the
    // caller's u64 buffer layout matches.
    return (uint32_t)((dtNavMeshQuery*)query)->findPath((dtPolyRef)start_ref, (dtPolyRef)end_ref, start_pos, end_pos, &filter, (dtPolyRef*)out_path, out_count, max_path);
}

uint32_t mangos_query_find_straight_path(void* query, const float* start_pos, const float* end_pos, const uint64_t* path, int path_len, float* out_points, int* out_count, int max_points) { return (uint32_t)((dtNavMeshQuery*)query)->findStraightPath(start_pos, end_pos, (const dtPolyRef*)path, path_len, out_points, nullptr, nullptr, out_count, max_points, 0); }

uint32_t mangos_query_closest_point_on_poly(void* query, uint64_t ref, const float* pos, float* out_closest, int* out_over_poly)
{
    bool over = false;
    dtStatus st = ((dtNavMeshQuery*)query)->closestPointOnPoly((dtPolyRef)ref, pos, out_closest, &over);
    if (out_over_poly)
        *out_over_poly = over ? 1 : 0;
    return (uint32_t)st;
}

uint32_t mangos_query_closest_point_on_poly_boundary(void* query, uint64_t ref, const float* pos, float* out_closest) { return (uint32_t)((dtNavMeshQuery*)query)->closestPointOnPolyBoundary((dtPolyRef)ref, pos, out_closest); }

} // extern "C"
