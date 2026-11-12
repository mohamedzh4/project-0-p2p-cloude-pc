//! Linux Pro QEMU + KVM + VFIO attach (phase 34).
//!
//! After vfio-pci bind, this module starts a distro `qemu-system-*` with
//! `-enable-kvm` and `-device vfio-pci,host=<BDF>`. Guest net is SLIRP
//! (`-netdev user`) so the Ubuntu image gets DHCP without a TAP device.
//! TCG and WHPX are not used. Tests inject a fake process; production spawn
//! is refused under `cfg(test)` and off Linux.
//!
//! QEMU is not fetched from Weilnetz on Linux (that installer is Windows).
//! Operators set `HOST_AGENT_QEMU_BIN` or install `qemu-system-x86_64` from
//! the distro (free).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::host_capabilities::HostCapabilities;
use crate::launch::LaunchConfig;
use crate::provision::{HostOs, LabTier};
use crate::qemu::{self, DEVICE_ARG, MEMORY_ARG, SMP_ARG};

pub const QEMU_BIN_ENV: &str = "HOST_AGENT_QEMU_BIN";
pub const ENABLE_KVM: &str = "-enable-kvm";
pub const MACHINE_Q35: &str = "q35";
pub const VFIO_DEVICE_PREFIX: &str = "vfio-pci,host=";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinuxQemuError {
    #[error("Linux Pro QEMU is Linux-only; not starting a guest on {0}")]
    LinuxOnly(HostOs),
    #[error("QEMU+KVM+VFIO is the Linux Pro backend; not used for {0} sessions")]
    ProOnly(&'static str),
    #[error("Pro sessions are not offered; not starting QEMU ({reason})")]
    ProNotOffered { reason: String },
    #[error(
        "QEMU is not installed. Set {QEMU_BIN_ENV} or install qemu-system-x86_64 from the distro (free). Weilnetz NSIS is Windows-only."
    )]
    BinaryMissing,
    #[error("verified Pro guest image is not present; fetch and verify it before boot")]
    DiskMissing,
    #[error("PCI address for VFIO attach is missing after bind")]
    PciAddressMissing,
    #[error("QEMU vCPU count {0} must be at least 1")]
    InvalidVcpu(u32),
    #[error("could not start QEMU ({program}): {detail}")]
    Spawn { program: String, detail: String },
    #[error("QEMU spawn is disabled in tests; inject a fake process")]
    RefusedInTests,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeResult {
    pub hypervisor_invoked: bool,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InvokeRequest<'a> {
    pub os: HostOs,
    pub lab_tier: LabTier,
    pub capabilities: &'a HostCapabilities,
    pub binary: &'a Path,
    pub disk: &'a Path,
    pub vcpu_count: u32,
    pub memory_mib: u64,
    pub pci_address: &'a str,
    pub arch: &'a str,
    pub bootstrap_origin: Option<&'a str>,
    pub bootstrap_token: Option<&'a str>,
}

pub trait LinuxQemuChild: Send {
    fn kill(&mut self);
}

pub trait LinuxQemuSpawner {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn LinuxQemuChild>, LinuxQemuError>;
    fn detach(&self, child: Box<dyn LinuxQemuChild>);
}

pub trait LinuxQemuRuntime: Send + Sync {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, LinuxQemuError>;
}

pub struct ProcessLinuxQemuRuntime;

impl LinuxQemuRuntime for ProcessLinuxQemuRuntime {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, LinuxQemuError> {
        invoke(request, &ProcessSpawner)
    }
}

struct ProcessSpawner;

impl LinuxQemuSpawner for ProcessSpawner {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn LinuxQemuChild>, LinuxQemuError> {
        if cfg!(test) {
            return Err(LinuxQemuError::RefusedInTests);
        }
        if cfg!(not(target_os = "linux")) {
            return Err(LinuxQemuError::LinuxOnly(HostOs::current()));
        }
        let child = std::process::Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| LinuxQemuError::Spawn {
                program: binary.display().to_string(),
                detail: error.to_string(),
            })?;
        Ok(Box::new(ProcessChild(Some(child))))
    }

    fn detach(&self, child: Box<dyn LinuxQemuChild>) {
        RUNNING.lock().expect("linux qemu running mutex").push(child);
    }
}

