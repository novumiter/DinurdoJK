//! Source 1 StudioMDL v48/v49 companion mesh data (VVD vertices, VTX topology).
//!
//! The single most important rule here: a VVD's raw vertex table is NOT the
//! array that MDL mesh offsets address. Valve's `Studio_LoadVertexes` first
//! gathers the table through `vertexFileFixup_t` records for the chosen root
//! LOD, and `Studio_SetRootLOD` then derives every model/mesh vertex offset
//! from the per-mesh `numLODVertexes`. Indexing the raw table directly passes
//! every bounds check while permuting positions, normals, UVs and bone weights
//! together, which looks exactly like a broken skinning matrix.

use crate::source1::SUPPORTED_MDL_VERSIONS;

#[derive(Clone, Debug)]
pub struct SourceVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub weights: [f32; 3],
    pub bones: [u8; 3],
    pub bone_count: u8,
}

/// Valve `vertexFileFixup_t`. `lod` is the coarsest LOD that still uses this
/// run, so a gather for root LOD `n` keeps every record with `lod >= n`.
#[derive(Clone, Copy, Debug)]
pub struct SourceVertexFixup {
    pub lod: usize,
    pub source_vertex: usize,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct SourceVvd {
    pub checksum: i32,
    pub lod_count: usize,
    pub lod_vertices: [u32; SOURCE_MAX_LODS],
    pub fixups: Vec<SourceVertexFixup>,
    /// Raw StudioMDL vertex table, exactly as stored. Use `source_lod_vertices`
    /// to obtain the array that MDL offsets actually index.
    pub vertices: Vec<SourceVertex>,
}

#[derive(Clone, Debug)]
pub struct SourceStripGroup {
    /// `origMeshVertID` per strip-group vertex: an MDL mesh-local vertex id.
    pub mesh_vertex_ids: Vec<u16>,
    /// Triangles index into `mesh_vertex_ids`, not into the VVD vertex table.
    pub triangles: Vec<[u16; 3]>,
}

#[derive(Clone, Debug)]
pub struct SourceVtxMesh {
    pub bodypart: usize,
    pub model: usize,
    pub lod: usize,
    pub mesh: usize,
    pub strip_groups: Vec<SourceStripGroup>,
}

#[derive(Clone, Debug)]
pub struct SourceVtx {
    pub checksum: i32,
    pub lod_count: usize,
    pub meshes: Vec<SourceVtxMesh>,
}

/// One MDL mesh: a material slot plus its per-LOD vertex counts.
#[derive(Clone, Debug)]
pub struct SourceMesh {
    /// `mstudiomesh_t::material` indexes the *skin reference* table, not the
    /// texture table directly.
    pub skin_ref: i32,
    pub lod_vertices: [usize; SOURCE_MAX_LODS],
}

/// One `$bodygroup` choice.
#[derive(Clone, Debug)]
pub struct SourceBodyPartModel {
    pub name: String,
    pub meshes: Vec<SourceMesh>,
}

#[derive(Clone, Debug)]
pub struct SourceBodyPart {
    pub name: String,
    pub models: Vec<SourceBodyPartModel>,
}

/// A material-indexed mesh section. `indices` address `source_lod_vertices`
/// for the same LOD this was assembled for.
#[derive(Clone, Debug)]
pub struct SourceDrawMesh {
    pub bodypart: usize,
    pub model: usize,
    pub mesh: usize,
    pub skin_ref: i32,
    pub indices: Vec<u32>,
}

pub const SOURCE_MAX_LODS: usize = 8;
/// Size of `mstudiovertex_t`.
const VVD_STRIDE: usize = 48;
/// Sizes of `mstudiobodyparts_t`, `mstudiomodel_t`, `mstudiomesh_t`.
const BODYPART_SIZE: usize = 16;
const MODEL_SIZE: usize = 148;
const MESH_SIZE: usize = 116;
/// `mstudiomesh_t::vertexdata.numLODVertexes[8]`, after the unused pointer.
const MESH_LOD_VERTEXES: usize = 52;

fn take(bytes: &[u8], offset: usize, n: usize) -> Result<&[u8], String> {
    bytes
        .get(offset..offset.checked_add(n).ok_or("offset overflow")?)
        .ok_or_else(|| format!("truncated Source geometry at byte {offset}"))
}
fn int(bytes: &[u8], p: usize) -> Result<i32, String> {
    Ok(i32::from_le_bytes(take(bytes, p, 4)?.try_into().unwrap()))
}
fn count(bytes: &[u8], p: usize, max: usize) -> Result<usize, String> {
    let n = usize::try_from(int(bytes, p)?).map_err(|_| "negative geometry count")?;
    if n > max {
        return Err(format!("unreasonable Source geometry count {n}"));
    }
    Ok(n)
}
fn rel(bytes: &[u8], base: usize, p: usize) -> Result<usize, String> {
    let offset = usize::try_from(int(bytes, p)?).map_err(|_| "negative geometry offset")?;
    base.checked_add(offset)
        .ok_or_else(|| "geometry offset overflow".to_owned())
}
fn float(bytes: &[u8], p: usize) -> Result<f32, String> {
    let value = f32::from_bits(int(bytes, p)? as u32);
    if !value.is_finite() {
        return Err("non-finite Source vertex component".into());
    }
    Ok(value)
}
fn cstring(bytes: &[u8], at: usize) -> Result<String, String> {
    let rest = bytes.get(at..).ok_or("MDL string outside file")?;
    let len = rest
        .iter()
        .position(|v| *v == 0)
        .ok_or("unterminated MDL string")?;
    Ok(String::from_utf8_lossy(&rest[..len]).into_owned())
}

/// Parse Source VVD v4. The fixup table is retained, not merely validated.
pub fn parse_source_vvd(bytes: &[u8]) -> Result<SourceVvd, String> {
    if take(bytes, 0, 4)? != b"IDSV" {
        return Err("invalid VVD signature".into());
    }
    if int(bytes, 4)? != 4 {
        return Err("unsupported VVD version".into());
    }
    let checksum = int(bytes, 8)?;
    let lod_count = count(bytes, 12, SOURCE_MAX_LODS)?;
    if lod_count == 0 {
        return Err("VVD without LODs".into());
    }
    let mut lod_vertices = [0; SOURCE_MAX_LODS];
    for (i, n) in lod_vertices.iter_mut().enumerate() {
        *n = u32::try_from(int(bytes, 16 + i * 4)?).map_err(|_| "negative LOD count")?;
    }
    let fixup_count = count(bytes, 48, 1_000_000)?;
    let fixup_start = count(bytes, 52, bytes.len())?;
    take(
        bytes,
        fixup_start,
        fixup_count.checked_mul(12).ok_or("fixup size overflow")?,
    )?;
    let vertex_start = count(bytes, 56, bytes.len())?;
    // The raw table holds every vertex the most detailed LOD uses.
    let vertex_count = lod_vertices[0] as usize;
    if vertex_count > 2_000_000 {
        return Err("VVD vertex count too large".into());
    }
    take(
        bytes,
        vertex_start,
        vertex_count
            .checked_mul(VVD_STRIDE)
            .ok_or("VVD size overflow")?,
    )?;

    let mut fixups = Vec::with_capacity(fixup_count);
    for i in 0..fixup_count {
        let p = fixup_start + i * 12;
        let lod = count(bytes, p, SOURCE_MAX_LODS)?;
        let source_vertex = count(bytes, p + 4, vertex_count)?;
        let run = count(bytes, p + 8, vertex_count)?;
        if lod >= lod_count
            || source_vertex
                .checked_add(run)
                .is_none_or(|end| end > vertex_count)
        {
            return Err("VVD fixup references invalid vertices/LOD".into());
        }
        fixups.push(SourceVertexFixup {
            lod,
            source_vertex,
            count: run,
        });
    }

    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let p = vertex_start + i * VVD_STRIDE;
        let weights = [float(bytes, p)?, float(bytes, p + 4)?, float(bytes, p + 8)?];
        let bones: [u8; 3] = take(bytes, p + 12, 3)?.try_into().unwrap();
        let bone_count = bytes[p + 15];
        if bone_count == 0 || bone_count > 3 || weights.iter().any(|w| *w < 0.0 || *w > 1.001) {
            return Err(format!("invalid weights for VVD vertex {i}"));
        }
        vertices.push(SourceVertex {
            position: [
                float(bytes, p + 16)?,
                float(bytes, p + 20)?,
                float(bytes, p + 24)?,
            ],
            normal: [
                float(bytes, p + 28)?,
                float(bytes, p + 32)?,
                float(bytes, p + 36)?,
            ],
            uv: [float(bytes, p + 40)?, float(bytes, p + 44)?],
            weights,
            bones,
            bone_count,
        });
    }
    Ok(SourceVvd {
        checksum,
        lod_count,
        lod_vertices,
        fixups,
        vertices,
    })
}

