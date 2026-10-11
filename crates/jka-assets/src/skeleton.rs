//! Independent mesh and animation rigs. All matrices use Ghoul2's row-major
//! affine convention, including the caller's root transform. Evaluator output
//! is a SKIN matrix S, not a joint frame: model-space joint J = S * BasePose.
//! Never feed decompressed parent-local GLA matrices to this API.
use crate::ghoul2::{multiply_3x4, parse_gla, GlaAnimation, GlaBone, Matrix3x4};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanoidRig {
    Jka53,
    Jk2_72,
}

/// Exact supplied skeletons, with only one reference frame retained. These
/// references bind meshes; they are not gameplay animation sources.
pub fn humanoid_reference(rig: HumanoidRig) -> Arc<GlaAnimation> {
    static JKA: OnceLock<Arc<GlaAnimation>> = OnceLock::new();
    static JK2: OnceLock<Arc<GlaAnimation>> = OnceLock::new();
    let (slot, bytes): (_, &[u8]) = match rig {
        HumanoidRig::Jka53 => (&JKA, include_bytes!("skeleton/jka-reference.gla")),
        HumanoidRig::Jk2_72 => (&JK2, include_bytes!("skeleton/jk2-reference.gla")),
    };
    Arc::clone(
        slot.get_or_init(|| Arc::new(parse_gla(bytes).expect("validated humanoid reference"))),
    )
}

