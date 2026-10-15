#include "qcommon/cm_local.h"
#include "bridge.h"
#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <map>
#include <memory>
#include <mutex>
#include <numeric>
#include <stdexcept>
#include <string>
#include <vector>

static std::recursive_mutex engine_mutex;
extern "C" void jka_lock() { engine_mutex.lock(); }
extern "C" void jka_unlock() { engine_mutex.unlock(); }
static thread_local std::string last_error;
extern "C" const char *jka_collision_error() { return last_error.c_str(); }

extern clipMap_t SubBSP[MAX_SUB_BSP];
extern int NumSubBSP, TotalSubModels;
extern cmodel_t box_model;
extern cplane_t *box_planes;
extern cbrush_t *box_brush;
int CM_LoadSubBSP(const char *, qboolean);

// Experimental, OFF by default. A/B toggle does not rebuild maps or shaders.
static std::atomic<int> collision_merge_enabled{0};
extern "C" int jka_collision_merge_get() { return collision_merge_enabled.load(std::memory_order_relaxed); }
extern "C" void jka_collision_merge_set(int value) { collision_merge_enabled.store(value != 0, std::memory_order_relaxed); }

struct CollisionUnion {
    clipMap_t map{}; // Original BSP tree, patch surfaces, and inline models are shared.
    std::vector<cbrush_t> brushes;
    std::vector<int> leafbrushes;
    std::vector<cLeaf_t> leafs;
    std::vector<std::unique_ptr<cbrushside_t[]>> merged_sides;
    int removed_leaf_references = 0;
    int removed_brushes = 0;
    int removed_internal_faces = 0;
};

struct BoxGroup {
    bool eligible = false;
    bool merged = false;
    float mins[3]{}, maxs[3]{};
    cbrushside_t sides[6]{};
    int contents = 0;
};

static uint32_t float_key(float v) {
    // +0 and -0 compare equal, so they must hash equal too.
    if (v == 0.0f) return 0;
    uint32_t bits;
    memcpy(&bits, &v, sizeof(bits));
    return bits;
}

// CM_BoundBrush and OpenJK CM_TraceThroughBrush require EXACTLY this axial
// ordering, rather than merely six arbitrary plane normals.
static bool eligible_box(const cbrush_t &b, const clipMap_t &map) {
    if (b.numsides != 6 || b.contents != CONTENTS_SOLID || b.shaderNum < 0 || b.shaderNum >= map.numShaders)
        return false;
    for (int axis = 0; axis < 3; ++axis) {
        if (!std::isfinite(b.bounds[0][axis]) || !std::isfinite(b.bounds[1][axis]) ||
            !(b.bounds[0][axis] < b.bounds[1][axis])) return false;
        for (int sign = 0; sign < 2; ++sign) {
            const cbrushside_t &side = b.sides[axis * 2 + sign];
            if (!side.plane || side.shaderNum < 0 || side.shaderNum >= map.numShaders) return false;
            const cplane_t &p = *side.plane;
            for (int k = 0; k < 3; ++k) {
                if (p.normal[k] != (k == axis ? (sign == 0 ? -1.f : 1.f) : 0.f)) return false;
            }
            if (p.dist != (sign == 0 ? -b.bounds[0][axis] : b.bounds[1][axis])) return false;
        }
    }
    return true;
}

