use super::*;
use crate::ghoul2::{self, Ghoul2Animator, OPENJK_GHOUL2_ROOT_MATRIX};

const IDENTITY: Matrix3x4 = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];

fn close(a: &Matrix3x4, b: &Matrix3x4, tolerance: f32) {
    for r in 0..3 {
        for c in 0..4 {
            assert!(
                (a[r][c] - b[r][c]).abs() < tolerance,
                "[{r}][{c}] {} != {}",
                a[r][c],
                b[r][c]
            );
        }
    }
}

/// A composed synthetic animation, with arm swing, wrist roll, independent
/// fingers and toe flexion. Anatomical FK produces joint-space input even
/// though JK2 stores most limbs as animation-hierarchy siblings.
fn synthetic_pose(rig: &GlaAnimation, time: f32) -> Vec<Matrix3x4> {
    let mut skins = vec![OPENJK_GHOUL2_ROOT_MATRIX; rig.num_bones()];
    let mut complete = vec![false; rig.num_bones()];
    fn visit(
        i: usize,
        rig: &GlaAnimation,
        time: f32,
        skins: &mut [Matrix3x4],
        complete: &mut [bool],
    ) {
        if complete[i] {
            return;
        }
        let parent = anatomical_parent(&rig.skeleton, i);
        let inherited = if let Some(p) = parent {
            visit(p, rig, time, skins, complete);
            skins[p]
        } else {
            OPENJK_GHOUL2_ROOT_MATRIX
        };
        let name = rig.skeleton[i].name.as_str();
        let angle = if name == "rhumerus" {
            0.6 * time.sin()
        } else if name == "rhand" {
            0.35 * (2. * time).sin()
        } else if name.contains("_d") {
            let digit = name.as_bytes()[3] - b'0';
            let joint = name.as_bytes()[6] - b'0';
            (0.1 + digit as f32 * 0.06) * (time + joint as f32 * 0.4).sin()
        } else if name.ends_with("tarsal") {
            0.2 * time.sin()
        } else {
            0.0
        };
        let (s, c) = angle.sin_cos();
        let pivot = rig.skeleton[i].base_pose.map(|row| row[3]);
        let mut turn = [[c, -s, 0., 0.], [s, c, 0., 0.], [0., 0., 1., 0.]];
        let rotated = ghoul2::transform_point(&turn, pivot);
        for k in 0..3 {
            turn[k][3] = pivot[k] - rotated[k];
        }
        skins[i] = multiply_3x4(&inherited, &turn);
        complete[i] = true;
    }
    for i in 0..rig.num_bones() {
        visit(i, rig, time, &mut skins, &mut complete);
    }
    skins
        .iter()
        .zip(&rig.skeleton)
        .map(|(s, b)| multiply_3x4(s, &b.base_pose))
        .collect()
}

#[test]
fn actual_references_have_fifty_common_names_and_different_hierarchies() {
    let jka = humanoid_reference(HumanoidRig::Jka53);
    let jk2 = humanoid_reference(HumanoidRig::Jk2_72);
    assert_eq!((jka.num_bones(), jk2.num_bones()), (53, 72));
    assert_eq!(
        jka.skeleton
            .iter()
            .filter(|b| index(&jk2.skeleton, &b.name).is_some())
            .count(),
        50
    );
    for rig in [&jka, &jk2] {
        validate_skeleton(&rig.skeleton).unwrap();
    }
    let p = |rig: &GlaAnimation, name: &str| {
        rig.skeleton[rig.skeleton[index(&rig.skeleton, name).unwrap()].parent as usize]
            .name
            .clone()
    };
    assert_eq!(p(&jka, "rhand"), "rradius");
    assert_eq!(p(&jk2, "rhand"), "thoracic");
}

