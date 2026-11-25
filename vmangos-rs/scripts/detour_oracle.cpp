// Oracle harness for mangos-detour parity goldens.
//
// Uses the vendored Detour C++ API directly (no Rust shim), loading
// production .mmap/.mmtile files exactly as MoveMap.cpp does and
// running the PathFinder.cpp query policy (FindWalkPoly extents +
// above-check, creature-walk filter, target-allowed end filter,
// straight path). Prints JSON: {tiles_ok, near[], paths[]} with
// f32 values as bit patterns and positions in WoW XYZ.
//
// Usage: detour_oracle <mmaps_dir> <params_file> <tile_file>...

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "DetourAlloc.h"
#include "DetourCommon.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshQuery.h"

static const unsigned short NAV_GROUND = 0x01;
static const unsigned short NAV_MAGMA = 0x02;
static const unsigned short NAV_SLIME = 0x04;
static const unsigned short NAV_WATER = 0x08;
static const unsigned short NAV_STEEP_SLOPES = 0x10;

static uint32_t fbits(float f)
{
    uint32_t u;
    memcpy(&u, &f, 4);
    return u;
}

struct Sample
{
    dtPolyRef ref;
    float yzx[3];
};

static dtPolyRef findWalkPoly(dtNavMeshQuery const* query, float const* pointYZX, dtQueryFilter const& filter, float* closestPointYZX)
{
    float extents[3] = {5.0f, 10.0f, 5.0f};
    dtPolyRef polyRef;
    if (dtStatusFailed(query->findNearestPoly(pointYZX, extents, &filter, &polyRef, closestPointYZX)))
        return 0;
    if (closestPointYZX[1] > pointYZX[1] + 3.0f)
        return 0;
    return polyRef;
}

static std::vector<unsigned char> readFile(std::string const& path)
{
    std::vector<unsigned char> out;
    FILE* f = fopen(path.c_str(), "rb");
    if (!f)
        return out;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    out.resize(size > 0 ? (size_t)size : 0);
    if (!out.empty() && fread(out.data(), out.size(), 1, f) != 1)
        out.clear();
    fclose(f);
    return out;
}

