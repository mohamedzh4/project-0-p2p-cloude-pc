//! CPU and RAM detection.
//!
//! The probe asks `sysinfo` for the narrowest possible refresh: no process
//! list, no disks, no networks. Enumerating processes on every start would cost
//! far more than the numbers this agent actually needs.

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::gpu;
use crate::models::{CpuInfo, HardwareSnapshot, MemoryInfo};

pub fn detect() -> HardwareSnapshot {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );

    let probe = gpu::probe();

    HardwareSnapshot {
        cpu: read_cpu(&system),
        memory: MemoryInfo {
            total_bytes: system.total_memory(),
        },
        gpus: probe.gpus,
        gpu_note: probe.note,
    }
}

fn read_cpu(system: &System) -> CpuInfo {
    let cpus = system.cpus();

    CpuInfo {
        model: normalize_cpu_model(cpus.first().map(|cpu| cpu.brand()).unwrap_or_default()),
        // A machine always has at least one usable core, even if the probe
        // returns an empty list on an unsupported platform.
        logical_cores: (cpus.len() as u32).max(1),
        physical_cores: System::physical_core_count().map(|count| count as u32),
    }
}

fn normalize_cpu_model(raw: &str) -> String {
    let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");

    if cleaned.is_empty() {
        "Unknown CPU".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_padded_cpu_model_names() {
        assert_eq!(
            normalize_cpu_model("  AMD Ryzen 9   7950X  16-Core Processor \n"),
            "AMD Ryzen 9 7950X 16-Core Processor"
        );
    }

    #[test]
    fn falls_back_when_the_cpu_model_is_unavailable() {
        assert_eq!(normalize_cpu_model(""), "Unknown CPU");
        assert_eq!(normalize_cpu_model("   \t "), "Unknown CPU");
    }

    #[test]
    fn detects_a_usable_snapshot_of_the_current_machine() {
        let snapshot = detect();

        assert!(snapshot.cpu.logical_cores >= 1);
        assert!(!snapshot.cpu.model.is_empty());
        assert!(
            snapshot.memory.total_bytes > 0,
            "the host must report some RAM"
        );

        if let Some(physical) = snapshot.cpu.physical_cores {
            assert!(physical >= 1);
            assert!(physical <= snapshot.cpu.logical_cores);
        }

        let mut ids: Vec<&str> = snapshot.gpus.iter().map(|gpu| gpu.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), snapshot.gpus.len(), "each GPU needs a unique id");
    }

    #[test]
    fn reports_a_note_whenever_no_gpu_is_detected() {
        let snapshot = detect();

        assert_eq!(snapshot.has_gpu(), snapshot.gpu_note.is_none());
    }
}