#[test]
fn synthetic_72_joint_sequence_animates_both_rigs_without_wrist_or_foot_contamination() {
    let jk2 = humanoid_reference(HumanoidRig::Jk2_72);
    let jka = humanoid_reference(HumanoidRig::Jka53);
    let down = cached_retargeter(&jk2.skeleton, &jka.skeleton).unwrap();
    let same = cached_retargeter(&jk2.skeleton, &jk2.skeleton).unwrap();
    let up = cached_retargeter(&jka.skeleton, &jk2.skeleton).unwrap();
    assert!(Arc::ptr_eq(
        &down,
        &cached_retargeter(&jk2.skeleton, &jka.skeleton).unwrap()
    ));
    let mut output = Vec::with_capacity(72);
    for frame in 0..32 {
        let joints = synthetic_pose(&jk2, frame as f32 * 0.2);
        down.retarget_into(PoseInput::ModelSpaceJoints(&joints), &mut output)
            .unwrap();
        assert_eq!(output.len(), 53);
        let native = same.retarget(PoseInput::ModelSpaceJoints(&joints)).unwrap();
        let roundtrip = up.retarget(PoseInput::Ghoul2SkinMatrices(&output)).unwrap();
        for (i, b) in jka.skeleton.iter().enumerate() {
            if let Some(j) = index(&jk2.skeleton, &b.name) {
                close(&output[i], &native[j], 0.002);
                close(&roundtrip[j], &native[j], 0.004);
            }
        }
        for name in ["model_root", "ltalus", "rtalus"] {
            close(
                &output[index(&jka.skeleton, name).unwrap()],
                &OPENJK_GHOUL2_ROOT_MATRIX,
                0.002,
            );
        }
        for matrix in output.iter().chain(&native) {
            assert!(matrix.iter().flatten().all(|v| v.is_finite()));
            let determinant = matrix[0][0]
                * (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
                - matrix[0][1] * (matrix[1][0] * matrix[2][2] - matrix[1][2] * matrix[2][0])
                + matrix[0][2] * (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]);
            assert!((determinant - 1.).abs() < 0.002);
        }
        // Changing an unsupported fingertip must leave the supported hand,
        // forearm, root and hang/saber tag exactly unchanged.
        let mut changed = joints.clone();
        changed[index(&jk2.skeleton, "r_d5_j3").unwrap()][0][3] += 100.;
        let after = down
            .retarget(PoseInput::ModelSpaceJoints(&changed))
            .unwrap();
        assert_eq!(after, output);
    }
    assert_eq!(output.capacity(), 72);
}

#[test]
fn missing_joints_follow_anatomical_motion_instead_of_flat_jk2_parent() {
    let jka = humanoid_reference(HumanoidRig::Jka53);
    let jk2 = humanoid_reference(HumanoidRig::Jk2_72);
    let up = PoseRetargeter::new(&jka.skeleton, &jk2.skeleton).unwrap();
    let mut skin = vec![IDENTITY; 53];
    skin[index(&jka.skeleton, "ltalus").unwrap()][0][3] = 7.;
    skin[index(&jka.skeleton, "r_d2_j2").unwrap()][1][3] = 3.;
    let native = up.retarget(PoseInput::Ghoul2SkinMatrices(&skin)).unwrap();
    close(
        &native[index(&jk2.skeleton, "ltarsal").unwrap()],
        &skin[index(&jka.skeleton, "ltalus").unwrap()],
        0.002,
    );
    close(
        &native[index(&jk2.skeleton, "r_d3_j3").unwrap()],
        &skin[index(&jka.skeleton, "r_d2_j2").unwrap()],
        0.002,
    );
    close(
        &native[index(&jk2.skeleton, "rhand").unwrap()],
        &IDENTITY,
        0.002,
    );
}

#[test]
fn rejects_invalid_pose_and_reference_data() {
    let rig = humanoid_reference(HumanoidRig::Jk2_72);
    let plan = PoseRetargeter::new(&rig.skeleton, &rig.skeleton).unwrap();
    assert!(plan.retarget(PoseInput::ModelSpaceJoints(&[])).is_err());
    let mut pose = vec![IDENTITY; 72];
    pose[10][1][2] = f32::NAN;
    assert!(plan.retarget(PoseInput::Ghoul2SkinMatrices(&pose)).is_err());
    let mut bones = rig.skeleton.clone();
    bones[1].parent = 1;
    assert!(PoseRetargeter::new(&bones, &rig.skeleton).is_err());
    bones = rig.skeleton.clone();
    bones[1].base_pose_inv = [[0.; 4]; 3];
    assert!(PoseRetargeter::new(&bones, &rig.skeleton).is_err());
}

#[test]
fn skin_and_joint_inputs_agree_with_real_ghoul2_evaluator_root_and_overrides() {
    let source = humanoid_reference(HumanoidRig::Jka53);
    let target = humanoid_reference(HumanoidRig::Jk2_72);
    let plan = PoseRetargeter::new(&source.skeleton, &target.skeleton).unwrap();
    let mut animator = Ghoul2Animator::new(&source);
    animator
        .set_bone_angles_postmult(&source, "lower_lumbar", [12., 5., 0.], 3, 4, 5)
        .unwrap();
    let skin = animator.evaluate_pose_openjk_root(&source, 0).unwrap();
    let joints: Vec<_> = skin
        .iter()
        .zip(&source.skeleton)
        .map(|(s, b)| multiply_3x4(s, &b.base_pose))
        .collect();
    assert_eq!(
        plan.retarget(PoseInput::Ghoul2SkinMatrices(&skin)).unwrap(),
        plan.retarget(PoseInput::ModelSpaceJoints(&joints)).unwrap()
    );
}

