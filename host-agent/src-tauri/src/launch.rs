//! Launch configuration from clamped sharing sliders (phase 21).
//!
//! Maps vCPU / RAM (and a Pro GPU id) onto the OS-conditional stub backend
//! already chosen in [`crate::provision`]. Kernel and rootfs paths come from
//! the phase-20 image cache when those files exist. This module never invokes
//! a hypervisor. Tests assert JSON fields, not a running VM.
//!
//! Phase 23: when a capability probe is supplied, an unoffered tier (missing
//! KVM/WHPX, no Pro display path, or CPU/RAM/disk below the documented floor)
//! returns a plain-language error and does not emit a config.

use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::eligibility::{self, format_bytes_plain, EligibilityRequest};
use crate::host_capabilities::HostCapabilities;
use crate::image::{cached_kernel_path, cached_rootfs_path, GuestImageStatus};
use crate::models::{HardwareSnapshot, SharingConfig};
use crate::provision::{
    plan, ComputeBackend, HostOs, LabTier, ProvisionError, ProvisionRequest, StubProvisioner,
};

pub use crate::eligibility::{MIN_DISK_BYTES, MIN_MEMORY_BYTES, MIN_VCPU};

const BYTES_PER_MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest<'a> {
    pub os: HostOs,
    pub lab_tier: LabTier,
    pub hardware: &'a HardwareSnapshot,
    pub sharing: &'a SharingConfig,
    pub image_cache_dir: Option<&'a Path>,
    pub guest_image: Option<&'a GuestImageStatus>,
    /// When set, an unoffered tier (probe or resources) does not emit a config.
    pub capabilities: Option<&'a HostCapabilities>,
    pub spare_disk_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuAssignment {
    pub id: String,
    /// Always true. GPU is never a concurrent slice like CPU/RAM.
    pub exclusive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    #[serde(serialize_with = "serialize_backend")]
    pub backend: ComputeBackend,
    #[serde(serialize_with = "serialize_lab_tier")]
    pub lab_tier: LabTier,
    pub vcpu_count: u32,
    pub memory_mib: u64,
    pub kernel_path: Option<String>,
    pub rootfs_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuAssignment>,
    /// Always false in this phase. Later phases boot from this document.
    pub hypervisor_invoked: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LaunchError {
    #[error("{0}")]
    Provision(#[from] ProvisionError),
    #[error("this PC would lend {requested} vCPU, but a session needs at least {minimum} vCPU")]
    InsufficientVcpu { requested: u32, minimum: u32 },
    #[error("this PC would lend {requested} of RAM, but a session needs at least {minimum}")]
    InsufficientMemory { requested: String, minimum: String },
    #[error(
        "a Pro session needs you to dedicate a GPU (shareGpu); that GPU is exclusive for the session, not a concurrent slice"
    )]
    GpuNotDedicated,
    #[error("{0}")]
    TierNotOffered(String),
    #[error("this PC has {requested} of spare disk, but a session needs at least {minimum}")]
    InsufficientDisk { requested: String, minimum: String },
}

fn serialize_backend<S: serde::Serializer>(
    backend: &ComputeBackend,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(backend.slug())
}

fn serialize_lab_tier<S: serde::Serializer>(
    tier: &LabTier,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(tier.as_str())
}

