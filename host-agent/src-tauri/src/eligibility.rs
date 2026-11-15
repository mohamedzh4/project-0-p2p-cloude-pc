//! Combined hosting offer (phase 23).
//!
//! The window must show one Standard/Pro decision: the phase-18 probe, the
//! phase-21 CPU/RAM/disk floor, and the phase-22 VFIO/DDA plan. A blocked
//! tier is not offered. Reasons are plain language. The UI never asks which
//! backend to use. GPU exclusivity copy stays when Pro *is* offered.

use serde::Serialize;

use crate::gpu_bind::{GpuBindAction, GpuBindStatus};
use crate::host_capabilities::{CapabilityOs, HostCapabilities, LabOffer};
use crate::models::{HardwareSnapshot, SharingConfig, BYTES_PER_GIB};
use crate::provision::LabTier;

/// Official Kasm single-server minimum (`docs/runbooks/gate-kvm.md`).
pub const MIN_VCPU: u32 = 2;
pub const MIN_MEMORY_BYTES: u64 = 4 * BYTES_PER_GIB;
/// Kasm single-server disk floor (~50 GiB). Recommended guest disks are 64 GiB.
pub const MIN_DISK_BYTES: u64 = 50 * BYTES_PER_GIB;
const BYTES_PER_MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct EligibilityRequest<'a> {
    pub capabilities: &'a HostCapabilities,
    pub hardware: &'a HardwareSnapshot,
    pub sharing: &'a SharingConfig,
    /// Injected in tests. `None` means unknown — do not block on disk.
    pub spare_disk_bytes: Option<u64>,
    /// When shareGpu is on, a refused VFIO/DDA plan blocks Pro.
    pub gpu_bind: Option<&'a GpuBindStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EligibilityReport {
    pub standard: LabOffer,
    pub pro: LabOffer,
    pub summary: String,
}

/// Wire probe + sliders + optional GPU plan into offered vs blocked.
pub fn evaluate(request: EligibilityRequest<'_>) -> EligibilityReport {
    let standard = standard_offer(&request);
    let pro = pro_offer(&request, standard.offered);
    let summary = summary_line(request.capabilities, &standard, &pro);

    EligibilityReport {
        standard,
        pro,
        summary,
    }
}

/// True when this PC can spare the documented session floor after clamp.
pub fn resources_meet_minimum(
    hardware: &HardwareSnapshot,
    sharing: &SharingConfig,
    spare_disk_bytes: Option<u64>,
) -> Result<(), String> {
    let clamped = sharing.clamped_to(hardware);
    if clamped.shared_logical_cores < MIN_VCPU {
        return Err(format!(
            "this PC would lend {} vCPU, but a session needs at least {MIN_VCPU} vCPU",
            clamped.shared_logical_cores
        ));
    }
    if clamped.shared_memory_bytes < MIN_MEMORY_BYTES {
        return Err(format!(
            "this PC would lend {} of RAM, but a session needs at least {}",
            format_bytes_plain(clamped.shared_memory_bytes),
            format_bytes_plain(MIN_MEMORY_BYTES)
        ));
    }
    if let Some(disk) = spare_disk_bytes {
        if disk < MIN_DISK_BYTES {
            return Err(format!(
                "this PC has {} of spare disk, but a session needs at least {}",
                format_bytes_plain(disk),
                format_bytes_plain(MIN_DISK_BYTES)
            ));
        }
    }
    Ok(())
}

pub fn offer_for_tier<'a>(report: &'a EligibilityReport, lab_tier: LabTier) -> &'a LabOffer {
    match lab_tier {
        LabTier::Standard => &report.standard,
        LabTier::Pro => &report.pro,
    }
}

pub(crate) fn format_bytes_plain(bytes: u64) -> String {
    if bytes >= BYTES_PER_GIB && bytes % BYTES_PER_GIB == 0 {
        format!("{} GiB", bytes / BYTES_PER_GIB)
    } else {
        format!("{} MiB", bytes / BYTES_PER_MIB)
    }
}

fn standard_offer(request: &EligibilityRequest<'_>) -> LabOffer {
    if !request.capabilities.standard.offered {
        return request.capabilities.standard.clone();
    }
    if let Err(reason) =
        resources_meet_minimum(request.hardware, request.sharing, request.spare_disk_bytes)
    {
        return LabOffer {
            offered: false,
            reason,
        };
    }
    request.capabilities.standard.clone()
}

fn pro_offer(request: &EligibilityRequest<'_>, standard_offered: bool) -> LabOffer {
    if !standard_offered {
        if !request.capabilities.standard.offered {
            return LabOffer {
                offered: false,
                reason: request.capabilities.pro.reason.clone(),
            };
        }
        return LabOffer {
            offered: false,
            reason: "Pro sessions are not available until this PC can host Standard sessions."
                .to_string(),
        };
    }
    if !request.capabilities.pro.offered {
        return request.capabilities.pro.clone();
    }
    if let Err(reason) =
        resources_meet_minimum(request.hardware, request.sharing, request.spare_disk_bytes)
    {
        return LabOffer {
            offered: false,
            reason,
        };
    }
    if let Some(reason) = pro_bind_blocks(request) {
        return LabOffer {
            offered: false,
            reason,
        };
    }
    request.capabilities.pro.clone()
}