static std::unique_ptr<CollisionUnion> build_collision_union(const clipMap_t &original) {
    auto opt = std::make_unique<CollisionUnion>();
    const int n = original.numBrushes;
    opt->map = original;
    // Include the temporary box brush at index numBrushes. It must be separate
    // from the original because CM_TempBoxModel mutates its bounds each call.
    opt->brushes.assign(original.brushes, original.brushes + n + 1);
    opt->leafbrushes.assign(original.leafbrushes, original.leafbrushes + original.numLeafBrushes + 1);
    opt->map.brushes = opt->brushes.data();
    opt->map.leafbrushes = opt->leafbrushes.data();

    std::vector<uint8_t> in_world(n, 0), in_inline(n, 0);
    for (int i = 0; i < original.numLeafBrushes; ++i) {
        const int brush = original.leafbrushes[i];
        if (brush >= 0 && brush < n) in_world[brush] = 1;
    }
    // Never modify collision owned by entities/movers or misc_bsp submodels.
    for (int model = 1; model < original.numSubModels; ++model) {
        const cLeaf_t &leaf = original.cmodels[model].leaf;
        for (int j = 0; j < leaf.numLeafBrushes; ++j) {
            const int index = original.leafbrushes[leaf.firstLeafBrush + j];
            if (index >= 0 && index < n) in_inline[index] = 1;
        }
    }
    std::vector<BoxGroup> groups(n);
    std::vector<int> parent(n);
    std::iota(parent.begin(), parent.end(), 0);
    for (int i = 0; i < n; ++i) {
        const cbrush_t &brush = original.brushes[i];
        if (!in_world[i] || in_inline[i] || !eligible_box(brush, original)) continue;
        BoxGroup &group = groups[i];
        group.eligible = true;
        group.contents = brush.contents;
        for (int axis = 0; axis < 3; ++axis) {
            group.mins[axis] = brush.bounds[0][axis];
            group.maxs[axis] = brush.bounds[1][axis];
        }
        std::copy(brush.sides, brush.sides + 6, group.sides);
    }
    auto root = [&](int id) {
        int r = id;
        while (parent[r] != r) r = parent[r];
        while (parent[id] != id) { const int next = parent[id]; parent[id] = r; id = next; }
        return r;
    };
    // A union is a single rectangular prism only when both brushes have
    // IDENTICAL spans on the other two axes and meet exactly on this axis.
    // Four extending faces must agree on surface flags/shaders: otherwise
    // replacing the boundary changes trace.surfaceFlags.
    for (int round = 0; round < 4; ++round) {
        bool progress = false;
        for (int axis = 0; axis < 3; ++axis) {
            using Key = std::array<uint32_t, 9>;
            std::map<Key, std::vector<int>> buckets;
            for (int i = 0; i < n; ++i) {
                const BoxGroup &g = groups[i];
                if (!g.eligible) continue;
                Key key{};
                key[0] = static_cast<uint32_t>(g.contents);
                int at = 1;
                for (int k = 0; k < 3; ++k) {
                    if (k == axis) continue;
                    key[at++] = float_key(g.mins[k]);
                    key[at++] = float_key(g.maxs[k]);
                    key[at++] = static_cast<uint32_t>(original.shaders[g.sides[2 * k].shaderNum].surfaceFlags);
                    key[at++] = static_cast<uint32_t>(original.shaders[g.sides[2 * k + 1].shaderNum].surfaceFlags);
                }
                buckets[key].push_back(i);
            }
            for (auto &entry : buckets) {
                auto &items = entry.second;
                std::sort(items.begin(), items.end(), [&](int a, int b) {
                    const BoxGroup &x = groups[a], &y = groups[b];
                    if (x.mins[axis] != y.mins[axis]) return x.mins[axis] < y.mins[axis];
                    return x.maxs[axis] < y.maxs[axis];
                });
                if (items.empty()) continue;
                int current = items[0];
                for (size_t j = 1; j < items.size(); ++j) {
                    const int next = items[j];
                    BoxGroup &left = groups[current], &right = groups[next];
                    if (left.maxs[axis] != right.mins[axis]) {
                        current = next;
                        continue;
                    }
                    // Keep the original outer planes; the mutual internal
                    // planes disappear. Leaves referring to either input will
                    // reference the new canonical brush after the sweep.
                    left.maxs[axis] = right.maxs[axis];
                    left.merged = true;
                    left.sides[axis * 2 + 1] = right.sides[axis * 2 + 1];
                    right.eligible = false;
                    parent[next] = current;
                    ++opt->removed_brushes;
                    opt->removed_internal_faces += 2;
                    progress = true;
                }
            }
        }
        if (!progress) break;
    }
    if (!opt->removed_brushes) return opt;
    for (int i = 0; i < n; ++i) {
        const BoxGroup &g = groups[i];
        if (!g.eligible || !g.merged || root(i) != i) continue;
        auto sides = std::make_unique<cbrushside_t[]>(6);
        std::copy(g.sides, g.sides + 6, sides.get());
        cbrush_t &brush = opt->brushes[i];
        brush.sides = sides.get();
        brush.numsides = 6;
        for (int axis = 0; axis < 3; ++axis) {
            brush.bounds[0][axis] = g.mins[axis];
            brush.bounds[1][axis] = g.maxs[axis];
        }
        opt->merged_sides.push_back(std::move(sides));
    }
    for (int &id : opt->leafbrushes) {
        if (id >= 0 && id < n) id = root(id);
    }
    // The stock brush checkcount suppresses duplicate traces, but
    // CM_PointContents has no checkcount. Compact each world's leaf list
    // in place, keeping its firstLeafBrush unchanged. Preserve the total
    // array length so original inline-model and TempBoxModel offsets remain
    // valid. No BSP nodes / PVS / area portals are touched.
    opt->leafs.assign(original.leafs, original.leafs + original.numLeafs);
    opt->map.leafs = opt->leafs.data();
    std::vector<uint32_t> seen(n, 0);
    uint32_t generation = 0;
    for (cLeaf_t &leaf : opt->leafs) {
        if (++generation == 0) { std::fill(seen.begin(), seen.end(), 0); ++generation; }
        const int start = static_cast<int>(leaf.firstLeafBrush);
        const int end = start + leaf.numLeafBrushes;
        int write = start;
        for (int k = start; k < end; ++k) {
            const int brush = opt->leafbrushes[k];
            if (brush >= 0 && brush < n) {
                if (seen[brush] == generation) continue;
                seen[brush] = generation;
            }
            opt->leafbrushes[write++] = brush;
        }
        opt->removed_leaf_references += end - write;
        leaf.numLeafBrushes = write - start;
    }
    return opt;
}