/// External pose input for future decoders. Joint frames are already composed
/// in model space, with the same entity-independent root convention as the
/// destination. Extra digit motion is never propagated back into a wrist.
pub enum PoseInput<'a> {
    ModelSpaceJoints(&'a [Matrix3x4]),
    Ghoul2SkinMatrices(&'a [Matrix3x4]),
}

#[derive(Debug)]
struct JointMapping {
    source: usize,
    /// source bind joint -> target bind joint, in source joint coordinates.
    bind_correction: Matrix3x4,
    target_inverse_bind: Matrix3x4,
}

#[derive(Debug)]
pub struct PoseRetargeter {
    source_bind: Vec<Matrix3x4>,
    mappings: Vec<JointMapping>,
}

fn index(bones: &[GlaBone], name: &str) -> Option<usize> {
    bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
}

// Anatomical ancestry is deliberately separate from the original animation
// hierarchy: JK2's limbs/digits are mostly siblings under pelvis/thoracic.
pub fn anatomical_parent(bones: &[GlaBone], bone: usize) -> Option<usize> {
    let name = bones.get(bone)?.name.to_ascii_lowercase();
    let parent = match name.as_str() {
        "ltarsal" => "ltalus",
        "rtarsal" => "rtalus",
        "lfemurx" | "ltibia" | "ltail" => "lfemurYZ",
        "rfemurx" | "rtibia" | "rtail" => "rfemurYZ",
        "ltalus" => "ltibia",
        "rtalus" => "rtibia",
        "lhumerusx" | "lradius" => "lhumerus",
        "rhumerusx" | "rradius" => "rhumerus",
        "lradiusx" | "lhand" => "lradius",
        "rradiusx" | "rhand" => "rradius",
        "lhang_tag_bone" => "lhand",
        "rhang_tag_bone" => "rhand",
        "mc5" => "lhand",
        "mc7" => "rhand",
        _ => {
            if let Some((digit, joint)) = name
                .rsplit_once("_j")
                .filter(|_| name.starts_with("l_d") || name.starts_with("r_d"))
            {
                if let Ok(joint) = joint.parse::<usize>() {
                    let parent = if joint > 1 {
                        format!("{digit}_j{}", joint - 1)
                    } else {
                        format!("{}hand", &name[..1])
                    };
                    if let Some(found) = index(bones, &parent) {
                        return Some(found);
                    }
                }
            }
            return usize::try_from(bones[bone].parent).ok();
        }
    };
    index(bones, parent).or_else(|| usize::try_from(bones[bone].parent).ok())
}

/// Physical ancestry for canonical native JK2 meshes. The original GLA
/// animation hierarchy is never modified. Cache the anatomical indices once
/// so Source rebasing and ragdolls avoid per-frame string mappings. Other rigs
/// retain their authored ancestry exactly.
pub fn mesh_pose_parent(bones: &[GlaBone], bone: usize) -> Option<usize> {
    bones.get(bone)?;
    let canonical_jk2 = bones.len() == 72
        && [
            (7, "ltarsal"),
            (31, "rhand"),
            (54, "lhand"),
            (55, "mc5"),
            (71, "face"),
        ]
        .iter()
        .all(|(index, name)| bones[*index].name.eq_ignore_ascii_case(name));
    if canonical_jk2 {
        static PARENTS: OnceLock<Vec<Option<usize>>> = OnceLock::new();
        let parents = PARENTS.get_or_init(|| {
            let reference = humanoid_reference(HumanoidRig::Jk2_72);
            (0..72)
                .map(|i| anatomical_parent(&reference.skeleton, i))
                .collect()
        });
        parents[bone]
    } else {
        usize::try_from(bones[bone].parent).ok()
    }
}

pub(crate) fn source_for_target(
    source: &[GlaBone],
    target: &[GlaBone],
    bone: usize,
) -> Option<usize> {
    let name = target[bone].name.to_ascii_lowercase();
    if let Some(found) = index(source, &name) {
        return Some(found);
    }
    // Missing JK2 digits follow the corresponding reduced JKA digit, retaining
    // their own bind offsets. Terminal joints keep the last supported curl.
    if let Some((digit, joint)) = name.rsplit_once("_j") {
        let digit = digit.replace("_d3", "_d2").replace("_d5", "_d4");
        let joint = if joint == "3" { "2" } else { joint };
        if let Some(found) = index(source, &format!("{digit}_j{joint}")) {
            return Some(found);
        }
    }
    let mut ancestor = anatomical_parent(target, bone);
    for _ in 0..target.len() {
        let parent = ancestor?;
        if let Some(found) = index(source, &target[parent].name) {
            return Some(found);
        }
        ancestor = anatomical_parent(target, parent);
    }
    None
}

fn validate_skeleton(bones: &[GlaBone]) -> Result<(), String> {
    if bones.is_empty() {
        return Err("empty pose skeleton".into());
    }
    let mut names = std::collections::HashSet::new();
    for (i, bone) in bones.iter().enumerate() {
        if !names.insert(bone.name.to_ascii_lowercase()) {
            return Err("duplicate pose bone name".into());
        }
        if !bone
            .base_pose
            .iter()
            .flatten()
            .chain(bone.base_pose_inv.iter().flatten())
            .all(|v| v.is_finite())
        {
            return Err("non-finite pose bind matrix".into());
        }
        let product = multiply_3x4(&bone.base_pose, &bone.base_pose_inv);
        for r in 0..3 {
            for c in 0..4 {
                let expected = if r == c { 1.0 } else { 0.0 };
                if (product[r][c] - expected).abs() > 0.02 {
                    return Err("invalid pose inverse bind".into());
                }
            }
        }
        let mut parent = bone.parent;
        for step in 0..=bones.len() {
            if parent == -1 {
                break;
            }
            let p = usize::try_from(parent).map_err(|_| "invalid pose parent")?;
            if p >= bones.len() || p == i || step == bones.len() {
                return Err("invalid/cyclic pose hierarchy".into());
            }
            parent = bones[p].parent;
        }
    }
    Ok(())
}

impl PoseRetargeter {
    pub fn new(source: &[GlaBone], target: &[GlaBone]) -> Result<Self, String> {
        validate_skeleton(source)?;
        validate_skeleton(target)?;
        let mappings = (0..target.len())
            .map(|bone| {
                let from = source_for_target(source, target, bone)
                    .ok_or_else(|| format!("no pose correspondence for {}", target[bone].name))?;
                Ok(JointMapping {
                    source: from,
                    bind_correction: multiply_3x4(
                        &source[from].base_pose_inv,
                        &target[bone].base_pose,
                    ),
                    target_inverse_bind: target[bone].base_pose_inv,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            source_bind: source.iter().map(|b| b.base_pose).collect(),
            mappings,
        })
    }

    /// Reuse an output buffer; no hierarchy walks, name lookups or inversions
    /// happen during pose conversion. Both directions use this same API.
    pub fn retarget_into(
        &self,
        input: PoseInput<'_>,
        output: &mut Vec<Matrix3x4>,
    ) -> Result<(), String> {
        let (matrices, skin) = match input {
            PoseInput::ModelSpaceJoints(p) => (p, false),
            PoseInput::Ghoul2SkinMatrices(p) => (p, true),
        };
        if matrices.len() != self.source_bind.len() {
            return Err("pose input bone count mismatch".into());
        }
        if !matrices.iter().flatten().flatten().all(|v| v.is_finite()) {
            return Err("non-finite pose input".into());
        }
        output.resize(self.mappings.len(), [[0.0; 4]; 3]);
        for (out, mapping) in output.iter_mut().zip(&self.mappings) {
            let joint = if skin {
                multiply_3x4(&matrices[mapping.source], &self.source_bind[mapping.source])
            } else {
                matrices[mapping.source]
            };
            let target_joint = multiply_3x4(&joint, &mapping.bind_correction);
            *out = multiply_3x4(&target_joint, &mapping.target_inverse_bind);
        }
        Ok(())
    }

    pub fn retarget(&self, input: PoseInput<'_>) -> Result<Vec<Matrix3x4>, String> {
        let mut output = Vec::with_capacity(self.mappings.len());
        self.retarget_into(input, &mut output)?;
        Ok(output)
    }
}

fn skeleton_key(bones: &[GlaBone]) -> Vec<u8> {
    let mut key = Vec::new();
    for b in bones {
        key.extend_from_slice(b.name.to_ascii_lowercase().as_bytes());
        key.push(0);
        key.extend_from_slice(&b.parent.to_le_bytes());
        for value in b
            .base_pose
            .iter()
            .flatten()
            .chain(b.base_pose_inv.iter().flatten())
        {
            key.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    key
}

/// Share plans across skins/models and asset worker jobs. Cache keys include
/// the actual bind transforms and hierarchy, not just bone count or rig name.
pub fn cached_retargeter(
    source: &[GlaBone],
    target: &[GlaBone],
) -> Result<Arc<PoseRetargeter>, String> {
    type Cache = HashMap<(Vec<u8>, Vec<u8>), Weak<PoseRetargeter>>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let key = (skeleton_key(source), skeleton_key(target));
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| "pose cache poisoned")?;
    if let Some(plan) = cache.get(&key).and_then(Weak::upgrade) {
        return Ok(plan);
    }
    let plan = Arc::new(PoseRetargeter::new(source, target)?);
    cache.retain(|_, value| value.strong_count() > 0);
    cache.insert(key, Arc::downgrade(&plan));
    Ok(plan)
}

#[cfg(test)]
mod tests;
