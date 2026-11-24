//! Linux Standard QEMU+KVM runtime.
//!
//! The signed Ubuntu image boots its own kernel and initramfs. The guest gets
//! outbound networking without a privileged TAP device, then joins Headscale
//! and exposes Kasm to the gateway over that mesh.
//!
//! Network backend (behind the same virtio-net card):
//! - **passt** when the `passt` binary is installed and QEMU has the `stream`
//!   netdev (QEMU 7.2+). Much faster than SLIRP, no root. It is started with
//!   `--no-map-gw`, so the guest cannot reach the host through the gateway
//!   address, and with no inbound port forwards.
//! - **SLIRP** otherwise, or when `HOST_AGENT_NET_BACKEND=slirp`, or when
//!   passt does not come up. A failed passt never blocks a session.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::host_capabilities::HostCapabilities;
use crate::launch::LaunchConfig;
use crate::provision::{HostOs, LabTier};
use crate::qemu_whpx::{self, MEMORY_ARG, SMP_ARG};

pub const ENABLE_KVM: &str = "-enable-kvm";
pub const MACHINE_Q35: &str = "q35";
pub const QEMU_BIN_ENV: &str = "HOST_AGENT_QEMU_BIN";
/// `slirp` forces the old backend. Anything else lets the agent pick.
pub const NET_BACKEND_ENV: &str = "HOST_AGENT_NET_BACKEND";
const PASST_PATHS: [&str; 2] = ["/usr/bin/passt", "/usr/local/bin/passt"];
const PASST_SOCKET_WAIT_STEPS: u32 = 40;
const PASST_SOCKET_STEP: std::time::Duration = std::time::Duration::from_millis(50);

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

    /// Where `passt` is, when this host should use it. None keeps SLIRP.
    fn passt_binary(&self) -> Option<PathBuf> {
        None
    }

    /// Whether this QEMU knows `-netdev stream` (needed to talk to passt).
    fn qemu_supports_stream(&self, _qemu: &Path) -> bool {
        true
    }

    /// Wait for passt's socket to appear. False when it never does.
    fn wait_for_socket(&self, _socket: &Path) -> bool {
        true
    }
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

    fn passt_binary(&self) -> Option<PathBuf> {
        if cfg!(test) || std::env::var(NET_BACKEND_ENV).is_ok_and(|v| v.trim() == "slirp") {
            return None;
        }
        PASST_PATHS.iter().map(PathBuf::from).find(|path| path.is_file())
    }

    fn qemu_supports_stream(&self, qemu: &Path) -> bool {
        std::process::Command::new(qemu)
            .args(["-netdev", "help"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).lines().any(|l| l.trim() == "stream"))
            .unwrap_or(false)
    }

    fn wait_for_socket(&self, socket: &Path) -> bool {
        for _ in 0..PASST_SOCKET_WAIT_STEPS {
            if socket.exists() {
                return true;
            }
            std::thread::sleep(PASST_SOCKET_STEP);
        }
        false
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

/// Default arguments: SLIRP networking. Used by the Windows/Linux parity test.
#[cfg(test)]
pub fn argv(vcpu_count: u32, memory_mib: u64, disk: &Path) -> Vec<String> {
    argv_with_net(vcpu_count, memory_mib, disk, qemu_whpx::net_argv())
}

/// `-netdev stream` to a running passt, plus the same virtio-net card.
pub fn passt_net_argv(socket: &Path) -> Vec<String> {
    vec![
        qemu_whpx::NETDEV_ARG.to_string(),
        format!(
            "stream,id=net0,server=off,addr.type=unix,addr.path={}",
            socket.display()
        ),
        qemu_whpx::DEVICE_ARG.to_string(),
        qemu_whpx::VIRTIO_NET.to_string(),
    ]
}

/// passt in the foreground, on `socket`, no inbound forwards, and the
/// gateway address not mapped to the host.
pub fn passt_args(socket: &Path) -> Vec<String> {
    [
        "-f",
        "--no-map-gw",
        "-t",
        "none",
        "-u",
        "none",
        "-s",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .chain(std::iter::once(socket.display().to_string()))
    .collect()
}

fn passt_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!("dcl-passt-{}.sock", std::process::id()))
}

/// Start passt and wait for its socket. None keeps SLIRP: no passt, QEMU too
/// old, spawn failed, or the socket never appeared.
fn start_passt(qemu: &Path, spawner: &dyn QemuKvmSpawner) -> Option<(PathBuf, Box<dyn QemuKvmChild>)> {
    let passt = spawner.passt_binary()?;
    if !spawner.qemu_supports_stream(qemu) {
        return None;
    }
    let socket = passt_socket_path();
    let _ = std::fs::remove_file(&socket);
    let mut child = spawner.spawn(&passt, &passt_args(&socket)).ok()?;
    if !spawner.wait_for_socket(&socket) {
        child.kill();
        return None;
    }
    Some((socket, child))
}