struct World {
    clipMap_t map{};
    std::unique_ptr<CollisionUnion> optimized;
    std::vector<clipMap_t> submaps;
    int total_models = 0;
    cmodel_t box{};
    cplane_t *box_planes = nullptr;
    cbrush_t *box_brush = nullptr;
    // CM's inline models store offsets between hunk allocations. Keep a single
    // contiguous arena, as the stock engine does, so those offsets remain valid.
    static constexpr size_t capacity = 256u * 1024u * 1024u;
    unsigned char *arena = static_cast<unsigned char *>(malloc(capacity));
    size_t used = 0;
    std::vector<void *> temporary;
    World() { if (!arena) throw std::bad_alloc(); }
    ~World() { for (void *p : temporary) free(p); free(arena); }
};
// OpenJK globals must travel with their owning arena, including sub-BSP
// checkcounts and the temporary box hull used to clip snapshot player boxes.
static void activate_world(World *world, bool merged = false) {
    cmg = (merged && world->optimized && world->optimized->removed_brushes)
        ? world->optimized->map : world->map;
    NumSubBSP = static_cast<int>(world->submaps.size());
    std::copy(world->submaps.begin(), world->submaps.end(), SubBSP);
    TotalSubModels = world->total_models;
    box_model = world->box;
    box_planes = world->box_planes;
    // Temporary encoded player/vehicle boxes must belong to the ACTIVE brush
    // array. Their planes and brush sides are still in the original hunk.
    box_brush = &cmg.brushes[cmg.numBrushes];
}
static void capture_world(World *world, bool merged = false) {
    if (merged && world->optimized && world->optimized->removed_brushes)
        world->optimized->map = cmg;
    else
        world->map = cmg;
    world->submaps.assign(SubBSP, SubBSP + NumSubBSP);
    world->total_models = TotalSubModels;
    world->box = box_model;
    world->box_planes = box_planes;
    world->box_brush = box_brush;
}
static World *loading;
static const unsigned char *map_data;
static int map_length;
static const char *map_name;
static cvar_t zero_cvar{}, one_cvar{};
cvar_t *com_dedicated = &one_cvar;
qboolean CM_DeleteCachedMap(qboolean);

