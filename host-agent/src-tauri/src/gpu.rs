//! GPU detection.
//!
//! Unlike CPU and RAM, there is no portable way to enumerate GPUs. NVIDIA
//! exposes NVML, which is what this module uses; every other case (AMD, Intel,
//! Apple, no driver, no card) degrades to an empty result with a note instead
//! of an error, because a machine without a supported GPU is still a perfectly
//! good Standard Lab host.

use crate::models::{GpuInfo, GpuProbe, GpuVendor};

pub fn probe() -> GpuProbe {
    platform::probe()
}

// Only the NVML path and the tests build these, so they are dead code on
// platforms without NVML.
#[cfg_attr(
    not(any(target_os = "windows", target_os = "linux", test)),
    allow(dead_code)
)]
fn nvidia_gpu(raw_name: &str, vram_bytes: u64, id: impl Into<String>) -> GpuInfo {
    GpuInfo {
        id: id.into(),
        model: normalize_model(raw_name, GpuVendor::Nvidia),
        vendor: GpuVendor::Nvidia,
        vram_bytes: (vram_bytes > 0).then_some(vram_bytes),
    }
}

/// NVML UUID when the driver reports one; otherwise a per-index fallback so
/// two unnamed cards are still distinguishable in the config file.
#[cfg_attr(
    not(any(target_os = "windows", target_os = "linux", test)),
    allow(dead_code)
)]
pub fn gpu_identity(uuid: Option<&str>, index: u32) -> String {
    match uuid {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => format!("nvidia:{index}"),
    }
}

#[cfg_attr(
    not(any(target_os = "windows", target_os = "linux", test)),
    allow(dead_code)
)]
fn normalize_model(raw: &str, vendor: GpuVendor) -> String {
    let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");

    if !cleaned.is_empty() {
        return cleaned;
    }

    match vendor {
        GpuVendor::Nvidia => "Unknown NVIDIA GPU".to_string(),
        GpuVendor::Unknown => "Unknown GPU".to_string(),
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod platform {
    use super::{gpu_identity, nvidia_gpu};
    use crate::models::GpuProbe;
    use nvml_wrapper::error::NvmlError;
    use nvml_wrapper::Nvml;

    pub fn probe() -> GpuProbe {
        match Nvml::init() {
            Ok(nvml) => collect(&nvml),
            Err(NvmlError::LibloadingError(_)) | Err(NvmlError::LibraryNotFound) => GpuProbe::none(
                "No NVIDIA management library (NVML) found. Install the NVIDIA driver if this machine has an NVIDIA GPU.",
            ),
            Err(NvmlError::DriverNotLoaded) => {
                GpuProbe::none("The NVIDIA driver is not loaded on this machine.")
            }
            Err(NvmlError::NoPermission) => GpuProbe::none(
                "Access to the NVIDIA driver was denied. Run the agent with permission to query the GPU.",
            ),
            Err(error) => GpuProbe::none(format!("NVIDIA GPU detection failed: {error}")),
        }
    }

    fn collect(nvml: &Nvml) -> GpuProbe {
        let count = match nvml.device_count() {
            Ok(count) => count,
            Err(error) => return GpuProbe::none(format!("Could not count NVIDIA GPUs: {error}")),
        };

        let mut gpus = Vec::new();
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                // One unreadable device should not hide the others.
                continue;
            };

            let name = device.name().unwrap_or_default();
            let vram_bytes = device.memory_info().map(|memory| memory.total).unwrap_or(0);
            let id = gpu_identity(device.uuid().ok().as_deref(), index);
            gpus.push(nvidia_gpu(&name, vram_bytes, id));
        }

        if gpus.is_empty() {
            GpuProbe::none("No supported GPU detected.")
        } else {
            GpuProbe::detected(gpus)
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use crate::models::GpuProbe;

    pub fn probe() -> GpuProbe {
        GpuProbe::none("GPU detection currently supports NVIDIA GPUs on Windows and Linux only.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_reported_nvidia_model_name() {
        let gpu = nvidia_gpu("NVIDIA GeForce RTX 4070", 12_884_901_888, "GPU-4070");

        assert_eq!(gpu.model, "NVIDIA GeForce RTX 4070");
        assert_eq!(gpu.id, "GPU-4070");
        assert_eq!(gpu.vendor, GpuVendor::Nvidia);
        assert_eq!(gpu.vram_bytes, Some(12_884_901_888));
    }

    #[test]
    fn treats_zero_vram_as_unknown() {
        assert_eq!(
            nvidia_gpu("NVIDIA GeForce RTX 4070", 0, "GPU-4070").vram_bytes,
            None
        );
    }

    #[test]
    fn falls_back_when_the_driver_reports_no_name() {
        assert_eq!(nvidia_gpu("   ", 0, "nvidia:0").model, "Unknown NVIDIA GPU");
        assert_eq!(normalize_model("", GpuVendor::Unknown), "Unknown GPU");
    }

    #[test]
    fn collapses_padded_model_names() {
        assert_eq!(
            normalize_model("  NVIDIA   RTX  A4000 \n", GpuVendor::Nvidia),
            "NVIDIA RTX A4000"
        );
    }

    #[test]
    fn probe_never_panics_and_explains_an_empty_result() {
        let probe = probe();

        if probe.gpus.is_empty() {
            assert!(probe.note.is_some(), "an empty probe must say why");
        } else {
            let mut ids: Vec<&str> = probe.gpus.iter().map(|gpu| gpu.id.as_str()).collect();
            assert!(probe.gpus.iter().all(|gpu| !gpu.model.is_empty()));
            assert!(ids.iter().all(|id| !id.is_empty()));
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), probe.gpus.len(), "each GPU needs a unique id");
        }
    }

    #[test]
    fn prefers_the_driver_uuid_over_an_index_fallback() {
        assert_eq!(gpu_identity(Some("GPU-ABCDEF"), 0), "GPU-ABCDEF");
        assert_eq!(gpu_identity(Some("  GPU-ABCDEF  "), 0), "GPU-ABCDEF");
    }

    #[test]
    fn enumerates_multiple_gpus_with_distinct_fallback_ids() {
        let first = gpu_identity(None, 0);
        let second = gpu_identity(Some(""), 1);
        let third = gpu_identity(Some("   "), 2);

        assert_eq!(first, "nvidia:0");
        assert_eq!(second, "nvidia:1");
        assert_eq!(third, "nvidia:2");

        let gpus = [
            nvidia_gpu("NVIDIA GeForce RTX 4070", 1, first),
            nvidia_gpu("NVIDIA GeForce RTX 3060", 1, second),
            nvidia_gpu("NVIDIA RTX A4000", 1, third),
        ];
        let ids: std::collections::HashSet<_> = gpus.iter().map(|gpu| gpu.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
    }
}
