//! Data shared between the hardware probes, the config store and the UI.

use serde::{Deserialize, Serialize};

pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Bumped whenever the on-disk shape of [`SharingConfig`] changes in a way that
/// older files cannot be read as-is. v1 files (no `sharedGpuId`) still load
/// because that field is serde-defaulted, then [`SharingConfig::clamped_to`]
/// fills in a GPU that exists on this machine.
pub const CONFIG_VERSION: u32 = 2;

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
    Amd,
    Intel,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    /// Stable identity for this card, stored in the config. Linux: NVML UUID
    /// for NVIDIA cards (or `nvidia:{index}`), PCI address for the rest.
    /// Windows: PnP instance ID (what Hyper-V looks up). See `gpu.rs`.
    pub id: String,
    pub model: String,
    pub vendor: GpuVendor,
    /// `None` when the driver reports a device but not its memory size.
    pub vram_bytes: Option<u64>,
    /// An iGPU sharing the CPU package. It keeps the owner's screen, so it
    /// counts as a second display path and is never offered for dedication.
    #[serde(default)]
    pub integrated: bool,
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

    pub fn gpu_id_exists(&self, id: &str) -> bool {
        self.gpus.iter().any(|gpu| gpu.id == id)
    }

    /// GPUs the owner may dedicate to a guest (not an iGPU).
    pub fn dedicatable_gpus(&self) -> impl Iterator<Item = &GpuInfo> {
        self.gpus.iter().filter(|gpu| !gpu.integrated)
    }

    pub fn has_dedicatable_gpu(&self) -> bool {
        self.dedicatable_gpus().next().is_some()
    }

    pub fn dedicatable_gpu_id_exists(&self, id: &str) -> bool {
        self.dedicatable_gpus().any(|gpu| gpu.id == id)
    }

    pub fn first_gpu_id(&self) -> Option<String> {
        self.dedicatable_gpus().next().map(|gpu| gpu.id.clone())
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
    /// Exclusive for the session on consumer hardware — not a concurrent slice.
    pub share_gpu: bool,
    /// Which card to dedicate when [`Self::share_gpu`] is true. Absent on v1
    /// files and on machines with no supported GPU. Kept even while sharing is
    /// off so a later schedule can name the same card.
    #[serde(default)]
    pub shared_gpu_id: Option<String>,
}

impl SharingConfig {
    /// Conservative starting point: half the machine, GPU opt-in, sharing off
    /// until the host turns it on. Pre-selects the first GPU so the picker has
    /// a value before anyone opts in.
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
            shared_gpu_id: hardware.first_gpu_id(),
        }
    }

    /// Forces a config to fit the machine it was loaded on. Config files travel
    /// with backups and profile syncs, so the stored numbers cannot be trusted
    /// to match the current hardware.
    pub fn clamped_to(&self, hardware: &HardwareSnapshot) -> Self {
        let shared_gpu_id = match &self.shared_gpu_id {
            Some(id) if hardware.dedicatable_gpu_id_exists(id) => Some(id.clone()),
            _ => hardware.first_gpu_id(),
        };

        Self {
            version: CONFIG_VERSION,
            sharing_enabled: self.sharing_enabled,
            shared_logical_cores: self.shared_logical_cores.min(hardware.cpu.logical_cores),
            shared_memory_bytes: self.shared_memory_bytes.min(hardware.memory.total_bytes),
            share_gpu: self.share_gpu && hardware.has_dedicatable_gpu(),
            shared_gpu_id,
        }
    }
}

