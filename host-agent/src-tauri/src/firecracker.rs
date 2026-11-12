//! Linux Standard Firecracker invoke (phase 27).
//!
//! After the capability probe offers Standard, and the phase-19 binary plus a
//! verified phase-20/26 `rootfs.img` and a kernel are present, this module
//! starts the already-fetched Firecracker binary and talks to its Unix API
//! the way Firecracker v1.16.1 documents:
//!
//! 1. `firecracker --api-sock <sock> --enable-pci`
//! 2. `PUT /boot-source` (kernel cmdline includes a static TAP address)
//! 3. `PUT /drives/rootfs`
//! 4. `PUT /machine-config` (clamped vCPU/RAM; defaults are 1 vCPU / 128 MiB)
//! 5. `PUT /network-interfaces/eth0` (TAP `dcl-tap0`)
//! 6. `PUT /actions` with `InstanceStart`
//!
//! Jailer is unused. Tests inject a fake process, TAP, and API so CI never
//! execs Firecracker, `ip`, or needs `/dev/kvm`. Production spawn is refused
//! under `cfg(test)` and off Linux.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use thiserror::Error;

use crate::guest_net::{self, TapError, TapOps, GUEST_MAC, HOST_IP, TAP_DEV, TAP_PREFIX};
use crate::host_capabilities::HostCapabilities;
use crate::image::cached_kernel_path;
use crate::launch::LaunchConfig;
use crate::provision::{HostOs, LabTier};

/// Operator kernel path. Packer does not invent `vmlinux` from GRUB.
pub const KERNEL_ENV: &str = "FIRECRACKER_KERNEL";
pub const API_SOCK_ARG: &str = "--api-sock";
/// Documented in Firecracker v1.16.1 getting-started (optional PCI virtio).
pub const ENABLE_PCI_ARG: &str = "--enable-pci";
pub const ROOTFS_DRIVE_ID: &str = "rootfs";
pub const INSTANCE_START: &str = "InstanceStart";
/// Firecracker v1.16.1 swagger: successful PUT /actions, /boot-source, /drives,
/// /machine-config, /network-interfaces/{id} return 204.
pub const NETWORK_IFACE_ID: &str = "eth0";
pub const NETWORK_IFACE_PATH: &str = "/network-interfaces/eth0";
pub const API_SUCCESS_STATUS: u16 = 204;
/// Firecracker v1.16.1 `vcpu_count` maximum.
pub const MAX_VCPU: u32 = 32;
/// Getting-started kernel cmdline (x86_64). aarch64 prefixes `keep_bootcon`.
pub const BOOT_ARGS_X86: &str = "console=ttyS0 reboot=k panic=1";