void QDECL JKA_CM_Printf(const char *, ...) {}
void QDECL Com_DPrintf(const char *, ...) {}
void BotDrawDebugPolygons(void (*)(int, int, float *), int) {}
void NORETURN QDECL JKA_CM_Error(int, const char *format, ...) {
    char buffer[1024];
    va_list args;
    va_start(args, format);
    vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    throw std::runtime_error(buffer);
}
void *Hunk_Alloc(int size, ha_pref) {
    if (!loading || size < 0) throw std::runtime_error("Invalid collision allocation");
    size_t length = (static_cast<size_t>(std::max(size, 1)) + 15u) & ~size_t(15u);
    if (length > World::capacity - loading->used) throw std::runtime_error("Collision arena exceeds 256 MiB");
    void *p = loading->arena + loading->used;
    loading->used += length;
    memset(p, 0, length);
    return p;
}
void *Z_Malloc(int size, memtag_t, qboolean zero, int) {
    if (size < 0) throw std::runtime_error("Invalid zone allocation");
    void *p = zero ? calloc(1, std::max(size, 1)) : malloc(std::max(size, 1));
    if (!p) throw std::bad_alloc();
    if (loading) loading->temporary.push_back(p);
    return p;
}
void Z_Free(void *p) {
    if (loading) {
        auto &blocks = loading->temporary;
        auto found = std::find(blocks.begin(), blocks.end(), p);
        if (found != blocks.end()) blocks.erase(found);
    }
    free(p);
}
void *Hunk_AllocateTempMemory(int size) { return Z_Malloc(size, TAG_TEMP_WORKSPACE, qtrue, 4); }
void Hunk_FreeTempMemory(void *p) { Z_Free(p); }
cvar_t *Cvar_Get(const char *, const char *value, uint32_t, const char *) {
    one_cvar.integer = 1; one_cvar.value = 1;
    return atoi(value) ? &one_cvar : &zero_cvar;
}
long FS_FOpenFileRead(const char *name, fileHandle_t *handle, qboolean) {
    if (!map_name || strcmp(name, map_name)) { *handle = 0; return -1; }
    *handle = 1; return map_length;
}
int FS_Read(void *buffer, int length, fileHandle_t handle) {
    if (handle != 1 || length < 0 || length > map_length) throw std::runtime_error("Invalid BSP read");
    memcpy(buffer, map_data, length); return length;
}
void FS_FCloseFile(fileHandle_t) {}
qboolean Sys_LowPhysicalMemory() { return qtrue; }
/* The checksum is unused by this offline host. Network checksum support is separate. */
uint32_t Com_BlockChecksum(const void *, int) { return 0; }

