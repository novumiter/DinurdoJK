//! Process-lifetime switches for isolating native RT pipeline compiler crashes.
use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::renderer) struct ReceiverFeatures {
    pub denoiser: bool,
    pub glass: bool,
}

pub(in crate::renderer) struct Diagnostics {
    pub receiver: ReceiverFeatures,
    pub trace: bool,
    pub serialize: bool,
    pub dump_dir: Option<PathBuf>,
}

pub(in crate::renderer) fn config() -> &'static Diagnostics {
    static CONFIG: OnceLock<Diagnostics> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let flag = |name: &str, default: bool| {
            std::env::var(name).ok().map_or(default, |v| match v.trim() {
                "0" => false,
                "1" => true,
                _ => {
                    eprintln!("[RT isolation] {name} expects 0 or 1; using {default}");
                    default
                }
            })
        };
        let receiver = ReceiverFeatures {
            denoiser: flag("JKA_RT_RECEIVER_DENOISER", true),
            glass: flag("JKA_RT_RECEIVER_GLASS", true),
        };
        let serialize = flag("JKA_RT_SERIALIZE_WORLD_PIPELINES", false);
        let dump_dir = std::env::var_os("JKA_RT_SHADER_DUMP_DIR")
            .filter(|value| !value.is_empty()).map(PathBuf::from);
        let trace = flag("JKA_RT_PIPELINE_TRACE", false)
            || !receiver.denoiser || !receiver.glass || serialize || dump_dir.is_some();
        if trace {
            println!("[RT isolation] receiver={receiver:?} serialize_world={serialize} dump_dir={dump_dir:?}; switches require a process restart");
        }
        Diagnostics { receiver, trace, serialize, dump_dir }
    })
}

pub(in crate::renderer) fn compile_guard(is_rt: bool) -> Option<MutexGuard<'static, ()>> {
    static COMPILE: Mutex<()> = Mutex::new(());
    (is_rt && config().serialize).then(|| {
        COMPILE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    })
}

pub(in crate::renderer) fn world_shader_hash(enhanced: bool, denoiser: bool) -> u64 {
    static HASHES: OnceLock<[u64; 4]> = OnceLock::new();
    HASHES.get_or_init(|| {
        std::array::from_fn(|index| {
            let base = if index % 2 == 1 {
                include_str!("../../bsp.wgsl")
            } else {
                include_str!("../../bsp_lean.wgsl")
            };
            let features = ReceiverFeatures {
                denoiser: index >= 2,
                glass: config().receiver.glass,
            };
            let source =
                super::ray_tracing::ray_traced_world_shader_source_with_features(base, features)
                    .expect("RT world shader anchors");
            crate::pipeline_jobs::hash_config(&super::ray_tracing::compose_world_shader(&source))
        })
    })[usize::from(enhanced) + 2 * usize::from(denoiser)]
}

pub(in crate::renderer) fn dump_shader(family: &str, source: &str) {
    if !config().trace {
        return;
    }
    let hash = crate::pipeline_jobs::hash_config(&source);
    println!(
        "[RT shader] family={family} hash={hash:016x} bytes={} receiver={:?}",
        source.len(),
        config().receiver
    );
    if let Some(directory) = &config().dump_dir {
        let path = directory.join(format!("{family}-{hash:016x}.wgsl"));
        let result =
            std::fs::create_dir_all(directory).and_then(|()| std::fs::write(&path, source));
        match result {
            Ok(()) => println!("[RT shader] saved {}", path.display()),
            Err(error) => eprintln!("[RT shader] cannot save {}: {error}", path.display()),
        }
    }
}