const SOCKET_WAIT: Duration = Duration::from_secs(2);
const SOCKET_POLL: Duration = Duration::from_millis(20);
const API_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Locked API order for v1.16.1: configure (including TAP), then `InstanceStart`.
pub const API_PATH_ORDER: [&str; 5] = [
    "/boot-source",
    "/drives/rootfs",
    "/machine-config",
    NETWORK_IFACE_PATH,
    "/actions",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FirecrackerError {
    #[error("Firecracker is Linux-only; not starting a microVM on {0}")]
    LinuxOnly(HostOs),
    #[error("Firecracker is the Standard Linux backend; not used for {0} sessions")]
    StandardOnly(&'static str),
    #[error("Standard sessions are not offered; not starting Firecracker ({reason})")]
    StandardNotOffered { reason: String },
    #[error(
        "Firecracker kernel is missing. Set {KERNEL_ENV} to a vmlinux that can mount this Ubuntu rootfs (virtio-blk), or place vmlinux next to the verified rootfs. Packer does not invent a kernel from GRUB."
    )]
    KernelMissing,
    #[error(
        "Firecracker binary is not installed. Locate or fetch it first (user hypervisor cache)."
    )]
    BinaryMissing,
    #[error(
        "verified Standard guest image (rootfs.img) is not present; fetch and verify it before boot"
    )]
    RootfsMissing,
    #[error("Firecracker vCPU count {0} is outside 1–{MAX_VCPU}")]
    InvalidVcpu(u32),
    #[error("could not start Firecracker ({program}): {detail}")]
    Spawn { program: String, detail: String },
    #[error("Firecracker API socket was not ready at {path}: {detail}")]
    Socket { path: String, detail: String },
    #[error("Firecracker API {path} returned HTTP {status}")]
    ApiRejected { path: String, status: u16 },
    #[error("Firecracker API {path} failed: {detail}")]
    ApiIo { path: String, detail: String },
    #[error("Firecracker spawn is disabled in tests; inject a fake process and API")]
    RefusedInTests,
    #[error("{0}")]
    Tap(TapError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCall {
    pub method: String,
    pub path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeResult {
    pub hypervisor_invoked: bool,
    pub api_sock: PathBuf,
    pub argv: Vec<String>,
    pub api_calls: Vec<ApiCall>,
}

#[derive(Debug, Clone)]
pub struct InvokeRequest<'a> {
    pub os: HostOs,
    pub lab_tier: LabTier,
    pub capabilities: &'a HostCapabilities,
    pub binary: &'a Path,
    pub kernel: &'a Path,
    pub rootfs: &'a Path,
    pub vcpu_count: u32,
    pub memory_mib: u64,
    pub api_sock: &'a Path,
    pub arch: &'a str,
    /// Matchmaker origin the guest may call (no Headscale preauth).
    pub bootstrap_origin: Option<&'a str>,
    /// One-time guest bootstrap token (not a Headscale preauth, not S3 keys).
    pub bootstrap_token: Option<&'a str>,
}

pub trait FirecrackerChild: Send {
    fn kill(&mut self);
}

pub trait FirecrackerSpawner {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn FirecrackerChild>, FirecrackerError>;
    fn detach(&self, child: Box<dyn FirecrackerChild>);
}

pub trait FirecrackerApi {
    fn wait_ready(&self, socket: &Path) -> Result<(), FirecrackerError>;
    fn put(&self, socket: &Path, path: &str, body: &str) -> Result<u16, FirecrackerError>;
}

pub trait FirecrackerRuntime: Send + Sync {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, FirecrackerError>;
}

/// Production runtime: spawn the cached binary and PUT over the Unix socket.
pub struct ProcessFirecrackerRuntime;

impl FirecrackerRuntime for ProcessFirecrackerRuntime {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, FirecrackerError> {
        invoke_with_tap(request, &ProcessSpawner, &UnixHttpApi, &guest_net::ProcessTapOps)
    }
}

struct ProcessSpawner;

impl FirecrackerSpawner for ProcessSpawner {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn FirecrackerChild>, FirecrackerError> {
        if cfg!(test) {
            return Err(FirecrackerError::RefusedInTests);
        }
        if cfg!(not(target_os = "linux")) {
            return Err(FirecrackerError::LinuxOnly(HostOs::current()));
        }
        let child = std::process::Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| FirecrackerError::Spawn {
                program: binary.display().to_string(),
                detail: error.to_string(),
            })?;
        Ok(Box::new(ProcessChild(Some(child))))
    }

    fn detach(&self, child: Box<dyn FirecrackerChild>) {
        RUNNING.lock().expect("firecracker running mutex").push(child);
    }
}

/// Kill microVMs detached after a successful InstanceStart.
pub fn kill_running() {
    let mut running = RUNNING.lock().expect("firecracker running mutex");
    for child in running.drain(..) {
        let mut child = child;
        child.kill();
    }
}

struct ProcessChild(Option<std::process::Child>);

impl FirecrackerChild for ProcessChild {
    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct UnixHttpApi;

impl FirecrackerApi for UnixHttpApi {
    fn wait_ready(&self, socket: &Path) -> Result<(), FirecrackerError> {
        wait_for_unix_socket(socket)
    }

    fn put(&self, socket: &Path, path: &str, body: &str) -> Result<u16, FirecrackerError> {
        unix_http_put(socket, path, body)
    }
}

static RUNNING: Mutex<Vec<Box<dyn FirecrackerChild>>> = Mutex::new(Vec::new());

/// `firecracker --api-sock <sock> --enable-pci` as Firecracker v1.16.1 documents.
pub fn argv(api_sock: &Path) -> Vec<String> {
    vec![
        API_SOCK_ARG.to_string(),
        api_sock.to_string_lossy().into_owned(),
        ENABLE_PCI_ARG.to_string(),
    ]
}

pub fn api_sock_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("firecracker").join("api.sock")
}

pub fn boot_args(arch: &str) -> String {
    boot_args_with_bootstrap(arch, None, None)
}

