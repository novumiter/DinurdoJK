//! Dedicated presentation-only jiggle solver. All spring integration runs off
//! the game/render thread. The renderer consumes immutable completed offsets.
use super::jiggle::{JiggleProfile, JiggleSystem, JiggleTuning};
use jka_assets::ghoul2::Matrix3x4;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    mpsc::{self, Receiver, SyncSender, TryRecvError},
    Arc,
};

const QUEUE_CAPACITY: usize = 32;
const MAX_RESULT_AGE_MS: i32 = 100;

struct Request {
    epoch: u64,
    entity: u16,
    model: String,
    profile: Arc<JiggleProfile>,
    pose: Vec<Matrix3x4>,
    axis: [[f32; 3]; 3],
    origin: [f32; 3],
    now_ms: i32,
    enabled: bool,
    hz: u32,
    max_substeps: u32,
    tuning: JiggleTuning,
}

struct ResultFrame {
    epoch: u64,
    entity: u16,
    model: String,
    now_ms: i32,
    solved_time_ms: f64,
    offsets: Vec<[f32; 3]>,
}

struct FramePair {
    previous: Option<ResultFrame>,
    latest: ResultFrame,
}

pub(crate) struct AsyncJiggleSystem {
    // The main-thread instance is configuration only; it never integrates.
    settings: JiggleSystem,
    hz: u32,
    max_substeps: u32,
    tx: Option<SyncSender<Request>>,
    rx: Option<Receiver<ResultFrame>>,
    epoch: u64,
    generation: Arc<AtomicU64>,
    pending: HashSet<u16>,
    last_sent: HashMap<u16, i32>,
    frames: HashMap<u16, FramePair>,
    profiles: HashMap<String, Arc<JiggleProfile>>,
}

impl Default for AsyncJiggleSystem {
    fn default() -> Self {
        Self {
            settings: JiggleSystem::default(),
            hz: 60,
            max_substeps: 4,
            tx: None,
            rx: None,
            epoch: 0,
            generation: Arc::new(AtomicU64::new(0)),
            pending: HashSet::new(),
            last_sent: HashMap::new(),
            frames: HashMap::new(),
            profiles: HashMap::new(),
        }
    }
}

