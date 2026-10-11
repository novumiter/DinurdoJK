//! Discover all live process threads, including renderer pools and library threads.
//! Sampling runs off the render thread and is suspended when the advanced HUD is off.
use super::{SLOTS, Task, ThreadActivitySample, slot_label, snapshot};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Threading::{
            GetCurrentThread, GetCurrentThreadId, GetThreadDescription, GetThreadTimes, OpenThread,
            THREAD_QUERY_LIMITED_INFORMATION,
        },
        WindowsProgramming::{SYSTEM_PROCESS_INFORMATION, SYSTEM_THREAD_INFORMATION},
    },
};

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn ticks(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

pub(super) fn current_identity() -> (u32, u64) {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        );
        (GetCurrentThreadId(), ticks(created))
    }
}

/// Win32 owns the returned UTF-16 allocation; always release it with LocalFree.
fn description(handle: HANDLE) -> Option<Arc<str>> {
    unsafe {
        let mut ptr = std::ptr::null_mut();
        if GetThreadDescription(handle, &mut ptr) < 0 || ptr.is_null() {
            return None;
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        LocalFree(ptr.cast());
        (!name.is_empty()).then(|| name.into())
    }
}

struct Previous {
    name: Arc<str>,
    named: bool,
    cpu: u64,
    observed: Instant,
}

#[derive(Default)]
struct Sampler {
    // A Windows thread ID can be reused. Creation time distinguishes lifetimes.
    previous: HashMap<(u32, u64), Previous>,
    system_buffer: Vec<u64>,
}

type QuerySystemInformation =
    unsafe extern "system" fn(i32, *mut std::ffi::c_void, u32, *mut u32) -> i32;

/// Resolve the optional native entry point dynamically. SDK layouts and every
/// record boundary are validated; any unavailable/changed API uses Toolhelp.
fn bulk_thread_ids(buffer: &mut Vec<u64>) -> Option<Vec<u32>> {
    static QUERY: OnceLock<Option<QuerySystemInformation>> = OnceLock::new();
    let query = (*QUERY.get_or_init(|| unsafe {
        let dll: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
        let module = GetModuleHandleW(dll.as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, c"NtQuerySystemInformation".as_ptr().cast()).map(|function| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, QuerySystemInformation>(
                function,
            )
        })
    }))?;
    if buffer.is_empty() {
        buffer.resize(512 * 1024 / 8, 0);
    }
    for _ in 0..4 {
        let mut returned = 0;
        let bytes = buffer.len().checked_mul(8)?;
        let status = unsafe {
            query(
                5,
                buffer.as_mut_ptr().cast(),
                bytes.try_into().ok()?,
                &mut returned,
            )
        };
        if status >= 0 {
            if returned == 0 || returned as usize > bytes {
                return None;
            }
            // u64 storage is aligned; parser still uses read_unaligned to avoid
            // imposing alignment requirements on offsets returned by Windows.
            let data = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), returned as usize)
            };
            return parse_thread_ids(data, std::process::id());
        }
        if status as u32 != 0xc0000004 {
            return None;
        } // STATUS_INFO_LENGTH_MISMATCH
        let grow = (returned as usize)
            .saturating_add(64 * 1024)
            .max(bytes.saturating_mul(2));
        if grow > 64 * 1024 * 1024 {
            return None;
        }
        buffer.resize(grow.div_ceil(8), 0);
    }
    None
}

fn parse_thread_ids(data: &[u8], pid: u32) -> Option<Vec<u32>> {
    let process_size = std::mem::size_of::<SYSTEM_PROCESS_INFORMATION>();
    let thread_size = std::mem::size_of::<SYSTEM_THREAD_INFORMATION>();
    let mut offset = 0usize;
    loop {
        let header_end = offset.checked_add(process_size)?;
        if header_end > data.len() {
            return None;
        }
        let process = unsafe {
            std::ptr::read_unaligned(
                data.as_ptr()
                    .add(offset)
                    .cast::<SYSTEM_PROCESS_INFORMATION>(),
            )
        };
        let record_end = if process.NextEntryOffset == 0 {
            data.len()
        } else {
            offset.checked_add(process.NextEntryOffset as usize)?
        };
        if record_end < header_end || record_end > data.len() {
            return None;
        }
        let threads_end =
            header_end.checked_add((process.NumberOfThreads as usize).checked_mul(thread_size)?)?;
        if threads_end > record_end {
            return None;
        }
        if process.UniqueProcessId as usize == pid as usize {
            let mut ids = Vec::with_capacity(process.NumberOfThreads as usize);
            for start in (header_end..threads_end).step_by(thread_size) {
                let thread = unsafe {
                    std::ptr::read_unaligned(
                        data.as_ptr().add(start).cast::<SYSTEM_THREAD_INFORMATION>(),
                    )
                };
                if thread.ClientId.UniqueProcess as usize != pid as usize {
                    return None;
                }
                ids.push((thread.ClientId.UniqueThread as usize).try_into().ok()?);
            }
            return Some(ids);
        }
        if process.NextEntryOffset == 0 {
            return None;
        }
        offset = record_end;
    }
}

