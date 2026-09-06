//! Host capability probe (phase 18).
//!
//! Plain types only — no Tauri. The agent uses this to decide whether Standard
//! or Pro sessions can be offered. Selection is internal (OS + KVM / Windows
//! hypervisor / IOMMU / GPUs). The UI must never ask which backend to use.
//!
//! Live Linux checks `/dev/kvm` and IOMMU sysfs. Live Windows returns WHPX as
//! unknown (no QEMU spawn). Tests inject probe results so this Mac and
//! Linux-without-KVM CI do not need real virtualization hardware.

use std::fs::OpenOptions;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::GpuInfo;
use crate::provision::HostOs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityOs {
    Linux,
    Windows,
    #[serde(rename = "macos")]
    MacOs,
    Other,
}

impl From<HostOs> for CapabilityOs {
    fn from(os: HostOs) -> Self {
        match os {
            HostOs::Linux => Self::Linux,
            HostOs::Windows => Self::Windows,
            HostOs::MacOs => Self::MacOs,
            HostOs::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KvmAccess {
    PresentReadWrite,
    PresentNotWritable,
    Missing,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhpxCapability {
    Available,
    Unavailable,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IommuStatus {
    Present,
    Absent,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabOffer {
    pub offered: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCapabilities {
    pub os: CapabilityOs,
    pub kvm: KvmAccess,
    pub whpx: WhpxCapability,
    pub iommu: IommuStatus,
    pub second_display_path: bool,
    pub gpu_count: usize,
    pub standard: LabOffer,
    pub pro: LabOffer,
    pub summary: String,
}

#[derive(Debug, Clone, Copy)]
pub struct EvaluateInput<'a> {
    pub os: HostOs,
    pub kvm: KvmAccess,
    pub whpx: WhpxCapability,
    pub iommu: IommuStatus,
    pub gpus: &'a [GpuInfo],
}

/// Intel/AMD integrated names, or 2+ GPUs, mean the host can keep a display
/// while one card is dedicated. A single discrete GPU is not a second path.
pub fn looks_like_integrated(model: &str) -> bool {
    let name = model.to_ascii_lowercase();
    if name.contains("geforce")
        || name.contains("quadro")
        || name.contains("rtx ")
        || name.contains("gtx ")
        || name.contains("radeon rx")
        || name.contains("radeon pro")
        || name.contains("nvidia tesla")
        || name.contains("nvidia a")
    {
        return false;
    }
    name.contains("uhd")
        || name.contains("iris")
        || name.contains("intel hd")
        || name.contains("intel graphics")
        || name.contains("radeon graphics")
        || name.contains("integrated")
}

pub fn has_second_display_path(gpus: &[GpuInfo]) -> bool {
    gpus.len() >= 2 || gpus.iter().any(|gpu| looks_like_integrated(&gpu.model))
}

/// Mockable KVM check: any path, including a tempfile in tests.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn probe_kvm_path(path: &Path) -> KvmAccess {
    if !path.exists() {
        return KvmAccess::Missing;
    }
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => KvmAccess::PresentReadWrite,
        Err(_) => KvmAccess::PresentNotWritable,
    }
}

/// Mockable IOMMU sysfs: `root` is `/` in production, a temp tree in tests.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn probe_iommu_sysfs(root: &Path) -> IommuStatus {
    let groups = root.join("sys/kernel/iommu_groups");
    match read_dir_nonempty(&groups) {
        DirProbe::NonEmpty => return IommuStatus::Present,
        DirProbe::Empty => return IommuStatus::Absent,
        DirProbe::Missing => {}
        DirProbe::Unreadable => return IommuStatus::Unknown,
    }

    let class_iommu = root.join("sys/class/iommu");
    match read_dir_nonempty(&class_iommu) {
        DirProbe::NonEmpty => IommuStatus::Present,
        DirProbe::Empty | DirProbe::Missing => IommuStatus::Absent,
        DirProbe::Unreadable => IommuStatus::Unknown,
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
enum DirProbe {
    NonEmpty,
    Empty,
    Missing,
    Unreadable,
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn read_dir_nonempty(path: &Path) -> DirProbe {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            if entries.filter_map(Result::ok).next().is_some() {
                DirProbe::NonEmpty
            } else {
                DirProbe::Empty
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DirProbe::Missing,
        Err(_) => DirProbe::Unreadable,
    }
}

pub fn live_kvm(os: HostOs) -> KvmAccess {
    if os != HostOs::Linux {
        return KvmAccess::NotApplicable;
    }
    #[cfg(target_os = "linux")]
    {
        probe_kvm_path(Path::new("/dev/kvm"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        KvmAccess::NotApplicable
    }
}

pub fn live_whpx(os: HostOs) -> WhpxCapability {
    if os != HostOs::Windows {
        return WhpxCapability::NotApplicable;
    }
    #[cfg(target_os = "windows")]
    {
        // Do not spawn QEMU. Unknown until a later phase can prove WHPX.
        WhpxCapability::Unknown
    }
    #[cfg(not(target_os = "windows"))]
    {
        WhpxCapability::NotApplicable
    }
}

pub fn live_iommu(os: HostOs) -> IommuStatus {
    if os != HostOs::Linux {
        return IommuStatus::Unknown;
    }
    #[cfg(target_os = "linux")]
    {
        probe_iommu_sysfs(Path::new("/"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        IommuStatus::Unknown
    }
}

pub fn evaluate(input: EvaluateInput<'_>) -> HostCapabilities {
    let second_display_path = has_second_display_path(input.gpus);
    let standard = standard_offer(input.os, input.kvm, input.whpx);
    let pro = pro_offer(input.os, standard.offered, input.iommu, second_display_path);
    let summary = summary_line(input.os, &standard, &pro);

    HostCapabilities {
        os: CapabilityOs::from(input.os),
        kvm: input.kvm,
        whpx: input.whpx,
        iommu: input.iommu,
        second_display_path,
        gpu_count: input.gpus.len(),
        standard,
        pro,
        summary,
    }
}

/// Probe this process's OS with live sysfs/device checks. Tests that need
/// Linux/Windows outcomes on a Mac must call [`evaluate`] instead.
pub fn probe_live(os: HostOs, gpus: &[GpuInfo]) -> HostCapabilities {
    evaluate(EvaluateInput {
        os,
        kvm: live_kvm(os),
        whpx: live_whpx(os),
        iommu: live_iommu(os),
        gpus,
    })
}

fn standard_offer(os: HostOs, kvm: KvmAccess, whpx: WhpxCapability) -> LabOffer {
    match os {
        HostOs::MacOs | HostOs::Other => LabOffer {
            offered: false,
            reason: macos_or_other_reason(os, "Standard"),
        },
        HostOs::Linux => match kvm {
            KvmAccess::PresentReadWrite => LabOffer {
                offered: true,
                reason: "This PC can host Standard sessions.".to_string(),
            },
            KvmAccess::PresentNotWritable => LabOffer {
                offered: false,
                reason: "Standard sessions are not available: hardware virtualization is present but this account cannot use it.".to_string(),
            },
            KvmAccess::Missing | KvmAccess::NotApplicable => LabOffer {
                offered: false,
                reason: "Standard sessions are not available: this Linux PC does not have usable hardware virtualization.".to_string(),
            },
        },
        HostOs::Windows => match whpx {
            WhpxCapability::Available => LabOffer {
                offered: true,
                reason: "This PC can host Standard sessions.".to_string(),
            },
            WhpxCapability::Unavailable => LabOffer {
                offered: false,
                reason: "Standard sessions are not available: the Windows hypervisor is not enabled on this PC.".to_string(),
            },
            WhpxCapability::Unknown | WhpxCapability::NotApplicable => LabOffer {
                offered: false,
                reason: "Standard sessions are not confirmed: this agent could not verify the Windows hypervisor on this PC.".to_string(),
            },
        },
    }
}

fn pro_offer(
    os: HostOs,
    standard_offered: bool,
    iommu: IommuStatus,
    second_display_path: bool,
) -> LabOffer {
    match os {
        HostOs::MacOs | HostOs::Other => LabOffer {
            offered: false,
            reason: macos_or_other_reason(os, "Pro"),
        },
        HostOs::Linux | HostOs::Windows => {
            if !standard_offered {
                return LabOffer {
                    offered: false,
                    reason:
                        "Pro sessions are not available until this PC can host Standard sessions."
                            .to_string(),
                };
            }
            if !second_display_path {
                return LabOffer {
                    offered: false,
                    reason: "Pro sessions are not available: this PC needs a second display path (an iGPU or another GPU) so you can keep using it while a guest has the dedicated GPU.".to_string(),
                };
            }
            if os == HostOs::Linux {
                match iommu {
                    IommuStatus::Present => pro_available(),
                    IommuStatus::Absent => LabOffer {
                        offered: false,
                        reason: "Pro sessions are not available: this Linux PC does not report IOMMU, which is needed to assign a GPU to a guest.".to_string(),
                    },
                    IommuStatus::Unknown | IommuStatus::NotApplicable => LabOffer {
                        offered: false,
                        reason: "Pro sessions are not confirmed: this agent could not verify IOMMU on this Linux PC.".to_string(),
                    },
                }
            } else {
                // Windows: IOMMU/VFIO-ish flags are optional/unknown.
                pro_available()
            }
        }
    }
}

fn pro_available() -> LabOffer {
    LabOffer {
        offered: true,
        reason: "This PC can host Pro sessions when you dedicate a GPU. That GPU is exclusive for the session — not a concurrent slice like CPU or RAM.".to_string(),
    }
}

fn macos_or_other_reason(os: HostOs, tier: &str) -> String {
    match os {
        HostOs::MacOs => format!(
            "This Mac cannot host {tier} sessions. Hosting needs a Linux PC with hardware virtualization, or a Windows PC with the Windows hypervisor."
        ),
        _ => format!(
            "This computer cannot host {tier} sessions. Hosting needs a Linux PC with hardware virtualization, or a Windows PC with the Windows hypervisor."
        ),
    }
}

fn summary_line(os: HostOs, standard: &LabOffer, pro: &LabOffer) -> String {
    match (os, standard.offered, pro.offered) {
        (HostOs::MacOs, false, false) => {
            "This Mac cannot host Standard or Pro sessions.".to_string()
        }
        (_, true, true) => format!(
            "This {} PC can host Standard and Pro sessions.",
            os_plain(os)
        ),
        (_, true, false) => format!(
            "This {} PC can host Standard sessions. Pro is not available.",
            os_plain(os)
        ),
        (_, false, _) => format!(
            "This {} PC cannot host Standard or Pro sessions yet.",
            os_plain(os)
        ),
    }
}

fn os_plain(os: HostOs) -> &'static str {
    match os {
        HostOs::Linux => "Linux",
        HostOs::Windows => "Windows",
        HostOs::MacOs => "macOS",
        HostOs::Other => "unknown-OS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with};
    use std::fs;

    fn igpu() -> GpuInfo {
        GpuInfo {
            id: "intel:0".to_string(),
            model: "Intel UHD Graphics 770".to_string(),
            vendor: crate::models::GpuVendor::Unknown,
            vram_bytes: None,
        }
    }

    fn linux_no_virt(gpus: &[GpuInfo]) -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus,
        })
    }

    fn linux_kvm(kvm: KvmAccess, iommu: IommuStatus, gpus: &[GpuInfo]) -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm,
            whpx: WhpxCapability::NotApplicable,
            iommu,
            gpus,
        })
    }

    fn assert_no_backend_picker(caps: &HostCapabilities) {
        let blob = format!(
            "{} {} {}",
            caps.summary, caps.standard.reason, caps.pro.reason
        );
        for banned in ["Firecracker", "QEMU", "WHPX", "choose", "which backend"] {
            assert!(
                !blob.contains(banned),
                "capability copy must not mention {banned:?}: {blob}"
            );
        }
    }

    #[test]
    fn macos_does_not_offer_standard_or_pro() {
        let caps = evaluate(EvaluateInput {
            os: HostOs::MacOs,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        });
        assert!(!caps.standard.offered, "{}", caps.standard.reason);
        assert!(!caps.pro.offered, "{}", caps.pro.reason);
        assert!(caps.summary.contains("Mac"));
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn linux_without_kvm_does_not_offer_standard_or_pro() {
        let two = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        let caps = linux_no_virt(&two);
        assert!(!caps.standard.offered);
        assert!(!caps.pro.offered);
        assert!(
            caps.second_display_path,
            "two GPUs still count as a second display path"
        );
        assert!(caps.standard.reason.contains("hardware virtualization"));
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn linux_kvm_not_writable_does_not_offer_standard() {
        let caps = linux_kvm(KvmAccess::PresentNotWritable, IommuStatus::Present, &[]);
        assert!(!caps.standard.offered);
        assert!(!caps.pro.offered);
    }

    #[test]
    fn linux_kvm_read_write_offers_standard() {
        let caps = linux_kvm(KvmAccess::PresentReadWrite, IommuStatus::Absent, &[]);
        assert!(caps.standard.offered);
        assert!(!caps.pro.offered);
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn two_gpus_mean_second_display_path() {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        assert!(has_second_display_path(&gpus));
        let caps = linux_kvm(KvmAccess::PresentReadWrite, IommuStatus::Present, &gpus);
        assert!(caps.second_display_path);
        assert_eq!(caps.gpu_count, 2);
        assert!(caps.standard.offered);
        assert!(caps.pro.offered);
        assert!(caps.pro.reason.contains("exclusive"));
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn one_discrete_gpu_is_not_a_second_display_path() {
        let gpus = [nvidia_gpu()];
        assert!(!has_second_display_path(&gpus));
        let caps = linux_kvm(KvmAccess::PresentReadWrite, IommuStatus::Present, &gpus);
        assert!(!caps.second_display_path);
        assert!(caps.standard.offered);
        assert!(!caps.pro.offered);
        assert!(caps.pro.reason.contains("second display path"));
    }

    #[test]
    fn integrated_gpu_counts_as_second_display_path() {
        let gpus = [igpu()];
        assert!(looks_like_integrated(&gpus[0].model));
        assert!(has_second_display_path(&gpus));
    }

    #[test]
    fn linux_two_gpus_without_iommu_offers_standard_not_pro() {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        let caps = linux_kvm(KvmAccess::PresentReadWrite, IommuStatus::Absent, &gpus);
        assert!(caps.standard.offered);
        assert!(!caps.pro.offered);
        assert!(caps.pro.reason.contains("IOMMU"));
    }

    #[test]
    fn windows_unknown_whpx_does_not_offer_labs() {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        let caps = evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Unknown,
            iommu: IommuStatus::Unknown,
            gpus: &gpus,
        });
        assert!(!caps.standard.offered);
        assert!(!caps.pro.offered);
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn windows_whpx_available_and_two_gpus_offers_pro_with_unknown_iommu() {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        let caps = evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Available,
            iommu: IommuStatus::Unknown,
            gpus: &gpus,
        });
        assert!(caps.standard.offered);
        assert!(caps.pro.offered);
        assert!(caps.second_display_path);
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn non_windows_live_whpx_is_not_applicable() {
        assert_eq!(live_whpx(HostOs::Linux), WhpxCapability::NotApplicable);
        assert_eq!(live_whpx(HostOs::MacOs), WhpxCapability::NotApplicable);
        assert_eq!(live_kvm(HostOs::MacOs), KvmAccess::NotApplicable);
        assert_eq!(live_kvm(HostOs::Windows), KvmAccess::NotApplicable);
        assert_eq!(live_iommu(HostOs::Windows), IommuStatus::Unknown);
        assert_eq!(live_iommu(HostOs::MacOs), IommuStatus::Unknown);
    }

    #[test]
    fn probe_kvm_path_missing_and_read_write() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("kvm-missing");
        assert_eq!(probe_kvm_path(&missing), KvmAccess::Missing);

        let present = dir.path().join("kvm");
        fs::write(&present, b"").unwrap();
        assert_eq!(probe_kvm_path(&present), KvmAccess::PresentReadWrite);
    }

    #[test]
    #[cfg(unix)]
    fn probe_kvm_path_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kvm");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(probe_kvm_path(&path), KvmAccess::PresentNotWritable);
    }

    #[test]
    fn probe_iommu_sysfs_present_and_absent() {
        let present_root = tempfile::tempdir().unwrap();
        let groups = present_root.path().join("sys/kernel/iommu_groups/0");
        fs::create_dir_all(&groups).unwrap();
        assert_eq!(probe_iommu_sysfs(present_root.path()), IommuStatus::Present);

        let empty_root = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty_root.path().join("sys/kernel/iommu_groups")).unwrap();
        assert_eq!(probe_iommu_sysfs(empty_root.path()), IommuStatus::Absent);

        let missing_root = tempfile::tempdir().unwrap();
        assert_eq!(probe_iommu_sysfs(missing_root.path()), IommuStatus::Absent);
    }

    #[test]
    fn looks_like_integrated_rejects_discrete_nvidia() {
        assert!(!looks_like_integrated("NVIDIA GeForce RTX 4070"));
        assert!(looks_like_integrated("Intel Iris Xe Graphics"));
        assert!(looks_like_integrated("AMD Radeon Graphics"));
    }

    #[test]
    fn live_probe_on_this_machine_never_panics() {
        let caps = probe_live(HostOs::current(), &[]);
        if HostOs::current() == HostOs::MacOs {
            assert!(!caps.standard.offered);
            assert!(!caps.pro.offered);
        }
        assert_no_backend_picker(&caps);
    }

    #[test]
    fn serde_round_trip_uses_plain_os_names() {
        let caps = linux_no_virt(&[]);
        let json = serde_json::to_value(&caps).unwrap();
        assert_eq!(json["os"], "linux");
        assert_eq!(json["standard"]["offered"], false);
        assert_eq!(json["kvm"], "missing");
        let _back: HostCapabilities = serde_json::from_value(json).unwrap();
    }
}
