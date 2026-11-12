//! Linux Standard QEMU+KVM runtime.
//!
//! The signed Ubuntu image boots its own kernel and initramfs. SLIRP gives the
//! guest outbound networking without a privileged TAP device; the guest then
//! joins Headscale and exposes Kasm to the gateway over that mesh.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::host_capabilities::HostCapabilities;
use crate::launch::LaunchConfig;
use crate::provision::{HostOs, LabTier};
use crate::qemu::{self, DEVICE_ARG, MEMORY_ARG, SMP_ARG};

pub const ENABLE_KVM: &str = "-enable-kvm";
pub const MACHINE_Q35: &str = "q35";
pub const QEMU_BIN_ENV: &str = "HOST_AGENT_QEMU_BIN";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QemuKvmError {
    #[error("QEMU+KVM is Linux-only; not starting a guest on {0}")]
    LinuxOnly(HostOs),
    #[error("QEMU+KVM is the Standard Linux backend; not used for {0} sessions")]
    StandardOnly(&'static str),
    #[error("Standard sessions are not offered; not starting QEMU ({reason})")]
    StandardNotOffered { reason: String },
    #[error("QEMU+KVM currently requires an x86_64 Linux host, not {0}")]
    UnsupportedArch(String),
    #[error(
        "QEMU is not installed. Set HOST_AGENT_QEMU_BIN or install qemu-system-x86 from the Linux distribution."
    )]
    BinaryMissing,
    #[error("verified Standard guest image is not present; fetch and verify it before boot")]
    DiskMissing,
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
    pub arch: &'a str,
    pub bootstrap_origin: Option<&'a str>,
    pub bootstrap_token: Option<&'a str>,
}

pub trait QemuKvmChild: Send {
    fn kill(&mut self);
}

pub trait QemuKvmSpawner {
    fn spawn(&self, binary: &Path, args: &[String])
        -> Result<Box<dyn QemuKvmChild>, QemuKvmError>;
    fn detach(&self, child: Box<dyn QemuKvmChild>);
}

pub trait QemuKvmRuntime: Send + Sync {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuKvmError>;
}

pub struct ProcessQemuKvmRuntime;

impl QemuKvmRuntime for ProcessQemuKvmRuntime {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuKvmError> {
        invoke(request, &ProcessSpawner)
    }
}

struct ProcessSpawner;

impl QemuKvmSpawner for ProcessSpawner {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn QemuKvmChild>, QemuKvmError> {
        if cfg!(test) {
            return Err(QemuKvmError::RefusedInTests);
        }
        if cfg!(not(target_os = "linux")) {
            return Err(QemuKvmError::LinuxOnly(HostOs::current()));
        }
        let child = std::process::Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| QemuKvmError::Spawn {
                program: binary.display().to_string(),
                detail: error.to_string(),
            })?;
        Ok(Box::new(ProcessChild(Some(child))))
    }

    fn detach(&self, child: Box<dyn QemuKvmChild>) {
        RUNNING.lock().expect("qemu kvm running mutex").push(child);
    }
}

struct ProcessChild(Option<std::process::Child>);