fn toolhelp_thread_ids() -> Result<Vec<u32>, ()> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(());
    }
    let handle = OwnedHandle(handle);
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Thread32First(handle.0, &mut entry) } != 0;
    if !more {
        return Err(());
    }
    let mut ids = Vec::new();
    while more {
        if entry.th32OwnerProcessID == std::process::id() {
            ids.push(entry.th32ThreadID);
        }
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        more = unsafe { Thread32Next(handle.0, &mut entry) } != 0;
    }
    Ok(ids)
}

fn percent(previous: u64, current: u64, elapsed: Duration) -> f32 {
    if elapsed.is_zero() {
        return 0.0;
    }
    // CPU ticks are 100 ns. 100% means one logical core, not the whole machine.
    ((current.saturating_sub(previous) as f64 * 1e-7 / elapsed.as_secs_f64()) * 100.0)
        .clamp(0.0, 100.0) as f32
}

fn priority(name: &str) -> u8 {
    match name {
        "MAIN" => 0,
        "RENDER" => 1,
        "jka-encoder-finish" => 2,
        name if name.starts_with("jka-model-pack-") => 3,
        name if name.starts_with("jka-g2-skin-") => 4,
        "jka-fx" => 5,
        name if name.starts_with("jka-fx-geom-") => 6,
        name if name.starts_with("jka-cloth") || name.starts_with("jka-jiggle") => 7,
        name if name.starts_with("jka-event-") => 8,
        name if name.starts_with("pipeline-compile") => 9,
        name if name.starts_with("jka-") => 10,
        _ => 11,
    }
}

fn work_label(name: &str) -> &'static str {
    if name == "jka-encoder-finish" {
        "ENCODER FINISH"
    } else if name.starts_with("jka-model-pack-") {
        "MODEL PACK"
    } else if name.starts_with("jka-g2-skin-") {
        "G2 SKIN"
    } else if name.starts_with("jka-fx-geom-") {
        "FX GEOMETRY"
    } else if name == "jka-fx" {
        "FX SIMULATION"
    } else if name.starts_with("pipeline-compile") {
        "PIPELINE COMPILE"
    } else if name.starts_with("jka-cloth") {
        "CLOTH PHYSICS"
    } else if name.starts_with("jka-jiggle") {
        "JIGGLE PHYSICS"
    } else if name.starts_with("jka-static-ao") {
        "STATIC AO"
    } else if name.starts_with("jka-snow") {
        "SNOW SHELL"
    } else if name.contains("audio") || name.contains("acoustics") {
        "AUDIO"
    } else {
        "CPU WORK"
    }
}

impl Sampler {
    fn collect(&mut self) -> Result<Arc<[ThreadActivitySample]>, ()> {
        let ids = match bulk_thread_ids(&mut self.system_buffer) {
            Some(ids) => ids,
            None => toolhelp_thread_ids()?,
        };
        let raw = snapshot();
        let mut next = HashMap::with_capacity(self.previous.len());
        let mut samples = Vec::with_capacity(self.previous.len());
        for id in ids {
            let thread = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, id) };
            if !thread.is_null() {
                let thread = OwnedHandle(thread);
                let (mut created, mut exited, mut kernel, mut user) = (
                    FILETIME::default(),
                    FILETIME::default(),
                    FILETIME::default(),
                    FILETIME::default(),
                );
                if unsafe {
                    GetThreadTimes(thread.0, &mut created, &mut exited, &mut kernel, &mut user)
                } != 0
                {
                    let observed = Instant::now();
                    let key = (id, ticks(created));
                    let cpu = ticks(kernel).saturating_add(ticks(user));
                    let previous = self.previous.remove(&key);
                    let busy = previous.as_ref().map_or(0.0, |p| {
                        percent(p.cpu, cpu, observed.saturating_duration_since(p.observed))
                    });
                    let semantic = raw.iter().find(|s| {
                        let slot = &SLOTS[s.slot as usize];
                        slot.thread_id.load(Ordering::Acquire) == id
                            && slot.thread_created.load(Ordering::Relaxed) == key.1
                    });
                    let (name, named) = if let Some(s) = semantic {
                        // Preserve existing task names without using their wall-time percentages.
                        (Arc::from(slot_label(s.slot)), true)
                    } else if let Some(p) = previous.as_ref().filter(|p| p.named) {
                        (p.name.clone(), true)
                    } else if let Some(name) = description(thread.0) {
                        (name, true)
                    } else {
                        (Arc::from(format!("THREAD {id}")), false)
                    };
                    let active = busy >= 0.1;
                    let task = if previous.is_none() {
                        "WARMUP"
                    } else if let Some(s) = semantic.filter(|s| s.task != Task::Idle) {
                        s.task.label()
                    } else if active {
                        work_label(&name)
                    } else {
                        "WAIT / IDLE"
                    };
                    samples.push(ThreadActivitySample {
                        id,
                        name: name.clone(),
                        active,
                        task,
                        busy_percent: busy,
                    });
                    next.insert(
                        key,
                        Previous {
                            name,
                            named,
                            cpu,
                            observed,
                        },
                    );
                } else {
                    samples.push(unavailable(id));
                }
            } else {
                // Includes a thread that exited between snapshot and OpenThread.
                samples.push(unavailable(id));
            }
        }
        self.previous = next; // Remove exited threads, rather than retaining stale rows.
        // Stable pool order: CPU activity cannot move a worker to another page.
        samples.sort_by(|a, b| {
            (priority(&a.name), &a.name, a.id).cmp(&(priority(&b.name), &b.name, b.id))
        });
        Ok(samples.into())
    }
}