struct ProcessChild(Option<std::process::Child>);

impl LinuxQemuChild for ProcessChild {
    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static RUNNING: Mutex<Vec<Box<dyn LinuxQemuChild>>> = Mutex::new(Vec::new());

pub fn kill_running() {
    let mut running = RUNNING.lock().expect("linux qemu running mutex");
    for child in running.drain(..) {
        let mut child = child;
        child.kill();
    }
}

pub fn is_aarch64(arch: &str) -> bool {
    matches!(arch, "aarch64" | "arm64")
}

pub fn default_binary_name(arch: &str) -> &'static str {
    if is_aarch64(arch) {
        "qemu-system-aarch64"
    } else {
        "qemu-system-x86_64"
    }
}

/// Distro QEMU or `HOST_AGENT_QEMU_BIN`. Never the Weilnetz `.exe`.
pub fn resolve_binary(
    arch: &str,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, LinuxQemuError> {
    if let Some(raw) = getenv(QEMU_BIN_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(LinuxQemuError::BinaryMissing);
    }
    let name = default_binary_name(arch);
    for candidate in [
        PathBuf::from("/usr/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(LinuxQemuError::BinaryMissing)
}

pub fn vfio_device_arg(pci_address: &str) -> String {
    format!("{VFIO_DEVICE_PREFIX}{pci_address}")
}

/// KVM + q35 + VFIO + SLIRP. No TCG, no WHPX, no Firecracker.
pub fn argv(arch: &str, vcpu_count: u32, memory_mib: u64, disk: &Path, pci_address: &str) -> Vec<String> {
    let smp = format!("cores={vcpu_count}");
    let memory = qemu::memory_flag(memory_mib);
    let disk = disk.to_string_lossy().into_owned();
    let mut args = vec![
        ENABLE_KVM.to_string(),
        qemu::MACHINE_ARG.to_string(),
        if is_aarch64(arch) {
            qemu::MACHINE_VIRT.to_string()
        } else {
            MACHINE_Q35.to_string()
        },
        qemu::CPU_ARG.to_string(),
        qemu::CPU_HOST.to_string(),
        SMP_ARG.to_string(),
        smp,
        MEMORY_ARG.to_string(),
        memory,
        "-drive".to_string(),
        format!("file={disk},if=virtio,format=raw"),
        DEVICE_ARG.to_string(),
        vfio_device_arg(pci_address),
    ];
    args.extend(qemu::net_argv());
    args
}

pub fn boot_from_launch(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    pci_address: &str,
    arch: &str,
    runtime: &dyn LinuxQemuRuntime,
) -> Result<InvokeResult, LinuxQemuError> {
    if os != HostOs::Linux {
        return Err(LinuxQemuError::LinuxOnly(os));
    }
    if lab_tier != LabTier::Pro {
        return Err(LinuxQemuError::ProOnly(lab_tier.as_str()));
    }
    let disk = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(LinuxQemuError::DiskMissing)?;
    if pci_address.trim().is_empty() {
        return Err(LinuxQemuError::PciAddressMissing);
    }
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        disk,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        pci_address,
        arch,
        bootstrap_origin: None,
        bootstrap_token: None,
    })
}

pub fn boot_from_launch_with_bootstrap(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    pci_address: &str,
    arch: &str,
    bootstrap_origin: Option<&str>,
    bootstrap_token: Option<&str>,
    runtime: &dyn LinuxQemuRuntime,
) -> Result<InvokeResult, LinuxQemuError> {
    if os != HostOs::Linux {
        return Err(LinuxQemuError::LinuxOnly(os));
    }
    if lab_tier != LabTier::Pro {
        return Err(LinuxQemuError::ProOnly(lab_tier.as_str()));
    }
    let disk = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(LinuxQemuError::DiskMissing)?;
    if pci_address.trim().is_empty() {
        return Err(LinuxQemuError::PciAddressMissing);
    }
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        disk,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        pci_address,
        arch,
        bootstrap_origin,
        bootstrap_token,
    })
}