fn pro_bind_blocks(request: &EligibilityRequest<'_>) -> Option<String> {
    let clamped = request.sharing.clamped_to(request.hardware);
    if !clamped.share_gpu {
        return None;
    }
    let bind = request.gpu_bind?;
    if bind.action != GpuBindAction::Refused {
        return None;
    }
    if bind.log.is_empty() {
        return Some(
            "Pro sessions are not available: this PC cannot assign the dedicated GPU exclusively."
                .to_string(),
        );
    }
    Some(bind.log.clone())
}

fn summary_line(caps: &HostCapabilities, standard: &LabOffer, pro: &LabOffer) -> String {
    let os = os_plain(caps.os);
    match (standard.offered, pro.offered) {
        (true, true) => format!("This {os} PC can host Standard and Pro sessions."),
        (true, false) => format!("This {os} PC can host Standard sessions. Pro is not available."),
        (false, _) => {
            if caps.standard.offered {
                format!(
                    "This {os} PC cannot host Standard or Pro sessions with the current CPU, RAM, or disk share."
                )
            } else if caps.os == CapabilityOs::MacOs {
                "This Mac cannot host Standard or Pro sessions.".to_string()
            } else {
                format!("This {os} PC cannot host Standard or Pro sessions yet.")
            }
        }
    }
}