/// Exact port of the gather loop in Valve `Studio_LoadVertexes`: records whose
/// `lod` is below the requested root LOD describe higher-detail runs and are
/// skipped; everything else is appended in table order. With no fixups the raw
/// table is already in LOD order. For root LOD 0 nothing is skipped, but the
/// gather is still a permutation of the raw table and is never a no-op.
pub fn source_lod_vertices(vvd: &SourceVvd, root_lod: usize) -> Result<Vec<SourceVertex>, String> {
    if root_lod >= vvd.lod_count {
        return Err(format!(
            "Source root LOD {root_lod} outside the model's {} LODs",
            vvd.lod_count
        ));
    }
    let expected = vvd.lod_vertices[root_lod] as usize;
    if vvd.fixups.is_empty() {
        if expected > vvd.vertices.len() {
            return Err("VVD LOD vertex count exceeds the stored table".into());
        }
        return Ok(vvd.vertices[..expected].to_vec());
    }
    let mut out = Vec::with_capacity(expected);
    for fixup in &vvd.fixups {
        if fixup.lod < root_lod {
            continue;
        }
        let run = vvd
            .vertices
            .get(fixup.source_vertex..fixup.source_vertex + fixup.count)
            .ok_or("VVD fixup run outside the vertex table")?;
        out.extend_from_slice(run);
    }
    if out.len() != expected {
        return Err(format!(
            "VVD fixup gather produced {} vertices for LOD {root_lod}, header declares {expected}",
            out.len()
        ));
    }
    Ok(out)
}