pub fn invoke(
    request: &InvokeRequest<'_>,
    spawner: &dyn LinuxQemuSpawner,
) -> Result<InvokeResult, LinuxQemuError> {
    if request.os != HostOs::Linux {
        return Err(LinuxQemuError::LinuxOnly(request.os));
    }
    if request.lab_tier != LabTier::Pro {
        return Err(LinuxQemuError::ProOnly(request.lab_tier.as_str()));
    }
    if !request.capabilities.pro.offered {
        return Err(LinuxQemuError::ProNotOffered {
            reason: request.capabilities.pro.reason.clone(),
        });
    }
    if request.pci_address.trim().is_empty() {
        return Err(LinuxQemuError::PciAddressMissing);
    }
    require_file(request.binary, LinuxQemuError::BinaryMissing)?;
    require_file(request.disk, LinuxQemuError::DiskMissing)?;
    if request.vcpu_count == 0 {
        return Err(LinuxQemuError::InvalidVcpu(request.vcpu_count));
    }

    let mut args = argv(
        request.arch,
        request.vcpu_count,
        request.memory_mib,
        request.disk,
        request.pci_address,
    );
    args.extend(crate::guest_net::qemu_fw_cfg_argv(
        request.bootstrap_origin,
        request.bootstrap_token,
    ));
    debug_assert!(
        !args.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")),
        "Linux Pro must not pass TCG"
    );
    debug_assert!(
        !args.iter().any(|arg| arg.contains("whpx")),
        "Linux Pro must not pass WHPX"
    );
    let child = spawner.spawn(request.binary, &args)?;
    spawner.detach(child);
    Ok(InvokeResult {
        hypervisor_invoked: true,
        argv: args,
    })
}