fn os_plain(os: CapabilityOs) -> &'static str {
    match os {
        CapabilityOs::Linux => "Linux",
        CapabilityOs::Windows => "Windows",
        CapabilityOs::MacOs => "macOS",
        CapabilityOs::Other => "unknown-OS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_bind::GpuBindStatus;
    use crate::host_capabilities::{
        evaluate as evaluate_probe, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::models::{GpuInfo, CONFIG_VERSION};
    use crate::provision::HostOs;
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with, snapshot};

    fn second_gpu() -> GpuInfo {
        nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12)
    }

    fn two_gpus() -> [GpuInfo; 2] {
        [nvidia_gpu(), second_gpu()]
    }

    fn sharing(
        cores: u32,
        memory_bytes: u64,
        share_gpu: bool,
        gpu_id: Option<&str>,
    ) -> SharingConfig {
        SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: cores,
            shared_memory_bytes: memory_bytes,
            share_gpu,
            shared_gpu_id: gpu_id.map(str::to_string),
        }
    }

    fn ample_disk() -> Option<u64> {
        Some(64 * BYTES_PER_GIB)
    }

    fn linux_kvm_iommu() -> HostCapabilities {
        evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &two_gpus(),
        })
    }

    fn linux_no_kvm() -> HostCapabilities {
        evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &two_gpus(),
        })
    }

    fn macos_caps() -> HostCapabilities {
        evaluate_probe(EvaluateInput {
            os: HostOs::MacOs,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn planned_bind() -> GpuBindStatus {
        GpuBindStatus {
            action: GpuBindAction::Planned,
            backend: Some("qemu-vfio".to_string()),
            gpu_id: Some("GPU-TEST-4070".to_string()),
            pci_address: Some("0000:01:00.0".to_string()),
            previous_driver: Some("nvidia".to_string()),
            target_driver: Some("vfio-pci".to_string()),
            steps: Vec::new(),
            bound: false,
            rolled_back: false,
            exclusive: true,
            log: "plan: exclusive GPU assignment. Not applied.".to_string(),
            hypervisor_invoked: false,
        }
    }

    fn refused_bind(log: &str) -> GpuBindStatus {
        GpuBindStatus {
            action: GpuBindAction::Refused,
            backend: None,
            gpu_id: None,
            pci_address: None,
            previous_driver: None,
            target_driver: None,
            steps: Vec::new(),
            bound: false,
            rolled_back: false,
            exclusive: false,
            log: log.to_string(),
            hypervisor_invoked: false,
        }
    }

    fn report(
        caps: &HostCapabilities,
        hardware: &HardwareSnapshot,
        sharing: &SharingConfig,
        spare_disk_bytes: Option<u64>,
        gpu_bind: Option<&GpuBindStatus>,
    ) -> EligibilityReport {
        evaluate(EligibilityRequest {
            capabilities: caps,
            hardware,
            sharing,
            spare_disk_bytes,
            gpu_bind,
        })
    }

    fn assert_no_backend_picker(report: &EligibilityReport) {
        let blob = format!(
            "{} {} {}",
            report.summary, report.standard.reason, report.pro.reason
        );
        for banned in ["QEMU", "WHPX", "choose", "which backend"] {
            assert!(
                !blob.contains(banned),
                "eligibility copy must not mention {banned:?}: {blob}"
            );
        }
    }

    #[test]
    fn macos_blocks_both_tiers_with_probe_reason() {
        let caps = macos_caps();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, Vec::new());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(!report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.standard.reason.contains("Mac"));
        assert!(report.summary.contains("Mac"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn linux_without_kvm_blocks_both_tiers() {
        let caps = linux_no_kvm();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(8, 16 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let bind = planned_bind();
        let report = report(&caps, &hardware, &sharing, ample_disk(), Some(&bind));

        assert!(!report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.standard.reason.contains("hardware virtualization"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn insufficient_vcpu_is_a_clean_rejection_not_a_later_error() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(1, 8 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(!report.standard.offered);
        assert!(!report.pro.offered);
        assert_eq!(
            report.standard.reason,
            "this PC would lend 1 vCPU, but a session needs at least 2 vCPU"
        );
        assert!(
            !report
                .standard
                .reason
                .to_lowercase()
                .contains("left for later"),
            "{}",
            report.standard.reason
        );
        assert!(report.summary.contains("current CPU, RAM, or disk share"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn insufficient_memory_is_a_clean_rejection() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 2 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(!report.standard.offered);
        assert_eq!(
            report.standard.reason,
            "this PC would lend 2 GiB of RAM, but a session needs at least 4 GiB"
        );
        assert_no_backend_picker(&report);
    }

    #[test]
    fn insufficient_disk_is_a_clean_rejection() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, Some(10 * BYTES_PER_GIB), None);

        assert!(!report.standard.offered);
        assert_eq!(
            report.standard.reason,
            "this PC has 10 GiB of spare disk, but a session needs at least 50 GiB"
        );
        assert_no_backend_picker(&report);
    }

    #[test]
    fn unknown_disk_does_not_block_when_cpu_and_ram_meet_the_floor() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(2, 4 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, None, None);

        assert!(report.standard.offered);
        assert!(report.standard.reason.contains("Standard"));
    }

    #[test]
    fn machine_too_small_to_spare_the_minimum_never_offers_standard() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(1, 3 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(8, 16 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(!report.standard.offered);
        assert!(report.standard.reason.contains("vCPU") || report.standard.reason.contains("RAM"));
    }

    #[test]
    fn documented_minimum_with_kvm_offers_standard() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(MIN_VCPU, MIN_MEMORY_BYTES, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(report.standard.offered);
        assert!(report.pro.offered);
        assert!(report.pro.reason.contains("exclusive"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn no_second_display_path_blocks_pro_and_keeps_standard() {
        let caps = evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &[nvidia_gpu()],
        });
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.pro.reason.contains("second display path"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn linux_without_iommu_blocks_pro() {
        let caps = evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &two_gpus(),
        });
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.pro.reason.contains("IOMMU"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn share_gpu_off_still_offers_pro_with_exclusivity_copy() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, Some("GPU-TEST-4070"));
        let bind = refused_bind(
            "a Pro GPU bind needs you to dedicate a GPU (shareGpu); that GPU is exclusive for the session, not a concurrent slice",
        );
        let report = report(&caps, &hardware, &sharing, ample_disk(), Some(&bind));

        assert!(report.pro.offered);
        assert!(report.pro.reason.contains("exclusive"));
        assert!(report.pro.reason.contains("not a concurrent slice"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn refused_vfio_plan_with_gpu_dedicated_blocks_pro() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let bind = refused_bind(
            "GPU 0000:01:00.0 is not in an IOMMU group; not binding so this PC keeps its display path",
        );
        let report = report(&caps, &hardware, &sharing, ample_disk(), Some(&bind));

        assert!(report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.pro.reason.contains("IOMMU group"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn planned_vfio_with_gpu_dedicated_keeps_pro_offered() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(8, 16 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let bind = planned_bind();
        let report = report(&caps, &hardware, &sharing, ample_disk(), Some(&bind));

        assert!(report.standard.offered);
        assert!(report.pro.offered);
        assert!(report.pro.reason.contains("exclusive"));
        assert_eq!(
            report.summary,
            "This Linux PC can host Standard and Pro sessions."
        );
    }

    #[test]
    fn windows_without_hypervisor_blocks_standard() {
        let caps = evaluate_probe(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Unavailable,
            iommu: IommuStatus::Unknown,
            gpus: &two_gpus(),
        });
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let report = report(&caps, &hardware, &sharing, ample_disk(), None);

        assert!(!report.standard.offered);
        assert!(!report.pro.offered);
        assert!(report.standard.reason.contains("Windows hypervisor"));
        assert_no_backend_picker(&report);
    }

    #[test]
    fn serde_round_trip_is_camel_case() {
        let caps = linux_kvm_iommu();
        let hardware = snapshot(16, 32 * BYTES_PER_GIB, two_gpus().to_vec());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let bind = planned_bind();
        let report = report(&caps, &hardware, &sharing, ample_disk(), Some(&bind));
        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(json["standard"]["offered"], true);
        assert_eq!(json["pro"]["offered"], true);
        assert!(json["pro"]["reason"]
            .as_str()
            .unwrap()
            .contains("exclusive"));
    }
}