impl AsyncJiggleSystem {
    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.generation.store(self.epoch, Ordering::Release);
        self.pending.clear();
        self.last_sent.clear();
        self.frames.clear();
    }

    fn ensure_worker(&mut self) {
        if self.tx.is_some() {
            return;
        }
        let (tx, rx_work) = mpsc::sync_channel::<Request>(QUEUE_CAPACITY);
        let (tx_done, rx) = mpsc::channel::<ResultFrame>();
        let generation = Arc::clone(&self.generation);
        std::thread::Builder::new()
            .name("jka-jiggle-physics".into())
            .spawn(move || Self::run_worker(rx_work, tx_done, generation))
            .expect("spawn JKA jiggle physics worker");
        self.tx = Some(tx);
        self.rx = Some(rx);
    }

    fn run_worker(
        rx_work: Receiver<Request>,
        tx_done: mpsc::Sender<ResultFrame>,
        generation: Arc<AtomicU64>,
    ) {
        let mut solver = JiggleSystem::default();
        let mut epoch: Option<u64> = None;
        let mut seen = HashMap::<u16, i32>::new();
        let mut last_prune = None::<i32>;
        while let Ok(job) = rx_work.recv() {
            if job.epoch != generation.load(Ordering::Acquire) {
                continue;
            }
            if epoch != Some(job.epoch) {
                solver.clear();
                seen.clear();
                last_prune = None;
                epoch = Some(job.epoch);
            }
            seen.insert(job.entity, job.now_ms);
            if last_prune.is_none_or(|last| job.now_ms < last || job.now_ms - last >= 1000) {
                seen.retain(|_, last| job.now_ms.saturating_sub(*last) <= 5000);
                let live = seen.keys().copied().collect::<HashSet<_>>();
                solver.retain_entities(&live);
                last_prune = Some(job.now_ms);
            }
            solver.set_config(job.enabled, job.hz, job.max_substeps, job.tuning);
            let snapshot = solver.simulate_snapshot(
                job.entity,
                &job.model,
                &job.profile,
                &job.pose,
                job.axis,
                job.origin,
                job.now_ms,
            );
            if job.epoch != generation.load(Ordering::Acquire) {
                continue;
            }
            if tx_done
                .send(ResultFrame {
                    epoch: job.epoch,
                    entity: job.entity,
                    model: job.model,
                    now_ms: job.now_ms,
                    solved_time_ms: snapshot.time_ms,
                    offsets: snapshot.offsets,
                })
                .is_err()
            {
                break;
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.invalidate();
        self.settings.clear();
        self.profiles.clear();
    }

    pub(crate) fn set_config(
        &mut self,
        enabled: bool,
        hz: u32,
        max_substeps: u32,
        tuning: JiggleTuning,
    ) {
        let hz = hz.clamp(30, 240);
        if (self.settings.enabled() && !enabled)
            || self.settings.tuning().solver != tuning.solver
            || self.hz != hz
        {
            self.clear();
        }
        self.settings.set_config(enabled, hz, max_substeps, tuning);
        self.hz = hz;
        self.max_substeps = max_substeps;
    }

    pub(crate) fn enabled(&self) -> bool {
        self.settings.enabled()
    }
    pub(crate) fn tuning(&self) -> JiggleTuning {
        self.settings.tuning()
    }

    pub(crate) fn retain_entities(&mut self, live: &HashSet<u16>) {
        self.frames.retain(|entity, _| live.contains(entity));
        self.last_sent.retain(|entity, _| live.contains(entity));
        // Worker state is also pruned after five seconds without work.
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(frame) => {
                    if frame.epoch != self.epoch {
                        continue;
                    }
                    self.pending.remove(&frame.entity);
                    let previous = self.frames.remove(&frame.entity).and_then(|pair| {
                        if pair.latest.model != frame.model {
                            return None;
                        }
                        if pair.latest.solved_time_ms < frame.solved_time_ms {
                            Some(pair.latest)
                        } else {
                            // A request can arrive before the next solver tick.
                            // Keep the interpolation history for that same solve.
                            pair.previous
                        }
                    });
                    self.frames.insert(
                        frame.entity,
                        FramePair {
                            previous,
                            latest: frame,
                        },
                    );
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn simulate(
        &mut self,
        entity: u16,
        model: &str,
        profile: &JiggleProfile,
        pose: &[Matrix3x4],
        axis: [[f32; 3]; 3],
        origin: [f32; 3],
        now_ms: i32,
    ) -> Vec<[f32; 3]> {
        if !self.enabled() {
            return vec![[0.0; 3]; profile.region_count()];
        }
        self.poll();
        let offsets = self.latest_offsets(entity, model, profile.region_count(), now_ms);
        let interval_ms = (1000 / self.hz.max(1)) as i32;
        let should_send = !self.pending.contains(&entity)
            && self
                .last_sent
                .get(&entity)
                .is_none_or(|last| now_ms < *last || now_ms - *last >= interval_ms);
        if should_send {
            self.ensure_worker();
            let profile = Arc::clone(
                self.profiles
                    .entry(model.to_owned())
                    .or_insert_with(|| Arc::new(profile.clone())),
            );
            let job = Request {
                epoch: self.epoch,
                entity,
                model: model.to_owned(),
                profile,
                pose: pose.to_vec(),
                axis,
                origin,
                now_ms,
                enabled: self.enabled(),
                hz: self.hz,
                max_substeps: self.max_substeps,
                tuning: self.tuning(),
            };
            if let Ok(()) = self.tx.as_ref().expect("worker initialized").try_send(job) {
                self.pending.insert(entity);
                self.last_sent.insert(entity, now_ms);
            }
        }
        offsets
    }

    fn latest_offsets(
        &self,
        entity: u16,
        model: &str,
        regions: usize,
        now_ms: i32,
    ) -> Vec<[f32; 3]> {
        let Some(pair) = self.frames.get(&entity) else {
            return vec![[0.0; 3]; regions];
        };
        let latest = &pair.latest;
        if latest.model != model
            || latest.offsets.len() != regions
            || now_ms < latest.now_ms
            || now_ms as f64 - latest.solved_time_ms > MAX_RESULT_AGE_MS as f64
        {
            return vec![[0.0; 3]; regions];
        }
        let previous = pair.previous.as_ref().filter(|p| {
            p.model == model
                && p.offsets.len() == regions
                && p.solved_time_ms < latest.solved_time_ms
        });
        let alpha = previous.map_or(1.0, |p| {
            let target = now_ms as f64 - 1000.0 / self.hz.max(1) as f64;
            ((target - p.solved_time_ms) / (latest.solved_time_ms - p.solved_time_ms))
                .clamp(0.0, 1.0) as f32
        });
        latest
            .offsets
            .iter()
            .enumerate()
            .map(|(i, value)| {
                if let Some(prior) = previous {
                    std::array::from_fn(|j| {
                        prior.offsets[i][j] + (value[j] - prior.offsets[i][j]) * alpha
                    })
                } else {
                    *value
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolated_offsets_do_not_use_future_or_stale_results() {
        let mut worker = AsyncJiggleSystem::default();
        worker.hz = 125; // Exact 8ms tick for assertions with integral timestamps.
        let snapshot = |time, value| ResultFrame {
            epoch: 0,
            entity: 7,
            model: "jan".into(),
            now_ms: time,
            solved_time_ms: time as f64,
            offsets: vec![[value, 0.0, 0.0]],
        };
        worker.frames.insert(
            7,
            FramePair {
                previous: Some(snapshot(0, 1.0)),
                latest: snapshot(16, 3.0),
            },
        );
        assert_eq!(worker.latest_offsets(7, "jan", 1, 8), vec![[0.0, 0.0, 0.0]]);
        assert_eq!(
            worker.latest_offsets(7, "jan", 1, 16),
            vec![[2.0, 0.0, 0.0]]
        );
        assert_eq!(
            worker.latest_offsets(7, "jan", 1, 32),
            vec![[3.0, 0.0, 0.0]]
        );
        assert_eq!(
            worker.latest_offsets(7, "kyle", 1, 32),
            vec![[0.0, 0.0, 0.0]]
        );
        assert_eq!(
            worker.latest_offsets(7, "jan", 1, 200),
            vec![[0.0, 0.0, 0.0]]
        );
        worker.clear();
        assert!(worker.frames.is_empty());
    }

    #[test]
    fn interpolation_uses_solve_time_and_one_exact_tick_of_delay() {
        let mut worker = AsyncJiggleSystem::default();
        let frame = |now_ms, solved_time_ms, value| ResultFrame {
            epoch: 0,
            entity: 7,
            model: "jan".into(),
            now_ms,
            solved_time_ms,
            offsets: vec![[value, 0.0, 0.0]],
        };
        worker.frames.insert(
            7,
            FramePair {
                previous: Some(frame(16, 0.0, 0.0)),
                latest: frame(32, 1000.0 / 60.0, 2.0),
            },
        );
        // At 32ms, the target is 15.333ms, close to the latest actual solve.
        // Using request timestamps or applying another delay gives a different result.
        let output = worker.latest_offsets(7, "jan", 1, 32);
        assert!((output[0][0] - 1.84).abs() < 0.00001);
        assert_eq!(
            worker.latest_offsets(7, "jan", 1, 34),
            vec![[2.0, 0.0, 0.0]]
        );
    }

    #[test]
    fn clear_skips_jobs_already_queued_before_the_worker_runs() {
        let mut worker = AsyncJiggleSystem::default();
        let (tx, requests) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (results, rx) = mpsc::channel();
        let profile = Arc::new(JiggleProfile::worker_test_profile());
        let job = |epoch| Request {
            epoch,
            entity: 7,
            model: "jan".into(),
            profile: Arc::clone(&profile),
            pose: vec![],
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            origin: [0.0; 3],
            now_ms: 0,
            enabled: true,
            hz: 60,
            max_substeps: 4,
            tuning: JiggleTuning::default(),
        };
        tx.send(job(0)).unwrap();
        worker.clear();
        tx.send(job(worker.epoch)).unwrap();
        drop(tx);
        AsyncJiggleSystem::run_worker(requests, results, Arc::clone(&worker.generation));
        let frames = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].epoch, worker.epoch);
    }

    #[test]
    fn repeated_solve_timestamp_keeps_interpolation_history() {
        let mut worker = AsyncJiggleSystem::default();
        let frame = |now_ms, solved_time_ms, value| ResultFrame {
            epoch: 0,
            entity: 7,
            model: "jan".into(),
            now_ms,
            solved_time_ms,
            offsets: vec![[value, 0.0, 0.0]],
        };
        worker.frames.insert(
            7,
            FramePair {
                previous: Some(frame(0, 0.0, 0.0)),
                latest: frame(17, 1000.0 / 60.0, 2.0),
            },
        );
        let (tx, rx) = mpsc::channel();
        worker.rx = Some(rx);
        tx.send(frame(18, 1000.0 / 60.0, 2.0)).unwrap();
        worker.poll();
        let output = worker.latest_offsets(7, "jan", 1, 20);
        assert!((output[0][0] - 0.4).abs() < 0.00001);
    }
}