pub fn floor_to_gib(bytes: u64) -> u64 {
    (bytes / BYTES_PER_GIB) * BYTES_PER_GIB
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with, snapshot};

    fn second_gpu() -> GpuInfo {
        nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12)
    }

    fn igpu() -> GpuInfo {
        GpuInfo {
            id: "0000:00:02.0".to_string(),
            model: "Intel UHD Graphics 770".to_string(),
            vendor: GpuVendor::Intel,
            vram_bytes: None,
            integrated: true,
        }
    }

    // Issue 35: now that iGPUs are listed, the default and any saved pick
    // must never be the owner's own screen.
    #[test]
    fn an_igpu_is_never_picked_for_dedication() {
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, vec![igpu(), nvidia_gpu()]);
        assert_eq!(
            SharingConfig::recommended_for(&hardware).shared_gpu_id.as_deref(),
            Some("GPU-TEST-4070")
        );

        let mut saved = SharingConfig::recommended_for(&hardware);
        saved.share_gpu = true;
        saved.shared_gpu_id = Some("0000:00:02.0".to_string());
        let clamped = saved.clamped_to(&hardware);
        assert_eq!(clamped.shared_gpu_id.as_deref(), Some("GPU-TEST-4070"));

        let igpu_only = snapshot(16, 32 * BYTES_PER_GIB, vec![igpu()]);
        let clamped = saved.clamped_to(&igpu_only);
        assert!(!clamped.share_gpu);
        assert_eq!(clamped.shared_gpu_id, None);
    }

    #[test]
    fn old_config_and_hardware_json_without_integrated_still_load() {
        let gpu: GpuInfo = serde_json::from_str(
            r#"{"id":"GPU-1","model":"NVIDIA GeForce RTX 4070","vendor":"nvidia","vramBytes":null}"#,
        )
        .unwrap();
        assert!(!gpu.integrated);
    }

    #[test]
    fn recommends_half_the_machine_with_sharing_off() {
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, vec![nvidia_gpu()]);

        let config = SharingConfig::recommended_for(&hardware);

        assert_eq!(config.shared_logical_cores, 8);
        assert_eq!(config.shared_memory_bytes, 16 * BYTES_PER_GIB);
        assert!(!config.sharing_enabled);
        assert!(!config.share_gpu, "GPU sharing is opt-in");
        assert_eq!(config.shared_gpu_id.as_deref(), Some("GPU-TEST-4070"));
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
            shared_gpu_id: Some("GPU-GONE".to_string()),
        };

        let clamped = stored.clamped_to(&hardware);

        assert_eq!(clamped.shared_logical_cores, 8);
        assert_eq!(clamped.shared_memory_bytes, 16 * BYTES_PER_GIB);
        assert!(!clamped.share_gpu, "no GPU on this machine to share");
        assert!(clamped.shared_gpu_id.is_none());
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
            shared_gpu_id: Some("GPU-TEST-4070".to_string()),
        };

        assert_eq!(stored.clamped_to(&hardware), stored);
    }

    #[test]
    fn keeps_the_selected_gpu_when_several_are_present() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu(), second_gpu()]);
        let stored = SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: 4,
            shared_memory_bytes: 8 * BYTES_PER_GIB,
            share_gpu: true,
            shared_gpu_id: Some("GPU-TEST-3060".to_string()),
        };

        assert_eq!(
            stored.clamped_to(&hardware).shared_gpu_id.as_deref(),
            Some("GPU-TEST-3060")
        );
    }

    #[test]
    fn falls_back_to_the_first_gpu_when_the_stored_id_is_gone() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu(), second_gpu()]);
        let stored = SharingConfig {
            version: 1,
            sharing_enabled: true,
            shared_logical_cores: 4,
            shared_memory_bytes: 8 * BYTES_PER_GIB,
            share_gpu: true,
            shared_gpu_id: Some("GPU-REMOVED".to_string()),
        };

        let clamped = stored.clamped_to(&hardware);

        assert_eq!(clamped.version, CONFIG_VERSION);
        assert_eq!(clamped.shared_gpu_id.as_deref(), Some("GPU-TEST-4070"));
        assert!(clamped.share_gpu);
    }

    #[test]
    fn loads_a_v1_config_that_has_no_gpu_id() {
        let json = r#"{
            "version": 1,
            "sharingEnabled": true,
            "sharedLogicalCores": 4,
            "sharedMemoryBytes": 8589934592,
            "shareGpu": true
        }"#;

        let stored: SharingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(stored.shared_gpu_id, None);

        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu(), second_gpu()]);
        let clamped = stored.clamped_to(&hardware);

        assert_eq!(clamped.shared_gpu_id.as_deref(), Some("GPU-TEST-4070"));
        assert!(clamped.share_gpu);
    }

    #[test]
    fn floor_to_gib_rounds_down_to_whole_gibibytes() {
        assert_eq!(floor_to_gib(0), 0);
        assert_eq!(floor_to_gib(BYTES_PER_GIB - 1), 0);
        assert_eq!(floor_to_gib(BYTES_PER_GIB), BYTES_PER_GIB);
        assert_eq!(floor_to_gib(3 * BYTES_PER_GIB + 999), 3 * BYTES_PER_GIB);
    }
}
