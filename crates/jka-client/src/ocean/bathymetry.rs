//! Checkpoint 1 only: asynchronous static geometry, no changes to FFT/rendering.
use crate::scene::{DrawBatch, GpuVertex};
use jka_assets::bsp::{
    bathymetry::{self, Atlas, Geometry, Options, Region},
    Bsp,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

pub enum Status {
    Pending,
    Ready {
        atlas: Arc<Atlas>,
        cache_hit: bool,
        total_ms: f64,
        report: PathBuf,
        report_error: Option<String>,
    },
    Failed(String),
}

pub struct Job {
    pub status: Mutex<Status>,
    /// CPU selection/descriptor work on the loader; geometry and IO are excluded.
    pub selection_ms: f64,
}

/// This runs on the map loader, uses the exact uploader's faces/clustering/cap,
/// then hands all geometry work and cache IO to a dedicated background thread.
pub fn start(
    bsp: Arc<Bsp>,
    vertices: &[GpuVertex],
    coarse: &[DrawBatch],
    full: &[DrawBatch],
    map_hash: u64,
    base: &Path,
    game: Option<&Path>,
) -> Option<Arc<Job>> {
    let selection_started = Instant::now();
    let entries = super::surfaces::collect(vertices, coarse, full);
    let faces: Vec<_> = entries.iter().map(|e| e.face).collect();
    let clusters = super::cluster_water_faces(&faces);
    let mut regions = Vec::new();
    let mut warnings = Vec::new();
    for (slot, cluster) in clusters.iter().take(super::OCEAN_MAX_SURFACES).enumerate() {
        if faces[cluster.members[0]].authored_ocean.is_some() {
            continue;
        }
        let planar = cluster.members.iter().all(|&i| {
            let s = entries[i].source;
            vertices[s.vertices.start as usize..s.vertices.end as usize]
                .iter()
                .all(|v| (v.position[1] - cluster.plane).abs() < 0.01)
        });
        if !planar {
            warnings.push(format!(
                "slot {slot}: nonplanar promoted surface; retain legacy water"
            ));
            continue;
        }
        regions.push(Region {
            slot: slot as u32,
            level: cluster.plane,
            triangles: cluster
                .members
                .iter()
                .flat_map(|&i| entries[i].triangles.iter().map(|t| t.map(|[x, z]| [x, -z])))
                .collect(),
        });
    }
    if regions.is_empty() && warnings.is_empty() {
        return None;
    }
    let options = Options::default();
    let mut descriptor = Vec::new();
    for v in [
        options.cell_size.to_bits(),
        options.padding_cells,
        options.max_cells_per_region as u32,
    ] {
        descriptor.extend(v.to_le_bytes());
    }
    // Rejections also form part of identity. A cache from a previously eligible
    // shader/footprint must not survive a changed runtime material definition.
    for warning in &warnings {
        descriptor.extend(warning.as_bytes());
        descriptor.push(0);
    }
    for r in &regions {
        descriptor.extend(r.slot.to_le_bytes());
        descriptor.extend(r.level.to_le_bytes());
        descriptor.extend((r.triangles.len() as u32).to_le_bytes());
        for v in r.triangles.iter().flatten().flatten() {
            descriptor.extend(v.to_le_bytes());
        }
    }
    let key = [map_hash, bathymetry::fingerprint(&descriptor)];
    let directory = game.unwrap_or(base).join("cache/coastal").join(format!(
        "v{}-{:016x}-{:016x}",
        bathymetry::CACHE_VERSION,
        key[0],
        key[1]
    ));
    let job = Arc::new(Job {
        status: Mutex::new(Status::Pending),
        selection_ms: selection_started.elapsed().as_secs_f64() * 1000.0,
    });
    let output = Arc::clone(&job);
    let spawned = std::thread::Builder::new()
        .name("jka-coastal-bake".into())
        .spawn(move || {
            let started = Instant::now();
            let work = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<Status, String> {
                    let file = directory.join("bathymetry.bin");
                    let cached = std::fs::metadata(&file)
                        .ok()
                        .filter(|m| m.len() <= 128 * 1024 * 1024)
                        .and_then(|_| std::fs::read(&file).ok())
                        .and_then(|b| Atlas::decode(&b, key).ok());
                    let cache_hit = cached.is_some();
                    let mut atlas = if let Some(atlas) = cached {
                        atlas
                    } else {
                        let geometry = Geometry::from_bsp(&bsp)?;
                        let mut atlas = geometry.bake(&regions, options)?;
                        atlas.warnings.extend(warnings);
                        atlas.bake_ms = started.elapsed().as_secs_f64() * 1000.0;
                        atlas
                    };
                    let mut report_error = None;
                    if !cache_hit {
                        let save = (|| -> std::io::Result<()> {
                            std::fs::create_dir_all(&directory)?;
                            let temporary =
                                directory.join(format!("bathymetry-{}.tmp", std::process::id()));
                            std::fs::write(&temporary, atlas.encode(key))?;
                            std::fs::rename(&temporary, &file)
                        })();
                        if let Err(e) = save {
                            atlas.warnings.push(format!("cache write failed: {e}"));
                        }
                    }
                    if let Err(e) = atlas.write_debug(&directory) {
                        report_error = Some(e);
                    }
                    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
                    eprintln!(
                        "COASTAL GEOMETRY: {} grids; {} bytes; {:.2} ms {}; report {}",
                        atlas.grids.len(),
                        atlas.memory_bytes(),
                        total_ms,
                        if cache_hit {
                            "cache hit"
                        } else {
                            "background bake"
                        },
                        directory.join("bathymetry.html").display()
                    );
                    Ok(Status::Ready {
                        atlas: Arc::new(atlas),
                        cache_hit,
                        total_ms,
                        report: directory.join("bathymetry.html"),
                        report_error,
                    })
                },
            ));
            let status = match work {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => Status::Failed(e),
                Err(_) => Status::Failed("bathymetry bake panicked; legacy water retained".into()),
            };
            if let Ok(mut output) = output.status.lock() {
                *output = status;
            }
        });
    if let Err(e) = spawned {
        if let Ok(mut status) = job.status.lock() {
            *status = Status::Failed(format!("could not start coastal worker: {e}"));
        }
    }
    Some(job)
}