pub fn argv_with_net(
    vcpu_count: u32,
    memory_mib: u64,
    disk: &Path,
    net: Vec<String>,
) -> Vec<String> {
    let mut args = vec![
        ENABLE_KVM.to_string(),
        qemu_whpx::MACHINE_ARG.to_string(),
        MACHINE_Q35.to_string(),
        qemu_whpx::CPU_ARG.to_string(),
        qemu_whpx::CPU_HOST.to_string(),
        SMP_ARG.to_string(),
        format!("cores={vcpu_count}"),
        MEMORY_ARG.to_string(),
        qemu_whpx::memory_flag(memory_mib),
        "-drive".to_string(),
        qemu_whpx::virtio_drive(&disk.to_string_lossy()),
        "-display".to_string(),
        "none".to_string(),
        "-monitor".to_string(),
        "none".to_string(),
        "-serial".to_string(),
        "none".to_string(),
        "-no-reboot".to_string(),
    ];
    args.extend(net);
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

    let passt = start_passt(request.binary, spawner);
    let net = match &passt {
        Some((socket, _)) => passt_net_argv(socket),
        None => qemu_whpx::net_argv(),
    };
    let mut args = argv_with_net(request.vcpu_count, request.memory_mib, request.disk, net);
    args.extend(crate::guest_bootstrap::qemu_bootstrap_argv(
        request.bootstrap_origin,
        request.bootstrap_token,
    ));
    let (qga_port, guest_args) = crate::guest_power::prepare();
    args.extend(guest_args);
    debug_assert!(!args.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
    debug_assert!(!args.iter().any(|arg| arg.eq_ignore_ascii_case("whpx")));
    let child = match spawner.spawn(request.binary, &args) {
        Ok(child) => child,
        Err(error) => {
            if let Some((_, mut passt_child)) = passt {
                passt_child.kill();
            }
            return Err(error);
        }
    };
    if let Some((_, passt_child)) = passt {
        spawner.detach(passt_child);
    }
    spawner.detach(child);
    crate::guest_power::remember(qga_port);
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

/// Records spawns instead of starting QEMU. Used by provision HTTP tests.
#[cfg(test)]
pub(crate) struct RecordingQemuKvm {
    spawns: Mutex<Vec<(std::path::PathBuf, Vec<String>)>>,
}

#[cfg(test)]
impl RecordingQemuKvm {
    pub(crate) fn succeeding() -> Self {
        Self {
            spawns: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn spawn_calls(&self) -> Vec<(std::path::PathBuf, Vec<String>)> {
        self.spawns.lock().expect("spawn mutex").clone()
    }
}

#[cfg(test)]
struct RecordedChild;

#[cfg(test)]
impl QemuKvmChild for RecordedChild {
    fn kill(&mut self) {}
}

#[cfg(test)]
impl QemuKvmSpawner for RecordingQemuKvm {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn QemuKvmChild>, QemuKvmError> {
        self.spawns
            .lock()
            .expect("spawn mutex")
            .push((binary.to_path_buf(), args.to_vec()));
        Ok(Box::new(RecordedChild))
    }

    fn detach(&self, _child: Box<dyn QemuKvmChild>) {}
}

#[cfg(test)]
impl QemuKvmRuntime for RecordingQemuKvm {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuKvmError> {
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
        assert!(result.argv.iter().any(|arg| arg == qemu_whpx::NETDEV_USER));
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

    struct PasstSpawner {
        socket_ready: bool,
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        killed: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    struct CountingChild(std::sync::Arc<std::sync::atomic::AtomicU32>);

    impl QemuKvmChild for CountingChild {
        fn kill(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl QemuKvmSpawner for PasstSpawner {
        fn spawn(
            &self,
            binary: &Path,
            args: &[String],
        ) -> Result<Box<dyn QemuKvmChild>, QemuKvmError> {
            self.calls
                .lock()
                .unwrap()
                .push((binary.to_path_buf(), args.to_vec()));
            Ok(Box::new(CountingChild(self.killed.clone())))
        }
        fn detach(&self, _child: Box<dyn QemuKvmChild>) {}
        fn passt_binary(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/usr/bin/passt"))
        }
        fn wait_for_socket(&self, _socket: &Path) -> bool {
            self.socket_ready
        }
    }

    fn run_with_passt(socket_ready: bool) -> (InvokeResult, Vec<(PathBuf, Vec<String>)>, u32) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64");
        let disk = dir.path().join("rootfs.img");
        fs::write(&binary, b"qemu").unwrap();
        fs::write(&disk, b"disk").unwrap();
        let spawner = PasstSpawner {
            socket_ready,
            calls: Mutex::new(Vec::new()),
            killed: Default::default(),
        };
        let result = invoke(
            &InvokeRequest {
                os: HostOs::Linux,
                lab_tier: LabTier::Standard,
                capabilities: &caps(),
                binary: &binary,
                disk: &disk,
                vcpu_count: 2,
                memory_mib: 4096,
                arch: "x86_64",
                bootstrap_origin: None,
                bootstrap_token: None,
            },
            &spawner,
        )
        .unwrap();
        let calls = spawner.calls.lock().unwrap().clone();
        let killed = spawner.killed.load(std::sync::atomic::Ordering::SeqCst);
        (result, calls, killed)
    }

    #[test]
    fn passt_replaces_slirp_when_it_comes_up() {
        let (result, calls, killed) = run_with_passt(true);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, PathBuf::from("/usr/bin/passt"));
        let passt_args = &calls[0].1;
        assert!(passt_args.contains(&"--no-map-gw".to_string()));
        assert!(passt_args.windows(2).any(|p| p == ["-t", "none"]));
        assert!(passt_args.windows(2).any(|p| p == ["-u", "none"]));
        assert!(result.argv.iter().any(|a| a.starts_with("stream,id=net0,server=off,addr.type=unix")));
        assert!(!result.argv.iter().any(|a| a == qemu_whpx::NETDEV_USER));
        assert!(result.argv.iter().any(|a| a == qemu_whpx::VIRTIO_NET));
        assert_eq!(killed, 0);
    }

    #[test]
    fn a_passt_that_never_comes_up_falls_back_to_slirp_and_is_stopped() {
        let (result, calls, killed) = run_with_passt(false);
        assert_eq!(calls.len(), 2, "passt was tried, then QEMU started");
        assert!(result.argv.iter().any(|a| a == qemu_whpx::NETDEV_USER));
        assert!(!result.argv.iter().any(|a| a.starts_with("stream,")));
        assert_eq!(killed, 1, "the failed passt process is killed");
    }
}