/// Build a launch document for the selected stub backend. Never starts a VM.
pub fn build(request: LaunchRequest<'_>) -> Result<LaunchConfig, LaunchError> {
    if let Some(capabilities) = request.capabilities {
        let report = eligibility::evaluate(EligibilityRequest {
            capabilities,
            hardware: request.hardware,
            sharing: request.sharing,
            spare_disk_bytes: request.spare_disk_bytes,
            gpu_bind: None,
        });
        let offer = eligibility::offer_for_tier(&report, request.lab_tier);
        if !offer.offered {
            return Err(LaunchError::TierNotOffered(offer.reason.clone()));
        }
    }

    StubProvisioner::select(request.os, request.lab_tier)?;
    let clamped = request.sharing.clamped_to(request.hardware);

    if clamped.shared_logical_cores < MIN_VCPU {
        return Err(LaunchError::InsufficientVcpu {
            requested: clamped.shared_logical_cores,
            minimum: MIN_VCPU,
        });
    }
    if clamped.shared_memory_bytes < MIN_MEMORY_BYTES {
        return Err(LaunchError::InsufficientMemory {
            requested: format_bytes_plain(clamped.shared_memory_bytes),
            minimum: format_bytes_plain(MIN_MEMORY_BYTES),
        });
    }
    if let Some(disk) = request.spare_disk_bytes {
        if disk < MIN_DISK_BYTES {
            return Err(LaunchError::InsufficientDisk {
                requested: format_bytes_plain(disk),
                minimum: format_bytes_plain(MIN_DISK_BYTES),
            });
        }
    }

    let provision_request = match request.lab_tier {
        LabTier::Standard => ProvisionRequest::standard(),
        LabTier::Pro => {
            if !clamped.share_gpu {
                return Err(LaunchError::GpuNotDedicated);
            }
            ProvisionRequest {
                lab_tier: LabTier::Pro,
                shared_gpu_id: clamped.shared_gpu_id.clone(),
            }
        }
    };
    let plan = plan(request.os, &provision_request)?;

    let rootfs_path = resolve_rootfs(
        request.lab_tier,
        request.image_cache_dir,
        request.guest_image,
    );
    let kernel_path = resolve_kernel(
        request.lab_tier,
        request.image_cache_dir,
        rootfs_path.as_deref(),
    );

    let gpu = match plan.shared_gpu_id {
        Some(id) => Some(GpuAssignment {
            id,
            exclusive: true,
        }),
        None => None,
    };

    Ok(LaunchConfig {
        backend: plan.backend,
        lab_tier: plan.lab_tier,
        vcpu_count: clamped.shared_logical_cores,
        memory_mib: clamped.shared_memory_bytes / BYTES_PER_MIB,
        kernel_path,
        rootfs_path,
        gpu,
        hypervisor_invoked: false,
    })
}

fn resolve_rootfs(
    lab_tier: LabTier,
    cache_dir: Option<&Path>,
    guest_image: Option<&GuestImageStatus>,
) -> Option<String> {
    if let Some(status) = guest_image {
        if status.lab_tier.as_deref() == Some(lab_tier.as_str()) {
            if let Some(path) = status.path.as_ref().filter(|path| !path.is_empty()) {
                return Some(path.clone());
            }
        }
    }
    existing_file(cache_dir.map(|dir| cached_rootfs_path(dir, lab_tier)))
}

fn resolve_kernel(
    lab_tier: LabTier,
    cache_dir: Option<&Path>,
    rootfs_path: Option<&str>,
) -> Option<String> {
    if let Some(path) = existing_file(cache_dir.map(|dir| cached_kernel_path(dir, lab_tier))) {
        return Some(path);
    }
    let rootfs = rootfs_path?;
    existing_file(Some(Path::new(rootfs).with_file_name("vmlinux")))
}