/// Kernel cmdline: console + static TAP address + optional guest bootstrap
/// (Matchmaker origin + token). Never a Headscale preauth or S3 key.
pub fn boot_args_with_bootstrap(
    arch: &str,
    origin: Option<&str>,
    token: Option<&str>,
) -> String {
    let mut args = if matches!(arch, "aarch64" | "arm64") {
        format!("keep_bootcon {BOOT_ARGS_X86}")
    } else {
        BOOT_ARGS_X86.to_string()
    };
    args.push(' ');
    args.push_str(&guest_net::firecracker_ip_boot_arg());
    if let (Some(origin), Some(token)) = (origin, token) {
        let origin = origin.trim();
        let token = token.trim();
        if !origin.is_empty() && !token.is_empty() && !origin.contains(' ') && !token.contains(' ')
        {
            args.push_str(" dcl.matchmaker=");
            args.push_str(origin);
            args.push_str(" dcl.bootstrap=");
            args.push_str(token);
        }
    }
    args
}

/// PUT bodies in the locked order (boot-source, drives, machine-config, TAP, actions).
pub fn api_calls(request: &InvokeRequest<'_>) -> Vec<ApiCall> {
    let boot_args = boot_args_with_bootstrap(
        request.arch,
        request.bootstrap_origin,
        request.bootstrap_token,
    );
    let boot = serde_json::json!({
        "kernel_image_path": request.kernel.to_string_lossy(),
        "boot_args": boot_args,
    });
    let drive = serde_json::json!({
        "drive_id": ROOTFS_DRIVE_ID,
        "path_on_host": request.rootfs.to_string_lossy(),
        "is_root_device": true,
        "is_read_only": false,
    });
    let machine = MachineConfigBody {
        vcpu_count: request.vcpu_count,
        mem_size_mib: request.memory_mib,
        smt: false,
    };
    let net = serde_json::json!({
        "iface_id": NETWORK_IFACE_ID,
        "guest_mac": GUEST_MAC,
        "host_dev_name": TAP_DEV,
    });
    let action = serde_json::json!({ "action_type": INSTANCE_START });
    vec![
        ApiCall {
            method: "PUT".to_string(),
            path: API_PATH_ORDER[0].to_string(),
            body: boot.to_string(),
        },
        ApiCall {
            method: "PUT".to_string(),
            path: API_PATH_ORDER[1].to_string(),
            body: drive.to_string(),
        },
        ApiCall {
            method: "PUT".to_string(),
            path: API_PATH_ORDER[2].to_string(),
            body: serde_json::to_string(&machine).expect("machine-config json"),
        },
        ApiCall {
            method: "PUT".to_string(),
            path: API_PATH_ORDER[3].to_string(),
            body: net.to_string(),
        },
        ApiCall {
            method: "PUT".to_string(),
            path: API_PATH_ORDER[4].to_string(),
            body: action.to_string(),
        },
    ]
}

#[derive(Serialize)]
struct MachineConfigBody {
    vcpu_count: u32,
    mem_size_mib: u64,
    smt: bool,
}

pub fn resolve_kernel(
    getenv: impl Fn(&str) -> Option<String>,
    launch_kernel: Option<&str>,
    cache_dir: Option<&Path>,
    lab_tier: LabTier,
    rootfs: Option<&Path>,
) -> Result<PathBuf, FirecrackerError> {
    if let Some(raw) = getenv(KERNEL_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(raw);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(FirecrackerError::KernelMissing)
        };
    }
    if let Some(path) = launch_kernel.map(PathBuf::from).filter(|path| path.is_file()) {
        return Ok(path);
    }
    if let Some(dir) = cache_dir {
        let cached = cached_kernel_path(dir, lab_tier);
        if cached.is_file() {
            return Ok(cached);
        }
    }
    if let Some(rootfs) = rootfs {
        let sibling = rootfs.with_file_name("vmlinux");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Err(FirecrackerError::KernelMissing)
}

