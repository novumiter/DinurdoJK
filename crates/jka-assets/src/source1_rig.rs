//! Rebind a Source 1 model onto Jedi Academy's `_humanoid.gla` skeleton.
//!
//! The result is an ordinary [`GlmModel`] whose vertex weights address GLA bone
//! indices, so every existing Ghoul2 consumer - animation, torso/legs blending,
//! bolts, saber and weapon attachment, ragdoll, gore, shadows, ray tracing -
//! works unchanged. Nothing downstream needs to know the geometry came from a
//! `.mdl`.
//!
//! The baked mesh must end up in *base-pose space*, which is the space a stock
//! GLM's vertices already live in: a Ghoul2 pose matrix is
//! `world_animated * inverse(basePose)`, so skinning reduces to the shared
//! Ghoul2 root matrix whenever the animation sits at its reference
//! configuration, and every animated frame is a rigid motion away from that.
//! Keeping the Source bind pose unchanged would reference every animation delta
//! against the wrong rest configuration - `_humanoid`'s base pose has notably
//! splayed legs, so a straight-legged Source bind pose would walk with its knees
//! crossed.
//!
//! What that does **not** require is for the geometry to adopt the GLA bone's
//! *axes*. The obvious-looking re-rig
//!
//! ```text
//! p_bake = sum_b w_b * ( GLA_basePose[map(b)] * scale * inverse(SRC_rest[b]) * p_src )
//! ```
//!
//! is wrong for exactly that reason, and wrong by a lot. It re-interprets
//! Source-bone-local coordinates as GLA-bone-local ones, so it rotates every
//! vertex about its joint by the difference between the two rest *conventions*.
//! ValveBiped puts X down the bone; `_humanoid` does not, and measured on
//! HEV MK2 that difference is **100 to 145 degrees per bone** while the real
//! joint-position disagreement is only about 1 to 2 units. On a 63-unit-tall
//! model it moved the median vertex 6.7 units and the worst 14.7. Because
//! `world_animated * inverse(basePose)` cancels the convention at runtime, none
//! of that rotation buys anything.
//!
//! So the rebind keeps the authored Source orientation and corrects only what
//! actually matters - where the joint is, and which way the bone points:
//!
//! ```text
//! M_j = T(GLA_joint[j]) * R_align * T(-fit(SRC_joint[owner(j)])) * fit
//! ```
//!
//! `fit` is the fitted global similarity (yaw, uniform scale, translation);
//! `T(-fit(joint))` then `T(GLA_joint)` move the joint onto the GLA joint so
//! animation pivots about the right point; and `R_align` is the *minimal*
//! rotation taking the bone's rest direction onto the mapped GLA bone's rest
//! direction, which is convention-free because both directions are derived from
//! joint positions.
//!
//! Note the matrix is indexed by *`_humanoid` joint*, not by Source bone. Many
//! Source bones share one joint - the toe on the ankle, a five-bone spine on
//! three lumbar bones, two fingers on one digit, every helper on its nearest
//! mapped ancestor - and they all animate as that one GLA bone, so they must
//! share its matrix too. Giving each bone its own would substitute its own rest
//! joint for `SRC_joint` and seat *that* on the shared GLA joint, translating the
//! parts onto each other: the front of the foot collapses into the ankle, the
//! ears into the skull, a tunic skirt into the pelvis. See [`joint_owners`].
//!
//! See [`SourceRebindMode`] for the two comparison modes, and
//! `tools/source1/rebind_probe.py` to measure any of them against a real `.mdl`
//! without building the client.
use crate::ghoul2::{
    GlaAnimation, GlmLod, GlmModel, GlmSurface, GlmSurfaceHierarchy, GlmVertex, GlmWeight,
    Matrix3x4,
};
use crate::source1::{SourceMaterialTable, SourceSkeleton};
use crate::source1_geometry::{
    assemble_source_lod, parse_source_bodyparts, source_lod_vertices, SourceVtx, SourceVvd,
};
use std::collections::BTreeMap;

/// Ghoul2 packs at most four weights per vertex.
const MAX_WEIGHTS: usize = 4;

/// How a Source bone's geometry is re-placed onto its mapped GLA bone. All three
/// put the mesh in base-pose space; they differ in how much of the rest pose they
/// correct, and therefore in how much they reshape the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceRebindMode {
    /// Move the joint and align the bone direction. The default: animation pivots
    /// about the right point *and* each limb starts out pointing the way the GLA
    /// bone it is driven by points, so rest-relative animation deltas land
    /// correctly. Measured on HEV MK2: median vertex moves 2.5 units.
    #[default]
    Direction,
    /// Move the joint only. Preserves the authored Source shape most closely
    /// (median 1.3 units on HEV MK2) but leaves limbs pointing along the Source
    /// rest pose, so where the two rest poses genuinely disagree - `_humanoid`
    /// splays the legs, most Source bind poses do not - animation is applied to a
    /// limb that starts in the wrong direction.
    Translation,
    /// Adopt the GLA bone's full rest frame. Applies the two skeletons'
    /// axis-convention difference as a rotation and badly warps the mesh (median
    /// 6.7 units, max 14.7 on HEV MK2). Retained only so the regression can be
    /// reproduced and measured; never use it for a real character.
    BoneFrame,
}

