//! Shared asynchronous GPU pipeline/shader compiler.
//!
//! Runtime-lazy GPU compilation must never call `Device::create_render_pipeline` on the
//! render thread.  Callers identify a logical pipeline slot (`family`, `slot`) and
//! a configuration fingerprint.  A request supersedes any older configuration for
//! the same slot, which makes scene rebuilds/vid_restart-style format changes safe:
//! stale worker completions are simply discarded.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    panic::{self, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PipelineJobKey {
    pub family: &'static str,
    pub slot: u32,
    pub config: u64,
}

impl PipelineJobKey {
    pub const fn new(family: &'static str, slot: u32, config: u64) -> Self {
        Self {
            family,
            slot,
            config,
        }
    }
}

pub(crate) fn hash_config<T: Hash>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Live diagnostic toggle; the fast path is one relaxed load per pipeline/job,
/// not per rendered frame. Kept independent of `r_verbose` to avoid flooding.
static PIPELINE_TRACE: AtomicBool = AtomicBool::new(false);

pub(crate) fn pipeline_trace_enabled() -> bool {
    PIPELINE_TRACE.load(Ordering::Relaxed)
}

pub(crate) fn set_pipeline_trace(enabled: bool) {
    PIPELINE_TRACE.store(enabled, Ordering::Relaxed);
    println!("Renderer pipeline trace: {} (new compilations only)", if enabled { "ON" } else { "OFF" });
}

type PipelineJobValue = Box<dyn std::any::Any + Send>;
type BuildPipeline = Box<dyn FnOnce() -> PipelineJobValue + Send + 'static>;

enum WorkMessage {
    Compile {
        id: u64,
        key: PipelineJobKey,
        label: &'static str,
        build: BuildPipeline,
    },
    Shutdown,
}

struct CompletedJob {
    id: u64,
    key: PipelineJobKey,
    label: &'static str,
    elapsed: Duration,
    result: Result<PipelineJobValue, String>,
}

enum JobState {
    Compiling { id: u64 },
    Ready(PipelineJobValue),
    Failed,
}

/// Progress for the current burst of actual worker-submitted shader/PSO jobs.
/// `finished` counts successful, failed, and superseded completions so the bar
/// cannot stall forever when the user changes settings during a compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PipelineCompileProgress {
    pub finished: u32,
    pub total: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PipelineJobStats {
    pub queued: u64,
    pub completed: u64,
    pub stale: u64,
    pub failed: u64,
    pub compiling: usize,
    pub ready: usize,
    pub slowest_ms: f64,
}

/// Bounded persistent worker pool for gameplay-lazy render pipelines.
///
/// The queue is intentionally non-blocking from the render thread.  Worker count
/// defaults to two: enough to overlap independent driver compiles without turning
/// a first-sighting event into a whole-machine CPU spike at very high frame rates.
pub(crate) struct PipelineJobManager {
    work_tx: mpsc::Sender<WorkMessage>,
    world_work_tx: mpsc::Sender<WorkMessage>,
    done_rx: mpsc::Receiver<CompletedJob>,
    workers: Vec<JoinHandle<()>>,
    states: HashMap<PipelineJobKey, JobState>,
    next_id: u64,
    queued: u64,
    completed: u64,
    stale: u64,
    failed: u64,
    slowest_ms: f64,
    burst_total: u32,
    burst_finished: u32,
    burst_pending: u32,
    burst_finished_at: Option<Instant>,
    stopping: Arc<AtomicBool>,
}

