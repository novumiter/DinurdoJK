//! Short, opt-in input/presentation diagnostic capture.
//!
//! `/inputdiag` arms the recorder. The first captured raw mouse motion starts a
//! five-second window; mouse events are sampled on the winit thread and render
//! frames on the render thread. Nothing is recorded while idle/armed, so normal
//! play only pays a couple of relaxed atomic loads at the instrumentation sites.

use std::{
    collections::{BTreeSet, VecDeque},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use super::diagnostics::profiling::{GpuPass, GpuTiming};
use super::{ClientFramePerf, ViewSampleMeasurement};

const STATE_IDLE: u8 = 0;
const STATE_ARMED: u8 = 1;
const STATE_RECORDING: u8 = 2;
const STATE_FINALIZING: u8 = 3;
const CAPTURE_DURATION: Duration = Duration::from_secs(5);
const CAPTURE_NS: u64 = 5_000_000_000;
const MAX_MOUSE_SAMPLES: usize = 32_768;
const MAX_FRAME_SAMPLES: usize = 65_536;
const MAX_EVENT_LOOP_SAMPLES: usize = 32_768;
const MAX_WIN32_SAMPLES: usize = 32_768;

static INPUT_DIAG_EPOCH: OnceLock<Instant> = OnceLock::new();

#[inline]
fn shared_epoch() -> Instant {
    *INPUT_DIAG_EPOCH.get_or_init(Instant::now)
}

#[inline]
fn instant_ns_from_epoch(instant: Instant) -> u64 {
    instant
        .checked_duration_since(shared_epoch())
        .unwrap_or_default()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Copy, Debug, Default)]
struct Win32LatestInput {
    sequence: u64,
    hook_ns: u64,
    hook_gap_ms: f64,
    message_time_ms: u32,
    message_time_gap_ms: u32,
    wparam: usize,
}

#[derive(Clone, Copy, Debug)]
struct Win32InputSample {
    sequence: u64,
    hook_ns: u64,
    hook_gap_ms: f64,
    message_time_ms: u32,
    message_time_gap_ms: u32,
    wparam: usize,
}

#[cfg(target_os = "windows")]
struct Win32InputProbe {
    state: AtomicU8,
    sequence: AtomicU64,
    last_hook_ns: AtomicU64,
    last_message_time_ms: AtomicU64,
    latest_sequence: AtomicU64,
    latest_hook_ns: AtomicU64,
    latest_hook_gap_bits: AtomicU64,
    latest_message_time_ms: AtomicU64,
    latest_message_time_gap_ms: AtomicU64,
    latest_wparam: AtomicUsize,
    samples: Mutex<Vec<Win32InputSample>>,
}

#[cfg(target_os = "windows")]
impl Win32InputProbe {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_IDLE),
            sequence: AtomicU64::new(0),
            last_hook_ns: AtomicU64::new(0),
            last_message_time_ms: AtomicU64::new(u64::MAX),
            latest_sequence: AtomicU64::new(0),
            latest_hook_ns: AtomicU64::new(0),
            latest_hook_gap_bits: AtomicU64::new(0.0f64.to_bits()),
            latest_message_time_ms: AtomicU64::new(0),
            latest_message_time_gap_ms: AtomicU64::new(0),
            latest_wparam: AtomicUsize::new(0),
            samples: Mutex::new(Vec::new()),
        }
    }

    fn arm(&self) {
        self.sequence.store(0, Ordering::Release);
        self.last_hook_ns.store(0, Ordering::Release);
        self.last_message_time_ms.store(u64::MAX, Ordering::Release);
        self.latest_sequence.store(0, Ordering::Release);
        self.latest_hook_ns.store(0, Ordering::Release);
        self.latest_hook_gap_bits.store(0.0f64.to_bits(), Ordering::Release);
        self.latest_message_time_ms.store(0, Ordering::Release);
        self.latest_message_time_gap_ms.store(0, Ordering::Release);
        self.latest_wparam.store(0, Ordering::Release);
        let mut samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        samples.clear();
        let capacity = samples.capacity();
        if capacity < 8_192 {
            samples.reserve(8_192 - capacity);
        }
        self.state.store(STATE_ARMED, Ordering::Release);
    }

    fn begin_capture(&self, event_ns: u64) {
        if self
            .state
            .compare_exchange(STATE_ARMED, STATE_RECORDING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // The WM_INPUT which caused this DeviceEvent passed through the message
        // hook before the app knew the diagnostic should start. Preserve that
        // pre-trigger message so we can measure hook -> DeviceEvent delay.
        let latest = self.latest();
        if latest.sequence != 0
            && event_ns >= latest.hook_ns
            && event_ns.saturating_sub(latest.hook_ns) <= 250_000_000
        {
            self.push_sample(latest);
        }
    }

    fn latest(&self) -> Win32LatestInput {
        Win32LatestInput {
            sequence: self.latest_sequence.load(Ordering::Acquire),
            hook_ns: self.latest_hook_ns.load(Ordering::Acquire),
            hook_gap_ms: f64::from_bits(self.latest_hook_gap_bits.load(Ordering::Acquire)),
            message_time_ms: self.latest_message_time_ms.load(Ordering::Acquire) as u32,
            message_time_gap_ms: self.latest_message_time_gap_ms.load(Ordering::Acquire) as u32,
            wparam: self.latest_wparam.load(Ordering::Acquire),
        }
    }

    fn push_sample(&self, latest: Win32LatestInput) {
        let mut samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        if samples.len() < MAX_WIN32_SAMPLES
            && samples.last().is_none_or(|row| row.sequence != latest.sequence)
        {
            samples.push(Win32InputSample {
                sequence: latest.sequence,
                hook_ns: latest.hook_ns,
                hook_gap_ms: latest.hook_gap_ms,
                message_time_ms: latest.message_time_ms,
                message_time_gap_ms: latest.message_time_gap_ms,
                wparam: latest.wparam,
            });
        }
    }

    fn finish(&self) -> Vec<Win32InputSample> {
        self.state.store(STATE_IDLE, Ordering::Release);
        let mut samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut *samples)
    }
}

#[cfg(target_os = "windows")]
static WIN32_INPUT_PROBE: OnceLock<Win32InputProbe> = OnceLock::new();

#[cfg(target_os = "windows")]
fn win32_probe() -> &'static Win32InputProbe {
    WIN32_INPUT_PROBE.get_or_init(Win32InputProbe::new)
}