#[test]
#[ignore = "requires supplied Development Docs/Models and tools/model-converter/temp.pk3"]
fn supplied_kyles_and_converted_source_skin_synthetic_72_sequence() {
    use crate::source1::{parse_source1_materials, parse_source1_skeleton};
    use crate::source1_geometry::{parse_source_vtx, parse_source_vvd};
    use crate::source1_rig::{
        attach_humanoid_bolts_between_rigs, build_source_rig, rebase_source_ghoul2_pose,
        SourceRigOptions,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &str| std::fs::read(root.join(path)).unwrap();
    let jka_bytes = read("Development Docs/Models/jka _humanoid.gla");
    let source = ghoul2::parse_gla(&jka_bytes).unwrap();
    let supplied72 = ghoul2::parse_gla(&read("Development Docs/Models/jk2 _humanoid.gla")).unwrap();
    let jka = humanoid_reference(HumanoidRig::Jka53);
    let jk2 = humanoid_reference(HumanoidRig::Jk2_72);
    assert_eq!(skeleton_key(&source.skeleton), skeleton_key(&jka.skeleton));
    assert_eq!(
        skeleton_key(&supplied72.skeleton),
        skeleton_key(&jk2.skeleton)
    );
    let kyle53 =
        ghoul2::parse_glm_native(&read("Development Docs/Models/Kyle JKA model.glm")).unwrap();
    let kyle72 =
        ghoul2::parse_glm_native(&read("Development Docs/Models/Kyle JK2 model.glm")).unwrap();
    assert_eq!((kyle53.num_bones, kyle72.num_bones), (53, 72));
    let base = "models/player/fwhykay/lollipop_chainsaw/juliet_starling_cine/juliet_starling_cheerleader/juliet_starling_cheerleader";
    let mut archive = zip::ZipArchive::new(
        std::fs::File::open(root.join("tools/model-converter/temp.pk3")).unwrap(),
    )
    .unwrap();
    let mut read_source = |name: &str| {
        use std::io::Read;
        let mut bytes = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    };
    let mdl = read_source(&format!("{base}.mdl"));
    let skeleton = parse_source1_skeleton(&mdl).unwrap();
    let materials = parse_source1_materials(&mdl).unwrap();
    let vvd = parse_source_vvd(&read_source(&format!("{base}.vvd"))).unwrap();
    let vtx = parse_source_vtx(&read_source(&format!("{base}.dx90.vtx"))).unwrap();
    let mut converted = build_source_rig(
        base,
        &mdl,
        &vvd,
        &vtx,
        &skeleton,
        &materials,
        &jk2,
        &SourceRigOptions::default(),
    )
    .unwrap();
    attach_humanoid_bolts_between_rigs(&mut converted, &kyle53, &jka, &jk2).unwrap();
    assert_eq!(converted.glm.num_bones, 72);
    for name in ["r_d3_j1", "r_d4_j1", "r_d5_j1", "r_d2_j3", "ltarsal"] {
        let bone = index(&jk2.skeleton, name).unwrap();
        assert!(
            converted.glm.lods[0]
                .surfaces
                .iter()
                .any(|s| s.bone_references.contains(&bone)),
            "no Source influences on {name}"
        );
    }
    let down = PoseRetargeter::new(&jk2.skeleton, &jka.skeleton).unwrap();
    let same = PoseRetargeter::new(&jk2.skeleton, &jk2.skeleton).unwrap();
    let up = PoseRetargeter::new(&source.skeleton, &jk2.skeleton).unwrap();
    let mut first_positions: Vec<Vec<[f32; 3]>> = Vec::new();
    let mut first_feet = Vec::new();
    let capture = std::env::var_os("JKA_SKELETON_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &capture {
        std::fs::create_dir_all(path).unwrap();
    }
    for frame in 0..32 {
        let joints = synthetic_pose(&jk2, frame as f32 * 0.2);
        let pose53 = down.retarget(PoseInput::ModelSpaceJoints(&joints)).unwrap();
        let pose72 = same.retarget(PoseInput::ModelSpaceJoints(&joints)).unwrap();
        let source_pose =
            rebase_source_ghoul2_pose(&jk2, &pose72, converted.native_pivots.as_ref().unwrap())
                .unwrap();
        for (n, (model, rig, pose)) in [
            (&kyle53, &jka, &pose53),
            (&kyle72, &jk2, &pose72),
            (&converted.glm, &jk2, &source_pose),
        ]
        .iter()
        .enumerate()
        {
            if let Some(path) = &capture {
                // Optional review artifacts use the exact CPU skin result,
                // without mutating the assets or adding a runtime player.
                use std::io::Write;
                let mut file = std::io::BufWriter::new(
                    std::fs::File::create(path.join(format!("model{n}-frame{frame:02}.obj")))
                        .unwrap(),
                );
                let mut offset = 1u32;
                for surface in &model.lods[0].surfaces {
                    let hierarchy = &model.hierarchy[surface.surface_index];
                    if hierarchy.name.starts_with('*') || hierarchy.name.ends_with("_off") {
                        continue;
                    }
                    let skinned = ghoul2::skin_glm_surface(surface, pose).unwrap();
                    for v in &skinned.vertices {
                        writeln!(
                            file,
                            "v {} {} {}",
                            v.position[0], v.position[1], v.position[2]
                        )
                        .unwrap();
                    }
                    for t in &surface.triangles {
                        writeln!(
                            file,
                            "f {} {} {}",
                            t[0] + offset,
                            t[1] + offset,
                            t[2] + offset
                        )
                        .unwrap();
                    }
                    offset += skinned.vertices.len() as u32;
                }
            }
            let mut positions = Vec::new();
            for lod in &model.lods {
                for surface in &lod.surfaces {
                    let skinned = ghoul2::skin_glm_surface(surface, pose).unwrap();
                    assert!(skinned.vertices.iter().all(|v| v
                        .position
                        .iter()
                        .chain(&v.normal)
                        .all(|x| x.is_finite())));
                    positions.extend(skinned.vertices.into_iter().map(|v| v.position));
                }
            }
            assert!(!positions.is_empty());
            if frame == 0 {
                first_positions.push(positions);
            } else if frame == 8 {
                assert_ne!(first_positions[n], positions, "model {n} failed to animate");
            }
            for tag in ["*r_hand", "*l_leg_foot", "*r_leg_foot"] {
                let bolt = ghoul2::model_bolt_matrix(model, rig, pose, tag)
                    .unwrap()
                    .expect(tag);
                assert!(bolt.iter().flatten().all(|v| v.is_finite()));
            }
            let feet = ["*l_leg_foot", "*r_leg_foot"].map(|tag| {
                ghoul2::model_bolt_matrix(model, rig, pose, tag)
                    .unwrap()
                    .unwrap()
            });
            if frame == 0 {
                first_feet.push(feet);
            } else {
                for (a, b) in feet.iter().zip(&first_feet[n]) {
                    close(a, b, 0.003);
                }
            }
            // The authored tag frame remains fixed relative to its wrist as
            // the arm swings and the hand rolls through the synthetic clip.
            let hand = index(&rig.skeleton, "rhand").unwrap();
            let bolt = ghoul2::model_bolt_matrix(model, rig, pose, "*r_hand")
                .unwrap()
                .unwrap();
            let neutral =
                ghoul2::model_bolt_matrix(model, rig, &vec![IDENTITY; rig.num_bones()], "*r_hand")
                    .unwrap()
                    .unwrap();
            let expected = multiply_3x4(&pose[hand], &neutral);
            close(&bolt, &expected, 0.008);
        }
        // Source native pivots carry the hand and saber together; independent
        // unsupported finger details cannot affect either Kyle's wrist tags.
        let mut changed = joints.clone();
        changed[index(&jk2.skeleton, "r_d5_j3").unwrap()][0][3] += 100.;
        let changed53 = down
            .retarget(PoseInput::ModelSpaceJoints(&changed))
            .unwrap();
        assert_eq!(
            ghoul2::model_bolt_matrix(&kyle53, &jka, &pose53, "*r_hand").unwrap(),
            ghoul2::model_bolt_matrix(&kyle53, &jka, &changed53, "*r_hand").unwrap()
        );
    }
    // Gameplay frames and postmult overrides remain on the original evaluator.
    // Native conversion preserves all common joint deformation, independent of
    // how JK2 stores its animation ancestry.
    for frame in [0, 100, source.num_frames - 1] {
        let stock = source.compose_frame(frame).unwrap();
        let native = up.retarget(PoseInput::Ghoul2SkinMatrices(&stock)).unwrap();
        for (i, b) in source.skeleton.iter().enumerate() {
            if let Some(j) = index(&jk2.skeleton, &b.name) {
                close(&stock[i], &native[j], 0.003);
            }
        }
    }
    for frame in [0, 100, supplied72.num_frames - 1] {
        let decoded = supplied72.compose_frame(frame).unwrap();
        let reduced = down
            .retarget(PoseInput::Ghoul2SkinMatrices(&decoded))
            .unwrap();
        for surface in &kyle53.lods[0].surfaces {
            assert!(ghoul2::skin_glm_surface(surface, &reduced)
                .unwrap()
                .vertices
                .iter()
                .all(|v| v.position.iter().all(|x| x.is_finite())));
        }
    }
    println!("32 synthetic 72-bone frames: stock JKA Kyle, native JK2 Kyle, native Source Juliet; gameplay frames 0/100/last preserve common joint motion");
}