impl PipelineJobManager {
    pub fn new() -> Self {
        let (work_tx, work_rx) = mpsc::channel::<WorkMessage>();
        let (world_work_tx, world_work_rx) = mpsc::channel::<WorkMessage>();
        let world_work_rx = Arc::new(Mutex::new(world_work_rx));
        let (done_tx, done_rx) = mpsc::channel::<CompletedJob>();
        let work_rx = Arc::new(Mutex::new(work_rx));
        let stopping = Arc::new(AtomicBool::new(false));
        let available = thread::available_parallelism().map_or(2, |count| count.get());
        let worker_count = available.saturating_sub(1).clamp(1, 2);
        let mut workers = Vec::with_capacity(worker_count);
        // A long BSP lighting variant must not consume the workers that make
        // newly visible sabers, models and FX renderable.
        for index in 0..=worker_count {
            let world_worker = index == worker_count;
            let work_rx = Arc::clone(if world_worker {
                &world_work_rx
            } else {
                &work_rx
            });
            let done_tx = done_tx.clone();
            let stopping_worker = Arc::clone(&stopping);
            let name = if world_worker {
                "pipeline-compile-world".into()
            } else {
                format!("pipeline-compile-{}", index + 1)
            };
            if let Ok(handle) = thread::Builder::new().name(name).spawn(move || loop {
                let message = {
                    let receiver = match work_rx.lock() {
                        Ok(receiver) => receiver,
                        Err(_) => break,
                    };
                    receiver.recv()
                };
                let Ok(message) = message else { break };
                match message {
                    WorkMessage::Shutdown => break,
                    WorkMessage::Compile {
                        id,
                        key,
                        label,
                        build,
                    } => {
                        // Renderer teardown/vid_restart should not grind through jobs that
                        // were only queued speculatively. An in-flight driver compile cannot
                        // be cancelled safely, but queued work can be dropped immediately.
                        if stopping_worker.load(Ordering::Acquire) {
                            continue;
                        }
                        let traced = pipeline_trace_enabled();
                        if traced {
                            println!("[pipeline job BEGIN] id={id} family={} slot={} hash={:016x} label={label} thread={:?}",
                                key.family, key.slot, key.config, thread::current().name());
                        }
                        let started = Instant::now();
                        let result =
                            panic::catch_unwind(AssertUnwindSafe(build)).map_err(panic_message);
                        if traced || pipeline_trace_enabled() {
                            println!("[pipeline job END] id={id} family={} slot={} hash={:016x} label={label} elapsed={:.1}ms result={}",
                                key.family, key.slot, key.config, started.elapsed().as_secs_f64() * 1000.0,
                                if result.is_ok() { "ok" } else { "PANIC" });
                        }
                        let _ = done_tx.send(CompletedJob {
                            id,
                            key,
                            label,
                            elapsed: started.elapsed(),
                            result,
                        });
                    }
                }
            }) {
                workers.push(handle);
            }
        }
        Self {
            work_tx,
            world_work_tx,
            done_rx,
            workers,
            states: HashMap::new(),
            next_id: 1,
            queued: 0,
            completed: 0,
            stale: 0,
            failed: 0,
            slowest_ms: 0.0,
            burst_total: 0,
            burst_finished: 0,
            burst_pending: 0,
            burst_finished_at: None,
            stopping,
        }
    }