fn unavailable(id: u32) -> ThreadActivitySample {
    ThreadActivitySample {
        id,
        name: if id == 0 {
            "CPU MONITOR".into()
        } else {
            format!("THREAD {id}").into()
        },
        active: false,
        task: if id == 0 {
            "QUERY FAILED"
        } else {
            "CPU UNAVAILABLE"
        },
        busy_percent: 0.0,
    }
}

struct Monitor {
    enabled: AtomicBool,
    latest: Mutex<Arc<[ThreadActivitySample]>>,
    gate: Mutex<()>,
    wake: Condvar,
}

static MONITOR: OnceLock<Option<Arc<Monitor>>> = OnceLock::new();

pub(super) fn sample(enabled: bool) -> Arc<[ThreadActivitySample]> {
    // No native sampling thread, enumeration, or allocation until first enabled.
    if !enabled && MONITOR.get().is_none() {
        return Arc::from([]);
    }
    let monitor = MONITOR.get_or_init(|| {
        let monitor = Arc::new(Monitor {
            enabled: AtomicBool::new(false),
            latest: Mutex::new(Arc::from([])),
            gate: Mutex::new(()),
            wake: Condvar::new(),
        });
        let worker = monitor.clone();
        match std::thread::Builder::new()
            .name("jka-thread-monitor".into())
            .spawn(move || run(worker))
        {
            Ok(_) => Some(monitor),
            Err(error) => {
                eprintln!("Thread CPU monitor: {error}");
                None
            }
        }
    });
    let Some(monitor) = monitor else {
        return Arc::from([unavailable(0)]);
    };
    let _gate = monitor.gate.lock().unwrap_or_else(|e| e.into_inner());
    if monitor.enabled.swap(enabled, Ordering::Relaxed) != enabled {
        if enabled {
            *monitor.latest.lock().unwrap_or_else(|e| e.into_inner()) = Arc::from([]);
        }
        monitor.wake.notify_one();
    }
    if enabled {
        monitor
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    } else {
        Arc::from([])
    }
}