int main(int argc, char** argv)
{
    if (argc < 4)
    {
        fprintf(stderr, "usage: detour_oracle <mmaps_dir> <params> <tile>...\n");
        return 2;
    }
    std::string dir = argv[1];

    auto paramsBytes = readFile(dir + "/" + argv[2]);
    if (paramsBytes.size() < sizeof(dtNavMeshParams))
    {
        fprintf(stderr, "params file too small\n");
        return 1;
    }
    dtNavMeshParams params;
    memcpy(&params, paramsBytes.data(), sizeof(params));

    dtNavMesh* mesh = dtAllocNavMesh();
    if (!mesh || dtStatusFailed(mesh->init(&params)))
    {
        fprintf(stderr, "navmesh init failed\n");
        return 1;
    }

    int tilesOk = 0;
    for (int i = 3; i < argc; ++i)
    {
        auto bytes = readFile(dir + "/" + argv[i]);
        if (bytes.size() < 20)
        {
            fprintf(stderr, "tile file too small: %s\n", argv[i]);
            return 1;
        }
        uint32_t magic, size;
        memcpy(&magic, bytes.data(), 4);
        memcpy(&size, bytes.data() + 12, 4);
        if (magic != 0x4d4d4150 || bytes.size() < 20u + size)
        {
            fprintf(stderr, "bad tile file: %s\n", argv[i]);
            return 1;
        }
        unsigned char* data = (unsigned char*)dtAlloc(size, DT_ALLOC_PERM);
        memcpy(data, bytes.data() + 20, size);
        dtTileRef ref = 0;
        if (dtStatusSucceed(mesh->addTile(data, (int)size, DT_TILE_FREE_DATA, 0, &ref)))
            tilesOk++;
        else
        {
            dtFree(data);
            fprintf(stderr, "addTile failed: %s\n", argv[i]);
            return 1;
        }
    }

    dtNavMeshQuery* query = dtAllocNavMeshQuery();
    if (!query || dtStatusFailed(query->init(mesh, 2048)))
    {
        fprintf(stderr, "query init failed\n");
        return 1;
    }

    // Sample on-mesh points: first 4 poly centroids per loaded tile.
    // (const handle: the mutable getTile overload is private.)
    const dtNavMesh* cmesh = mesh;
    std::vector<Sample> samples;
    for (int ti = 0; ti < cmesh->getMaxTiles() && samples.size() < 24; ++ti)
    {
        const dtMeshTile* tile = cmesh->getTile(ti);
        if (!tile || !tile->header)
            continue;
        int taken = 0;
        for (int pi = 0; pi < tile->header->polyCount && taken < 4; ++pi)
        {
            const dtPoly* poly = &tile->polys[pi];
            if (poly->vertCount < 3 || poly->flags == 0)
                continue;
            Sample s;
            s.ref = mesh->encodePolyId(tile->salt, (unsigned)ti, (unsigned)pi);
            s.yzx[0] = s.yzx[1] = s.yzx[2] = 0.0f;
            for (int v = 0; v < poly->vertCount; ++v)
            {
                const float* vert = &tile->verts[poly->verts[v] * 3];
                s.yzx[0] += vert[0];
                s.yzx[1] += vert[1];
                s.yzx[2] += vert[2];
            }
            float n = (float)poly->vertCount;
            s.yzx[0] /= n;
            s.yzx[1] /= n;
            s.yzx[2] /= n;
            samples.push_back(s);
            taken++;
        }
    }

    dtQueryFilter walkFilter;
    walkFilter.setIncludeFlags(NAV_GROUND | NAV_WATER | NAV_MAGMA | NAV_SLIME);
    dtQueryFilter endFilter = walkFilter;
    endFilter.setIncludeFlags(walkFilter.getIncludeFlags() | (NAV_WATER | NAV_SLIME | NAV_MAGMA | NAV_GROUND | NAV_STEEP_SLOPES));

    printf("{\"tiles_ok\": %d, \"near\": [", tilesOk);

    // Nearest-poly probes: centroid + YZX offsets, reported in XYZ.
    const float offs[3][3] = {{0.0f, 0.0f, 0.0f}, {2.5f, 1.0f, -2.5f}, {-4.0f, 2.5f, 4.0f}};
    bool first = true;
    for (auto const& s : samples)
    {
        for (auto const& o : offs)
        {
            float q[3] = {s.yzx[0] + o[0], s.yzx[1] + o[1], s.yzx[2] + o[2]};
            float closest[3] = {0.0f, 0.0f, 0.0f};
            dtPolyRef ref = findWalkPoly(query, q, walkFilter, closest);
            float dist = ref ? dtVdist(closest, q) : 0.0f;
            printf("%s{\"q\": [%u, %u, %u], \"ref\": %llu, \"c\": [%u, %u, %u], \"d\": %u}", first ? "" : ", ", fbits(q[2]), fbits(q[0]), fbits(q[1]), (unsigned long long)ref, fbits(closest[2]), fbits(closest[0]), fbits(closest[1]), fbits(dist));
            first = false;
        }
    }

    printf("], \"paths\": [");
    first = true;
    size_t n = samples.size();
    for (size_t i = 0; i < n && i < 8; ++i)
    {
        size_t j = (i * 3 + 2) % n;
        if (j == i)
            continue;
        const float* sp = samples[i].yzx;
        const float* ep = samples[j].yzx;
        float sc[3] = {0, 0, 0}, ec[3] = {0, 0, 0};
        dtPolyRef sref = findWalkPoly(query, sp, walkFilter, sc);
        dtPolyRef eref = findWalkPoly(query, ep, endFilter, ec);

        printf("%s{\"s\": [%u, %u, %u], \"e\": [%u, %u, %u], \"sref\": %llu, \"eref\": %llu", first ? "" : ", ", fbits(sp[2]), fbits(sp[0]), fbits(sp[1]), fbits(ep[2]), fbits(ep[0]), fbits(ep[1]), (unsigned long long)sref, (unsigned long long)eref);
        first = false;

        if (!sref || !eref)
        {
            printf(", \"status\": 0, \"refs\": [], \"sstatus\": 0, \"pts\": []}");
            continue;
        }

        dtPolyRef path[256];
        int pathCount = 0;
        dtStatus st = query->findPath(sref, eref, sp, ep, &walkFilter, path, &pathCount, 256);
        printf(", \"status\": %u, \"refs\": [", (unsigned)st);
        for (int k = 0; k < pathCount; ++k)
            printf("%s%llu", k ? ", " : "", (unsigned long long)path[k]);
        printf("]");

        float pts[256 * 3];
        int ptCount = 0;
        dtStatus sst = dtStatus(0);
        if (dtStatusSucceed(st))
            sst = query->findStraightPath(sp, ep, path, pathCount, pts, nullptr, nullptr, &ptCount, 256, 0);
        printf(", \"sstatus\": %u, \"pts\": [", (unsigned)sst);
        for (int k = 0; k < ptCount; ++k)
            printf("%s[%u, %u, %u]", k ? ", " : "", fbits(pts[k * 3 + 2]), fbits(pts[k * 3]), fbits(pts[k * 3 + 1]));
        printf("]}");
    }
    printf("]}\n");

    dtFreeNavMeshQuery(query);
    dtFreeNavMesh(mesh);
    return 0;
}