#[cfg(target_os = "windows")]
pub(crate) fn record_win32_message(message: *const std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_INPUT};

    let probe = win32_probe();
    let state = probe.state.load(Ordering::Acquire);
    if state != STATE_ARMED && state != STATE_RECORDING {
        return;
    }
    if message.is_null() {
        return;
    }
    // Winit guarantees the hook pointer is a valid MSG for the duration of the
    // callback. We only inspect metadata; returning false from the hook leaves
    // normal Winit dispatch completely untouched.
    let message = unsafe { &*(message.cast::<MSG>()) };
    if message.message != WM_INPUT {
        return;
    }

    let hook_ns = instant_ns_from_epoch(Instant::now());
    let previous_hook_ns = probe.last_hook_ns.swap(hook_ns, Ordering::AcqRel);
    let hook_gap_ms = if previous_hook_ns == 0 {
        0.0
    } else {
        hook_ns.saturating_sub(previous_hook_ns) as f64 / 1_000_000.0
    };
    let message_time_ms = message.time;
    let previous_message_time = probe
        .last_message_time_ms
        .swap(u64::from(message_time_ms), Ordering::AcqRel);
    let message_time_gap_ms = if previous_message_time == u64::MAX {
        0
    } else {
        message_time_ms.wrapping_sub(previous_message_time as u32)
    };
    let sequence = probe.sequence.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let latest = Win32LatestInput {
        sequence,
        hook_ns,
        hook_gap_ms,
        message_time_ms,
        message_time_gap_ms,
        wparam: message.wParam,
    };
    probe.latest_sequence.store(sequence, Ordering::Release);
    probe.latest_hook_ns.store(hook_ns, Ordering::Release);
    probe.latest_hook_gap_bits.store(hook_gap_ms.to_bits(), Ordering::Release);
    probe.latest_message_time_ms.store(u64::from(message_time_ms), Ordering::Release);
    probe.latest_message_time_gap_ms.store(u64::from(message_time_gap_ms), Ordering::Release);
    probe.latest_wparam.store(message.wParam, Ordering::Release);

    if state == STATE_RECORDING {
        probe.push_sample(latest);
    }
}

#[cfg(target_os = "windows")]
fn win32_arm() {
    win32_probe().arm();
}

#[cfg(not(target_os = "windows"))]
fn win32_arm() {}

#[cfg(target_os = "windows")]
fn win32_begin(event_ns: u64) {
    win32_probe().begin_capture(event_ns);
}

#[cfg(not(target_os = "windows"))]
fn win32_begin(_event_ns: u64) {}

#[cfg(target_os = "windows")]
fn win32_latest() -> Win32LatestInput {
    win32_probe().latest()
}

#[cfg(not(target_os = "windows"))]
fn win32_latest() -> Win32LatestInput {
    Win32LatestInput::default()
}

#[cfg(target_os = "windows")]
fn win32_finish() -> Vec<Win32InputSample> {
    win32_probe().finish()
}

#[cfg(not(target_os = "windows"))]
fn win32_finish() -> Vec<Win32InputSample> {
    Vec::new()
}

#[derive(Clone, Copy, Debug)]
pub(in crate::renderer) enum InputDiagLatchPhase {
    FrameStart,
    PreAcquire,
    PostAcquire,
    None,
}