/// Start Firecracker from a phase-21 launch document. Never used for Pro.
pub fn boot_from_launch(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    getenv: impl Fn(&str) -> Option<String>,
    cache_dir: &Path,
    arch: &str,
    runtime: &dyn FirecrackerRuntime,
) -> Result<InvokeResult, FirecrackerError> {
    if os != HostOs::Linux {
        return Err(FirecrackerError::LinuxOnly(os));
    }
    if lab_tier != LabTier::Standard {
        return Err(FirecrackerError::StandardOnly(lab_tier.as_str()));
    }
    let rootfs = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(FirecrackerError::RootfsMissing)?;
    let kernel = resolve_kernel(
        getenv,
        launch.kernel_path.as_deref(),
        Some(cache_dir),
        lab_tier,
        Some(rootfs),
    )?;
    let api_sock = api_sock_path(cache_dir);
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        kernel: kernel.as_path(),
        rootfs,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        api_sock: api_sock.as_path(),
        arch,
        bootstrap_origin: None,
        bootstrap_token: None,
    })
}

/// Same as [`boot_from_launch`], with a guest bootstrap token on the kernel
/// cmdline (Matchmaker origin + token; never Headscale preauth or S3 keys).
pub fn boot_from_launch_with_bootstrap(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    getenv: impl Fn(&str) -> Option<String>,
    cache_dir: &Path,
    arch: &str,
    bootstrap_origin: Option<&str>,
    bootstrap_token: Option<&str>,
    runtime: &dyn FirecrackerRuntime,
) -> Result<InvokeResult, FirecrackerError> {
    if os != HostOs::Linux {
        return Err(FirecrackerError::LinuxOnly(os));
    }
    if lab_tier != LabTier::Standard {
        return Err(FirecrackerError::StandardOnly(lab_tier.as_str()));
    }
    let rootfs = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(FirecrackerError::RootfsMissing)?;
    let kernel = resolve_kernel(
        getenv,
        launch.kernel_path.as_deref(),
        Some(cache_dir),
        lab_tier,
        Some(rootfs),
    )?;
    let api_sock = api_sock_path(cache_dir);
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        kernel: kernel.as_path(),
        rootfs,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        api_sock: api_sock.as_path(),
        arch,
        bootstrap_origin,
        bootstrap_token,
    })
}

pub fn invoke(
    request: &InvokeRequest<'_>,
    spawner: &dyn FirecrackerSpawner,
    api: &dyn FirecrackerApi,
) -> Result<InvokeResult, FirecrackerError> {
    invoke_with_tap(request, spawner, api, &guest_net::NoopTap)
}

pub fn invoke_with_tap(
    request: &InvokeRequest<'_>,
    spawner: &dyn FirecrackerSpawner,
    api: &dyn FirecrackerApi,
    tap: &dyn TapOps,
) -> Result<InvokeResult, FirecrackerError> {
    if request.os != HostOs::Linux {
        return Err(FirecrackerError::LinuxOnly(request.os));
    }
    if request.lab_tier != LabTier::Standard {
        return Err(FirecrackerError::StandardOnly(request.lab_tier.as_str()));
    }
    if !request.capabilities.standard.offered {
        return Err(FirecrackerError::StandardNotOffered {
            reason: request.capabilities.standard.reason.clone(),
        });
    }
    require_file(request.binary, FirecrackerError::BinaryMissing)?;
    require_file(request.kernel, FirecrackerError::KernelMissing)?;
    require_file(request.rootfs, FirecrackerError::RootfsMissing)?;
    if request.vcpu_count == 0 || request.vcpu_count > MAX_VCPU {
        return Err(FirecrackerError::InvalidVcpu(request.vcpu_count));
    }

    tap.ensure(TAP_DEV, HOST_IP, TAP_PREFIX)
        .map_err(FirecrackerError::Tap)?;

    if let Some(parent) = request.api_sock.parent() {
        fs::create_dir_all(parent).map_err(|error| FirecrackerError::Socket {
            path: request.api_sock.display().to_string(),
            detail: error.to_string(),
        })?;
    }
    let _ = fs::remove_file(request.api_sock);

    let args = argv(request.api_sock);
    let mut child = spawner.spawn(request.binary, &args)?;
    let calls = api_calls(request);
    let started = configure_and_start(request.api_sock, &calls, api);
    match started {
        Ok(()) => {
            spawner.detach(child);
            Ok(InvokeResult {
                hypervisor_invoked: true,
                api_sock: request.api_sock.to_path_buf(),
                argv: args,
                api_calls: calls,
            })
        }
        Err(error) => {
            child.kill();
            let _ = tap.teardown(TAP_DEV);
            Err(error)
        }
    }
}

