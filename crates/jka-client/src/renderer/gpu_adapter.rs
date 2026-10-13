//! Startup-only adapter selection. This does not move individual draw calls to the CPU.
//! No adapter enumeration occurs in the frame loop.

/// 0=automatic, 1=discrete, 2=integrated, 3=CPU/software when exposed by WGPU.
pub(crate) const LABELS: [&str; 4] = ["Automatic", "Dedicated GPU", "Integrated GPU", "Software / CPU"];

pub(crate) fn label(mode: u8) -> &'static str {
    LABELS.get(usize::from(mode)).copied().unwrap_or(LABELS[0])
}

pub(crate) fn parse(value: &str) -> Option<u8> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "auto" | "automatic" => Some(0),
        "1" | "dedicated" | "discrete" | "dgpu" => Some(1),
        "2" | "integrated" | "igpu" | "apu" => Some(2),
        "3" | "software" | "cpu" | "fallback" => Some(3),
        _ => None,
    }
}

pub(crate) fn device_type(mode: u8) -> Option<wgpu::DeviceType> {
    match mode {
        1 => Some(wgpu::DeviceType::DiscreteGpu),
        2 => Some(wgpu::DeviceType::IntegratedGpu),
        3 => Some(wgpu::DeviceType::Cpu),
        _ => None,
    }
}

pub(crate) async fn enumerate(instance: &wgpu::Instance) -> Vec<wgpu::Adapter> {
    instance.enumerate_adapters(wgpu::Backends::all()).await
}

pub(crate) async fn select(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
    mode: u8,
) -> Result<wgpu::Adapter, String> {
    if let Some(kind) = device_type(mode) {
        let adapters = enumerate(instance).await;
        if let Some(adapter) = adapters.into_iter().find(|adapter| {
            adapter.get_info().device_type == kind && adapter.is_surface_supported(surface)
        }) {
            return Ok(adapter);
        }
        // A stale config must never strand a user on a machine with no matching GPU.
        eprintln!("[GPU] Requested {} is unavailable for this backend/surface; using Automatic", label(mode));
    }
    instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(surface),
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }).await.map_err(|err| format!("No compatible GPU adapter: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_choices_map_to_the_intended_hardware_categories() {
        assert_eq!(parse("auto"), Some(0));
        assert_eq!(parse("dedicated"), Some(1));
        assert_eq!(parse("apu"), Some(2));
        assert_eq!(parse("software"), Some(3));
        assert_eq!(device_type(1), Some(wgpu::DeviceType::DiscreteGpu));
        assert_eq!(device_type(2), Some(wgpu::DeviceType::IntegratedGpu));
        assert_eq!(device_type(3), Some(wgpu::DeviceType::Cpu));
        assert!(parse("4").is_none());
    }
}