fn require_file(path: &Path, missing: LinuxQemuError) -> Result<(), LinuxQemuError> {
    if path.as_os_str().is_empty() || !path.is_file() {
        Err(missing)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct RecordingLinuxQemu {
    pub spawns: Mutex<Vec<(PathBuf, Vec<String>)>>,
}

#[cfg(test)]
impl RecordingLinuxQemu {
    pub(crate) fn succeeding() -> Self {
        Self {
            spawns: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn spawn_calls(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.spawns.lock().expect("spawn mutex").clone()
    }
}

#[cfg(test)]
struct RecordingChild;

#[cfg(test)]
impl LinuxQemuChild for RecordingChild {
    fn kill(&mut self) {}
}

#[cfg(test)]
impl LinuxQemuSpawner for RecordingLinuxQemu {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn LinuxQemuChild>, LinuxQemuError> {
        self.spawns
            .lock()
            .expect("spawn mutex")
            .push((binary.to_path_buf(), args.to_vec()));
        Ok(Box::new(RecordingChild))
    }

    fn detach(&self, _child: Box<dyn LinuxQemuChild>) {}
}

#[cfg(test)]
impl LinuxQemuRuntime for RecordingLinuxQemu {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, LinuxQemuError> {
        invoke(request, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::provision::ComputeBackend;
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with};
    use std::fs;

    fn linux_pro_caps() -> HostCapabilities {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &gpus,
        })
    }

    fn fixture_tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64");
        let disk = dir.path().join("pro").join("rootfs.img");
        fs::create_dir_all(disk.parent().unwrap()).unwrap();
        fs::write(&binary, b"fake-qemu").unwrap();
        fs::write(&disk, b"fake-rootfs").unwrap();
        (dir, binary, disk)
    }

    fn request<'a>(
        caps: &'a HostCapabilities,
        binary: &'a Path,
        disk: &'a Path,
        bdf: &'a str,
    ) -> InvokeRequest<'a> {
        InvokeRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Pro,
            capabilities: caps,
            binary,
            disk,
            vcpu_count: 4,
            memory_mib: 8192,
            pci_address: bdf,
            arch: "x86_64",
            bootstrap_origin: None,
            bootstrap_token: None,
        }
    }

    #[test]
    fn linux_pro_locks_kvm_vfio_and_user_net() {
        let caps = linux_pro_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingLinuxQemu::succeeding();
        let result = invoke(&request(&caps, &binary, &disk, "0000:01:00.0"), &recorder).unwrap();
        assert!(result.hypervisor_invoked);
        let args = &result.argv;
        assert_eq!(args[0], ENABLE_KVM);
        assert!(args.contains(&MACHINE_Q35.to_string()));
        assert!(args.iter().any(|arg| arg == "vfio-pci,host=0000:01:00.0"));
        assert!(args.windows(2).any(|w| w[0] == qemu::NETDEV_ARG && w[1] == qemu::NETDEV_USER));
        assert!(!args.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
        assert!(!args.iter().any(|arg| arg.contains("whpx")));
        assert_eq!(recorder.spawn_calls().len(), 1);
        assert_eq!(caps.pro.offered, true);
        let _ = ComputeBackend::QemuVfio;
    }

    #[test]
    fn linux_pro_fw_cfg_carries_bootstrap_not_preauth() {
        let caps = linux_pro_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingLinuxQemu::succeeding();
        let mut req = request(&caps, &binary, &disk, "0000:01:00.0");
        req.bootstrap_origin = Some("http://mm.test");
        req.bootstrap_token = Some("guest-token");
        let result = invoke(&req, &recorder).unwrap();
        assert!(result.argv.windows(2).any(|w| {
            w[0] == "-fw_cfg" && w[1].contains("opt/org.dcl/bootstrap") && w[1].contains("guest-token")
        }));
        assert!(!result.argv.iter().any(|arg| arg.contains("tskey-auth")));
    }

    #[test]
    fn windows_is_rejected() {
        let caps = linux_pro_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingLinuxQemu::succeeding();
        let mut req = request(&caps, &binary, &disk, "0000:01:00.0");
        req.os = HostOs::Windows;
        let error = invoke(&req, &recorder).unwrap_err();
        assert_eq!(error, LinuxQemuError::LinuxOnly(HostOs::Windows));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn standard_is_rejected() {
        let caps = linux_pro_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingLinuxQemu::succeeding();
        let mut req = request(&caps, &binary, &disk, "0000:01:00.0");
        req.lab_tier = LabTier::Standard;
        let error = invoke(&req, &recorder).unwrap_err();
        assert_eq!(error, LinuxQemuError::ProOnly("standard"));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn missing_bdf_fails_before_spawn() {
        let caps = linux_pro_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingLinuxQemu::succeeding();
        let error = invoke(&request(&caps, &binary, &disk, ""), &recorder).unwrap_err();
        assert_eq!(error, LinuxQemuError::PciAddressMissing);
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn production_spawner_refuses_under_cfg_test() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64");
        fs::write(&binary, b"fake").unwrap();
        let error = match ProcessSpawner.spawn(&binary, &[]) {
            Err(error) => error,
            Ok(_) => panic!("production spawner must refuse under cfg(test)"),
        };
        assert_eq!(error, LinuxQemuError::RefusedInTests);
    }

    #[test]
    fn resolve_binary_uses_env_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("custom-qemu");
        fs::write(&bin, b"qemu").unwrap();
        let path = resolve_binary("x86_64", |_| Some(bin.to_string_lossy().into_owned())).unwrap();
        assert_eq!(path, bin);
    }

    #[test]
    fn resolve_binary_env_missing_fails() {
        let error = resolve_binary("x86_64", |_| {
            Some("/tmp/dcl-missing-qemu-phase34".to_string())
        })
        .unwrap_err();
        assert_eq!(error, LinuxQemuError::BinaryMissing);
    }
}