fn configure_and_start(
    socket: &Path,
    calls: &[ApiCall],
    api: &dyn FirecrackerApi,
) -> Result<(), FirecrackerError> {
    api.wait_ready(socket)?;
    for call in calls {
        let status = api.put(socket, &call.path, &call.body)?;
        if status != API_SUCCESS_STATUS {
            return Err(FirecrackerError::ApiRejected {
                path: call.path.clone(),
                status,
            });
        }
    }
    Ok(())
}

fn require_file(path: &Path, missing: FirecrackerError) -> Result<(), FirecrackerError> {
    if path.as_os_str().is_empty() || !path.is_file() {
        Err(missing)
    } else {
        Ok(())
    }
}

fn wait_for_unix_socket(socket: &Path) -> Result<(), FirecrackerError> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let deadline = Instant::now() + SOCKET_WAIT;
        loop {
            if UnixStream::connect(socket).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(FirecrackerError::Socket {
                    path: socket.display().to_string(),
                    detail: "timed out waiting for --api-sock".to_string(),
                });
            }
            std::thread::sleep(SOCKET_POLL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
        Err(FirecrackerError::LinuxOnly(HostOs::Windows))
    }
}

fn unix_http_put(socket: &Path, path: &str, body: &str) -> Result<u16, FirecrackerError> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(socket).map_err(|error| FirecrackerError::ApiIo {
            path: path.to_string(),
            detail: error.to_string(),
        })?;
        stream
            .set_read_timeout(Some(API_IO_TIMEOUT))
            .map_err(|error| FirecrackerError::ApiIo {
                path: path.to_string(),
                detail: error.to_string(),
            })?;
        stream
            .set_write_timeout(Some(API_IO_TIMEOUT))
            .map_err(|error| FirecrackerError::ApiIo {
                path: path.to_string(),
                detail: error.to_string(),
            })?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let request = format!(
            "PUT {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| FirecrackerError::ApiIo {
                path: path.clone(),
                detail: error.to_string(),
            })?;
        let _ = stream.flush();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if headers_complete(&buf) {
                        break;
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(error) => {
                    return Err(FirecrackerError::ApiIo {
                        path,
                        detail: error.to_string(),
                    });
                }
            }
        }
        parse_http_status(&buf, &path)
    }
    #[cfg(not(unix))]
    {
        let _ = (socket, path, body);
        Err(FirecrackerError::LinuxOnly(HostOs::Windows))
    }
}

fn headers_complete(buf: &[u8]) -> bool {
    buf.windows(4).any(|window| window == b"\r\n\r\n")
}

fn parse_http_status(buf: &[u8], path: &str) -> Result<u16, FirecrackerError> {
    let text = String::from_utf8_lossy(buf);
    let line = text.lines().next().unwrap_or("");
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|token| token.parse::<u16>().ok())
        .ok_or_else(|| FirecrackerError::ApiIo {
            path: path.to_string(),
            detail: format!("unreadable HTTP status: {line:?}"),
        })?;
    Ok(status)
}

#[cfg(test)]
pub(crate) struct RecordingFirecracker {
    pub spawns: Mutex<Vec<(PathBuf, Vec<String>)>>,
    pub puts: Mutex<Vec<(String, String)>>,
    pub killed: Mutex<u32>,
    put_status: u16,
}

#[cfg(test)]
impl RecordingFirecracker {
    pub(crate) fn succeeding() -> Self {
        Self {
            spawns: Mutex::new(Vec::new()),
            puts: Mutex::new(Vec::new()),
            killed: Mutex::new(0),
            put_status: API_SUCCESS_STATUS,
        }
    }

    pub(crate) fn rejecting(status: u16) -> Self {
        Self {
            spawns: Mutex::new(Vec::new()),
            puts: Mutex::new(Vec::new()),
            killed: Mutex::new(0),
            put_status: status,
        }
    }

    pub(crate) fn spawn_calls(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.spawns.lock().expect("spawn mutex").clone()
    }

    pub(crate) fn put_calls(&self) -> Vec<(String, String)> {
        self.puts.lock().expect("put mutex").clone()
    }
}

#[cfg(test)]
struct RecordingChild {
    killed: std::sync::Arc<Mutex<u32>>,
}

#[cfg(test)]
impl FirecrackerChild for RecordingChild {
    fn kill(&mut self) {
        *self.killed.lock().expect("killed mutex") += 1;
    }
}

