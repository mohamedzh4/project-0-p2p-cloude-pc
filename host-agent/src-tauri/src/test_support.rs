//! Builders shared by the unit tests so each module does not repeat fixtures.

use crate::models::{CpuInfo, GpuInfo, GpuVendor, HardwareSnapshot, MemoryInfo, BYTES_PER_GIB};

pub fn snapshot(logical_cores: u32, total_bytes: u64, gpus: Vec<GpuInfo>) -> HardwareSnapshot {
    HardwareSnapshot {
        cpu: CpuInfo {
            model: "Test CPU".to_string(),
            logical_cores,
            physical_cores: Some((logical_cores / 2).max(1)),
        },
        memory: MemoryInfo { total_bytes },
        gpus,
        gpu_note: None,
    }
}

pub fn nvidia_gpu() -> GpuInfo {
    nvidia_gpu_with("NVIDIA GeForce RTX 4070", "GPU-TEST-4070", 12)
}

pub fn nvidia_gpu_with(model: &str, id: &str, vram_gib: u64) -> GpuInfo {
    GpuInfo {
        id: id.to_string(),
        model: model.to_string(),
        vendor: GpuVendor::Nvidia,
        vram_bytes: Some(vram_gib * BYTES_PER_GIB),
        integrated: false,
    }
}