    /// Queue a compile if this exact pipeline configuration is neither compiling
    /// nor ready.  Any older configuration for the same logical family+slot is
    /// invalidated immediately; a late completion from it cannot be installed.
    pub fn request<T: Send + 'static>(
        &mut self,
        key: PipelineJobKey,
        label: &'static str,
        build: impl FnOnce() -> T + Send + 'static,
    ) -> bool {
        self.poll();
        self.states.retain(|other, _| {
            other.family != key.family || other.slot != key.slot || *other == key
        });
        if self.states.contains_key(&key) {
            return false;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.states.insert(key, JobState::Compiling { id });
        let work_tx = if key.family == "world-variant" {
            &self.world_work_tx
        } else {
            &self.work_tx
        };
        if pipeline_trace_enabled() {
            println!("[pipeline job QUEUED] id={id} family={} slot={} hash={:016x} label={label}",
                key.family, key.slot, key.config);
        }
        if work_tx
            .send(WorkMessage::Compile {
                id,
                key,
                label,
                build: Box::new(move || Box::new(build()) as PipelineJobValue),
            })
            .is_ok()
        {
            self.queued = self.queued.saturating_add(1);
            if self.burst_pending == 0 {
                self.burst_total = 0;
                self.burst_finished = 0;
                self.burst_finished_at = None;
            }
            self.burst_total = self.burst_total.saturating_add(1);
            self.burst_pending = self.burst_pending.saturating_add(1);
            true
        } else {
            self.states.insert(key, JobState::Failed);
            self.failed = self.failed.saturating_add(1);
            false
        }
    }

    /// Returns a finished pipeline without ever waiting for a worker.
    pub fn take_ready<T: Send + 'static>(&mut self, key: PipelineJobKey) -> Option<T> {
        self.poll();
        if !matches!(self.states.get(&key), Some(JobState::Ready(_))) {
            return None;
        }
        match self.states.remove(&key) {
            Some(JobState::Ready(value)) => match value.downcast::<T>() {
                Ok(value) => Some(*value),
                Err(_) => {
                    self.failed = self.failed.saturating_add(1);
                    eprintln!(
                        "Renderer pipeline job type mismatch for {}:{}",
                        key.family, key.slot
                    );
                    None
                }
            },
            _ => None,
        }
    }

    pub fn stats(&mut self) -> PipelineJobStats {
        self.poll();
        let mut compiling = 0;
        let mut ready = 0;
        for state in self.states.values() {
            match state {
                JobState::Compiling { .. } => compiling += 1,
                JobState::Ready(_) => ready += 1,
                JobState::Failed => {}
            }
        }
        PipelineJobStats {
            queued: self.queued,
            completed: self.completed,
            stale: self.stale,
            failed: self.failed,
            compiling,
            ready,
            slowest_ms: self.slowest_ms,
        }
    }

    /// Non-blocking progress sample, intended for a throttled UI refresh.
    /// A just-finished burst stays at 100% briefly before disappearing.
    pub fn progress(&mut self) -> Option<PipelineCompileProgress> {
        self.poll();
        if self.burst_total == 0 {
            return None;
        }
        if self.burst_pending == 0
            && !self.burst_finished_at.is_some_and(|at| at.elapsed() < Duration::from_millis(450))
        {
            return None;
        }
        Some(PipelineCompileProgress {
            finished: self.burst_finished,
            total: self.burst_total,
        })
    }

    fn poll(&mut self) {
        while let Ok(done) = self.done_rx.try_recv() {
            // Count even superseded jobs: they still consumed real compiler work.
            self.burst_pending = self.burst_pending.saturating_sub(1);
            self.burst_finished = self.burst_finished.saturating_add(1);
            if self.burst_pending == 0 {
                self.burst_finished_at = Some(Instant::now());
            }
            let current = matches!(
                self.states.get(&done.key),
                Some(JobState::Compiling { id }) if *id == done.id
            );
            if !current {
                self.stale = self.stale.saturating_add(1);
                continue;
            }
            let elapsed_ms = done.elapsed.as_secs_f64() * 1000.0;
            self.slowest_ms = self.slowest_ms.max(elapsed_ms);
            self.completed = self.completed.saturating_add(1);
            match done.result {
                Ok(pipeline) => {
                    self.states.insert(done.key, JobState::Ready(pipeline));
                }
                Err(error) => {
                    self.states.insert(done.key, JobState::Failed);
                    self.failed = self.failed.saturating_add(1);
                    eprintln!(
                        "Renderer pipeline job failed: {} ({:.1} ms): {}",
                        done.label, elapsed_ms, error
                    );
                }
            }
        }
    }
}

impl Drop for PipelineJobManager {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        for _ in 0..self.workers.len() {
            let _ = self.work_tx.send(WorkMessage::Shutdown);
        }
        let _ = self.world_work_tx.send(WorkMessage::Shutdown);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "pipeline compiler panicked".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_world_variants_do_not_starve_visible_fx() {
        let mut jobs = PipelineJobManager::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        for slot in 0..2 {
            let started = started_tx.clone();
            let release = Arc::clone(&release_rx);
            jobs.request(
                PipelineJobKey::new("world-variant", slot, 1),
                "blocked world",
                move || {
                    let _ = started.send(());
                    let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
                    1u32
                },
            );
        }
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let fx = PipelineJobKey::new("fx-sprite", 0, 1);
        jobs.request(fx, "visible FX", || 42u32);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut result = None;
        while Instant::now() < deadline {
            result = jobs.take_ready::<u32>(fx);
            if result.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let _ = release_tx.send(());
        let _ = release_tx.send(());
        assert_eq!(
            result,
            Some(42),
            "visible FX must compile while world variants are blocked"
        );
    }

    #[test]
    fn progress_counts_real_jobs_and_clears_after_completion() {
        let mut jobs = PipelineJobManager::new();
        assert_eq!(jobs.progress(), None);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let key = PipelineJobKey::new("progress-test", 1, 1);
        assert!(jobs.request(key, "blocked test", move || {
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            7u32
        }));
        assert_eq!(jobs.progress(), Some(PipelineCompileProgress { finished: 0, total: 1 }));
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while jobs.progress().is_some_and(|progress| progress.finished != 1)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(jobs.progress(), Some(PipelineCompileProgress { finished: 1, total: 1 }));
        assert_eq!(jobs.take_ready::<u32>(key), Some(7));
        thread::sleep(Duration::from_millis(475));
        assert_eq!(jobs.progress(), None);
    }

}