fn run(monitor: Arc<Monitor>) {
    let mut sampler = Sampler::default();
    loop {
        let mut gate = monitor.gate.lock().unwrap_or_else(|e| e.into_inner());
        while !monitor.enabled.load(Ordering::Relaxed) {
            sampler.previous.clear();
            gate = monitor.wake.wait(gate).unwrap_or_else(|e| e.into_inner());
        }
        drop(gate);
        let samples = sampler
            .collect()
            .unwrap_or_else(|_| Arc::from([unavailable(0)]));
        *monitor.latest.lock().unwrap_or_else(|e| e.into_inner()) = samples;
        let gate = monitor.gate.lock().unwrap_or_else(|e| e.into_inner());
        if monitor.enabled.load(Ordering::Relaxed) {
            // Hold the gate while entering the wait so a disable cannot be lost.
            drop(
                monitor
                    .wake
                    .wait_timeout(gate, Duration::from_millis(500))
                    .unwrap_or_else(|e| e.into_inner()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, mpsc};

    #[test]
    fn bulk_discovery_and_toolhelp_both_find_the_current_thread() {
        let current = current_identity().0;
        let mut buffer = Vec::new();
        let bulk = bulk_thread_ids(&mut buffer).expect("bulk query must work on this Windows test host");
        assert!(bulk.contains(&current));
        assert!(toolhelp_thread_ids().unwrap().contains(&current));
        assert!(bulk.iter().all(|id| *id != 0));
    }

    #[test]
    fn bulk_parser_rejects_truncated_and_inconsistent_records() {
        let process_size = std::mem::size_of::<SYSTEM_PROCESS_INFORMATION>();
        let thread_size = std::mem::size_of::<SYSTEM_THREAD_INFORMATION>();
        let mut data = vec![0u8; process_size + thread_size];
        let mut process = SYSTEM_PROCESS_INFORMATION { NumberOfThreads: 1, UniqueProcessId: 7usize as HANDLE, ..Default::default() };
        let mut thread = SYSTEM_THREAD_INFORMATION::default();
        thread.ClientId.UniqueProcess = 7usize as HANDLE;
        thread.ClientId.UniqueThread = 42usize as HANDLE;
        unsafe {
            std::ptr::write_unaligned(data.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>(), process);
            std::ptr::write_unaligned(data.as_mut_ptr().add(process_size).cast::<SYSTEM_THREAD_INFORMATION>(), thread);
        }
        assert_eq!(parse_thread_ids(&data, 7), Some(vec![42]));
        assert!(parse_thread_ids(&data[..data.len() - 1], 7).is_none());
        assert!(parse_thread_ids(&data, 9).is_none());
        process.NumberOfThreads = u32::MAX;
        unsafe { std::ptr::write_unaligned(data.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>(), process); }
        assert!(parse_thread_ids(&data, 7).is_none());
        process.NumberOfThreads = 1;
        process.NextEntryOffset = 1;
        unsafe { std::ptr::write_unaligned(data.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>(), process); }
        assert!(parse_thread_ids(&data, 7).is_none());
    }

    #[test]
    fn cpu_ticks_exclude_waits_and_handle_reset() {
        assert_eq!(percent(10, 5, Duration::from_millis(500)), 0.0);
        assert_eq!(percent(0, 2_500_000, Duration::from_millis(500)), 50.0);
        assert_eq!(percent(0, 10_000_000, Duration::from_millis(500)), 100.0);
        assert_eq!(percent(0, 1, Duration::ZERO), 0.0);
    }

    #[test]
    fn native_discovers_unregistered_workers_and_excludes_blocking_waits() {
        let running = Arc::new(AtomicBool::new(true));
        let phase = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = mpsc::channel();
        let run = running.clone();
        let p = phase.clone();
        let spin = std::thread::Builder::new()
            .name("jka-model-pack-audit".into())
            .spawn(move || {
                ready_tx.send(()).unwrap();
                while run.load(Ordering::Relaxed) {
                    p.fetch_add(1, Ordering::Relaxed);
                    std::hint::spin_loop();
                }
            })
            .unwrap();
        let (stop, stopped) = mpsc::channel::<()>();
        let wait = std::thread::Builder::new()
            .name("jka-encoder-finish-audit".into())
            .spawn(move || {
                let _ = stopped.recv();
            })
            .unwrap();
        ready_rx.recv().unwrap();
        let mut sampler = Sampler::default();
        sampler.collect().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let samples = sampler.collect().unwrap();
        running.store(false, Ordering::Relaxed);
        stop.send(()).unwrap();
        spin.join().unwrap();
        wait.join().unwrap();
        assert!(phase.load(Ordering::Relaxed) > 0);
        let spin = samples
            .iter()
            .find(|s| &*s.name == "jka-model-pack-audit")
            .unwrap();
        let wait = samples
            .iter()
            .find(|s| &*s.name == "jka-encoder-finish-audit")
            .unwrap();
        assert!(spin.busy_percent > 10.0, "{spin:?}");
        assert!(wait.busy_percent < 5.0, "{wait:?}");
        eprintln!(
            "Native thread audit: {} discovered threads; packing worker {:.1}% CPU; blocked finisher {:.1}% CPU",
            samples.len(),
            spin.busy_percent,
            wait.busy_percent
        );
        let after = sampler.collect().unwrap();
        assert!(!after.iter().any(|s| s.id == spin.id || s.id == wait.id));
    }

    #[test]
    #[ignore = "measurement of native thread enumeration overhead"]
    fn native_sampler_overhead() {
        let mut sampler = Sampler::default();
        sampler.collect().unwrap();
        let mut durations = Vec::new();
        let mut count = 0;
        for _ in 0..20 {
            let start = Instant::now();
            count = sampler.collect().unwrap().len();
            durations.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        durations.sort_by(f64::total_cmp);
        eprintln!(
            "Native sampler: {count} live threads; warm collection p50 {:.3} ms, p95 {:.3} ms; runs on monitor only, at 2 Hz",
            durations[10], durations[18]
        );
    }
}