impl Job {
    pub fn report_lines(&self) -> Vec<String> {
        let Ok(status) = self.status.try_lock() else {
            return vec!["Coastal geometry worker is publishing its result.".into()];
        };
        match &*status {
            Status::Pending => vec!["Coastal geometry bake is running in the background.".into()],
            Status::Failed(e) => vec![format!("Coastal geometry unavailable: {e}")],
            Status::Ready {
                atlas,
                cache_hit,
                total_ms,
                report,
                report_error,
            } => {
                let mut lines = vec![format!(
                    "Coastal geometry: {} grids; {:.3} MiB; {:.2} ms {}; selection {:.3} ms; report {}",
                    atlas.grids.len(),
                    atlas.memory_bytes() as f64 / 1048576.0,
                    total_ms,
                    if *cache_hit {
                        "cache hit"
                    } else {
                        "background bake"
                    },
                    self.selection_ms,
                    report.display()
                )];
                lines.extend(atlas.summary().lines().map(str::to_owned));
                if let Some(e) = report_error {
                    lines.push(format!("Report write failed: {e}"));
                }
                lines
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises real runtime shader resolution and PVS batches, rather than
    /// the probe's raw CONTENTS_WATER discovery. Requires the user's local assets.
    #[test]
    #[ignore = "requires target/release/base retail and RaceArena assets"]
    fn coastal_checkpoint_real_runtime_maps() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base = workspace.join("target/release/base");
        for map_name in ["racearena_pro", "yavin1"] {
            let options = crate::scene::MapPrepareOptions {
                ocean: true,
                grass: false,
                voxel_probe_gi: false,
                client_physics: false,
                gen_normal_maps: false,
                steam_audio: false,
                planar_reflections: false,
                planar_environment: false,
                ..Default::default()
            };
            let started = Instant::now();
            let map = crate::scene::prepare_with_options(&base, None, map_name, options)
                .expect("prepare real map");
            let prepare_ms = started.elapsed().as_secs_f64() * 1000.0;
            let job = map
                .coastal_bathymetry
                .as_ref()
                .expect("promoted-water background job");
            let deadline = Instant::now() + std::time::Duration::from_secs(60);
            loop {
                let status = job.status.lock().unwrap();
                match &*status {
                    Status::Pending => {}
                    Status::Failed(e) => panic!("{map_name}: {e}"),
                    Status::Ready {
                        atlas,
                        cache_hit,
                        total_ms,
                        ..
                    } => {
                        assert!(!atlas.grids.is_empty());
                        assert!(atlas
                            .grids
                            .iter()
                            .flat_map(|g| &g.cells)
                            .any(|c| c.class == bathymetry::Class::Wet));
                        let report = workspace
                            .join("target/coastal-checkpoint1/runtime")
                            .join(map_name);
                        atlas.write_debug(&report).expect("write runtime results");
                        std::fs::write(report.join("runtime.txt"),format!("Runtime shader/PVS discovery\nMap: {map_name}\nPrepare returned in {prepare_ms:.3} ms without waiting for the bake\nCoastal selection on loader: {:.3} ms\nWorker including cache/report IO: {total_ms:.3} ms\nCache hit: {cache_hit}\n",job.selection_ms)).unwrap();
                        eprintln!("{map_name}: prepare={prepare_ms:.3} ms, worker={total_ms:.3} ms, cache_hit={cache_hit}\n{}",atlas.summary());
                        break;
                    }
                }
                drop(status);
                assert!(
                    Instant::now() < deadline,
                    "coastal geometry worker timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}