fn existing_file(path: Option<PathBuf>) -> Option<String> {
    let path = path?;
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate as evaluate_probe, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::image::{ImageBackend, ImageEnsureAction};
    use crate::models::{GpuInfo, BYTES_PER_GIB, CONFIG_VERSION};
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with, snapshot};
    use std::fs;
    use tempfile::TempDir;

    const GPU_ID: &str = "GPU-TEST-4070";

    fn second_gpu() -> GpuInfo {
        nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12)
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

    fn hardware() -> HardwareSnapshot {
        snapshot(16, 32 * BYTES_PER_GIB, vec![nvidia_gpu(), second_gpu()])
    }

    fn request<'a>(
        os: HostOs,
        lab_tier: LabTier,
        hardware: &'a HardwareSnapshot,
        sharing: &'a SharingConfig,
    ) -> LaunchRequest<'a> {
        LaunchRequest {
            os,
            lab_tier,
            hardware,
            sharing,
            image_cache_dir: None,
            guest_image: None,
            capabilities: None,
            spare_disk_bytes: None,
        }
    }

    fn guest_status(lab_tier: &str, path: &str) -> GuestImageStatus {
        GuestImageStatus {
            action: ImageEnsureAction::SkippedPresent,
            lab_tier: Some(lab_tier.to_string()),
            backend: ImageBackend::Firecracker,
            path: Some(path.to_string()),
            digest: None,
            log: "test".to_string(),
            image_booted: false,
        }
    }

    #[test]
    fn linux_standard_uses_slider_vcpu_and_memory() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, Some(GPU_ID));
        let config = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(config.backend, ComputeBackend::Firecracker);
        assert_eq!(config.lab_tier, LabTier::Standard);
        assert_eq!(config.vcpu_count, 4);
        assert_eq!(config.memory_mib, 8192);
        assert!(config.gpu.is_none());
        assert!(!config.hypervisor_invoked);
        assert_eq!(json["backend"], "firecracker");
        assert_eq!(json["labTier"], "standard");
        assert_eq!(json["vcpuCount"], 4);
        assert_eq!(json["memoryMib"], 8192);
        assert!(json.get("gpu").is_none());
        assert_eq!(json["hypervisorInvoked"], false);
        assert!(json["kernelPath"].is_null());
        assert!(json["rootfsPath"].is_null());
    }

    #[test]
    fn windows_standard_selects_qemu_whpx() {
        let hardware = hardware();
        let sharing = sharing(6, 12 * BYTES_PER_GIB, false, None);
        let config = build(request(
            HostOs::Windows,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(config.backend, ComputeBackend::QemuWhpx);
        assert_eq!(config.vcpu_count, 6);
        assert_eq!(config.memory_mib, 12288);
        assert!(config.gpu.is_none());
        assert_eq!(json["backend"], "qemu-whpx");
        assert!(json.get("gpu").is_none());
        assert!(!config.hypervisor_invoked);
    }

    #[test]
    fn linux_pro_assigns_exclusive_gpu() {
        let hardware = hardware();
        let sharing = sharing(8, 16 * BYTES_PER_GIB, true, Some("GPU-TEST-3060"));
        let config = build(request(HostOs::Linux, LabTier::Pro, &hardware, &sharing)).unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(config.backend, ComputeBackend::QemuVfio);
        assert_eq!(config.vcpu_count, 8);
        assert_eq!(config.memory_mib, 16384);
        assert_eq!(
            config.gpu,
            Some(GpuAssignment {
                id: "GPU-TEST-3060".to_string(),
                exclusive: true,
            })
        );
        assert_eq!(json["backend"], "qemu-vfio");
        assert_eq!(json["gpu"]["id"], "GPU-TEST-3060");
        assert_eq!(json["gpu"]["exclusive"], true);
        assert!(!config.hypervisor_invoked);
    }

    #[test]
    fn windows_pro_selects_hyperv_gpu() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some(GPU_ID));
        let config = build(request(HostOs::Windows, LabTier::Pro, &hardware, &sharing)).unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(config.backend, ComputeBackend::HyperVGpu);
        assert_eq!(config.gpu.as_ref().map(|gpu| gpu.id.as_str()), Some(GPU_ID));
        assert!(config.gpu.as_ref().is_some_and(|gpu| gpu.exclusive));
        assert_eq!(json["backend"], "hyperv-gpu");
        assert!(!config.hypervisor_invoked);
    }

    #[test]
    fn standard_omits_gpu_even_when_share_gpu_and_id_are_stored() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some(GPU_ID));
        let config = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(config.backend, ComputeBackend::Firecracker);
        assert!(config.gpu.is_none());
        assert!(json.get("gpu").is_none());
    }

    #[test]
    fn pro_requires_share_gpu_even_if_an_id_is_stored() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, Some(GPU_ID));
        let error = build(request(HostOs::Linux, LabTier::Pro, &hardware, &sharing)).unwrap_err();

        assert_eq!(error, LaunchError::GpuNotDedicated);
        assert!(error.to_string().contains("exclusive"));
        assert!(error.to_string().contains("not a concurrent slice"));
    }

    #[test]
    fn pro_uses_the_clamped_gpu_id_when_none_is_stored() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, None);
        let config = build(request(HostOs::Linux, LabTier::Pro, &hardware, &sharing)).unwrap();
        assert_eq!(config.gpu.as_ref().map(|gpu| gpu.id.as_str()), Some(GPU_ID));
        assert!(config.gpu.as_ref().is_some_and(|gpu| gpu.exclusive));
    }

    #[test]
    fn pro_refuses_when_this_machine_has_no_gpu_to_dedicate() {
        let no_gpu = snapshot(16, 32 * BYTES_PER_GIB, Vec::new());
        let sharing = sharing(4, 8 * BYTES_PER_GIB, true, Some(GPU_ID));
        let error = build(request(HostOs::Linux, LabTier::Pro, &no_gpu, &sharing)).unwrap_err();
        assert_eq!(error, LaunchError::GpuNotDedicated);
    }

    #[test]
    fn macos_does_not_emit_a_firecracker_config() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let error = build(request(
            HostOs::MacOs,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap_err();

        assert_eq!(
            error,
            LaunchError::Provision(ProvisionError::UnsupportedHostOs(HostOs::MacOs))
        );
        assert!(error.to_string().contains("macOS"));
        assert!(build(request(HostOs::MacOs, LabTier::Pro, &hardware, &sharing)).is_err());
    }

    #[test]
    fn rejects_below_documented_minimum_vcpu() {
        let hardware = hardware();
        let sharing = sharing(1, 8 * BYTES_PER_GIB, false, None);
        let error = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap_err();

        assert_eq!(
            error,
            LaunchError::InsufficientVcpu {
                requested: 1,
                minimum: 2
            }
        );
        assert_eq!(
            error.to_string(),
            "this PC would lend 1 vCPU, but a session needs at least 2 vCPU"
        );
    }

    #[test]
    fn rejects_below_documented_minimum_memory() {
        let hardware = hardware();
        let sharing = sharing(4, 2 * BYTES_PER_GIB, false, None);
        let error = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap_err();

        assert_eq!(
            error,
            LaunchError::InsufficientMemory {
                requested: "2 GiB".to_string(),
                minimum: "4 GiB".to_string(),
            }
        );
        assert_eq!(
            error.to_string(),
            "this PC would lend 2 GiB of RAM, but a session needs at least 4 GiB"
        );
    }

    #[test]
    fn accepts_the_documented_minimum() {
        let hardware = hardware();
        let sharing = sharing(MIN_VCPU, MIN_MEMORY_BYTES, false, None);
        let config = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap();

        assert_eq!(config.vcpu_count, 2);
        assert_eq!(config.memory_mib, 4096);
    }

    #[test]
    fn clamps_sliders_to_this_machine_before_emitting_config() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let sharing = sharing(64, 512 * BYTES_PER_GIB, false, None);
        let config = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &sharing,
        ))
        .unwrap();

        assert_eq!(config.vcpu_count, 8);
        assert_eq!(config.memory_mib, 16384);
    }

    #[test]
    fn includes_rootfs_from_phase20_guest_status_when_tier_matches() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let guest = guest_status(
            "standard",
            "/home/host/.cache/dcl-host-agent/images/standard/rootfs.img",
        );
        let config = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: Some(&guest),
            capabilities: None,
            spare_disk_bytes: None,
        })
        .unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(
            config.rootfs_path.as_deref(),
            Some("/home/host/.cache/dcl-host-agent/images/standard/rootfs.img")
        );
        assert!(config.kernel_path.is_none());
        assert_eq!(
            json["rootfsPath"],
            "/home/host/.cache/dcl-host-agent/images/standard/rootfs.img"
        );
    }

    #[test]
    fn standard_does_not_reuse_a_pro_rootfs_path() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let guest = guest_status("pro", "/cache/pro/rootfs.img");
        let config = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: Some(&guest),
            capabilities: None,
            spare_disk_bytes: None,
        })
        .unwrap();

        assert!(config.rootfs_path.is_none());
    }

    #[test]
    fn includes_kernel_and_rootfs_from_phase20_cache_when_present() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let cache = TempDir::new().unwrap();
        let rootfs = cached_rootfs_path(cache.path(), LabTier::Standard);
        let kernel = cached_kernel_path(cache.path(), LabTier::Standard);
        fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
        fs::write(&rootfs, b"rootfs-fixture\n").unwrap();
        fs::write(&kernel, b"vmlinux-fixture\n").unwrap();

        let config = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: Some(cache.path()),
            guest_image: None,
            capabilities: None,
            spare_disk_bytes: None,
        })
        .unwrap();
        let json = serde_json::to_value(&config).unwrap();

        assert_eq!(
            config.rootfs_path.as_deref(),
            Some(rootfs.to_str().unwrap())
        );
        assert_eq!(
            config.kernel_path.as_deref(),
            Some(kernel.to_str().unwrap())
        );
        assert_eq!(json["rootfsPath"], rootfs.to_str().unwrap());
        assert_eq!(json["kernelPath"], kernel.to_str().unwrap());
    }

    #[test]
    fn backends_are_first_class_peers() {
        let hardware = hardware();
        let standard = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let pro = sharing(4, 8 * BYTES_PER_GIB, true, Some(GPU_ID));

        let linux_std = build(request(
            HostOs::Linux,
            LabTier::Standard,
            &hardware,
            &standard,
        ))
        .unwrap();
        let win_std = build(request(
            HostOs::Windows,
            LabTier::Standard,
            &hardware,
            &standard,
        ))
        .unwrap();
        let linux_pro = build(request(HostOs::Linux, LabTier::Pro, &hardware, &pro)).unwrap();
        let win_pro = build(request(HostOs::Windows, LabTier::Pro, &hardware, &pro)).unwrap();

        assert_eq!(linux_std.backend, ComputeBackend::Firecracker);
        assert_eq!(win_std.backend, ComputeBackend::QemuWhpx);
        assert_eq!(linux_pro.backend, ComputeBackend::QemuVfio);
        assert_eq!(win_pro.backend, ComputeBackend::HyperVGpu);
        assert_ne!(linux_std.backend, win_std.backend);
        assert_ne!(linux_pro.backend, ComputeBackend::Firecracker);
        assert!(!linux_std.hypervisor_invoked);
        assert!(!win_std.hypervisor_invoked);
        assert!(!linux_pro.hypervisor_invoked);
        assert!(!win_pro.hypervisor_invoked);
    }

    fn linux_kvm_caps() -> HostCapabilities {
        evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &[nvidia_gpu(), second_gpu()],
        })
    }

    fn linux_no_kvm_caps() -> HostCapabilities {
        evaluate_probe(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[nvidia_gpu(), second_gpu()],
        })
    }

    #[test]
    fn probe_without_kvm_does_not_emit_a_config() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let caps = linux_no_kvm_caps();
        let error = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: None,
            capabilities: Some(&caps),
            spare_disk_bytes: Some(64 * BYTES_PER_GIB),
        })
        .unwrap_err();

        match error {
            LaunchError::TierNotOffered(reason) => {
                assert!(reason.contains("hardware virtualization"));
            }
            other => panic!("expected TierNotOffered, got {other:?}"),
        }
    }

    #[test]
    fn insufficient_resources_with_probe_is_a_clean_rejection() {
        let hardware = hardware();
        let sharing = sharing(1, 8 * BYTES_PER_GIB, false, None);
        let caps = linux_kvm_caps();
        let error = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: None,
            capabilities: Some(&caps),
            spare_disk_bytes: Some(64 * BYTES_PER_GIB),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "this PC would lend 1 vCPU, but a session needs at least 2 vCPU"
        );
        match error {
            LaunchError::TierNotOffered(_) => {}
            other => panic!("expected TierNotOffered so the UI can hide the tier, got {other:?}"),
        }
    }

    #[test]
    fn insufficient_disk_does_not_emit_a_config() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let error = build(LaunchRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: None,
            capabilities: None,
            spare_disk_bytes: Some(10 * BYTES_PER_GIB),
        })
        .unwrap_err();

        assert_eq!(
            error,
            LaunchError::InsufficientDisk {
                requested: "10 GiB".to_string(),
                minimum: "50 GiB".to_string(),
            }
        );
        assert_eq!(
            error.to_string(),
            "this PC has 10 GiB of spare disk, but a session needs at least 50 GiB"
        );
    }

    #[test]
    fn macos_probe_reason_is_used_when_capabilities_are_supplied() {
        let hardware = hardware();
        let sharing = sharing(4, 8 * BYTES_PER_GIB, false, None);
        let caps = evaluate_probe(EvaluateInput {
            os: HostOs::MacOs,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        });
        let error = build(LaunchRequest {
            os: HostOs::MacOs,
            lab_tier: LabTier::Standard,
            hardware: &hardware,
            sharing: &sharing,
            image_cache_dir: None,
            guest_image: None,
            capabilities: Some(&caps),
            spare_disk_bytes: None,
        })
        .unwrap_err();

        match error {
            LaunchError::TierNotOffered(reason) => {
                assert!(reason.contains("Mac"));
                assert!(!reason.contains("Firecracker"));
            }
            other => panic!("expected TierNotOffered, got {other:?}"),
        }
    }
}