extern "C" void *jka_world_new(const unsigned char *data, int length) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = nullptr;
    try {
        world = new World;
        loading = world; map_data = data; map_length = length; map_name = "maps/offline.bsp";
        int checksum;
        CM_LoadMap("maps/offline.bsp", qfalse, &checksum);
        capture_world(world);
        loading = nullptr; map_data = nullptr; map_length = 0; map_name = nullptr;
        return world;
    } catch (const std::exception &e) {
        last_error = e.what();
        CM_DeleteCachedMap(qtrue);
        CM_ClearMap();
        loading = nullptr; map_data = nullptr; map_length = 0; map_name = nullptr;
        delete world;
        return nullptr;
    }
}
extern "C" int jka_world_load_sub_bsp(void *handle, const char *name,
                                      const unsigned char *data, int length) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = static_cast<World *>(handle);
    activate_world(world);
    try {
        loading = world; map_data = data; map_length = length; map_name = name;
        const int offset = CM_LoadSubBSP(name, qfalse);
        capture_world(world);
        loading = nullptr; map_data = nullptr; map_length = 0; map_name = nullptr;
        return offset;
    } catch (const std::exception &e) {
        last_error = e.what();
        loading = nullptr; map_data = nullptr; map_length = 0; map_name = nullptr;
        return -1;
    }
}
// Called once from the BSP loader worker after all sub-BSPs have loaded.
// -1 signals an optimization failure; gameplay still uses the original BSP.
extern "C" int jka_world_prepare_merge(void *handle) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = static_cast<World *>(handle);
    if (!world) return -1;
    try {
        world->optimized = build_collision_union(world->map);
        return world->optimized->removed_brushes;
    } catch (const std::exception &e) {
        last_error = e.what();
        world->optimized.reset();
        return -1;
    }
}
extern "C" void jka_world_free(void *handle) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    CM_ClearMap();
    delete static_cast<World *>(handle);
}
static void export_trace(const trace_t &t, jka_trace *out) {
    out->fraction = t.fraction;
    VectorCopy(t.endpos, out->end); VectorCopy(t.plane.normal, out->normal);
    out->distance = t.plane.dist; out->surface_flags = t.surfaceFlags;
    out->contents = t.contents; out->entity = t.entityNum;
    out->start_solid = t.startsolid; out->all_solid = t.allsolid;
}
extern "C" int jka_world_trace(void *handle, jka_trace *out, const float *start, const float *mins,
                               const float *maxs, const float *end, int mask, int model,
                               const float *origin, const float *angles) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = static_cast<World *>(handle);
    const bool merged = model == 0 && collision_merge_enabled.load(std::memory_order_relaxed) != 0;
    activate_world(world, merged);
    try {
        trace_t t{};
        if (model) CM_TransformedBoxTrace(&t, start, end, mins, maxs, model, mask, origin, angles, 0);
        else CM_BoxTrace(&t, start, end, mins, maxs, 0, mask, 0);
        t.entityNum = (t.fraction < 1 || t.startsolid) ? ENTITYNUM_WORLD : ENTITYNUM_NONE;
        export_trace(t, out);
        capture_world(world, merged);
        return 1;
    } catch (const std::exception &e) { last_error = e.what(); return 0; }
}
extern "C" int jka_world_contents(void *handle, const float *point, int model, const float *origin, const float *angles) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = static_cast<World *>(handle);
    const bool merged = model == 0 && collision_merge_enabled.load(std::memory_order_relaxed) != 0;
    activate_world(world, merged);
    try {
        const int contents = model ? CM_TransformedPointContents(point, model, origin, angles) : CM_PointContents(point, 0);
        capture_world(world, merged);
        return contents;
    } catch (const std::exception &e) { last_error = e.what(); return -1; }
}
/* CG_ClipMoveToEntities for an encoded-bbox entity: CM_TempBoxModel placed at
 * the entity origin with no rotation (trap->CM_TempModel + CM_TransformedTrace). */
extern "C" int jka_world_trace_box_entity(void *handle, jka_trace *out, const float *start, const float *mins,
                                          const float *maxs, const float *end, int mask,
                                          const float *box_mins, const float *box_maxs, const float *origin) {
    std::lock_guard<std::recursive_mutex> lock(engine_mutex);
    World *world = static_cast<World *>(handle);
    activate_world(world);
    try {
        trace_t t{};
        const vec3_t no_rotation = {0, 0, 0};
        clipHandle_t box = CM_TempBoxModel(box_mins, box_maxs, 0);
        CM_TransformedBoxTrace(&t, start, end, mins, maxs, box, mask, origin, no_rotation, 0);
        export_trace(t, out);
        capture_world(world);
        return 1;
    } catch (const std::exception &e) { last_error = e.what(); return 0; }
}