impl QemuKvmChild for ProcessChild {
    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static RUNNING: Mutex<Vec<Box<dyn QemuKvmChild>>> = Mutex::new(Vec::new());

pub fn kill_running() {
    let mut running = RUNNING.lock().expect("qemu kvm running mutex");
    for mut child in running.drain(..) {
        child.kill();
    }
}

pub fn resolve_binary(
    arch: &str,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, QemuKvmError> {
    if !matches!(arch, "x86_64" | "amd64") {
        return Err(QemuKvmError::UnsupportedArch(arch.to_string()));
    }
    if let Some(raw) = getenv(QEMU_BIN_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(raw);
        return path
            .is_file()
            .then_some(path)
            .ok_or(QemuKvmError::BinaryMissing);
    }
    for candidate in [
        PathBuf::from("/usr/bin/qemu-system-x86_64"),
        PathBuf::from("/usr/local/bin/qemu-system-x86_64"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(QemuKvmError::BinaryMissing)
}

pub fn argv(vcpu_count: u32, memory_mib: u64, disk: &Path) -> Vec<String> {
    let mut args = vec![
        ENABLE_KVM.to_string(),
        qemu::MACHINE_ARG.to_string(),
        MACHINE_Q35.to_string(),
        qemu::CPU_ARG.to_string(),
        qemu::CPU_HOST.to_string(),
        SMP_ARG.to_string(),
        format!("cores={vcpu_count}"),
        MEMORY_ARG.to_string(),
        qemu::memory_flag(memory_mib),
        "-drive".to_string(),
        format!(
            "file={},if=virtio,format=raw",
            disk.to_string_lossy()
        ),
        "-display".to_string(),
        "none".to_string(),
        "-monitor".to_string(),
        "none".to_string(),
        "-serial".to_string(),
        "none".to_string(),
        "-no-reboot".to_string(),
    ];
    args.extend(qemu::net_argv());
    args
}

pub fn boot_from_launch_with_bootstrap(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    arch: &str,
    bootstrap_origin: Option<&str>,
    bootstrap_token: Option<&str>,
    runtime: &dyn QemuKvmRuntime,
) -> Result<InvokeResult, QemuKvmError> {
    if os != HostOs::Linux {
        return Err(QemuKvmError::LinuxOnly(os));
    }
    if lab_tier != LabTier::Standard {
        return Err(QemuKvmError::StandardOnly(lab_tier.as_str()));
    }
    let disk = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(QemuKvmError::DiskMissing)?;
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        disk,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        arch,
        bootstrap_origin,
        bootstrap_token,
    })
}

pub fn invoke(
    request: &InvokeRequest<'_>,
    spawner: &dyn QemuKvmSpawner,
) -> Result<InvokeResult, QemuKvmError> {
    if request.os != HostOs::Linux {
        return Err(QemuKvmError::LinuxOnly(request.os));
    }
    if request.lab_tier != LabTier::Standard {
        return Err(QemuKvmError::StandardOnly(request.lab_tier.as_str()));
    }
    if !request.capabilities.standard.offered {
        return Err(QemuKvmError::StandardNotOffered {
            reason: request.capabilities.standard.reason.clone(),
        });
    }
    if !matches!(request.arch, "x86_64" | "amd64") {
        return Err(QemuKvmError::UnsupportedArch(request.arch.to_string()));
    }
    require_file(request.binary, QemuKvmError::BinaryMissing)?;
    require_file(request.disk, QemuKvmError::DiskMissing)?;
    if request.vcpu_count == 0 {
        return Err(QemuKvmError::InvalidVcpu(request.vcpu_count));
    }

    let mut args = argv(request.vcpu_count, request.memory_mib, request.disk);
    args.extend(crate::guest_net::qemu_fw_cfg_argv(
        request.bootstrap_origin,
        request.bootstrap_token,
    ));
    debug_assert!(!args.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
    debug_assert!(!args.iter().any(|arg| arg.eq_ignore_ascii_case("whpx")));
    let child = spawner.spawn(request.binary, &args)?;
    spawner.detach(child);
    Ok(InvokeResult {
        hypervisor_invoked: true,
        argv: args,
    })
}

fn require_file(path: &Path, missing: QemuKvmError) -> Result<(), QemuKvmError> {
    if path.as_os_str().is_empty() || !path.is_file() {
        Err(missing)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::provision::ComputeBackend;
    use std::fs;

    struct RecordingSpawner {
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    }

    struct RecordingChild;

    impl QemuKvmChild for RecordingChild {
        fn kill(&mut self) {}
    }

    impl QemuKvmSpawner for RecordingSpawner {
        fn spawn(
            &self,
            binary: &Path,
            args: &[String],
        ) -> Result<Box<dyn QemuKvmChild>, QemuKvmError> {
            self.calls
                .lock()
                .unwrap()
                .push((binary.to_path_buf(), args.to_vec()));
            Ok(Box::new(RecordingChild))
        }

        fn detach(&self, _child: Box<dyn QemuKvmChild>) {}
    }

    fn caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        })
    }

    #[test]
    fn linux_standard_uses_kvm_user_net_and_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64");
        let disk = dir.path().join("rootfs.img");
        fs::write(&binary, b"qemu").unwrap();
        fs::write(&disk, b"disk").unwrap();
        let recorder = RecordingSpawner {
            calls: Mutex::new(Vec::new()),
        };
        let result = invoke(
            &InvokeRequest {
                os: HostOs::Linux,
                lab_tier: LabTier::Standard,
                capabilities: &caps(),
                binary: &binary,
                disk: &disk,
                vcpu_count: 4,
                memory_mib: 8192,
                arch: "x86_64",
                bootstrap_origin: Some("http://matchmaker.test"),
                bootstrap_token: Some("guest-token"),
            },
            &recorder,
        )
        .unwrap();

        assert!(result.hypervisor_invoked);
        assert_eq!(ComputeBackend::QemuKvm.slug(), "qemu-kvm");
        assert!(result.argv.iter().any(|arg| arg == ENABLE_KVM));
        assert!(result.argv.iter().any(|arg| arg == qemu::NETDEV_USER));
        assert!(result.argv.iter().any(|arg| arg.contains("opt/org.dcl/bootstrap")));
        assert!(result.argv.windows(2).any(|pair| pair == ["-display", "none"]));
        assert!(!result.argv.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
        assert_eq!(recorder.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn missing_kvm_offer_or_wrong_platform_does_not_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64");
        let disk = dir.path().join("rootfs.img");
        fs::write(&binary, b"qemu").unwrap();
        fs::write(&disk, b"disk").unwrap();
        let recorder = RecordingSpawner {
            calls: Mutex::new(Vec::new()),
        };
        let no_kvm = evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        });
        let mut request = InvokeRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            capabilities: &no_kvm,
            binary: &binary,
            disk: &disk,
            vcpu_count: 2,
            memory_mib: 4096,
            arch: "x86_64",
            bootstrap_origin: None,
            bootstrap_token: None,
        };
        assert!(matches!(
            invoke(&request, &recorder),
            Err(QemuKvmError::StandardNotOffered { .. })
        ));
        request.os = HostOs::Windows;
        assert_eq!(
            invoke(&request, &recorder),
            Err(QemuKvmError::LinuxOnly(HostOs::Windows))
        );
        assert!(recorder.calls.lock().unwrap().is_empty());
    }
}