impl InputDiagLatchPhase {
    fn label(self) -> &'static str {
        match self {
            Self::FrameStart => "frame_start",
            Self::PreAcquire => "pre_acquire",
            Self::PostAcquire => "post_acquire",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::renderer) struct InputDiagFrameSample {
    pub frame_started: Instant,
    pub frame_completed: Instant,
    pub frame_start_input_sequence: Option<u64>,
    pub present_latest_input_sequence: Option<u64>,
    pub latch: Option<ViewSampleMeasurement>,
    pub latch_phase: InputDiagLatchPhase,
    pub frame_ms: f64,
    pub cpu_prepare_ms: f64,
    pub cpu_acquire_ms: f64,
    pub cpu_encode_ms: f64,
    pub cpu_submit_ms: f64,
    pub cpu_present_ms: f64,
    pub submit_call_at: Option<Instant>,
    pub present_call_completed_at: Option<Instant>,
    pub command_drain_count: u32,
    pub command_drain_ms: f64,
    pub world_path: &'static str,
    pub fps_cap: u32,
    pub vsync: &'static str,
    pub present_mode: &'static str,
    pub max_frame_latency: u32,
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_config_generation: u64,
    pub surface_config_reason: &'static str,
    pub surface_config_age_ms: f64,
    pub input_latelatch: bool,
    pub pace_target_ms: f64,
    pub pace_wait_ms: f64,
    pub pace_wake_error_ms: f64,
    pub pace_late_before_wait_ms: f64,
    pub pace_anchor_reset: bool,
    pub client_perf: ClientFramePerf,
    /// GPU execution time of the most recently *resolved* profiler frame, not of
    /// this frame: timestamp readback lands a few frames later. Pair it with
    /// `gpu_sample_generation` to separate fresh measurements from repeats.
    pub gpu_frame_ms: Option<f64>,
    pub gpu_sample_generation: u64,
    pub gpu_timing: Option<GpuTiming>,
}

#[derive(Clone, Debug)]
pub(in crate::renderer) struct InputDiagRendererMetadata {
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub vendor: u32,
    pub device: u32,
    pub surface_format: String,
    pub alpha_mode: String,
    pub surface_usage: String,
    pub supported_present_modes: String,
}

#[derive(Clone, Copy, Debug)]
struct MouseSample {
    event_ns: u64,
    gap_ms: f64,
    dx: f64,
    dy: f64,
    input_sequence: Option<u64>,
    event_to_sim_ms: Option<f64>,
    callback_ms: f64,
    device_id_hash: u64,
    window_focused: bool,
    captured: bool,
    grab_mode: &'static str,
    win32_input_sequence: Option<u64>,
    win32_to_device_event_ms: Option<f64>,
    win32_hook_gap_ms: Option<f64>,
    win32_message_time_gap_ms: Option<u32>,
    win32_messages_since_previous_device_event: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct EventLoopSample {
    at_ns: u64,
    gap_ms: f64,
    cause: &'static str,
    window_focused: bool,
    captured: bool,
    grab_mode: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct FrameSample {
    frame_start_ns: u64,
    frame_interval_ms: f64,
    loop_elapsed_ms: f64,
    frame_ms: f64,
    cpu_prepare_ms: f64,
    cpu_acquire_ms: f64,
    cpu_encode_ms: f64,
    cpu_submit_ms: f64,
    cpu_present_ms: f64,
    command_drain_count: u32,
    command_drain_ms: f64,
    frame_start_input_sequence: Option<u64>,
    latch_sequence: Option<u64>,
    present_latest_input_sequence: Option<u64>,
    input_event_to_latch_ms: Option<f64>,
    input_latch_to_submit_ms: Option<f64>,
    input_latch_to_present_call_ms: Option<f64>,
    input_event_to_present_call_ms: Option<f64>,
    latch_phase: InputDiagLatchPhase,
    world_path: &'static str,
    fps_cap: u32,
    vsync: &'static str,
    present_mode: &'static str,
    max_frame_latency: u32,
    surface_width: u32,
    surface_height: u32,
    surface_config_generation: u64,
    surface_config_reason: &'static str,
    surface_config_age_ms: f64,
    input_latelatch: bool,
    pace_target_ms: f64,
    pace_wait_ms: f64,
    pace_wake_error_ms: f64,
    pace_late_before_wait_ms: f64,
    pace_anchor_reset: bool,
    client_total_ms: f64,
    client_snapshot_ms: f64,
    client_events_ms: f64,
    client_entity_present_ms: f64,
    client_fx_ms: f64,
    gpu_frame_ms: Option<f64>,
    gpu_sample_generation: u64,
    /// Whether this row carries a GPU measurement no earlier row reported.
    gpu_sample_fresh: bool,
    /// Per-pass breakdown, kept only on the frame that first reported this
    /// measurement so the summary can aggregate without counting repeats.
    gpu_timing: Option<GpuTiming>,
}

#[derive(Debug)]
struct CaptureConfig {
    stem: PathBuf,
}

/// Shared by the winit/app and render threads. The two hot streams use separate
/// mutexes, so a 1000 Hz mouse cannot contend with a high-FPS render recorder.
pub(in crate::renderer) struct InputDiagnostic {
    epoch: Instant,
    state: AtomicU8,
    start_ns: AtomicU64,
    last_mouse_ns: AtomicU64,
    last_mouse_win32_sequence: AtomicU64,
    last_frame_start_ns: AtomicU64,
    last_event_loop_ns: AtomicU64,
    last_gpu_generation: AtomicU64,
    mouse: Mutex<Vec<MouseSample>>,
    event_loop: Mutex<Vec<EventLoopSample>>,
    frames: Mutex<Vec<FrameSample>>,
    renderer_metadata: Mutex<Option<InputDiagRendererMetadata>>,
    config: Mutex<Option<CaptureConfig>>,
    notices: Arc<Mutex<VecDeque<String>>>,
    notice_pending: Arc<AtomicU8>,
}

impl InputDiagnostic {
    pub(in crate::renderer) fn new() -> Self {
        Self {
            epoch: shared_epoch(),
            state: AtomicU8::new(STATE_IDLE),
            start_ns: AtomicU64::new(0),
            last_mouse_ns: AtomicU64::new(0),
            last_mouse_win32_sequence: AtomicU64::new(0),
            last_frame_start_ns: AtomicU64::new(0),
            last_event_loop_ns: AtomicU64::new(0),
            last_gpu_generation: AtomicU64::new(0),
            mouse: Mutex::new(Vec::new()),
            event_loop: Mutex::new(Vec::new()),
            frames: Mutex::new(Vec::new()),
            renderer_metadata: Mutex::new(None),
            config: Mutex::new(None),
            notices: Arc::new(Mutex::new(VecDeque::new())),
            notice_pending: Arc::new(AtomicU8::new(0)),
        }
    }

    pub(in crate::renderer) fn set_renderer_metadata(&self, metadata: InputDiagRendererMetadata) {
        *self
            .renderer_metadata
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(metadata);
    }

    #[inline]
    fn instant_ns(&self, instant: Instant) -> u64 {
        debug_assert_eq!(self.epoch, shared_epoch());
        instant_ns_from_epoch(instant)
    }

    #[inline]
    pub(in crate::renderer) fn is_recording(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_RECORDING
    }

    pub(in crate::renderer) fn arm(&self, stem: PathBuf) -> Result<(), &'static str> {
        self.state
            .compare_exchange(
                STATE_IDLE,
                STATE_ARMED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|state| match state {
                STATE_ARMED => "input diagnostic is already armed",
                STATE_RECORDING | STATE_FINALIZING => "input diagnostic capture is already running",
                _ => "input diagnostic is busy",
            })?;

        self.start_ns.store(0, Ordering::Release);
        self.last_mouse_ns.store(0, Ordering::Release);
        self.last_mouse_win32_sequence.store(0, Ordering::Release);
        self.last_frame_start_ns.store(0, Ordering::Release);
        self.last_event_loop_ns.store(0, Ordering::Release);
        self.last_gpu_generation.store(0, Ordering::Release);
        {
            let mut mouse = self.mouse.lock().unwrap_or_else(|p| p.into_inner());
            mouse.clear();
            let capacity = mouse.capacity();
            if capacity < 8_192 {
                mouse.reserve(8_192 - capacity);
            }
        }
        {
            let mut event_loop = self.event_loop.lock().unwrap_or_else(|p| p.into_inner());
            event_loop.clear();
            let capacity = event_loop.capacity();
            if capacity < 8_192 {
                event_loop.reserve(8_192 - capacity);
            }
        }
        {
            let mut frames = self.frames.lock().unwrap_or_else(|p| p.into_inner());
            frames.clear();
            let capacity = frames.capacity();
            if capacity < 16_384 {
                frames.reserve(16_384 - capacity);
            }
        }
        win32_arm();
        *self.config.lock().unwrap_or_else(|p| p.into_inner()) = Some(CaptureConfig { stem });
        Ok(())
    }

    /// Called at the very beginning of an accepted raw-mouse callback. The
    /// first event transitions ARMED -> RECORDING before any gameplay work, so
    /// concurrently rendered frames during a slow callback are captured too.
    pub(in crate::renderer) fn begin_mouse_event(&self, event_at: Instant) {
        if self.state.load(Ordering::Acquire) != STATE_ARMED {
            return;
        }
        let start_ns = self.instant_ns(event_at);
        self.start_ns.store(start_ns, Ordering::Relaxed);
        self.last_mouse_ns.store(0, Ordering::Relaxed);
        self.last_mouse_win32_sequence.store(0, Ordering::Relaxed);
        self.last_frame_start_ns.store(0, Ordering::Relaxed);
        self.last_event_loop_ns.store(0, Ordering::Relaxed);
        let started = self.state.compare_exchange(
            STATE_ARMED,
            STATE_RECORDING,
            Ordering::Release,
            Ordering::Acquire,
        ).is_ok();
        if started {
            win32_begin(start_ns);
        }
    }

    pub(in crate::renderer) fn finish_mouse_event(
        &self,
        event_at: Instant,
        simulation_at: Option<Instant>,
        completed_at: Instant,
        dx: f64,
        dy: f64,
        input_sequence: Option<u64>,
        device_id_hash: u64,
        window_focused: bool,
        captured: bool,
        grab_mode: &'static str,
    ) {
        if !self.is_recording() {
            return;
        }
        let event_ns = self.instant_ns(event_at);
        let start_ns = self.start_ns.load(Ordering::Acquire);
        if event_ns.saturating_sub(start_ns) > CAPTURE_NS {
            return;
        }
        let previous = self.last_mouse_ns.swap(event_ns, Ordering::AcqRel);
        let gap_ms = if previous == 0 {
            0.0
        } else {
            event_ns.saturating_sub(previous) as f64 / 1_000_000.0
        };
        let event_to_sim_ms = simulation_at.map(|at| {
            at.saturating_duration_since(event_at).as_secs_f64() * 1000.0
        });
        let callback_ms = completed_at
            .saturating_duration_since(event_at)
            .as_secs_f64()
            * 1000.0;
        let win32 = win32_latest();
        let correlated_win32 = win32.sequence != 0
            && event_ns >= win32.hook_ns
            && event_ns.saturating_sub(win32.hook_ns) <= 250_000_000;
        let (win32_input_sequence, win32_to_device_event_ms, win32_hook_gap_ms, win32_message_time_gap_ms, win32_messages_since_previous_device_event) =
            if correlated_win32 {
                let previous_sequence = self
                    .last_mouse_win32_sequence
                    .swap(win32.sequence, Ordering::AcqRel);
                let since_previous = if previous_sequence == 0 {
                    1
                } else {
                    win32.sequence.saturating_sub(previous_sequence)
                };
                (
                    Some(win32.sequence),
                    Some(event_ns.saturating_sub(win32.hook_ns) as f64 / 1_000_000.0),
                    Some(win32.hook_gap_ms),
                    Some(win32.message_time_gap_ms),
                    Some(since_previous),
                )
            } else {
                (None, None, None, None, None)
            };
        let mut mouse = self.mouse.lock().unwrap_or_else(|p| p.into_inner());
        if mouse.len() < MAX_MOUSE_SAMPLES {
            mouse.push(MouseSample {
                event_ns,
                gap_ms,
                dx,
                dy,
                input_sequence,
                event_to_sim_ms,
                callback_ms,
                device_id_hash,
                window_focused,
                captured,
                grab_mode,
                win32_input_sequence,
                win32_to_device_event_ms,
                win32_hook_gap_ms,
                win32_message_time_gap_ms,
                win32_messages_since_previous_device_event,
            });
        }
    }

    pub(in crate::renderer) fn record_event_loop_turn(
        &self,
        at: Instant,
        cause: &'static str,
        window_focused: bool,
        captured: bool,
        grab_mode: &'static str,
    ) {
        if !self.is_recording() {
            return;
        }
        let at_ns = self.instant_ns(at);
        let start_ns = self.start_ns.load(Ordering::Acquire);
        if at_ns.saturating_sub(start_ns) > CAPTURE_NS {
            return;
        }
        let previous = self.last_event_loop_ns.swap(at_ns, Ordering::AcqRel);
        let gap_ms = if previous == 0 {
            0.0
        } else {
            at_ns.saturating_sub(previous) as f64 / 1_000_000.0
        };
        let mut rows = self.event_loop.lock().unwrap_or_else(|p| p.into_inner());
        if rows.len() < MAX_EVENT_LOOP_SAMPLES {
            rows.push(EventLoopSample {
                at_ns,
                gap_ms,
                cause,
                window_focused,
                captured,
                grab_mode,
            });
        }
    }

    pub(in crate::renderer) fn record_frame(&self, sample: InputDiagFrameSample) {
        if !self.is_recording() {
            return;
        }
        let frame_start_ns = self.instant_ns(sample.frame_started);
        // A frame that began just before the triggering mouse callback is useful,
        // and the first frame that completes after the five-second boundary is
        // useful too: it exposes a render/acquire stall that straddled the end.
        let previous = self
            .last_frame_start_ns
            .swap(frame_start_ns, Ordering::AcqRel);
        let frame_interval_ms = if previous == 0 {
            0.0
        } else {
            frame_start_ns.saturating_sub(previous) as f64 / 1_000_000.0
        };

        let latch_sequence = sample.latch.map(|measurement| measurement.input.sequence);
        let input_event_to_latch_ms = sample.latch.map(|measurement| {
            measurement
                .sampled_at
                .saturating_duration_since(measurement.input.event_at)
                .as_secs_f64()
                * 1000.0
        });
        let input_latch_to_submit_ms = sample.latch.and_then(|measurement| {
            sample.submit_call_at.map(|submit| {
                submit
                    .saturating_duration_since(measurement.sampled_at)
                    .as_secs_f64()
                    * 1000.0
            })
        });
        let input_latch_to_present_call_ms = sample.latch.and_then(|measurement| {
            sample.present_call_completed_at.map(|present| {
                present
                    .saturating_duration_since(measurement.sampled_at)
                    .as_secs_f64()
                    * 1000.0
            })
        });
        let input_event_to_present_call_ms = sample.latch.and_then(|measurement| {
            sample.present_call_completed_at.map(|present| {
                present
                    .saturating_duration_since(measurement.input.event_at)
                    .as_secs_f64()
                    * 1000.0
            })
        });

        // Generation 0 means the profiler has not resolved anything yet (off,
        // unsupported, or still filling its slots), so it is never fresh.
        let gpu_sample_fresh = sample.gpu_sample_generation != 0
            && self
                .last_gpu_generation
                .swap(sample.gpu_sample_generation, Ordering::AcqRel)
                != sample.gpu_sample_generation;

        let mut frames = self.frames.lock().unwrap_or_else(|p| p.into_inner());
        if frames.len() < MAX_FRAME_SAMPLES {
            frames.push(FrameSample {
                frame_start_ns,
                frame_interval_ms,
                loop_elapsed_ms: sample
                    .frame_completed
                    .saturating_duration_since(sample.frame_started)
                    .as_secs_f64()
                    * 1000.0,
                frame_ms: sample.frame_ms,
                cpu_prepare_ms: sample.cpu_prepare_ms,
                cpu_acquire_ms: sample.cpu_acquire_ms,
                cpu_encode_ms: sample.cpu_encode_ms,
                cpu_submit_ms: sample.cpu_submit_ms,
                cpu_present_ms: sample.cpu_present_ms,
                command_drain_count: sample.command_drain_count,
                command_drain_ms: sample.command_drain_ms,
                frame_start_input_sequence: sample.frame_start_input_sequence,
                latch_sequence,
                present_latest_input_sequence: sample.present_latest_input_sequence,
                input_event_to_latch_ms,
                input_latch_to_submit_ms,
                input_latch_to_present_call_ms,
                input_event_to_present_call_ms,
                latch_phase: sample.latch_phase,
                world_path: sample.world_path,
                fps_cap: sample.fps_cap,
                vsync: sample.vsync,
                present_mode: sample.present_mode,
                max_frame_latency: sample.max_frame_latency,
                surface_width: sample.surface_width,
                surface_height: sample.surface_height,
                surface_config_generation: sample.surface_config_generation,
                surface_config_reason: sample.surface_config_reason,
                surface_config_age_ms: sample.surface_config_age_ms,
                input_latelatch: sample.input_latelatch,
                pace_target_ms: sample.pace_target_ms,
                pace_wait_ms: sample.pace_wait_ms,
                pace_wake_error_ms: sample.pace_wake_error_ms,
                pace_late_before_wait_ms: sample.pace_late_before_wait_ms,
                pace_anchor_reset: sample.pace_anchor_reset,
                client_total_ms: sample.client_perf.total_ms,
                client_snapshot_ms: sample.client_perf.snapshot_ms,
                client_events_ms: sample.client_perf.events_ms,
                client_entity_present_ms: sample.client_perf.entity_present_ms,
                client_fx_ms: sample.client_perf.fx_ms,
                gpu_frame_ms: sample.gpu_frame_ms,
                gpu_sample_generation: sample.gpu_sample_generation,
                gpu_sample_fresh,
                gpu_timing: gpu_sample_fresh.then_some(sample.gpu_timing).flatten(),
            });
        }
    }

    /// Called by the render loop after its pacing decision. Exactly one thread
    /// wins RECORDING -> FINALIZING and snapshots the short capture for a writer.
    pub(in crate::renderer) fn maybe_finish(&self, now: Instant) {
        if !self.is_recording() {
            return;
        }
        let now_ns = self.instant_ns(now);
        let start_ns = self.start_ns.load(Ordering::Acquire);
        if now_ns.saturating_sub(start_ns) < CAPTURE_NS {
            return;
        }
        if self
            .state
            .compare_exchange(
                STATE_RECORDING,
                STATE_FINALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }

        let mouse = {
            let mut guard = self.mouse.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *guard)
        };
        let event_loop = {
            let mut guard = self.event_loop.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *guard)
        };
        let frames = {
            let mut guard = self.frames.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *guard)
        };
        let renderer_metadata = self
            .renderer_metadata
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let win32 = win32_finish();
        let config = self
            .config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        self.state.store(STATE_IDLE, Ordering::Release);