#[cfg(test)]
impl FirecrackerSpawner for RecordingFirecracker {
    fn spawn(
        &self,
        binary: &Path,
        args: &[String],
    ) -> Result<Box<dyn FirecrackerChild>, FirecrackerError> {
        self.spawns
            .lock()
            .expect("spawn mutex")
            .push((binary.to_path_buf(), args.to_vec()));
        Ok(Box::new(RecordingChild {
            killed: std::sync::Arc::new(Mutex::new(0)),
        }))
    }

    fn detach(&self, _child: Box<dyn FirecrackerChild>) {}
}

#[cfg(test)]
impl FirecrackerApi for RecordingFirecracker {
    fn wait_ready(&self, _socket: &Path) -> Result<(), FirecrackerError> {
        Ok(())
    }

    fn put(&self, _socket: &Path, path: &str, body: &str) -> Result<u16, FirecrackerError> {
        self.puts
            .lock()
            .expect("put mutex")
            .push((path.to_string(), body.to_string()));
        Ok(self.put_status)
    }
}

#[cfg(test)]
impl FirecrackerRuntime for RecordingFirecracker {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, FirecrackerError> {
        invoke(request, self, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::provision::ComputeBackend;
    use std::sync::Arc;

    fn linux_standard_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        })
    }

    fn linux_no_kvm_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        })
    }

    fn fixture_tree() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("bin").join("firecracker");
        let kernel = dir.path().join("vmlinux");
        let rootfs = dir.path().join("rootfs.img");
        let sock = dir.path().join("run").join("api.sock");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"fake-firecracker").unwrap();
        fs::write(&kernel, b"fake-vmlinux").unwrap();
        fs::write(&rootfs, b"fake-rootfs").unwrap();
        (dir, binary, kernel, rootfs, sock)
    }

    fn request<'a>(
        caps: &'a HostCapabilities,
        binary: &'a Path,
        kernel: &'a Path,
        rootfs: &'a Path,
        sock: &'a Path,
    ) -> InvokeRequest<'a> {
        InvokeRequest {
            os: HostOs::Linux,
            lab_tier: LabTier::Standard,
            capabilities: caps,
            binary,
            kernel,
            rootfs,
            vcpu_count: 4,
            memory_mib: 8192,
            api_sock: sock,
            arch: "x86_64",
            bootstrap_origin: None,
            bootstrap_token: None,
        }
    }

    #[test]
    fn linux_standard_invoke_locks_argv_and_api_order() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let result = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
        )
        .unwrap();

        assert!(result.hypervisor_invoked);
        let spawns = recorder.spawn_calls();
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].0, binary);
        assert_eq!(
            spawns[0].1,
            vec![
                API_SOCK_ARG.to_string(),
                sock.to_string_lossy().into_owned(),
                ENABLE_PCI_ARG.to_string()
            ]
        );
        assert_eq!(result.argv, spawns[0].1);

        let puts = recorder.put_calls();
        let paths: Vec<&str> = puts.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, API_PATH_ORDER);
        assert_eq!(
            paths,
            [
                "/boot-source",
                "/drives/rootfs",
                "/machine-config",
                "/network-interfaces/eth0",
                "/actions"
            ]
        );

        let boot: serde_json::Value = serde_json::from_str(&puts[0].1).unwrap();
        assert_eq!(boot["kernel_image_path"], kernel.to_string_lossy().as_ref());
        assert_eq!(boot["boot_args"], boot_args("x86_64"));
        assert!(boot["boot_args"].as_str().unwrap().contains(guest_net::GUEST_IP));

        let drive: serde_json::Value = serde_json::from_str(&puts[1].1).unwrap();
        assert_eq!(drive["drive_id"], ROOTFS_DRIVE_ID);
        assert_eq!(drive["path_on_host"], rootfs.to_string_lossy().as_ref());
        assert_eq!(drive["is_root_device"], true);
        assert_eq!(drive["is_read_only"], false);

        let machine: serde_json::Value = serde_json::from_str(&puts[2].1).unwrap();
        assert_eq!(machine["vcpu_count"], 4);
        assert_eq!(machine["mem_size_mib"], 8192);
        assert_eq!(machine["smt"], false);

        let net: serde_json::Value = serde_json::from_str(&puts[3].1).unwrap();
        assert_eq!(net["iface_id"], NETWORK_IFACE_ID);
        assert_eq!(net["guest_mac"], GUEST_MAC);
        assert_eq!(net["host_dev_name"], TAP_DEV);

        let action: serde_json::Value = serde_json::from_str(&puts[4].1).unwrap();
        assert_eq!(action["action_type"], INSTANCE_START);
        assert_eq!(*recorder.killed.lock().unwrap(), 0);
    }

    #[test]
    fn tap_is_ensured_before_instance_start() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let tap = guest_net::RecordingTap::default();
        invoke_with_tap(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
            &tap,
        )
        .unwrap();
        assert_eq!(
            tap.ensure_calls(),
            vec![(
                TAP_DEV.to_string(),
                HOST_IP.to_string(),
                TAP_PREFIX
            )]
        );
    }

    #[test]
    fn aarch64_boot_args_prefix_keep_bootcon() {
        assert!(boot_args("aarch64").starts_with("keep_bootcon "));
        assert!(boot_args("aarch64").contains(BOOT_ARGS_X86));
        assert!(boot_args("x86_64").contains(BOOT_ARGS_X86));
        assert!(boot_args("x86_64").contains(guest_net::GUEST_IP));
    }

    #[test]
    fn missing_kernel_fails_before_spawn() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        fs::remove_file(&kernel).unwrap();
        let recorder = RecordingFirecracker::succeeding();
        let error = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
        )
        .unwrap_err();

        assert_eq!(error, FirecrackerError::KernelMissing);
        assert!(error.to_string().contains(KERNEL_ENV));
        assert!(error.to_string().contains("Packer"));
        assert!(recorder.spawn_calls().is_empty());
        assert!(recorder.put_calls().is_empty());
    }

    #[test]
    fn missing_binary_fails_before_spawn() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        fs::remove_file(&binary).unwrap();
        let recorder = RecordingFirecracker::succeeding();
        let error = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
        )
        .unwrap_err();
        assert_eq!(error, FirecrackerError::BinaryMissing);
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn missing_rootfs_fails_before_spawn() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        fs::remove_file(&rootfs).unwrap();
        let recorder = RecordingFirecracker::succeeding();
        let error = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
        )
        .unwrap_err();
        assert_eq!(error, FirecrackerError::RootfsMissing);
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn macos_does_not_spawn_firecracker() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let mut req = request(&caps, &binary, &kernel, &rootfs, &sock);
        req.os = HostOs::MacOs;
        let error = invoke(&req, &recorder, &recorder).unwrap_err();
        assert_eq!(error, FirecrackerError::LinuxOnly(HostOs::MacOs));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn windows_does_not_spawn_firecracker() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let mut req = request(&caps, &binary, &kernel, &rootfs, &sock);
        req.os = HostOs::Windows;
        assert_eq!(
            invoke(&req, &recorder, &recorder).unwrap_err(),
            FirecrackerError::LinuxOnly(HostOs::Windows)
        );
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn pro_does_not_use_firecracker() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let mut req = request(&caps, &binary, &kernel, &rootfs, &sock);
        req.lab_tier = LabTier::Pro;
        assert_eq!(
            invoke(&req, &recorder, &recorder).unwrap_err(),
            FirecrackerError::StandardOnly("pro")
        );
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn probe_without_kvm_does_not_spawn() {
        let caps = linux_no_kvm_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::succeeding();
        let error = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &recorder,
            &recorder,
        )
        .unwrap_err();
        assert!(matches!(error, FirecrackerError::StandardNotOffered { .. }));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn api_rejection_kills_the_child_and_is_not_a_successful_boot() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, sock) = fixture_tree();
        let recorder = RecordingFirecracker::rejecting(400);
        let killed = Arc::new(Mutex::new(0u32));
        struct KillCounter {
            inner: RecordingFirecracker,
            killed: Arc<Mutex<u32>>,
        }
        impl FirecrackerSpawner for KillCounter {
            fn spawn(
                &self,
                binary: &Path,
                args: &[String],
            ) -> Result<Box<dyn FirecrackerChild>, FirecrackerError> {
                self.inner
                    .spawns
                    .lock()
                    .unwrap()
                    .push((binary.to_path_buf(), args.to_vec()));
                Ok(Box::new(RecordingChild {
                    killed: self.killed.clone(),
                }))
            }
            fn detach(&self, _child: Box<dyn FirecrackerChild>) {}
        }
        impl FirecrackerApi for KillCounter {
            fn wait_ready(&self, socket: &Path) -> Result<(), FirecrackerError> {
                self.inner.wait_ready(socket)
            }
            fn put(&self, socket: &Path, path: &str, body: &str) -> Result<u16, FirecrackerError> {
                self.inner.put(socket, path, body)
            }
        }
        let runtime = KillCounter {
            inner: recorder,
            killed: killed.clone(),
        };
        let error = invoke(
            &request(&caps, &binary, &kernel, &rootfs, &sock),
            &runtime,
            &runtime,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FirecrackerError::ApiRejected { status: 400, .. }
        ));
        assert_eq!(*killed.lock().unwrap(), 1);
        assert!(!runtime.inner.spawn_calls().is_empty());
    }

    #[test]
    fn firecracker_kernel_env_wins_when_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let env_kernel = dir.path().join("from-env");
        fs::write(&env_kernel, b"env-vmlinux").unwrap();
        let launch = dir.path().join("launch-vmlinux");
        fs::write(&launch, b"launch-vmlinux").unwrap();
        let path = resolve_kernel(
            |_| Some(env_kernel.to_string_lossy().into_owned()),
            Some(launch.to_str().unwrap()),
            None,
            LabTier::Standard,
            None,
        )
        .unwrap();
        assert_eq!(path, env_kernel);
    }

    #[test]
    fn firecracker_kernel_env_set_but_missing_fails_clearly() {
        let error = resolve_kernel(
            |_| Some("/tmp/dcl-missing-vmlinux-phase27".to_string()),
            None,
            None,
            LabTier::Standard,
            None,
        )
        .unwrap_err();
        assert_eq!(error, FirecrackerError::KernelMissing);
    }

    #[test]
    fn sibling_vmlinux_is_used_when_env_and_cache_are_absent() {
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs.img");
        let kernel = dir.path().join("vmlinux");
        fs::write(&rootfs, b"rootfs").unwrap();
        fs::write(&kernel, b"vmlinux").unwrap();
        let path = resolve_kernel(
            |_| None,
            None,
            None,
            LabTier::Standard,
            Some(rootfs.as_path()),
        )
        .unwrap();
        assert_eq!(path, kernel);
    }

    #[test]
    fn production_spawner_is_refused_under_cfg_test() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("firecracker");
        fs::write(&binary, b"fake").unwrap();
        let error = match ProcessSpawner.spawn(&binary, &argv(&dir.path().join("api.sock"))) {
            Err(error) => error,
            Ok(_) => panic!("production spawner must refuse under cfg(test)"),
        };
        assert_eq!(error, FirecrackerError::RefusedInTests);
    }

    #[test]
    fn boot_from_launch_uses_slider_vcpu_and_rootfs() {
        let caps = linux_standard_caps();
        let (_dir, binary, kernel, rootfs, _sock) = fixture_tree();
        let cache = tempfile::tempdir().unwrap();
        let recorder = RecordingFirecracker::succeeding();
        let launch = LaunchConfig {
            backend: ComputeBackend::Firecracker,
            lab_tier: LabTier::Standard,
            vcpu_count: 2,
            memory_mib: 4096,
            kernel_path: Some(kernel.to_string_lossy().into_owned()),
            rootfs_path: Some(rootfs.to_string_lossy().into_owned()),
            gpu: None,
            hypervisor_invoked: false,
        };
        let result = boot_from_launch(
            HostOs::Linux,
            LabTier::Standard,
            &caps,
            &launch,
            &binary,
            |_| None,
            cache.path(),
            "x86_64",
            &recorder,
        )
        .unwrap();
        assert!(result.hypervisor_invoked);
        let puts = recorder.put_calls();
        let machine: serde_json::Value = serde_json::from_str(&puts[2].1).unwrap();
        assert_eq!(machine["vcpu_count"], 2);
        assert_eq!(machine["mem_size_mib"], 4096);
        assert_eq!(
            recorder.spawn_calls()[0].1[1],
            api_sock_path(cache.path()).to_string_lossy()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_http_put_talks_http_over_a_fake_socket() {
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("api.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let status = unix_http_put(&sock, "/boot-source", "{\"kernel_image_path\":\"x\"}").unwrap();
        assert_eq!(status, 204);
        server.join().unwrap();
    }
}