/// Decode the VTX v7 hierarchy and triangulate strip groups.
pub fn parse_source_vtx(bytes: &[u8]) -> Result<SourceVtx, String> {
    if int(bytes, 0)? != 7 {
        return Err("unsupported VTX version".into());
    }
    take(bytes, 0, 36)?;
    let checksum = int(bytes, 16)?;
    let lod_count = count(bytes, 20, SOURCE_MAX_LODS)?;
    let bodyparts = count(bytes, 28, 1024)?;
    let bodyparts_at = count(bytes, 32, bytes.len())?;
    let mut meshes = Vec::new();
    for b in 0..bodyparts {
        let bp = bodyparts_at
            .checked_add(b * 8)
            .ok_or("VTX bodypart overflow")?;
        take(bytes, bp, 8)?;
        let models = count(bytes, bp, 4096)?;
        let models_at = rel(bytes, bp, bp + 4)?;
        for m in 0..models {
            let mo = models_at.checked_add(m * 8).ok_or("VTX model overflow")?;
            take(bytes, mo, 8)?;
            let lods = count(bytes, mo, SOURCE_MAX_LODS)?;
            let lods_at = rel(bytes, mo, mo + 4)?;
            for lod in 0..lods {
                let lo = lods_at.checked_add(lod * 12).ok_or("VTX LOD overflow")?;
                take(bytes, lo, 12)?;
                let nmesh = count(bytes, lo, 16384)?;
                let mesh_at = rel(bytes, lo, lo + 4)?;
                for mesh in 0..nmesh {
                    let me = mesh_at.checked_add(mesh * 9).ok_or("VTX mesh overflow")?;
                    take(bytes, me, 9)?;
                    let ngroups = count(bytes, me, 1024)?;
                    let groups_at = rel(bytes, me, me + 4)?;
                    let mut strip_groups = Vec::with_capacity(ngroups);
                    for g in 0..ngroups {
                        let gr = groups_at.checked_add(g * 25).ok_or("VTX group overflow")?;
                        take(bytes, gr, 25)?;
                        let nv = count(bytes, gr, 65536)?;
                        let vi = rel(bytes, gr, gr + 4)?;
                        take(bytes, vi, nv * 9)?;
                        let ni = count(bytes, gr + 8, 1_000_000)?;
                        let ii = rel(bytes, gr, gr + 12)?;
                        take(bytes, ii, ni * 2)?;
                        let ns = count(bytes, gr + 16, 65536)?;
                        let si = rel(bytes, gr, gr + 20)?;
                        let mut mesh_vertex_ids = Vec::with_capacity(nv);
                        for v in 0..nv {
                            let p = vi + v * 9;
                            mesh_vertex_ids.push(u16::from_le_bytes(
                                take(bytes, p + 4, 2)?.try_into().unwrap(),
                            ));
                        }
                        let mut triangles = Vec::new();
                        for s in 0..ns {
                            let st = si.checked_add(s * 27).ok_or("VTX strip overflow")?;
                            take(bytes, st, 27)?;
                            let n = count(bytes, st, ni)?;
                            let offset = count(bytes, st + 4, ni)?;
                            let flags = bytes[st + 18];
                            if offset.checked_add(n).is_none_or(|end| end > ni) {
                                return Err("VTX strip index range invalid".into());
                            }
                            let mut idx = Vec::with_capacity(n);
                            for k in offset..offset + n {
                                let x = u16::from_le_bytes(
                                    take(bytes, ii + k * 2, 2)?.try_into().unwrap(),
                                );
                                if x as usize >= nv {
                                    return Err("VTX vertex index out of range".into());
                                }
                                idx.push(x);
                            }
                            if flags & 1 != 0 {
                                for t in idx.chunks_exact(3) {
                                    triangles.push([t[0], t[1], t[2]]);
                                }
                            } else if flags & 2 != 0 {
                                for t in 2..idx.len() {
                                    let tri = if t % 2 == 0 {
                                        [idx[t - 2], idx[t - 1], idx[t]]
                                    } else {
                                        [idx[t - 1], idx[t - 2], idx[t]]
                                    };
                                    if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                                        triangles.push(tri);
                                    }
                                }
                            } else {
                                return Err("unsupported VTX strip topology".into());
                            }
                        }
                        strip_groups.push(SourceStripGroup {
                            mesh_vertex_ids,
                            triangles,
                        });
                    }
                    meshes.push(SourceVtxMesh {
                        bodypart: b,
                        model: m,
                        lod,
                        mesh,
                        strip_groups,
                    });
                }
            }
        }
    }
    Ok(SourceVtx {
        checksum,
        lod_count,
        meshes,
    })
}

