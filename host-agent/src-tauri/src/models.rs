//! Data shared between the hardware probes, the config store and the UI.

use serde::{Deserialize, Serialize};

pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Bumped whenever the on-disk shape of [`SharingConfig`] changes in a way that
/// older files cannot be read as-is.
pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub model: String,
    pub logical_cores: u32,
    pub physical_cores: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub model: String,
    pub vendor: GpuVendor,
    /// `None` when the driver reports a device but not its memory size.
    pub vram_bytes: Option<u64>,
}

/// Result of a GPU probe. `note` explains an empty `gpus` list so the UI can
/// tell "no NVIDIA driver here" apart from "driver present, no device".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProbe {
    pub gpus: Vec<GpuInfo>,
    pub note: Option<String>,
}

impl GpuProbe {
    // Only the NVML probe reports detected GPUs, so this is unused elsewhere.
    #[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
    pub fn detected(gpus: Vec<GpuInfo>) -> Self {
        Self { gpus, note: None }
    }

    pub fn none(note: impl Into<String>) -> Self {
        Self {
            gpus: Vec::new(),
            note: Some(note.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareSnapshot {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub gpus: Vec<GpuInfo>,
    pub gpu_note: Option<String>,
}

impl HardwareSnapshot {
    pub fn has_gpu(&self) -> bool {
        !self.gpus.is_empty()
    }
}

/// How much of the machine the host is willing to lend out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingConfig {
    pub version: u32,
    pub sharing_enabled: bool,
    pub shared_logical_cores: u32,
    pub shared_memory_bytes: u64,
    pub share_gpu: bool,
}

impl SharingConfig {
    /// Conservative starting point: half the machine, GPU opt-in, sharing off
    /// until the host turns it on.
    pub fn recommended_for(hardware: &HardwareSnapshot) -> Self {
        let half_memory = hardware.memory.total_bytes / 2;
        let whole_gib = floor_to_gib(half_memory);

        Self {
            version: CONFIG_VERSION,
            sharing_enabled: false,
            shared_logical_cores: (hardware.cpu.logical_cores / 2).max(1),
            shared_memory_bytes: if whole_gib == 0 {
                half_memory
            } else {
                whole_gib
            },
            share_gpu: false,
        }
    }

    /// Forces a config to fit the machine it was loaded on. Config files travel
    /// with backups and profile syncs, so the stored numbers cannot be trusted
    /// to match the current hardware.
    pub fn clamped_to(&self, hardware: &HardwareSnapshot) -> Self {
        Self {
            version: CONFIG_VERSION,
            sharing_enabled: self.sharing_enabled,
            shared_logical_cores: self.shared_logical_cores.min(hardware.cpu.logical_cores),
            shared_memory_bytes: self.shared_memory_bytes.min(hardware.memory.total_bytes),
            share_gpu: self.share_gpu && hardware.has_gpu(),
        }
    }
}

pub fn floor_to_gib(bytes: u64) -> u64 {
    (bytes / BYTES_PER_GIB) * BYTES_PER_GIB
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{nvidia_gpu, snapshot};

    #[test]
    fn recommends_half_the_machine_with_sharing_off() {
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, vec![nvidia_gpu()]);

        let config = SharingConfig::recommended_for(&hardware);

        assert_eq!(config.shared_logical_cores, 8);
        assert_eq!(config.shared_memory_bytes, 16 * BYTES_PER_GIB);
        assert!(!config.sharing_enabled);
        assert!(!config.share_gpu, "GPU sharing is opt-in");
    }

    #[test]
    fn recommends_at_least_one_core_on_a_single_core_machine() {
        let hardware = snapshot(1, 2 * BYTES_PER_GIB, Vec::new());

        assert_eq!(
            SharingConfig::recommended_for(&hardware).shared_logical_cores,
            1
        );
    }

    #[test]
    fn recommended_memory_falls_back_below_one_gib() {
        let hardware = snapshot(2, BYTES_PER_GIB, Vec::new());

        // Half of 1 GiB floors to 0 whole GiB, so the raw half is used instead.
        assert_eq!(
            SharingConfig::recommended_for(&hardware).shared_memory_bytes,
            BYTES_PER_GIB / 2
        );
    }

    #[test]
    fn clamps_values_that_exceed_the_machine() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, Vec::new());
        let stored = SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: 64,
            shared_memory_bytes: 512 * BYTES_PER_GIB,
            share_gpu: true,
        };

        let clamped = stored.clamped_to(&hardware);

        assert_eq!(clamped.shared_logical_cores, 8);
        assert_eq!(clamped.shared_memory_bytes, 16 * BYTES_PER_GIB);
        assert!(!clamped.share_gpu, "no GPU on this machine to share");
        assert!(clamped.sharing_enabled, "host intent is preserved");
    }

    #[test]
    fn keeps_values_that_already_fit() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let stored = SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: 4,
            shared_memory_bytes: 8 * BYTES_PER_GIB,
            share_gpu: true,
        };

        assert_eq!(stored.clamped_to(&hardware), stored);
    }

    #[test]
    fn floor_to_gib_rounds_down_to_whole_gibibytes() {
        assert_eq!(floor_to_gib(0), 0);
        assert_eq!(floor_to_gib(BYTES_PER_GIB - 1), 0);
        assert_eq!(floor_to_gib(BYTES_PER_GIB), BYTES_PER_GIB);
        assert_eq!(floor_to_gib(3 * BYTES_PER_GIB + 999), 3 * BYTES_PER_GIB);
    }
}