        let Some(config) = config else {
            return;
        };
        let notices = self.notices.clone();
        let notice_pending = self.notice_pending.clone();
        let start_ns_for_rows = start_ns;
        let spawn = thread::Builder::new()
            .name("jka-inputdiag-write".into())
            .spawn(move || {
                let result = write_capture(
                    &config.stem,
                    start_ns_for_rows,
                    &mouse,
                    &event_loop,
                    &win32,
                    &frames,
                    renderer_metadata.as_ref(),
                );
                let line = match result {
                    Ok(summary) => {
                        let acquire_flag = if summary.acquire_p95_ms >= 1.0 {
                            " ^1<- BLOCKING^7"
                        } else {
                            ""
                        };
                        // The CPU never waits on the GPU here, so a GPU that
                        // cannot hold the cap shows up in no other number.
                        let gpu_text = if summary.gpu_measurements == 0 {
                            " GPU FRAME: no timestamp data.".to_owned()
                        } else {
                            format!(
                                " GPU FRAME: median {:.3} ms max {:.3} ms over {} measurement(s), {} over budget{}.",
                                summary.gpu_frame_ms_p50,
                                summary.gpu_frame_ms_max,
                                summary.gpu_measurements,
                                summary.gpu_frames_over_budget,
                                if summary.gpu_frames_over_budget > 0 {
                                    " ^1<- GPU-BOUND^7"
                                } else {
                                    ""
                                },
                            )
                        };
                        format!(
                            "^2inputdiag:^7 captured {} mouse event(s), {} WM_INPUT message(s), {} frame(s); max mouse gap {:.3} ms, max frame interval {:.3} ms, max latch age {:.3} ms. SURFACE ACQUIRE: median {:.3} ms p95 {:.3} ms max {:.3} ms{}; surface gen {} -> {} ({} change(s)).{} Wrote {}",
                            mouse.len(),
                            win32.len(),
                            frames.len(),
                            summary.max_mouse_gap_ms,
                            summary.max_frame_interval_ms,
                            summary.max_latch_age_ms,
                            summary.acquire_p50_ms,
                            summary.acquire_p95_ms,
                            summary.max_acquire_ms,
                            acquire_flag,
                            summary.surface_generation_first,
                            summary.surface_generation_last,
                            summary.surface_generation_changes,
                            gpu_text,
                            summary.summary_path.display(),
                        )
                    }
                    Err(error) => format!(
                        "^1inputdiag:^7 couldn't write {}: {error}",
                        config.stem.display()
                    ),
                };
                notices
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push_back(line);
                notice_pending.store(1, Ordering::Release);
            });
        if let Err(error) = spawn {
            self.notices
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push_back(format!("^1inputdiag:^7 couldn't start writer: {error}"));
            self.notice_pending.store(1, Ordering::Release);
        }
    }

    pub(in crate::renderer) fn take_notices(&self) -> Vec<String> {
        if self.notice_pending.load(Ordering::Acquire) == 0 {
            return Vec::new();
        }
        let mut notices = self.notices.lock().unwrap_or_else(|p| p.into_inner());
        let result: Vec<_> = notices.drain(..).collect();
        self.notice_pending.store(0, Ordering::Release);
        result
    }
}