pub fn validate_source_companions(
    mdl: &[u8],
    vvd: &SourceVvd,
    vtx: &SourceVtx,
) -> Result<(), String> {
    let checksum = int(mdl, 8)?;
    if checksum != vvd.checksum || checksum != vtx.checksum {
        return Err("Source MDL/VVD/VTX checksums differ".into());
    }
    Ok(())
}

/// Read the `$bodygroup` tree with each mesh's material slot and per-LOD vertex
/// counts. Nothing here depends on the chosen LOD or bodygroup.
pub fn parse_source_bodyparts(mdl: &[u8]) -> Result<Vec<SourceBodyPart>, String> {
    if take(mdl, 0, 4)? != b"IDST" || !SUPPORTED_MDL_VERSIONS.contains(&int(mdl, 4)?) {
        return Err(format!(
            "expected Source StudioMDL {SUPPORTED_MDL_VERSIONS:?}"
        ));
    }
    let body_count = count(mdl, 232, 1024)?;
    let body_offset = count(mdl, 236, mdl.len())?;
    let mut bodyparts = Vec::with_capacity(body_count);
    for body in 0..body_count {
        let bp = body_offset
            .checked_add(body * BODYPART_SIZE)
            .ok_or("bodypart overflow")?;
        take(mdl, bp, BODYPART_SIZE)?;
        let name = cstring(mdl, rel(mdl, bp, bp)?)?;
        let model_count = count(mdl, bp + 4, 1024)?;
        let model_start = rel(mdl, bp, bp + 12)?;
        let mut models = Vec::with_capacity(model_count);
        for model_index in 0..model_count {
            let model = model_start
                .checked_add(model_index * MODEL_SIZE)
                .ok_or("model overflow")?;
            take(mdl, model, MODEL_SIZE)?;
            let model_name = cstring(mdl, model)?;
            let mesh_count = count(mdl, model + 72, 16384)?;
            let mesh_start = rel(mdl, model, model + 76)?;
            let mut meshes = Vec::with_capacity(mesh_count);
            for mesh in 0..mesh_count {
                let at = mesh_start
                    .checked_add(mesh * MESH_SIZE)
                    .ok_or("mesh overflow")?;
                take(mdl, at, MESH_SIZE)?;
                let mut lod_vertices = [0usize; SOURCE_MAX_LODS];
                for (lod, slot) in lod_vertices.iter_mut().enumerate() {
                    *slot = count(mdl, at + MESH_LOD_VERTEXES + lod * 4, 2_000_000)?;
                }
                meshes.push(SourceMesh {
                    skin_ref: int(mdl, at)?,
                    lod_vertices,
                });
            }
            models.push(SourceBodyPartModel {
                name: model_name,
                meshes,
            });
        }
        bodyparts.push(SourceBodyPart { name, models });
    }
    if bodyparts.is_empty() {
        return Err("Source model has no bodyparts".into());
    }
    Ok(bodyparts)
}