impl SourceRebindMode {
    /// `source_model.json` `"rebind"` value.
    pub fn from_descriptor(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "direction" => Self::Direction,
            "translation" => Self::Translation,
            "bone_frame" | "boneframe" => Self::BoneFrame,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SourceRigOptions {
    /// `$bodygroup` choice per bodypart; missing entries select choice 0.
    pub bodygroups: Vec<usize>,
    /// `$texturegroup` row used to resolve each mesh's skin reference.
    pub skin_family: usize,
    /// Upper bound on how many Source LODs become GLM LODs.
    pub max_lods: usize,
    /// Uniform Source-to-JKA scale. `None` fits it from the two skeletons.
    pub scale: Option<f32>,
    pub rebind: SourceRebindMode,
}

impl Default for SourceRigOptions {
    fn default() -> Self {
        Self {
            bodygroups: Vec::new(),
            skin_family: 0,
            max_lods: crate::source1_geometry::SOURCE_MAX_LODS,
            scale: None,
            rebind: SourceRebindMode::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SourceRigSurface {
    pub name: String,
    /// Candidate `materials/....vmt` qpaths, in `$cdmaterials` search order.
    pub vmt_candidates: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SourceLodStats {
    pub lod: usize,
    pub surfaces: usize,
    pub vertices: usize,
    pub triangles: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SourceRigDiagnostics {
    pub model_name: String,
    pub source_bones: usize,
    /// Source bones matched to a GLA bone by name or by the spine resolver.
    pub mapped_bones: usize,
    /// Source bones with no rule of their own, following a mapped ancestor.
    pub inherited_bones: Vec<(String, String)>,
    /// Uniform scale actually used, and whether it was fitted or configured.
    pub scale: f32,
    pub scale_was_fitted: bool,
    /// Residual of the skeleton alignment fit: how differently the two
    /// skeletons are proportioned, in JKA units.
    pub fit_pairs: usize,
    pub fit_yaw_degrees: f32,
    pub fit_rms: f32,
    pub fit_worst: Option<(String, String, f32)>,
    /// Largest disagreement between StudioMDL's authored `poseToBone` and the
    /// inverse derived from the bind chain. Large values mean the two disagree
    /// about the bind pose itself.
    pub max_inverse_bind_error: f32,
    /// Which rebind construction was used.
    pub rebind: SourceRebindMode,
    /// `_humanoid` joints whose rest direction was corrected onto the Source
    /// bone that owns them. The rest are leaves, or joints whose owner's children
    /// all share its GLA bone. Counted per joint, not per bone, because a joint
    /// gets one matrix however many Source bones land on it.
    pub direction_corrected: usize,
    /// Vertices whose merged GLA influence count exceeded Ghoul2's four. Source
    /// stores at most three influences per vertex and merging can only reduce
    /// that, so this must be 0; a nonzero value means the merge is wrong.
    pub weight_truncations: usize,
    /// Baked rest-pose bounds in JKA model units.
    pub rest_bounds: ([f32; 3], [f32; 3]),
    /// Largest distance any vertex moved relative to merely scaling and aligning
    /// the Source mesh, i.e. how much adopting the GLA joint centres reshaped the
    /// model. A few units is the expected cost of the rebind; tens of units means
    /// the bone correspondence is wrong.
    pub max_rebind_displacement: f32,
    pub lods: Vec<SourceLodStats>,
}

pub struct SourceRig {
    pub glm: GlmModel,
    /// Indexed by GLM surface index.
    pub surfaces: Vec<SourceRigSurface>,
    pub diagnostics: SourceRigDiagnostics,
}

fn mul(a: &Matrix3x4, b: &Matrix3x4) -> Matrix3x4 {
    let mut out = [[0.0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
        out[r][3] = a[r][3] + (0..3).map(|k| a[r][k] * b[k][3]).sum::<f32>();
    }
    out
}

/// Inverse of a rigid (rotation + translation) 3x4 matrix.
fn rigid_inverse(m: &Matrix3x4) -> Matrix3x4 {
    let mut out = [[0.0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = m[c][r];
        }
        out[r][3] = -(0..3).map(|k| out[r][k] * m[k][3]).sum::<f32>();
    }
    out
}

fn transform(m: &Matrix3x4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

fn rotate(m: &Matrix3x4, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn origin(m: &Matrix3x4) -> [f32; 3] {
    [m[0][3], m[1][3], m[2][3]]
}

fn identity() -> Matrix3x4 {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
    ]
}

fn translation(t: [f32; 3]) -> Matrix3x4 {
    [
        [1., 0., 0., t[0]],
        [0., 1., 0., t[1]],
        [0., 0., 1., t[2]],
    ]
}

fn normalized(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    (length.is_finite() && length > 1e-6).then(|| v.map(|c| c / length))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The minimal rotation taking unit vector `from` onto unit vector `to`, as a
/// linear 3x4 with no translation (Rodrigues' formula). Convention-free: it
/// introduces no roll about the axis, so it corrects a bone's direction without
/// imposing either skeleton's idea of which way is "up" along the bone.
fn minimal_rotation(from: [f32; 3], to: [f32; 3]) -> Matrix3x4 {
    let (Some(a), Some(b)) = (normalized(from), normalized(to)) else {
        return identity();
    };
    let dot = (0..3).map(|k| a[k] * b[k]).sum::<f32>().clamp(-1.0, 1.0);
    if dot > 1.0 - 1e-7 {
        return identity();
    }
    let axis = if dot < -1.0 + 1e-7 {
        // Antiparallel: any axis perpendicular to `a` gives a 180 degree turn.
        let helper = if a[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        match normalized(cross(a, helper)) {
            Some(axis) => axis,
            None => return identity(),
        }
    } else {
        match normalized(cross(a, b)) {
            Some(axis) => axis,
            None => return identity(),
        }
    };
    let (sin, cos) = dot.acos().sin_cos();
    let k = [
        [0., -axis[2], axis[1]],
        [axis[2], 0., -axis[0]],
        [-axis[1], axis[0], 0.],
    ];
    let mut out = identity();
    for r in 0..3 {
        for c in 0..3 {
            let k2 = (0..3).map(|m| k[r][m] * k[m][c]).sum::<f32>();
            out[r][c] = (if r == c { 1.0 } else { 0.0 }) + sin * k[r][c] + (1.0 - cos) * k2;
        }
    }
    out
}

fn quaternion_matrix(q: [f32; 4], position: [f32; 3]) -> Result<Matrix3x4, String> {
    let [x, y, z, w] = q;
    let norm = x * x + y * y + z * z + w * w;
    if !norm.is_finite() || norm < 1e-12 || !position.iter().all(|v| v.is_finite()) {
        return Err("Source bind quaternion or position is invalid".into());
    }
    let s = 2.0 / norm;
    Ok([
        [
            1. - s * (y * y + z * z),
            s * (x * y - z * w),
            s * (x * z + y * w),
            position[0],
        ],
        [
            s * (x * y + z * w),
            1. - s * (x * x + z * z),
            s * (y * z - x * w),
            position[1],
        ],
        [
            s * (x * z - y * w),
            s * (y * z + x * w),
            1. - s * (x * x + y * y),
            position[2],
        ],
    ])
}

/// The rotation (as a unit quaternion, `[x, y, z, w]`) a rigid 3x4 matrix
/// applies, by the numerically stable largest-diagonal-term method (Shepperd).
fn matrix_to_quaternion(m: &Matrix3x4) -> [f32; 4] {
    let (m00, m01, m02) = (m[0][0], m[0][1], m[0][2]);
    let (m10, m11, m12) = (m[1][0], m[1][1], m[1][2]);
    let (m20, m21, m22) = (m[2][0], m[2][1], m[2][2]);
    let trace = m00 + m11 + m22;
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).max(0.0).sqrt() * 2.0;
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).max(0.0).sqrt() * 2.0;
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).max(0.0).sqrt() * 2.0;
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    }
}

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// A rigid transform as a dual quaternion: `real` is its rotation, `dual`
/// encodes its translation (`dual = 0.5 * [t, 0] * real`). Blending these
/// instead of blending transformed vertex positions directly is the standard
/// fix for the "candy wrapper" bulge/pinch linear blend skinning produces
/// when two blended bones differ by a large rotation - exactly what the
/// rebind introduces by correcting a Source T-pose onto `_humanoid`'s A-pose,
/// independently per bone. See [`blend_dual_quats`].
fn dual_quat_from_matrix(m: &Matrix3x4) -> ([f32; 4], [f32; 4]) {
    let real = matrix_to_quaternion(m);
    let t = [m[0][3], m[1][3], m[2][3], 0.0];
    let dual = quat_mul(t, real).map(|c| c * 0.5);
    (real, dual)
}

/// Weighted blend of dual quaternions representing several bones' rebind
/// matrices, for one vertex. Each part is `(weight, real, dual)`.
///
/// Quaternions `q` and `-q` represent the same rotation, but average to zero
/// if blended as-is; every part is first flipped into the same hemisphere as
/// the first (by construction, its highest-weighted bone is pushed first by
/// the caller) so the blend takes the shortest path between them, same as the
/// `minimal_rotation` correction elsewhere in this module.
fn blend_dual_quats(parts: &[(f32, [f32; 4], [f32; 4])]) -> ([f32; 4], [f32; 4]) {
    let reference = parts[0].1;
    let mut real = [0.0f32; 4];
    let mut dual = [0.0f32; 4];
    for &(weight, part_real, part_dual) in parts {
        let dot: f32 = (0..4).map(|k| part_real[k] * reference[k]).sum();
        let sign = if dot < 0.0 { -weight } else { weight };
        for k in 0..4 {
            real[k] += sign * part_real[k];
            dual[k] += sign * part_dual[k];
        }
    }
    let len = (0..4).map(|k| real[k] * real[k]).sum::<f32>().sqrt().max(1e-9);
    (real.map(|c| c / len), dual.map(|c| c / len))
}

/// The equivalent single rigid matrix for a blended dual quaternion.
fn dual_quat_to_matrix(real: [f32; 4], dual: [f32; 4]) -> Matrix3x4 {
    let t = quat_mul(dual, quat_conjugate(real)).map(|c| c * 2.0);
    quaternion_matrix(real, [t[0], t[1], t[2]]).unwrap_or_else(|_| identity())
}

/// Source bone rest transforms in Source model space, composed down the chain.
fn source_rest_world(skeleton: &SourceSkeleton) -> Result<Vec<Matrix3x4>, String> {
    let mut world = Vec::with_capacity(skeleton.bones.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let local = quaternion_matrix(bone.bind_quaternion, bone.bind_position)?;
        let absolute = match bone.parent {
            Some(parent) if parent < i => mul(&world[parent], &local),
            Some(parent) => {
                return Err(format!(
                    "Source bone {i} has non-ancestral parent {parent}"
                ))
            }
            None => local,
        };
        world.push(absolute);
    }
    Ok(world)
}

/// Lower-case alphanumeric tail of a ValveBiped bone name:
/// `ValveBiped.Bip01_L_UpperArm` becomes `lupperarm`.
fn normalize_source_bone(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let tail = lower
        .strip_prefix("valvebiped.")
        .unwrap_or(&lower)
        .to_owned();
    let tail = tail.strip_prefix("bip01_").unwrap_or(&tail);
    tail.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Reusable ValveBiped to `_humanoid` correspondence. Model-independent by
/// construction: it is keyed on the standard ValveBiped names, never on any one
/// character's bone table.
///
/// `_humanoid` splits two joints that ValveBiped keeps whole. `lfemurX` and
/// `lhumerusX` are twist helpers hanging off `lfemurYZ`/`lhumerus` as *siblings*
/// of the next real joint, so the load-bearing chain is
/// `pelvis -> lfemurYZ -> ltibia -> ltalus` and
/// `thoracic -> lhumerus -> lradius -> lhand`. Those are the bones a limb must
/// follow; the `X` helpers only add skin twist and are deliberately not targets.
fn humanoid_bone_for(normalized: &str) -> Option<&'static str> {
    Some(match normalized {
        "pelvis" => "pelvis",
        // Spine* is resolved positionally; see `resolve_spine_chain`.
        "neck" | "neck1" | "neck2" => "cervical",
        "head" | "head1" | "forward" => "cranium",
        "lclavicle" => "lclavical",
        "rclavicle" => "rclavical",
        "lupperarm" => "lhumerus",
        "rupperarm" => "rhumerus",
        "lforearm" => "lradius",
        "rforearm" => "rradius",
        "lhand" | "animattachmentlh" => "lhand",
        "rhand" | "animattachmentrh" => "rhand",
        "lthigh" => "lfemurYZ",
        "rthigh" => "rfemurYZ",
        "lcalf" => "ltibia",
        "rcalf" => "rtibia",
        // `_humanoid` has no toe bone; the ankle carries the whole foot.
        "lfoot" | "ltoe0" | "ltoe" => "ltalus",
        "rfoot" | "rtoe0" | "rtoe" => "rtalus",
        // ValveBiped numbers digits thumb..pinky. `_humanoid` only carries a
        // thumb (d1), an index (d2) and an outer finger (d4), so middle folds
        // onto the index and ring onto the outer digit.
        "lfinger0" => "l_d1_j1",
        "lfinger01" | "lfinger02" => "l_d1_j2",
        "lfinger1" | "lfinger2" => "l_d2_j1",
        "lfinger11" | "lfinger12" | "lfinger21" | "lfinger22" => "l_d2_j2",
        "lfinger3" | "lfinger4" => "l_d4_j1",
        "lfinger31" | "lfinger32" | "lfinger41" | "lfinger42" => "l_d4_j2",
        "rfinger0" => "r_d1_j1",
        "rfinger01" | "rfinger02" => "r_d1_j2",
        "rfinger1" | "rfinger2" => "r_d2_j1",
        "rfinger11" | "rfinger12" | "rfinger21" | "rfinger22" => "r_d2_j2",
        "rfinger3" | "rfinger4" => "r_d4_j1",
        "rfinger31" | "rfinger32" | "rfinger41" | "rfinger42" => "r_d4_j2",
        _ => return None,
    })
}

/// `_humanoid` spine bones between the pelvis and the neck, root first.
const HUMANOID_SPINE: [&str; 3] = ["lower_lumbar", "upper_lumbar", "thoracic"];

/// The anchors used to fit the Source-to-JKA alignment. Deliberately excludes
/// the spine (whose subdivision differs per model and is resolved from the fit)
/// and every leaf helper, attachment and digit, which would bias it.
const FIT_ANCHORS: [&str; 17] = [
    "pelvis",
    "neck1",
    "head1",
    "lclavicle",
    "rclavicle",
    "lupperarm",
    "rupperarm",
    "lforearm",
    "rforearm",
    "lhand",
    "rhand",
    "lthigh",
    "rthigh",
    "lcalf",
    "rcalf",
    "lfoot",
    "rfoot",
];

fn gla_bone_index(gla: &GlaAnimation, name: &str) -> Option<usize> {
    gla.skeleton
        .iter()
        .position(|bone| bone.name.eq_ignore_ascii_case(name))
}

/// Best-fit similarity (yaw about Z, uniform scale, translation) taking Source
/// anchor positions onto their `_humanoid` counterparts. Only the scale is used
/// by the rebind - the rebind places every bone through its GLA base pose, so it
/// is orientation-agnostic by construction - but the fitted yaw and residual are
/// the honest measure of whether the bone correspondence is sane at all.
#[derive(Debug)]
struct SkeletonFit {
    scale: f32,
    yaw_degrees: f32,
    rms: f32,
    pairs: usize,
    worst: Option<(String, String, f32)>,
    /// The fitted similarity itself, so callers do not re-derive it.
    transform: Matrix3x4,
    /// Anchor centroids the similarity was built around, so it can be rebuilt at a
    /// different scale without sliding the model off the skeleton.
    source_centroid: [f32; 3],
    target_centroid: [f32; 3],
}

impl SkeletonFit {
    /// Source model space to JKA model space under the fitted similarity.
    fn align(&self, p: [f32; 3]) -> [f32; 3] {
        transform(&self.transform, p)
    }
}

fn fit_skeletons(
    anchors: &[(String, String, [f32; 3], [f32; 3])],
) -> Result<SkeletonFit, String> {
    let n = anchors.len();
    if n < 4 {
        return Err(format!(
            "only {n} ValveBiped anchor bones matched `_humanoid`; this does not look like a humanoid Source model"
        ));
    }
    let mean = |pick: fn(&(String, String, [f32; 3], [f32; 3])) -> [f32; 3]| {
        let mut c = [0.0f32; 3];
        for a in anchors {
            let p = pick(a);
            for k in 0..3 {
                c[k] += p[k];
            }
        }
        c.map(|v| v / n as f32)
    };
    let cs = mean(|a| a.2);
    let cj = mean(|a| a.3);
    let mut cross = 0.0f32;
    let mut dot_xy = 0.0f32;
    let mut src_norm = 0.0f32;
    for (_, _, s, j) in anchors {
        let (sx, sy) = (s[0] - cs[0], s[1] - cs[1]);
        let (jx, jy) = (j[0] - cj[0], j[1] - cj[1]);
        cross += sx * jy - sy * jx;
        dot_xy += sx * jx + sy * jy;
        src_norm += (0..3).map(|k| (s[k] - cs[k]).powi(2)).sum::<f32>();
    }
    if src_norm <= f32::EPSILON {
        return Err("Source anchor bones are coincident".into());
    }
    let yaw = cross.atan2(dot_xy);
    let (sin, cos) = yaw.sin_cos();
    let mut projection = 0.0f32;
    for (_, _, s, j) in anchors {
        let (sx, sy, sz) = (s[0] - cs[0], s[1] - cs[1], s[2] - cs[2]);
        let (jx, jy, jz) = (j[0] - cj[0], j[1] - cj[1], j[2] - cj[2]);
        let rotated = [cos * sx - sin * sy, sin * sx + cos * sy, sz];
        projection += rotated[0] * jx + rotated[1] * jy + rotated[2] * jz;
    }
    let scale = projection / src_norm;
    if !scale.is_finite() || scale <= 1e-4 {
        return Err(format!("Source-to-JKA skeleton fit produced a degenerate scale {scale}"));
    }
    let linear = [
        [cos * scale, -sin * scale, 0.0],
        [sin * scale, cos * scale, 0.0],
        [0.0, 0.0, scale],
    ];
    let mut fitted = [[0.0f32; 4]; 3];
    for r in 0..3 {
        fitted[r][..3].copy_from_slice(&linear[r]);
        fitted[r][3] = cj[r] - (0..3).map(|k| linear[r][k] * cs[k]).sum::<f32>();
    }
    let mut total = 0.0f32;
    let mut worst: Option<(String, String, f32)> = None;
    for (src_name, gla_name, s, j) in anchors {
        let mapped = transform(&fitted, *s);
        let error = (0..3)
            .map(|k| (mapped[k] - j[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        total += error * error;
        if worst.as_ref().is_none_or(|(_, _, e)| error > *e) {
            worst = Some((src_name.clone(), gla_name.clone(), error));
        }
    }
    Ok(SkeletonFit {
        scale,
        yaw_degrees: yaw.to_degrees(),
        rms: (total / n as f32).sqrt(),
        pairs: n,
        worst,
        transform: fitted,
        source_centroid: cs,
        target_centroid: cj,
    })
}

/// Assign each Source `Spine*` bone to the nearest `_humanoid` spine bone once
/// the skeletons are aligned. ValveBiped models carry anywhere from one to five
/// spine bones against `_humanoid`'s three, so a fixed table would squash or
/// stretch the torso of every model whose subdivision differs.
fn resolve_spine_chain(
    gla: &GlaAnimation,
    spine_candidates: &[(usize, [f32; 3])],
    aligned: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<(usize, usize)> {
    let targets: Vec<(usize, [f32; 3])> = HUMANOID_SPINE
        .iter()
        .filter_map(|name| {
            let index = gla_bone_index(gla, name)?;
            Some((index, origin(&gla.skeleton[index].base_pose)))
        })
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    spine_candidates
        .iter()
        .map(|(bone, position)| {
            let p = aligned(*position);
            let best = targets
                .iter()
                .min_by(|a, b| {
                    let d = |t: &(usize, [f32; 3])| {
                        (0..3).map(|k| (t.1[k] - p[k]).powi(2)).sum::<f32>()
                    };
                    d(a).total_cmp(&d(b))
                })
                .expect("targets is non-empty");
            (*bone, best.0)
        })
        .collect()
}

/// Per-Source-bone rebind state.
struct BoneRebind {
    /// GLA bone this Source bone drives.
    gla_bone: usize,
    /// Takes a Source model-space point into base-pose space. See the module
    /// documentation for the construction and why it is not `basePose * rest^-1`.
    matrix: Matrix3x4,
}

/// The Source child that continues a limb: the child with the largest rest offset
/// whose mapped GLA bone differs from this bone's, so the direction it defines has
/// a real counterpart on the GLA side. Returns the pair of rest directions, the
/// Source one already carried into JKA space.
fn bone_direction_pair(
    bone: usize,
    skeleton: &SourceSkeleton,
    rest_world: &[Matrix3x4],
    gla: &GlaAnimation,
    gla_for_bone: &[Option<usize>],
    fit: &SkeletonFit,
) -> Option<([f32; 3], [f32; 3])> {
    let gla_bone = gla_for_bone[bone]?;
    let here = origin(&rest_world[bone]);
    let child = skeleton
        .bones
        .iter()
        .enumerate()
        .filter(|(index, candidate)| {
            candidate.parent == Some(bone)
                && gla_for_bone[*index].is_some_and(|mapped| mapped != gla_bone)
        })
        .max_by(|(a, _), (b, _)| {
            let span = |index: usize| {
                let there = origin(&rest_world[index]);
                (0..3).map(|k| (there[k] - here[k]).powi(2)).sum::<f32>()
            };
            span(*a).total_cmp(&span(*b))
        })
        .map(|(index, _)| index)?;
    let gla_child = gla_for_bone[child]?;
    let source_offset: [f32; 3] =
        std::array::from_fn(|k| origin(&rest_world[child])[k] - here[k]);
    let gla_here = origin(&gla.skeleton[gla_bone].base_pose);
    let gla_offset: [f32; 3] =
        std::array::from_fn(|k| origin(&gla.skeleton[gla_child].base_pose)[k] - gla_here[k]);
    // The fit's linear part carries the Source direction into JKA space; its
    // uniform scale drops out when the direction is normalized.
    Some((rotate(&fit.transform, source_offset), gla_offset))
}

/// `_humanoid` splits several chain bones from a sibling twist helper
/// (`lhumerusX`/`rhumerusX`/`lradiusX`/`rradiusX`/`lfemurX`/`rfemurX`) that only
/// adds skin twist. No ValveBiped Source model carries a bone for that helper -
/// every Source file checked weights that skin to the chain bone instead - so
/// without this, every vertex along (say) the forearm is rigidly driven by the
/// chain bone's own rotation instead of easing into the twist the way a real
/// arm skins. Compared vertex-for-vertex in the shared `_humanoid` base-pose
/// space against a hand-ported reference model
/// (`zzzJulietStarling-S.pk3`/`jedi_jstarling`), the hand rig puts roughly 80%
/// of the sleeve's weight mass on the twist helper rather than the chain bone;
/// `twist_blend_fraction` approximates that by blending each chain-bone vertex
/// toward its twist helper by how far along the limb segment - the owning
/// joint to the next joint down the chain - it sits.
///
/// Keyed by chain-bone GLA index: `(twist helper GLA index, chain joint
/// origin, next joint origin)`, both origins in base-pose space (the space the
/// baked mesh already lives in), so this needs building only once per rig, not
/// per vertex.
fn twist_blend_table(gla: &GlaAnimation) -> BTreeMap<usize, (usize, [f32; 3], [f32; 3])> {
    const SIBLINGS: [(&str, &str, &str); 6] = [
        ("lhumerus", "lhumerusX", "lradius"),
        ("rhumerus", "rhumerusX", "rradius"),
        ("lradius", "lradiusX", "lhand"),
        ("rradius", "rradiusX", "rhand"),
        ("lfemurYZ", "lfemurX", "ltibia"),
        ("rfemurYZ", "rfemurX", "rtibia"),
    ];
    let mut table = BTreeMap::new();
    for (chain, twist, next) in SIBLINGS {
        let (Some(chain_index), Some(twist_index), Some(next_index)) = (
            gla_bone_index(gla, chain),
            gla_bone_index(gla, twist),
            gla_bone_index(gla, next),
        ) else {
            continue;
        };
        table.insert(
            chain_index,
            (
                twist_index,
                origin(&gla.skeleton[chain_index].base_pose),
                origin(&gla.skeleton[next_index].base_pose),
            ),
        );
    }
    table
}

/// How far `position` sits from `joint` towards `next_joint`, as a 0..1
/// fraction of that segment's length (clamped, so a vertex behind the joint or
/// past the next one saturates rather than extrapolating).
fn limb_fraction(position: [f32; 3], joint: [f32; 3], next_joint: [f32; 3]) -> f32 {
    let dir: [f32; 3] = std::array::from_fn(|k| next_joint[k] - joint[k]);
    let len2 = dir.iter().map(|c| c * c).sum::<f32>();
    if len2 <= 1e-6 {
        return 0.0;
    }
    let rel: [f32; 3] = std::array::from_fn(|k| position[k] - joint[k]);
    let dot: f32 = (0..3).map(|k| rel[k] * dir[k]).sum();
    (dot / len2).clamp(0.0, 1.0)
}

/// Smoothstep ramp from the owning joint (`t=0`, no twist yet) to the next
/// joint down the limb (`t=1`, twist helper carries most of the skin): see
/// [`twist_blend_table`] for where the 80% ceiling comes from.
fn twist_blend_fraction(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    0.85 * s
}

/// The Source bone each `_humanoid` joint takes its rebind matrix from.
///
/// Several Source bones routinely map to one joint - ValveBiped's toe onto the
/// ankle, its spine onto three lumbar bones, two fingers onto one digit, and
/// every helper without a rule of its own onto its nearest mapped ancestor. They
/// all animate as that one GLA bone, so they must also share one rebind matrix;
/// building a separate one per bone would seat each bone's own rest joint on the
/// shared GLA joint and collapse the parts onto each other.
///
/// The owner is the lowest-indexed bone on the joint. That is the shallowest one,
/// because [`source_rest_world`] rejects any bone table that does not store
/// parents before children.
fn joint_owners(gla_for_bone: &[Option<usize>]) -> BTreeMap<usize, usize> {
    let mut owners = BTreeMap::<usize, usize>::new();
    for (index, slot) in gla_for_bone.iter().enumerate() {
        if let Some(gla_bone) = slot {
            owners.entry(*gla_bone).or_insert(index);
        }
    }
    owners
}

/// One Source bone's model-space-to-base-pose-space transform.
///
/// `placement` is the global similarity (yaw, uniform scale, translation) that
/// carries Source model space into JKA model space. `align` is the rotation
/// correction to apply (see [`minimal_rotation`]), or `None` to keep the
/// authored orientation. See the module documentation for why
/// [`SourceRebindMode::BoneFrame`] is wrong, and [`resolve_align`] for why a
/// joint with no direction of its own still usually gets a non-`None` align.
fn rebind_matrix(
    mode: SourceRebindMode,
    placement: &Matrix3x4,
    scale: f32,
    source_rest: &Matrix3x4,
    gla_base: &Matrix3x4,
    align: Option<Matrix3x4>,
) -> Matrix3x4 {
    if mode == SourceRebindMode::BoneFrame {
        let uniform = [
            [scale, 0., 0., 0.],
            [0., scale, 0., 0.],
            [0., 0., scale, 0.],
        ];
        return mul(gla_base, &mul(&uniform, &rigid_inverse(source_rest)));
    }
    let align = align.unwrap_or_else(identity);
    // Carry the mesh into JKA space, bring this bone's joint to the origin, apply
    // the direction correction, then seat the joint on the GLA joint so animation
    // pivots about the right point.
    let joint = transform(placement, origin(source_rest));
    let centred = mul(&translation(joint.map(|v| -v)), placement);
    mul(&translation(origin(gla_base)), &mul(&align, &centred))
}

#[allow(clippy::too_many_arguments)]
pub fn build_source_rig(
    model_qpath: &str,
    mdl: &[u8],
    vvd: &SourceVvd,
    vtx: &SourceVtx,
    skeleton: &SourceSkeleton,
    materials: &SourceMaterialTable,
    gla: &GlaAnimation,
    options: &SourceRigOptions,
) -> Result<SourceRig, String> {
    let rest_world = source_rest_world(skeleton)?;

    // StudioMDL stores an independent inverse bind. Comparing it against the one
    // derived from the bind chain catches a misread bone table before it can
    // quietly deform the mesh.
    let mut max_inverse_bind_error = 0.0f32;
    for (bone, rest) in skeleton.bones.iter().zip(&rest_world) {
        let derived = rigid_inverse(rest);
        for r in 0..3 {
            for c in 0..4 {
                max_inverse_bind_error =
                    max_inverse_bind_error.max((bone.pose_to_bone[r][c] - derived[r][c]).abs());
            }
        }
    }

    // Name-based mapping first, so the fit anchors are available.
    let mut gla_for_bone: Vec<Option<usize>> = vec![None; skeleton.bones.len()];
    let mut spine_candidates = Vec::new();
    let mut anchors = Vec::new();
    let mut missing_humanoid = Vec::new();
    for (index, bone) in skeleton.bones.iter().enumerate() {
        let key = normalize_source_bone(&bone.name);
        // A model already rigged with JKA bone names needs no translation.
        let direct = gla_bone_index(gla, &bone.name);
        let mapped = match direct {
            Some(found) => Some(found),
            None => match humanoid_bone_for(&key) {
                Some(name) => {
                    let found = gla_bone_index(gla, name);
                    if found.is_none() {
                        missing_humanoid.push(name);
                    }
                    found
                }
                None => {
                    if key == "spine" || key.strip_prefix("spine").is_some_and(|n| n.parse::<u32>().is_ok()) {
                        spine_candidates.push((index, origin(&rest_world[index])));
                    }
                    None
                }
            },
        };
        gla_for_bone[index] = mapped;
        if let (Some(gla_index), true) = (mapped, FIT_ANCHORS.contains(&key.as_str())) {
            anchors.push((
                bone.name.clone(),
                gla.skeleton[gla_index].name.clone(),
                origin(&rest_world[index]),
                origin(&gla.skeleton[gla_index].base_pose),
            ));
        }

    }
    if !missing_humanoid.is_empty() {
        missing_humanoid.sort_unstable();
        missing_humanoid.dedup();
        return Err(format!(
            "{model_qpath}: animation skeleton is missing bones the ValveBiped profile needs: {}",
            missing_humanoid.join(", ")
        ));
    }

    let fit = fit_skeletons(&anchors)?;
    let scale = options.scale.unwrap_or(fit.scale);
    if !scale.is_finite() || scale <= 1e-4 {
        return Err(format!("{model_qpath}: invalid Source model scale {scale}"));
    }

    // Resolve the spine with the fitted alignment, then let every remaining bone
    // follow its nearest mapped ancestor so its vertices still move with the body
    // instead of being pinned to the model origin.
    for (bone, gla_index) in resolve_spine_chain(gla, &spine_candidates, |p| fit.align(p)) {
        gla_for_bone[bone] = Some(gla_index);
    }

    let mapped_bones = gla_for_bone.iter().filter(|slot| slot.is_some()).count();
    // Where an orphan Source root lands. `_humanoid`'s own root is `model_root`;
    // finding it by structure rather than by name keeps this working for any
    // animation skeleton a descriptor points at.
    let gla_root = gla
        .skeleton
        .iter()
        .position(|bone| bone.parent < 0)
        .ok_or_else(|| format!("{model_qpath}: animation skeleton has no root bone"))?;
    let mut inherited_bones = Vec::new();
    for index in 0..skeleton.bones.len() {
        if gla_for_bone[index].is_some() {
            continue;
        }
        let mut cursor = skeleton.bones[index].parent;
        while let Some(parent) = cursor {
            if let Some(found) = gla_for_bone[parent] {
                gla_for_bone[index] = Some(found);
                inherited_bones.push((
                    skeleton.bones[index].name.clone(),
                    gla.skeleton[found].name.clone(),
                ));
                break;
            }
            cursor = skeleton.bones[parent].parent;
        }
        if gla_for_bone[index].is_none() {
            // A parentless bone has no ancestor to inherit from, and Source
            // models routinely carry a second one: an exporter's stray `root`
            // at the origin alongside `ValveBiped.Bip01_Pelvis`, usually with
            // no vertices on it at all. Rejecting the model over that threw away
            // a character that was otherwise entirely loadable, so an orphan
            // root falls back to the GLA root - the structural counterpart of a
            // parentless Source bone. Anything weighted to it then rides the
            // model origin, which is the honest answer for a bone that is not
            // attached to the character's skeleton. A genuinely wrong skeleton
            // is still caught, earlier and with a better message, by
            // `fit_skeletons` refusing to fit too few anchors.
            gla_for_bone[index] = Some(gla_root);
            inherited_bones.push((
                skeleton.bones[index].name.clone(),
                gla.skeleton[gla_root].name.clone(),
            ));
        }
    }

    // An authored scale replaces the fitted one. Rebuild the similarity around the
    // anchor centroids instead of scaling the matrix wholesale, which would scale
    // the translation column too and slide the model off the skeleton.
    let placement = {
        let ratio = scale / fit.scale;
        let mut out = identity();
        for r in 0..3 {
            for c in 0..3 {
                out[r][c] = fit.transform[r][c] * ratio;
            }
            out[r][3] = fit.target_centroid[r]
                - (0..3)
                    .map(|k| out[r][k] * fit.source_centroid[k])
                    .sum::<f32>();
        }
        out
    };
    // Several Source bones routinely land on one `_humanoid` joint: ValveBiped's
    // toe on the ankle, its four- or five-bone spine on three lumbar bones, its
    // index and middle finger on one digit, and every helper without a rule of
    // its own on the nearest mapped ancestor. Each of those bones has its own
    // rest joint, so seating each one *independently* on the shared GLA joint
    // translates them by different amounts and drags the parts onto each other:
    // the toe collapses into the ankle, the ears into the skull, a tunic skirt
    // into the pelvis. The joint therefore gets exactly one matrix, built from
    // the bone that owns it, and every other bone on that joint rides along, so
    // their rest offsets stay rigid. Nothing is lost by sharing: the weight
    // merge below already maps all of them to that one GLA bone, so they could
    // never animate apart.
    //
    // The owner is the lowest-indexed bone on the joint, which is the shallowest
    // one because `source_rest_world` has already rejected any table that does
    // not store parents before children.
    let owner_of_gla = joint_owners(&gla_for_bone);
    let twist_blend = twist_blend_table(gla);

    // A joint with no continuing child of its own (a hand, a foot, the skull -
    // anything `bone_direction_pair` can't find a differently-mapped child
    // for) gets no rotation correction of its own; it keeps the authored
    // Source orientation. This used to stretch a vertex blended between two
    // such differently-rotated joints (the neck/skull seam is the model case)
    // to bridge the gap between them, which read as the neck sticking out.
    // The real fix is in how blended vertices are combined below (dual
    // quaternion skinning, see `blend_dual_quats`) rather than here: forcing
    // every leaf joint to inherit an ancestor's rotation was only a
    // workaround for the blending bug, and cost leaf joints their own
    // correct, independent orientation (Han Solo's head tilting back to match
    // his neck's rotation, though his head itself needed none).
    let mut direction_corrected = 0usize;
    let mut matrix_of_gla = BTreeMap::<usize, Matrix3x4>::new();
    for (&gla_bone, &owner) in &owner_of_gla {
        let align = if options.rebind == SourceRebindMode::Direction {
            let pair = bone_direction_pair(owner, skeleton, &rest_world, gla, &gla_for_bone, &fit);
            if pair.is_some() {
                direction_corrected += 1;
            }
            pair.map(|(source, target)| minimal_rotation(source, target))
        } else {
            None
        };
        matrix_of_gla.insert(
            gla_bone,
            rebind_matrix(
                options.rebind,
                &placement,
                scale,
                &rest_world[owner],
                &gla.skeleton[gla_bone].base_pose,
                align,
            ),
        );
    }

    let rebinds: Vec<BoneRebind> = gla_for_bone
        .iter()
        .map(|slot| {
            let gla_bone = slot.expect("every bone is mapped above");
            BoneRebind {
                gla_bone,
                matrix: matrix_of_gla[&gla_bone],
            }
        })
        .collect();

    // Surface identity must be stable across LODs, so it is keyed on the MDL
    // (bodypart, model, mesh) triple rather than on per-LOD ordering.
    let bodyparts = parse_source_bodyparts(mdl)?;
    let lod_count = vvd
        .lod_count
        .min(vtx.lod_count)
        .min(options.max_lods.max(1));
    let base_draws = assemble_source_lod(mdl, vvd, vtx, 0, &options.bodygroups)?;
    let mut surface_index_of = BTreeMap::new();
    let mut surfaces = Vec::with_capacity(base_draws.len());
    let mut hierarchy = Vec::with_capacity(base_draws.len());
    for draw in &base_draws {
        let index = surfaces.len();
        surface_index_of.insert((draw.bodypart, draw.model, draw.mesh), index);
        let body = &bodyparts[draw.bodypart];
        let model_stem = body.models[draw.model]
            .name
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".smd")
            .trim_end_matches(".SMD");
        let sanitize = |text: &str| -> String {
            let cleaned: String = text
                .to_ascii_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            cleaned.trim_matches('_').to_owned()
        };
        let name = format!(
            "{}_{}_{}",
            sanitize(&body.name),
            sanitize(model_stem),
            draw.mesh
        );
        let texture_index = materials.texture_index(options.skin_family, draw.skin_ref);
        hierarchy.push(GlmSurfaceHierarchy {
            name: name.clone(),
            flags: 0,
            shader: texture_index
                .and_then(|slot| materials.textures.get(slot).cloned())
                .unwrap_or_default(),
            parent_index: -1,
            children: Vec::new(),
        });
        surfaces.push(SourceRigSurface {
            name,
            vmt_candidates: texture_index
                .map(|slot| materials.vmt_candidates(slot))
                .unwrap_or_default(),
        });
    }

    let mut weight_truncations = 0usize;
    let mut max_rebind_displacement = 0.0f32;
    let mut rest_min = [f32::INFINITY; 3];
    let mut rest_max = [f32::NEG_INFINITY; 3];
    let mut lods = Vec::with_capacity(lod_count);
    let mut lod_stats = Vec::with_capacity(lod_count);
    for lod in 0..lod_count {
        let pool = source_lod_vertices(vvd, lod)?;
        let draws = if lod == 0 {
            base_draws.clone()
        } else {
            assemble_source_lod(mdl, vvd, vtx, lod, &options.bodygroups)?
        };
        let mut lod_surfaces = Vec::with_capacity(draws.len());
        let mut lod_vertices = 0usize;
        let mut lod_triangles = 0usize;
        for draw in &draws {
            let Some(&surface_index) = surface_index_of.get(&(draw.bodypart, draw.model, draw.mesh))
            else {
                // A coarser LOD may only drop meshes, never introduce new ones.
                return Err(format!(
                    "{model_qpath}: LOD{lod} introduces bodypart {} model {} mesh {} that LOD0 does not have",
                    draw.bodypart, draw.model, draw.mesh
                ));
            };
            if draw.indices.is_empty() {
                continue;
            }
            if draw.indices.len() % 3 != 0 {
                return Err(format!(
                    "{model_qpath}: LOD{lod} surface {surface_index} has an incomplete triangle"
                ));
            }
            // Compact the shared LOD pool into a per-surface vertex buffer.
            let mut remap = BTreeMap::<u32, u32>::new();
            let mut vertices: Vec<GlmVertex> = Vec::new();
            let mut texcoords: Vec<[f32; 2]> = Vec::new();
            let mut bone_references: Vec<usize> = Vec::new();
            let mut local_of_gla = BTreeMap::<usize, usize>::new();
            let mut triangles = Vec::with_capacity(draw.indices.len() / 3);
            let mut corner = [0u32; 3];
            for (slot, &pool_index) in draw.indices.iter().enumerate() {
                let local = match remap.get(&pool_index).copied() {
                    Some(found) => found,
                    None => {
                        let source = pool.get(pool_index as usize).ok_or_else(|| {
                            format!("{model_qpath}: LOD{lod} vertex {pool_index} outside the gathered pool")
                        })?;
                        let influences = source.bone_count as usize;
                        let mut total = 0.0f32;
                        let mut merged = BTreeMap::<usize, f32>::new();
                        // Collected instead of blended immediately: the
                        // dominant (highest-weight) influence must go first
                        // so `blend_dual_quats` flips every other quaternion
                        // into its hemisphere, not an arbitrary bone-table one.
                        let mut parts: Vec<(f32, [f32; 4], [f32; 4])> = Vec::with_capacity(influences);
                        for k in 0..influences {
                            let bone = source.bones[k] as usize;
                            let rebind = rebinds.get(bone).ok_or_else(|| {
                                format!(
                                    "{model_qpath}: vertex {pool_index} references Source bone {bone}, but the skeleton has {}",
                                    rebinds.len()
                                )
                            })?;
                            let weight = source.weights[k];
                            if !weight.is_finite() || weight < 0.0 {
                                return Err(format!(
                                    "{model_qpath}: vertex {pool_index} has invalid weight {weight}"
                                ));
                            }
                            if weight <= 0.0 {
                                continue;
                            }
                            total += weight;
                            // Several Source bones can share one GLA bone (a
                            // five-bone ValveBiped spine against three lumbar
                            // bones), and they all placed the vertex through
                            // that joint's single matrix, so merging their
                            // influence here only compacts the weight list.
                            *merged.entry(rebind.gla_bone).or_default() += weight;
                            let (real, dual) = dual_quat_from_matrix(&rebind.matrix);
                            parts.push((weight, real, dual));
                        }
                        if total <= 1e-6 || merged.is_empty() {
                            return Err(format!(
                                "{model_qpath}: vertex {pool_index} has no usable skinning weights"
                            ));
                        }
                        // Blend the bone *transforms* (dual quaternion
                        // skinning), not the positions each independently
                        // produces: the rebind corrects a Source T-pose onto
                        // `_humanoid`'s A-pose per bone, often by tens of
                        // degrees, and blending positions from two bones that
                        // disagree that much stretches the mesh between them
                        // (the classic linear-blend-skinning "candy wrapper"
                        // artifact) - confirmed on Juliet Starling, whose
                        // unrebound source mesh has an ordinary neck and limb
                        // proportions that only became stretched after this
                        // step.
                        parts.sort_by(|a, b| b.0.total_cmp(&a.0));
                        let (blended_real, blended_dual) = blend_dual_quats(&parts);
                        let blended = dual_quat_to_matrix(blended_real, blended_dual);
                        let position = transform(&blended, source.position);
                        let normal = rotate(&blended, source.normal);
                        // See `twist_blend_table`: ease each chain-bone's
                        // weight toward its twist-helper sibling by how far
                        // along the limb this vertex sits. Applied after the
                        // position is final so the limb fraction reflects
                        // where the vertex actually baked to, and before
                        // `influence_list` so it is truncated/normalized like
                        // any other weight.
                        let mut twist_additions: Vec<(usize, f32)> = Vec::new();
                        for (&bone, weight) in merged.iter_mut() {
                            if let Some(&(twist_bone, joint, next_joint)) = twist_blend.get(&bone) {
                                let t = limb_fraction(position, joint, next_joint);
                                let fraction = twist_blend_fraction(t);
                                if fraction > 0.0 {
                                    let moved = *weight * fraction;
                                    *weight -= moved;
                                    twist_additions.push((twist_bone, moved));
                                }
                            }
                        }
                        for (bone, moved) in twist_additions {
                            *merged.entry(bone).or_default() += moved;
                        }
                        let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
                        let normal = if length > 1e-6 {
                            normal.map(|v| v / length)
                        } else {
                            // Fully cancelling normals mean the bone frames
                            // disagree; a +Z fallback keeps the vertex usable.
                            [0.0, 0.0, 1.0]
                        };
                        for axis in 0..3 {
                            rest_min[axis] = rest_min[axis].min(position[axis]);
                            rest_max[axis] = rest_max[axis].max(position[axis]);
                        }
                        // How far the rebind moved this vertex compared with
                        // merely scaling/aligning the Source mesh in place.
                        let merely_aligned = transform(&placement, source.position);
                        let displacement = (0..3)
                            .map(|k| (position[k] - merely_aligned[k]).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        max_rebind_displacement = max_rebind_displacement.max(displacement);

                        let mut influence_list: Vec<(usize, f32)> = merged.into_iter().collect();
                        influence_list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                        if influence_list.len() > MAX_WEIGHTS {
                            influence_list.truncate(MAX_WEIGHTS);
                            weight_truncations += 1;
                        }
                        let sum: f32 = influence_list.iter().map(|(_, w)| *w).sum();
                        if sum <= 1e-6 {
                            return Err(format!(
                                "{model_qpath}: vertex {pool_index} lost all weight while merging"
                            ));
                        }
                        let weights: Vec<GlmWeight> = influence_list
                            .iter()
                            .map(|(gla_bone, weight)| {
                                let local_bone_index =
                                    *local_of_gla.entry(*gla_bone).or_insert_with(|| {
                                        bone_references.push(*gla_bone);
                                        bone_references.len() - 1
                                    });
                                GlmWeight {
                                    local_bone_index,
                                    weight: weight / sum,
                                }
                            })
                            .collect();
                        let local = u32::try_from(vertices.len())
                            .map_err(|_| "Source surface exceeds u32 vertices".to_owned())?;
                        vertices.push(GlmVertex {
                            normal,
                            position,
                            weights,
                        });
                        texcoords.push(source.uv);
                        remap.insert(pool_index, local);
                        local
                    }
                };
                corner[slot % 3] = local;
                if slot % 3 == 2 {
                    triangles.push(corner);
                }
            }
            lod_vertices += vertices.len();
            lod_triangles += triangles.len();
            lod_surfaces.push(GlmSurface {
                surface_index,
                vertices,
                texcoords,
                triangles,
                bone_references,
            });
        }
        if lod_surfaces.is_empty() {
            break;
        }
        lod_stats.push(SourceLodStats {
            lod,
            surfaces: lod_surfaces.len(),
            vertices: lod_vertices,
            triangles: lod_triangles,
        });
        lods.push(GlmLod {
            surfaces: lod_surfaces,
        });
    }
    if lods.is_empty() {
        return Err(format!("{model_qpath}: produced no drawable LODs"));
    }

    let diagnostics = SourceRigDiagnostics {
        model_name: skeleton.model_name.clone(),
        source_bones: skeleton.bones.len(),
        mapped_bones,
        inherited_bones,
        scale,
        scale_was_fitted: options.scale.is_none(),
        fit_pairs: fit.pairs,
        fit_yaw_degrees: fit.yaw_degrees,
        fit_rms: fit.rms,
        fit_worst: fit.worst,
        max_inverse_bind_error,
        rebind: options.rebind,
        direction_corrected,
        weight_truncations,
        rest_bounds: (rest_min, rest_max),
        max_rebind_displacement,
        lods: lod_stats,
    };
    Ok(SourceRig {
        glm: GlmModel {
            name: model_qpath.to_owned(),
            anim_name: "models/players/_humanoid/_humanoid".to_owned(),
            num_bones: gla.skeleton.len(),
            hierarchy,
            lods,
        },
        surfaces,
        diagnostics,
    })
}

/// Ghoul2 `G2SURFACEFLAG_ISBOLT`.
const G2SURFACEFLAG_ISBOLT: u32 = 0x0000_0001;

/// Copy the `*`-prefixed Ghoul2 tag surfaces from a stock `_humanoid` GLM onto a
/// rebound Source model, and report how many were attached.
///
/// A Source `.mdl` has no equivalent of a JKA tag surface, so without this a
/// rebound character has no bolts at all: `model_bolt_matrix` finds neither a
/// `*r_hand` surface nor a GLA bone of that name, returns `None`, and the saber,
/// weapon flash, jetpack and bolt-driven effects are all silently skipped.
///
/// Lifting them is sound rather than a hack. A tag surface is three vertices
/// weighted to `_humanoid` bones and authored in base-pose space - the very space
/// this module bakes into - so the donor's frames animate correctly on any
/// `_humanoid` pose, and because the rebind seats each Source joint on its GLA
/// counterpart the hand bolt lands in the rebound model's hand. Using JKA's own
/// authored frames also avoids inventing a hilt-in-palm orientation, which a raw
/// bone matrix would require.
pub fn attach_humanoid_bolts(
    rig: &mut SourceRig,
    donor: &GlmModel,
    gla: &GlaAnimation,
) -> Result<usize, String> {
    let Some(donor_lod) = donor.lods.first() else {
        return Err("bolt donor GLM has no LODs".into());
    };
    let mut attached = 0usize;
    for (donor_index, hierarchy) in donor.hierarchy.iter().enumerate() {
        if !hierarchy.name.starts_with('*') {
            continue;
        }
        let Some(surface) = donor_lod
            .surfaces
            .iter()
            .find(|surface| surface.surface_index == donor_index)
        else {
            continue;
        };
        // `G2_ProcessSurfaceBolt2` reads the first three vertices, so anything
        // shorter cannot define a frame.
        if surface.vertices.len() < 3 {
            continue;
        }
        for &bone in &surface.bone_references {
            if bone >= gla.skeleton.len() {
                return Err(format!(
                    "bolt donor surface {:?} references GLA bone {bone}, but the skeleton has {}",
                    hierarchy.name,
                    gla.skeleton.len()
                ));
            }
        }
        let surface_index = rig.glm.hierarchy.len();
        rig.glm.hierarchy.push(GlmSurfaceHierarchy {
            name: hierarchy.name.clone(),
            flags: hierarchy.flags | G2SURFACEFLAG_ISBOLT,
            shader: String::new(),
            // The donor's parent/child indices address the donor's hierarchy, not
            // this one. Anything that walks the tree (cg_fpls, surface on/off
            // descendants) must not be able to follow them into a foreign index.
            parent_index: -1,
            children: Vec::new(),
        });
        // Every LOD: a bolt is looked up in LOD 0, but dismemberment and effects
        // resolve surfaces against whichever LOD is current.
        for lod in rig.glm.lods.iter_mut() {
            lod.surfaces.push(GlmSurface {
                surface_index,
                vertices: surface.vertices.clone(),
                texcoords: surface.texcoords.clone(),
                triangles: surface.triangles.clone(),
                bone_references: surface.bone_references.clone(),
            });
        }
        attached += 1;
    }
    Ok(attached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_valvebiped_names() {
        assert_eq!(normalize_source_bone("ValveBiped.Bip01_Pelvis"), "pelvis");
        assert_eq!(
            normalize_source_bone("ValveBiped.Bip01_L_UpperArm"),
            "lupperarm"
        );
        assert_eq!(normalize_source_bone("ValveBiped.forward"), "forward");
        assert_eq!(
            normalize_source_bone("ValveBiped.Anim_Attachment_RH"),
            "animattachmentrh"
        );
        assert_eq!(normalize_source_bone("ValveBiped.Bip01_Spine4"), "spine4");
    }

    /// `_humanoid` splits the femur and humerus into a chain bone and a twist
    /// helper. A limb must follow the chain bone, because the next joint down
    /// parents to it, not to the helper.
    #[test]
    fn limbs_target_chain_bones_not_twist_helpers() {
        assert_eq!(humanoid_bone_for("lthigh"), Some("lfemurYZ"));
        assert_eq!(humanoid_bone_for("rupperarm"), Some("rhumerus"));
        assert_eq!(humanoid_bone_for("lforearm"), Some("lradius"));
        // No ValveBiped joint may target an `X` twist helper: the next joint
        // down parents to the chain bone, so a helper would not carry the limb.
        let helpers = ["lfemurx", "rfemurx", "lhumerusx", "rhumerusx", "lradiusx", "rradiusx"];
        for source in [
            "pelvis", "neck1", "head1", "lclavicle", "rclavicle", "lupperarm", "rupperarm",
            "lforearm", "rforearm", "lhand", "rhand", "lthigh", "rthigh", "lcalf", "rcalf",
            "lfoot", "rfoot", "ltoe0", "rtoe0",
        ] {
            let target = humanoid_bone_for(source).expect("mapped");
            assert!(
                !helpers.contains(&target.to_ascii_lowercase().as_str()),
                "{source} must not target twist helper {target}"
            );
        }
    }

    #[test]
    fn spine_is_not_name_mapped() {
        // Spine bones are resolved positionally, so the name table must not
        // claim them; otherwise models with a different subdivision squash.
        for name in ["spine", "spine1", "spine2", "spine3", "spine4"] {
            assert_eq!(humanoid_bone_for(name), None);
        }
    }

    #[test]
    fn twist_blend_ramps_from_the_joint_to_the_next_one_down() {
        let joint = [0.0_f32, 0.0, 40.0];
        let next_joint = [0.0_f32, 0.0, 20.0];
        assert_eq!(twist_blend_fraction(limb_fraction(joint, joint, next_joint)), 0.0);
        let far_past = [0.0_f32, 0.0, 10.0];
        assert!((twist_blend_fraction(limb_fraction(far_past, joint, next_joint)) - 0.85).abs() < 1e-6);
        let behind = [0.0_f32, 0.0, 50.0];
        assert_eq!(twist_blend_fraction(limb_fraction(behind, joint, next_joint)), 0.0);
        let midpoint = [0.0_f32, 0.0, 30.0];
        let mid_fraction = twist_blend_fraction(limb_fraction(midpoint, joint, next_joint));
        assert!(
            mid_fraction > 0.0 && mid_fraction < 0.85,
            "midpoint fraction {mid_fraction} should sit strictly between the ends"
        );
    }

    #[test]
    fn toes_fold_onto_the_ankle() {
        assert_eq!(humanoid_bone_for("ltoe0"), Some("ltalus"));
        assert_eq!(humanoid_bone_for("rfoot"), Some("rtalus"));
    }

    fn rigid(yaw: f32, t: [f32; 3]) -> Matrix3x4 {
        let (s, c) = yaw.sin_cos();
        [[c, -s, 0., t[0]], [s, c, 0., t[1]], [0., 0., 1., t[2]]]
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
    }

    #[test]
    fn minimal_rotation_turns_one_direction_onto_another() {
        for (from, to) in [
            ([1., 0., 0.], [0., 1., 0.]),
            ([0., 0., 1.], [0., 0., -1.]),
            ([1., 2., 3.], [-3., 1., 0.5]),
            ([1., 0., 0.], [1., 0., 0.]),
        ] {
            let turned = rotate(&minimal_rotation(from, to), from);
            let want = normalized(to).unwrap();
            let have = normalized(turned).unwrap();
            for axis in 0..3 {
                assert!(
                    (have[axis] - want[axis]).abs() < 1e-4,
                    "{from:?} -> {to:?} gave {have:?}"
                );
            }
            // A rotation must not change the vector's length.
            let length = |v: [f32; 3]| v.iter().map(|c| c * c).sum::<f32>().sqrt();
            assert!((length(turned) - length(from)).abs() < 1e-4);
        }
    }

    /// The regression this module exists to prevent. ValveBiped and `_humanoid`
    /// bone frames differ by 100-145 degrees of pure convention while their joints
    /// sit within a unit or two of each other. `BoneFrame` turns that convention
    /// difference into a real rotation of the mesh about the joint;
    /// `Translation`/`Direction` must not, because a Ghoul2 pose matrix
    /// (`world_animated * inverse(basePose)`) cancels the convention at runtime.
    #[test]
    fn rebind_does_not_apply_the_axis_convention_difference() {
        // Same joint position, rest frames 100 degrees apart: pure convention.
        let joint = [3.0_f32, -2.0, 40.0];
        let source_rest = rigid(0.0, joint);
        let gla_base = rigid(100.0_f32.to_radians(), joint);
        let placement = identity();
        // A vertex 10 units out from the joint, i.e. ordinary limb flesh.
        let vertex = [joint[0] + 10.0, joint[1], joint[2]];

        let kept = transform(
            &rebind_matrix(
                SourceRebindMode::Translation,
                &placement,
                1.0,
                &source_rest,
                &gla_base,
                None,
            ),
            vertex,
        );
        assert!(
            distance(kept, vertex) < 1e-3,
            "translation rebind must leave a coincident joint's geometry alone, moved {:.3}",
            distance(kept, vertex)
        );

        let warped = transform(
            &rebind_matrix(
                SourceRebindMode::BoneFrame,
                &placement,
                1.0,
                &source_rest,
                &gla_base,
                None,
            ),
            vertex,
        );
        // 10 units swung through 100 degrees is over 15 units of travel.
        assert!(
            distance(warped, vertex) > 10.0,
            "BoneFrame is expected to warp; it moved only {:.3}",
            distance(warped, vertex)
        );
    }

    #[test]
    fn bones_sharing_a_gla_joint_share_one_matrix_and_keep_their_offsets() {
        // ValveBiped's toe maps onto the ankle, because `_humanoid` has no toe
        // bone. Rebinding the toe through its own rest joint used to seat the toe
        // joint on the ankle joint, dragging the front of the foot back into the
        // leg; the same collapse hit ears, tunic skirts and twist helpers.
        let ankle = rigid(0.0, [3.0, 0.0, 6.0]);
        let toe = rigid(0.0, [3.0, -8.0, 2.0]);
        let gla_ankle = rigid(0.4, [2.0, 1.0, 7.0]);

        // foot at index 0, toe at index 1, both on GLA bone 9.
        let owners = joint_owners(&[Some(9), Some(9)]);
        assert_eq!(owners.get(&9), Some(&0), "the foot owns the ankle, not the toe");

        let shared = rebind_matrix(
            SourceRebindMode::Direction,
            &identity(),
            1.0,
            &rest_world_of(&owners, &[ankle, toe], 9),
            &gla_ankle,
            None,
        );
        // A toe-tip vertex and the ankle joint must stay exactly as far apart as
        // they were, which is what "the foot is still a foot" means here.
        let tip = [3.0, -11.0, 1.0];
        let before = distance(tip, origin(&ankle));
        let after = distance(transform(&shared, tip), transform(&shared, origin(&ankle)));
        assert!(
            (before - after).abs() < 1e-3,
            "the toe moved relative to the ankle: {before:.3} -> {after:.3}"
        );
        assert!(
            distance(transform(&shared, origin(&ankle)), origin(&gla_ankle)) < 1e-3,
            "the owning joint must still land on its GLA counterpart"
        );
    }

    /// The rest frame the joint's matrix is built from: its owner's.
    fn rest_world_of(
        owners: &BTreeMap<usize, usize>,
        rest_world: &[Matrix3x4],
        gla_bone: usize,
    ) -> Matrix3x4 {
        rest_world[owners[&gla_bone]]
    }

    #[test]
    fn a_joint_owner_is_the_shallowest_bone_on_it() {
        // Unmapped helpers inherit a mapped ancestor, so they always sort after it.
        let owners = joint_owners(&[Some(1), Some(4), None, Some(4), Some(1)]);
        assert_eq!(owners[&1], 0);
        assert_eq!(owners[&4], 1);
        assert_eq!(owners.len(), 2);
    }

    #[test]
    fn rebind_seats_the_joint_on_its_gla_counterpart() {
        // Animation pivots about the GLA joint, so the bone's own joint has to land
        // there or every rotation is applied about the wrong point.
        let source_rest = rigid(0.3, [10., 2., 30.]);
        let gla_base = rigid(-1.1, [1., -2., 40.]);
        for mode in [SourceRebindMode::Translation, SourceRebindMode::Direction] {
            let m = rebind_matrix(mode, &identity(), 1.0, &source_rest, &gla_base, None);
            let placed = transform(&m, origin(&source_rest));
            assert!(
                distance(placed, origin(&gla_base)) < 1e-3,
                "{mode:?} put the joint at {placed:?}, wanted {:?}",
                origin(&gla_base)
            );
        }
    }

    #[test]
    fn direction_rebind_points_the_bone_the_gla_way_without_scaling_it() {
        // Source bone runs along +X from its joint; its GLA counterpart runs along
        // +Z. The rebind must turn the limb without stretching it.
        let joint = [0.0_f32, 0.0, 20.0];
        let source_rest = rigid(0.0, joint);
        let gla_base = rigid(0.0, joint);
        let m = rebind_matrix(
            SourceRebindMode::Direction,
            &identity(),
            1.0,
            &source_rest,
            &gla_base,
            Some(minimal_rotation([1., 0., 0.], [0., 0., 1.])),
        );
        let tip = transform(&m, [joint[0] + 7.0, joint[1], joint[2]]);
        let offset: [f32; 3] = std::array::from_fn(|k| tip[k] - joint[k]);
        assert!(
            (offset[2] - 7.0).abs() < 1e-3 && offset[0].abs() < 1e-3,
            "limb should now run +Z for 7 units, got {offset:?}"
        );
    }

    fn tag_surface(index: usize, bone: usize) -> GlmSurface {
        GlmSurface {
            surface_index: index,
            vertices: (0..3)
                .map(|k| GlmVertex {
                    normal: [0., 0., 1.],
                    position: [k as f32, 0., 0.],
                    weights: vec![GlmWeight {
                        local_bone_index: 0,
                        weight: 1.0,
                    }],
                })
                .collect(),
            texcoords: vec![[0., 0.]; 3],
            triangles: vec![[0, 1, 2]],
            bone_references: vec![bone],
        }
    }

    fn donor_glm() -> GlmModel {
        GlmModel {
            name: "donor".into(),
            anim_name: "models/players/_humanoid/_humanoid".into(),
            num_bones: 4,
            hierarchy: vec![
                GlmSurfaceHierarchy {
                    name: "torso".into(),
                    flags: 0,
                    shader: "some/shader".into(),
                    parent_index: -1,
                    children: vec![1],
                },
                GlmSurfaceHierarchy {
                    name: "*r_hand".into(),
                    flags: 0,
                    shader: String::new(),
                    // Deliberately non-trivial: must not survive the copy.
                    parent_index: 0,
                    children: vec![0],
                },
            ],
            lods: vec![GlmLod {
                surfaces: vec![tag_surface(0, 1), tag_surface(1, 2)],
            }],
        }
    }

    fn empty_rig() -> SourceRig {
        SourceRig {
            glm: GlmModel {
                name: "source".into(),
                anim_name: "models/players/_humanoid/_humanoid".into(),
                num_bones: 4,
                hierarchy: vec![GlmSurfaceHierarchy {
                    name: "body_chest_0".into(),
                    flags: 0,
                    shader: "mk2chest".into(),
                    parent_index: -1,
                    children: Vec::new(),
                }],
                lods: vec![
                    GlmLod {
                        surfaces: vec![tag_surface(0, 0)],
                    },
                    GlmLod {
                        surfaces: vec![tag_surface(0, 0)],
                    },
                ],
            },
            surfaces: vec![SourceRigSurface {
                name: "body_chest_0".into(),
                vmt_candidates: Vec::new(),
            }],
            diagnostics: SourceRigDiagnostics::default(),
        }
    }

    fn stub_gla(bones: usize) -> GlaAnimation {
        crate::ghoul2::openjk_default_gla()
            .map(|mut gla| {
                while gla.skeleton.len() < bones {
                    let mut extra = gla.skeleton[0].clone();
                    extra.name = format!("bone{}", gla.skeleton.len());
                    gla.skeleton.push(extra);
                }
                gla
            })
            .expect("built-in default GLA parses")
    }

    /// Without this a rebound Source character has no saber: `model_bolt_matrix`
    /// finds no `*r_hand` surface and no GLA bone of that name.
    #[test]
    fn bolts_are_lifted_into_every_lod_and_detached_from_the_donor_tree() {
        let mut rig = empty_rig();
        let attached = attach_humanoid_bolts(&mut rig, &donor_glm(), &stub_gla(4)).unwrap();
        assert_eq!(attached, 1, "only the *-prefixed surface is a bolt");

        let bolt = rig
            .glm
            .hierarchy
            .iter()
            .position(|h| h.name == "*r_hand")
            .expect("bolt hierarchy entry");
        assert_eq!(rig.glm.hierarchy[bolt].flags & G2SURFACEFLAG_ISBOLT, G2SURFACEFLAG_ISBOLT);
        // Donor indices must not leak: they address the donor's hierarchy.
        assert_eq!(rig.glm.hierarchy[bolt].parent_index, -1);
        assert!(rig.glm.hierarchy[bolt].children.is_empty());
        // The drawable surface must keep index 0, or PlayerSurfaceAsset lookups break.
        assert_eq!(rig.glm.hierarchy[0].name, "body_chest_0");
        // Bolts resolve in LOD 0, but effects resolve against the current LOD.
        for (index, lod) in rig.glm.lods.iter().enumerate() {
            assert!(
                lod.surfaces.iter().any(|s| s.surface_index == bolt),
                "LOD {index} is missing the bolt surface"
            );
        }
        // A bolt is not a drawable surface, so it gets no PlayerSurfaceAsset.
        assert_eq!(rig.surfaces.len(), 1);
    }

    #[test]
    fn bolt_donor_bone_references_are_validated_against_the_skeleton() {
        let mut rig = empty_rig();
        // The donor's bolt references bone 2; a 2-bone skeleton cannot hold it.
        let error = attach_humanoid_bolts(&mut rig, &donor_glm(), &stub_gla(2)).unwrap_err();
        assert!(error.contains("*r_hand"), "{error}");
    }

    #[test]
    fn rebind_mode_parses_from_the_descriptor() {
        assert_eq!(
            SourceRebindMode::from_descriptor("direction"),
            Some(SourceRebindMode::Direction)
        );
        assert_eq!(
            SourceRebindMode::from_descriptor("BONE_FRAME"),
            Some(SourceRebindMode::BoneFrame)
        );
        assert_eq!(SourceRebindMode::from_descriptor("nonsense"), None);
        assert_eq!(SourceRebindMode::default(), SourceRebindMode::Direction);
    }

    #[test]
    fn rigid_inverse_round_trips() {
        let m = rigid(0.7, [3., -4., 5.]);
        let back = mul(&rigid_inverse(&m), &m);
        for r in 0..3 {
            for c in 0..4 {
                assert!(
                    (back[r][c] - identity()[r][c]).abs() < 1e-5,
                    "row {r} col {c} = {}",
                    back[r][c]
                );
            }
        }
    }

    /// The configured/fitted uniform scale reaches the baked mesh through
    /// `placement`, so a Source limb keeps its proportions at JKA size.
    #[test]
    fn rebind_applies_the_uniform_scale_to_limb_length() {
        let scale = 0.5_f32;
        let placement = [
            [scale, 0., 0., 0.],
            [0., scale, 0., 0.],
            [0., 0., scale, 0.],
        ];
        let joint = [0.0_f32, 0.0, 40.0];
        let source_rest = rigid(0.0, joint);
        // The GLA joint sits wherever it likes; the limb length must not depend on it.
        let gla_base = rigid(0.7, [5., -3., 33.]);
        let m = rebind_matrix(
            SourceRebindMode::Translation,
            &placement,
            scale,
            &source_rest,
            &gla_base,
            None,
        );
        let tip = transform(&m, [joint[0] + 10.0, joint[1], joint[2]]);
        let length = distance(tip, origin(&gla_base));
        assert!(
            (length - 10.0 * scale).abs() < 1e-3,
            "limb measured {length}, wanted {}",
            10.0 * scale
        );
    }

    #[test]
    fn fit_recovers_a_known_similarity() {
        // Build anchors from a synthetic skeleton transformed by a known yaw and
        // scale; the fit must recover both.
        let yaw = 0.9f32;
        let scale = 1.4f32;
        let (s, c) = yaw.sin_cos();
        let source = [
            [0.0f32, 0., 10.],
            [3., 1., 20.],
            [-3., -1., 20.],
            [6., 0., 5.],
            [-6., 2., 7.],
        ];
        let anchors: Vec<_> = source
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mapped = [
                    scale * (c * p[0] - s * p[1]) + 2.0,
                    scale * (s * p[0] + c * p[1]) - 3.0,
                    scale * p[2] + 4.0,
                ];
                (format!("src{i}"), format!("gla{i}"), *p, mapped)
            })
            .collect();
        let fit = fit_skeletons(&anchors).unwrap();
        assert!((fit.scale - scale).abs() < 1e-3, "scale {}", fit.scale);
        assert!(
            (fit.yaw_degrees - yaw.to_degrees()).abs() < 0.1,
            "yaw {}",
            fit.yaw_degrees
        );
        assert!(fit.rms < 1e-2, "rms {}", fit.rms);
    }

    #[test]
    fn fit_refuses_too_few_anchors() {
        let anchors = vec![(
            "a".to_owned(),
            "b".to_owned(),
            [0., 0., 0.],
            [0., 0., 0.],
        )];
        assert!(fit_skeletons(&anchors).unwrap_err().contains("humanoid"));
    }
}