#[derive(Debug)]
struct CaptureSummary {
    summary_path: PathBuf,
    max_mouse_gap_ms: f64,
    max_frame_interval_ms: f64,
    max_acquire_ms: f64,
    acquire_p50_ms: f64,
    acquire_p95_ms: f64,
    max_latch_age_ms: f64,
    gpu_frame_ms_p50: f64,
    gpu_frame_ms_max: f64,
    gpu_measurements: usize,
    gpu_frames_over_budget: usize,
    surface_generation_first: u64,
    surface_generation_last: u64,
    surface_generation_changes: usize,
}

fn write_capture(
    stem: &Path,
    start_ns: u64,
    mouse: &[MouseSample],
    event_loop: &[EventLoopSample],
    win32: &[Win32InputSample],
    frames: &[FrameSample],
    renderer_metadata: Option<&InputDiagRendererMetadata>,
) -> std::io::Result<CaptureSummary> {
    let mouse_path = suffixed_path(stem, "-mouse.csv");
    let event_loop_path = suffixed_path(stem, "-eventloop.csv");
    let win32_path = suffixed_path(stem, "-win32.csv");
    let frame_path = suffixed_path(stem, "-frames.csv");
    let summary_path = suffixed_path(stem, "-summary.json");

    if let Some(parent) = stem.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut mouse_csv = String::with_capacity(mouse.len().saturating_mul(180));
    mouse_csv.push_str(
        "t_ms,event_gap_ms,dx,dy,raw_distance,input_sequence,event_to_sim_ms,callback_ms,device_id_hash,window_focused,captured,grab_mode,win32_input_seq,win32_to_device_event_ms,win32_hook_gap_ms,win32_message_time_gap_ms,win32_messages_since_previous_device_event\n",
    );
    for row in mouse {
        let t_ms = row.event_ns.saturating_sub(start_ns) as f64 / 1_000_000.0;
        let distance = row.dx.hypot(row.dy);
        let _ = writeln!(
            mouse_csv,
            "{t_ms:.6},{:.6},{:.6},{:.6},{distance:.6},{},{},{},{:016x},{},{},{},{},{},{},{},{}",
            row.gap_ms,
            row.dx,
            row.dy,
            opt_u64(row.input_sequence),
            opt_f64(row.event_to_sim_ms),
            fmt_f64(row.callback_ms),
            row.device_id_hash,
            u8::from(row.window_focused),
            u8::from(row.captured),
            row.grab_mode,
            opt_u64(row.win32_input_sequence),
            opt_f64(row.win32_to_device_event_ms),
            opt_f64(row.win32_hook_gap_ms),
            row.win32_message_time_gap_ms.map_or_else(String::new, |value| value.to_string()),
            opt_u64(row.win32_messages_since_previous_device_event),
        );
    }
    std::fs::write(&mouse_path, mouse_csv)?;

    let mut event_loop_csv = String::with_capacity(event_loop.len().saturating_mul(64));
    event_loop_csv.push_str("t_ms,event_loop_gap_ms,cause,window_focused,captured,grab_mode\n");
    for row in event_loop {
        let t_ms = row.at_ns.saturating_sub(start_ns) as f64 / 1_000_000.0;
        let _ = writeln!(
            event_loop_csv,
            "{t_ms:.6},{:.6},{},{},{},{}",
            row.gap_ms,
            row.cause,
            u8::from(row.window_focused),
            u8::from(row.captured),
            row.grab_mode,
        );
    }
    std::fs::write(&event_loop_path, event_loop_csv)?;

    let mut win32_csv = String::with_capacity(win32.len().saturating_mul(96));
    win32_csv.push_str("t_ms,win32_input_seq,hook_gap_ms,msg_time_ms,msg_time_gap_ms,wparam\n");
    for row in win32 {
        let t_ms = row.hook_ns.saturating_sub(start_ns) as f64 / 1_000_000.0;
        let _ = writeln!(
            win32_csv,
            "{t_ms:.6},{},{:.6},{},{},{}",
            row.sequence,
            row.hook_gap_ms,
            row.message_time_ms,
            row.message_time_gap_ms,
            row.wparam,
        );
    }
    std::fs::write(&win32_path, win32_csv)?;

    let mut frame_csv = String::with_capacity(frames.len().saturating_mul(300));
    frame_csv.push_str(
        "t_ms,frame_interval_ms,loop_elapsed_ms,frame_ms,prepare_ms,acquire_ms,encode_ms,submit_ms,present_call_ms,command_drain_count,command_drain_ms,frame_start_seq,latch_seq,present_latest_seq,event_to_latch_ms,latch_to_submit_ms,latch_to_present_call_ms,event_to_present_call_ms,latch_phase,world_path,fps_cap,vsync,present_mode,max_frame_latency,surface_width,surface_height,surface_config_generation,surface_config_reason,surface_config_age_ms,input_latelatch,pace_target_ms,pace_wait_ms,pace_wake_error_ms,pace_late_before_wait_ms,pace_anchor_reset,client_total_ms,client_snapshot_ms,client_events_ms,client_entity_present_ms,client_fx_ms,gpu_frame_ms,gpu_sample_generation,gpu_sample_fresh,gpu_budget_ratio\n",
    );
    for row in frames {
        let t_ms = row.frame_start_ns.saturating_sub(start_ns) as f64 / 1_000_000.0;
        let _ = writeln!(
            frame_csv,
            "{t_ms:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{:.6},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.6},{},{:.6},{:.6},{:.6},{:.6},{},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{}",
            row.frame_interval_ms,
            row.loop_elapsed_ms,
            row.frame_ms,
            row.cpu_prepare_ms,
            row.cpu_acquire_ms,
            row.cpu_encode_ms,
            row.cpu_submit_ms,
            row.cpu_present_ms,
            row.command_drain_count,
            row.command_drain_ms,
            opt_u64(row.frame_start_input_sequence),
            opt_u64(row.latch_sequence),
            opt_u64(row.present_latest_input_sequence),
            opt_f64(row.input_event_to_latch_ms),
            opt_f64(row.input_latch_to_submit_ms),
            opt_f64(row.input_latch_to_present_call_ms),
            opt_f64(row.input_event_to_present_call_ms),
            row.latch_phase.label(),
            row.world_path,
            row.fps_cap,
            row.vsync,
            row.present_mode,
            row.max_frame_latency,
            row.surface_width,
            row.surface_height,
            row.surface_config_generation,
            row.surface_config_reason,
            row.surface_config_age_ms,
            u8::from(row.input_latelatch),
            row.pace_target_ms,
            row.pace_wait_ms,
            row.pace_wake_error_ms,
            row.pace_late_before_wait_ms,
            u8::from(row.pace_anchor_reset),
            row.client_total_ms,
            row.client_snapshot_ms,
            row.client_events_ms,
            row.client_entity_present_ms,
            row.client_fx_ms,
            opt_f64(row.gpu_frame_ms),
            row.gpu_sample_generation,
            u8::from(row.gpu_sample_fresh),
            // >1 means the GPU could not finish inside the pacer's own budget,
            // i.e. the configured cap is not actually achievable.
            opt_f64(match (row.gpu_frame_ms, row.pace_target_ms) {
                (Some(gpu), target) if target > 0.0 => Some(gpu / target),
                _ => None,
            }),
        );
    }
    std::fs::write(&frame_path, frame_csv)?;

    let mouse_gaps: Vec<f64> = mouse.iter().skip(1).map(|row| row.gap_ms).collect();
    let frame_intervals: Vec<f64> = frames
        .iter()
        .skip(1)
        .map(|row| row.frame_interval_ms)
        .collect();
    let acquire: Vec<f64> = frames.iter().map(|row| row.cpu_acquire_ms).collect();
    let latch_age: Vec<f64> = frames
        .iter()
        .filter_map(|row| row.input_event_to_latch_ms)
        .collect();
    let callback: Vec<f64> = mouse.iter().map(|row| row.callback_ms).collect();
    let command_drain: Vec<f64> = frames.iter().map(|row| row.command_drain_ms).collect();
    let pace_wake_error: Vec<f64> = frames.iter().map(|row| row.pace_wake_error_ms).collect();
    let event_loop_gaps: Vec<f64> = event_loop.iter().skip(1).map(|row| row.gap_ms).collect();
    let win32_hook_gaps: Vec<f64> = win32.iter().skip(1).map(|row| row.hook_gap_ms).collect();
    let win32_message_time_gaps: Vec<f64> = win32
        .iter()
        .skip(1)
        .map(|row| f64::from(row.message_time_gap_ms))
        .collect();
    let win32_to_device: Vec<f64> = mouse
        .iter()
        .filter_map(|row| row.win32_to_device_event_ms)
        .collect();
    let device_ids: BTreeSet<u64> = mouse.iter().map(|row| row.device_id_hash).collect();
    let device_gap_gt_5ms = mouse.iter().skip(1).filter(|row| row.gap_ms > 5.0).count();
    let device_gap_gt_5ms_with_win32_gap_gt_5ms = mouse
        .iter()
        .skip(1)
        .filter(|row| row.gap_ms > 5.0 && row.win32_hook_gap_ms.is_some_and(|gap| gap > 5.0))
        .count();
    let device_gap_gt_5ms_with_win32_gap_lt_3ms = mouse
        .iter()
        .skip(1)
        .filter(|row| row.gap_ms > 5.0 && row.win32_hook_gap_ms.is_some_and(|gap| gap < 3.0))
        .count();
    let win32_hook_gap_gt_5ms_with_msg_gap_lt_3ms = win32
        .iter()
        .skip(1)
        .filter(|row| row.hook_gap_ms > 5.0 && row.message_time_gap_ms < 3)
        .count();

    // Count every GPU measurement once. Timestamp readback is slower than the
    // render loop, so consecutive rows repeat the same resolved frame.
    let gpu_frame: Vec<f64> = frames
        .iter()
        .filter(|row| row.gpu_sample_fresh)
        .filter_map(|row| row.gpu_frame_ms)
        .collect();
    let gpu_budget_ratio: Vec<f64> = frames
        .iter()
        .filter(|row| row.gpu_sample_fresh && row.pace_target_ms > 0.0)
        .filter_map(|row| row.gpu_frame_ms.map(|gpu| gpu / row.pace_target_ms))
        .collect();
    let gpu_frames_over_budget = gpu_budget_ratio.iter().filter(|ratio| **ratio > 1.0).count();
    let gpu_timings: Vec<GpuTiming> = frames.iter().filter_map(|row| row.gpu_timing).collect();
    let mut gpu_pass_ms = serde_json::Map::new();
    for pass in GpuPass::ALL {
        let values: Vec<f64> = gpu_timings
            .iter()
            .filter_map(|timing| timing.milliseconds[pass.index()])
            .collect();
        if !values.is_empty() {
            gpu_pass_ms.insert(pass.label().to_owned(), stats_json(&values));
        }
    }
    let mut gpu_frame_sorted = gpu_frame.clone();
    gpu_frame_sorted.sort_by(f64::total_cmp);
    let gpu_frame_ms_p50 = percentile(&gpu_frame_sorted, 0.50);
    let gpu_frame_ms_max = max_or_zero(&gpu_frame);
    let gpu_measurements = gpu_frame.len();

    let max_mouse_gap_ms = max_or_zero(&mouse_gaps);
    let max_frame_interval_ms = max_or_zero(&frame_intervals);
    let max_acquire_ms = max_or_zero(&acquire);
    let max_latch_age_ms = max_or_zero(&latch_age);
    let mut acquire_sorted = acquire.clone();
    acquire_sorted.sort_by(f64::total_cmp);
    let acquire_p50_ms = percentile(&acquire_sorted, 0.50);
    let acquire_p95_ms = percentile(&acquire_sorted, 0.95);
    let surface_generation_first = frames
        .first()
        .map_or(0, |row| row.surface_config_generation);
    let surface_generation_last = frames
        .last()
        .map_or(0, |row| row.surface_config_generation);
    let surface_generation_changes = frames
        .windows(2)
        .filter(|pair| pair[0].surface_config_generation != pair[1].surface_config_generation)
        .count();
    let mut surface_events = Vec::new();
    let mut previous_surface_generation = None;
    for row in frames {
        if previous_surface_generation == Some(row.surface_config_generation) {
            continue;
        }
        previous_surface_generation = Some(row.surface_config_generation);
        surface_events.push(serde_json::json!({
            "t_ms": row.frame_start_ns.saturating_sub(start_ns) as f64 / 1_000_000.0,
            "generation": row.surface_config_generation,
            "reason": row.surface_config_reason,
            "configured_age_ms": row.surface_config_age_ms,
            "width": row.surface_width,
            "height": row.surface_height,
            "vsync": row.vsync,
            "present_mode": row.present_mode,
            "max_frame_latency": row.max_frame_latency,
        }));
    }
    let renderer_json = renderer_metadata.map_or(serde_json::Value::Null, |metadata| {
        serde_json::json!({
            "adapter_name": metadata.adapter_name.as_str(),
            "backend": metadata.backend.as_str(),
            "device_type": metadata.device_type.as_str(),
            "driver": metadata.driver.as_str(),
            "driver_info": metadata.driver_info.as_str(),
            "vendor": metadata.vendor,
            "device": metadata.device,
            "surface_format": metadata.surface_format.as_str(),
            "alpha_mode": metadata.alpha_mode.as_str(),
            "surface_usage": metadata.surface_usage.as_str(),
            "supported_present_modes": metadata.supported_present_modes.as_str(),
        })
    });
    // Split out of the summary literal: serde_json's json! recurses once
    // per entry, and one object with every key hit the recursion limit.
    let counts_json = serde_json::json!({
        "mouse_events": mouse.len(),
        "event_loop_turns": event_loop.len(),
        "win32_input_messages": win32.len(),
        "mouse_events_with_correlated_win32_input": mouse.iter().filter(|row| row.win32_input_sequence.is_some()).count(),
        "mouse_device_ids": device_ids.len(),
        "device_event_gap_gt_5ms": device_gap_gt_5ms,
        "device_event_gap_gt_5ms_with_win32_hook_gap_gt_5ms": device_gap_gt_5ms_with_win32_gap_gt_5ms,
        "device_event_gap_gt_5ms_with_win32_hook_gap_lt_3ms": device_gap_gt_5ms_with_win32_gap_lt_3ms,
        "win32_hook_gap_gt_5ms_with_msg_time_gap_lt_3ms": win32_hook_gap_gt_5ms_with_msg_gap_lt_3ms,
        "frames": frames.len(),
        "gpu_measurements": gpu_measurements,
        "gpu_frames_over_budget": gpu_frames_over_budget,
        "frames_with_gpu_sample": frames.iter().filter(|row| row.gpu_frame_ms.is_some()).count(),
        "pace_anchor_resets": frames.iter().filter(|row| row.pace_anchor_reset).count(),
        "pace_late_without_reset_frames": frames
            .iter()
            .filter(|row| row.pace_late_before_wait_ms > 0.0 && !row.pace_anchor_reset)
            .count(),
        "pre_acquire_latches": frames.iter().filter(|row| matches!(row.latch_phase, InputDiagLatchPhase::PreAcquire)).count(),
        "post_acquire_latches": frames.iter().filter(|row| matches!(row.latch_phase, InputDiagLatchPhase::PostAcquire)).count(),
        "newer_input_visible_after_latch": frames
            .iter()
            .filter(|row| matches!((row.latch_sequence, row.present_latest_input_sequence), (Some(latch), Some(latest)) if latest > latch))
            .count(),
        "pre_acquire_latch_with_newer_input": frames
            .iter()
            .filter(|row| matches!(row.latch_phase, InputDiagLatchPhase::PreAcquire)
                && matches!((row.latch_sequence, row.present_latest_input_sequence), (Some(latch), Some(latest)) if latest > latch))
            .count(),
    });
    // Split out of the summary literal: serde_json's json! recurses once
    // per entry, and one object with every key hit the recursion limit.
    let notes_json = serde_json::json!({
        "event_to_latch_ms": "Age of the exact mouse input sample when the renderer sampled it.",
        "gpu_frame_ms": "GPU execution time of one resolved profiler frame: the Frame timestamp interval, or the sum of the independent passes when Frame is absent. Readback lands a few render frames after the frame it measured, so only rows with gpu_sample_fresh=1 are distinct measurements and this block counts each one once.",
        "gpu_frame_budget_ratio": "gpu_frame_ms / pace_target_ms. Above 1.0 means the GPU cannot finish a frame inside the pacer's budget, so the configured FPS cap is not actually achievable and presents queue behind the GPU while every CPU-side timing still looks idle.",
        "gpu_pass_ms": "Per-pass GPU breakdown over the same distinct measurements. Several passes are sub-intervals of World/Post and overlap each other, so they do not sum to gpu_frame_ms.",
        "gpu_sample_fresh": "1 when this row is the first to report its GPU measurement; later rows repeat the previous value because timestamp readback is slower than the render loop.",
        "gpu_timestamps": "The capture force-enables GPU timestamps for its duration and restores the previous r_gpuTimings state afterwards. The timestamp writes themselves add a small amount of GPU work.",
        "present_latest_seq": "Newest input sequence visible immediately after the frame's render/present call completed.",
        "pace_late_before_wait_ms": "Positive means the render loop reached its software-cap deadline late before calling the wait routine.",
        "pace_wake_error_ms": "Signed actual wake time minus the software-cap deadline; positive is late.",
        "pace_anchor_reset": "Whether the pacer rebased because the render loop had already missed this frame's software-cap deadline before waiting.",
        "surface_config_generation": "Monotonic primary-surface configure generation. A change means Surface::configure ran between captured frames.",
        "surface_config_reason": "Reason recorded by the renderer for the most recent primary-surface configure.",
        "surface_config_age_ms": "Age of the current primary-surface configuration when this frame completed.",
        "present_call_ms": "CPU time spent in SurfaceTexture::present; this is not scanout latency.",
        "mouse_event_gap_ms": "Gap between raw DeviceEvent::MouseMotion callbacks delivered to the app; this is not a direct hardware polling-rate measurement.",
        "win32_hook_gap_ms": "Gap between WM_INPUT messages as observed by Winit's pre-dispatch Win32 message hook. This is before Winit converts the message to DeviceEvent, but after the Windows message pump receives it.",
        "win32_message_time_gap_ms": "Gap between successive MSG.time values carried by WM_INPUT. A large hook gap with a small MSG.time gap suggests queued messages were dispatched late; both gaps large suggests starvation before message dispatch.",
        "win32_to_device_event_ms": "Time from the correlated WM_INPUT pre-dispatch hook timestamp to the beginning of DeviceEvent::MouseMotion.",
        "event_loop_gap_ms": "Gap between ApplicationHandler::new_events callbacks while the capture is running.",
        "grab_mode": "Cursor grab mode last successfully applied by the app: locked, confined, or none.",
        "newer_input_visible_after_latch": "Frames where the input mailbox had a newer mouse sequence after render/present returned than the sequence actually latched for that frame.",
    });
    let summary = serde_json::json!({
        "schema_version": 4,
        "capture_seconds": CAPTURE_DURATION.as_secs_f64(),
        "client_version": env!("CARGO_PKG_VERSION"),
        "renderer": renderer_json,
        "surface_config": {
            "first_generation": surface_generation_first,
            "last_generation": surface_generation_last,
            "generation_changes_during_capture": surface_generation_changes,
            "events": surface_events,
        },
        "files": {
            "mouse": mouse_path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "event_loop": event_loop_path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "win32": win32_path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "frames": frame_path.file_name().map(|name| name.to_string_lossy().into_owned()),
        },
        "counts": counts_json,
        "mouse_event_gap_ms": stats_json(&mouse_gaps),
        "mouse_callback_ms": stats_json(&callback),
        "event_loop_gap_ms": stats_json(&event_loop_gaps),
        "win32_hook_gap_ms": stats_json(&win32_hook_gaps),
        "win32_message_time_gap_ms": stats_json(&win32_message_time_gaps),
        "win32_to_device_event_ms": stats_json(&win32_to_device),
        "frame_interval_ms": stats_json(&frame_intervals),
        "gpu_frame_ms": stats_json(&gpu_frame),
        "gpu_frame_budget_ratio": stats_json(&gpu_budget_ratio),
        "gpu_pass_ms": serde_json::Value::Object(gpu_pass_ms),
        "surface_acquire_ms": stats_json(&acquire),
        "input_event_to_latch_ms": stats_json(&latch_age),
        "render_command_drain_ms": stats_json(&command_drain),
        "pacer_wake_error_ms": stats_json(&pace_wake_error),
        "pacer_late_without_reset_ms": stats_json(
            &frames
                .iter()
                .filter(|row| row.pace_late_before_wait_ms > 0.0 && !row.pace_anchor_reset)
                .map(|row| row.pace_late_before_wait_ms)
                .collect::<Vec<_>>()
        ),
        "notes": notes_json,
    });
    let json = serde_json::to_vec_pretty(&summary).map_err(std::io::Error::other)?;
    std::fs::write(&summary_path, json)?;

    Ok(CaptureSummary {
        summary_path,
        max_mouse_gap_ms,
        max_frame_interval_ms,
        max_acquire_ms,
        acquire_p50_ms,
        acquire_p95_ms,
        max_latch_age_ms,
        gpu_frame_ms_p50,
        gpu_frame_ms_max,
        gpu_measurements,
        gpu_frames_over_budget,
        surface_generation_first,
        surface_generation_last,
        surface_generation_changes,
    })
}

fn suffixed_path(stem: &Path, suffix: &str) -> PathBuf {
    let mut value = stem.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn opt_u64(value: Option<u64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn opt_f64(value: Option<f64>) -> String {
    value.map_or_else(String::new, fmt_f64)
}

fn fmt_f64(value: f64) -> String {
    format!("{value:.6}")
}

fn max_or_zero(values: &[f64]) -> f64 {
    values.iter().copied().fold(0.0, f64::max)
}

fn stats_json(values: &[f64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::json!({"count": 0});
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let sum: f64 = sorted.iter().sum();
    serde_json::json!({
        "count": sorted.len(),
        "avg": sum / sorted.len() as f64,
        "p50": percentile(&sorted, 0.50),
        "p95": percentile(&sorted, 0.95),
        "p99": percentile(&sorted, 0.99),
        "max": sorted.last().copied().unwrap_or(0.0),
    })
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}