/// Resolve VTX strip-local indices into `source_lod_vertices(vvd, lod)` for one
/// bodygroup selection.
///
/// `bodygroups[i]` picks the model of bodypart `i` (clamped; missing entries
/// mean choice 0). The vertex base of each model is *derived*, not read: Valve's
/// `Studio_SetRootLOD` rewrites `mstudiomodel_t::vertexindex` and
/// `mstudiomesh_t::vertexoffset` from the per-mesh `numLODVertexes` whenever the
/// root LOD changes, so the stored values are only correct for LOD 0. Every
/// model advances the running base whether or not its bodygroup is selected,
/// because the gathered array holds them all.
pub fn assemble_source_lod(
    mdl: &[u8],
    vvd: &SourceVvd,
    vtx: &SourceVtx,
    lod: usize,
    bodygroups: &[usize],
) -> Result<Vec<SourceDrawMesh>, String> {
    validate_source_companions(mdl, vvd, vtx)?;
    if lod >= vvd.lod_count || lod >= vtx.lod_count {
        return Err(format!(
            "Source LOD {lod} is not present in both VVD ({}) and VTX ({})",
            vvd.lod_count, vtx.lod_count
        ));
    }
    let bodyparts = parse_source_bodyparts(mdl)?;
    let lod_vertex_total = vvd.lod_vertices[lod] as usize;
    let mut draws = Vec::new();
    let mut model_base = 0usize;
    for (body_index, body) in bodyparts.iter().enumerate() {
        let selected = bodygroups
            .get(body_index)
            .copied()
            .unwrap_or(0)
            .min(body.models.len().saturating_sub(1));
        for (model_index, model) in body.models.iter().enumerate() {
            let mut mesh_base = 0usize;
            for (mesh_index, mesh) in model.meshes.iter().enumerate() {
                let mesh_vertices = mesh.lod_vertices[lod];
                if model_index != selected {
                    mesh_base += mesh_vertices;
                    continue;
                }
                let topology = vtx
                    .meshes
                    .iter()
                    .find(|candidate| {
                        candidate.bodypart == body_index
                            && candidate.model == model_index
                            && candidate.mesh == mesh_index
                            && candidate.lod == lod
                    })
                    .ok_or_else(|| {
                        format!(
                            "missing VTX LOD{lod} topology for bodypart {body_index} model {model_index} mesh {mesh_index}"
                        )
                    })?;
                let mut indices = Vec::new();
                for group in &topology.strip_groups {
                    for triangle in &group.triangles {
                        for local in triangle {
                            let mesh_local = *group
                                .mesh_vertex_ids
                                .get(*local as usize)
                                .ok_or("VTX index out of bounds")? as usize;
                            if mesh_local >= mesh_vertices {
                                return Err(format!(
                                    "VTX LOD{lod} vertex {mesh_local} exceeds the {mesh_vertices} vertices MDL declares for bodypart {body_index} mesh {mesh_index}"
                                ));
                            }
                            let index = model_base + mesh_base + mesh_local;
                            if index >= lod_vertex_total {
                                return Err(format!(
                                    "Source LOD{lod} vertex {index} outside the {lod_vertex_total}-vertex gathered pool"
                                ));
                            }
                            indices
                                .push(u32::try_from(index).map_err(|_| "vertex index overflow")?);
                        }
                    }
                }
                mesh_base += mesh_vertices;
                draws.push(SourceDrawMesh {
                    bodypart: body_index,
                    model: model_index,
                    mesh: mesh_index,
                    skin_ref: mesh.skin_ref,
                    indices,
                });
            }
            model_base += mesh_base;
        }
    }
    // A mismatch here means the derived offsets disagree with the VVD, which is
    // the failure mode that silently scrambles geometry. Refuse instead.
    if model_base != lod_vertex_total {
        return Err(format!(
            "Source LOD{lod} mesh vertex counts total {model_base}, VVD header declares {lod_vertex_total}"
        ));
    }
    if draws.is_empty() {
        return Err("Source model contains no drawable meshes".into());
    }
    Ok(draws)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32) -> SourceVertex {
        SourceVertex {
            position: [x, 0., 0.],
            normal: [0., 0., 1.],
            uv: [0., 0.],
            weights: [1., 0., 0.],
            bones: [0, 0, 0],
            bone_count: 1,
        }
    }

    #[test]
    fn rejects_bad_geometry() {
        assert!(parse_source_vvd(b"not a vvd").is_err());
        assert!(parse_source_vtx(&[0u8; 64]).is_err());
    }

    /// The whole point of the fixup table: for root LOD 0 it keeps every record
    /// yet still reorders the raw table, so a loader that skips it reads a
    /// scrambled vertex for every index.
    #[test]
    fn gather_reorders_lod0_when_runs_are_interleaved() {
        // Mirrors real studiomdl output: the coarse run is listed first but
        // sits last in the raw table.
        let vvd = SourceVvd {
            checksum: 0,
            lod_count: 2,
            lod_vertices: [4, 1, 0, 0, 0, 0, 0, 0],
            fixups: vec![
                SourceVertexFixup {
                    lod: 1,
                    source_vertex: 3,
                    count: 1,
                },
                SourceVertexFixup {
                    lod: 0,
                    source_vertex: 0,
                    count: 3,
                },
            ],
            vertices: vec![vertex(0.), vertex(1.), vertex(2.), vertex(3.)],
        };
        assert_eq!(
            source_lod_vertices(&vvd, 0)
                .unwrap()
                .iter()
                .map(|v| v.position[0])
                .collect::<Vec<_>>(),
            vec![3., 0., 1., 2.],
            "LOD0 gather must follow fixup order, not raw table order"
        );
        // A coarser root LOD drops every run below it.
        assert_eq!(
            source_lod_vertices(&vvd, 1)
                .unwrap()
                .iter()
                .map(|v| v.position[0])
                .collect::<Vec<_>>(),
            vec![3.]
        );
    }

    #[test]
    fn gather_without_fixups_truncates_to_the_lod() {
        let vvd = SourceVvd {
            checksum: 0,
            lod_count: 2,
            lod_vertices: [3, 2, 0, 0, 0, 0, 0, 0],
            fixups: Vec::new(),
            vertices: vec![vertex(0.), vertex(1.), vertex(2.)],
        };
        assert_eq!(source_lod_vertices(&vvd, 0).unwrap().len(), 3);
        assert_eq!(source_lod_vertices(&vvd, 1).unwrap().len(), 2);
    }

    #[test]
    fn rejects_lod_beyond_the_model() {
        let vvd = SourceVvd {
            checksum: 0,
            lod_count: 1,
            lod_vertices: [1, 0, 0, 0, 0, 0, 0, 0],
            fixups: Vec::new(),
            vertices: vec![vertex(0.)],
        };
        assert!(source_lod_vertices(&vvd, 1).is_err());
    }

    #[test]
    fn gather_total_must_match_the_header() {
        let vvd = SourceVvd {
            checksum: 0,
            lod_count: 1,
            lod_vertices: [4, 0, 0, 0, 0, 0, 0, 0],
            fixups: vec![SourceVertexFixup {
                lod: 0,
                source_vertex: 0,
                count: 2,
            }],
            vertices: vec![vertex(0.), vertex(1.), vertex(2.), vertex(3.)],
        };
        assert!(source_lod_vertices(&vvd, 0)
            .unwrap_err()
            .contains("header declares"));
    }
}
