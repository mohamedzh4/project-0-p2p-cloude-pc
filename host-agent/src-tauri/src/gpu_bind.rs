//! Pro GPU assignment with rollback (phase 22).
//!
//! Linux: plan and optionally apply a vfio-pci bind. Sysfs and driver control
//! paths are injectable so tests never touch real devices. On apply failure the
//! previous host driver is restored.
//!
//! Windows: Hyper-V DDA / GPU-P is a plan/log path only. This module never
//! spawns PowerShell (live assignment is leftover 22b) so a missing hypervisor
//! cannot hang the agent.
//!
//! Firecracker has no PCI passthrough — Pro never uses it. The Host Agent
//! computes the PCI address from sysfs; the volunteer does not run `lspci`.
//! Nothing here invokes a hypervisor. GATE-PRO stays open.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::host_capabilities::{HostCapabilities, IommuStatus};
use crate::provision::{
    ComputeBackend, HostOs, LabTier, ProvisionError, Provisioner, StubProvisioner,
};

pub const VFIO_PCI: &str = "vfio-pci";

/// Injectable sysfs / driver layout. Production uses [`PciSysfs::live`] (`/`).
/// Tests pass a temp tree so CI never needs root or a real GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciSysfs {
    pub root: PathBuf,
    pub pci_devices: PathBuf,
    pub drivers: PathBuf,
    pub vfio_pci: PathBuf,
    pub nvidia_proc: PathBuf,
}

impl PciSysfs {
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let drivers = root.join("sys/bus/pci/drivers");
        Self {
            pci_devices: root.join("sys/bus/pci/devices"),
            vfio_pci: drivers.join(VFIO_PCI),
            nvidia_proc: root.join("proc/driver/nvidia/gpus"),
            drivers,
            root,
        }
    }

    pub fn live() -> Self {
        Self::from_root("/")
    }

    pub fn device_dir(&self, bdf: &str) -> PathBuf {
        self.pci_devices.join(bdf)
    }

    pub fn driver_dir(&self, name: &str) -> PathBuf {
        if name == VFIO_PCI {
            self.vfio_pci.clone()
        } else {
            self.drivers.join(name)
        }
    }
}

/// Writes to injectable bind / unbind / new_id files.
pub trait PciBindOps {
    fn write_unbind(&self, unbind_path: &Path, bdf: &str) -> Result<(), GpuBindError>;
    fn write_bind(&self, bind_path: &Path, bdf: &str) -> Result<(), GpuBindError>;
    fn write_new_id(
        &self,
        new_id_path: &Path,
        vendor: &str,
        device: &str,
    ) -> Result<(), GpuBindError>;
}

/// Default ops: append to the given sysfs control files. Tests pass a temp
/// tree; production would pass `/sys/bus/pci/drivers/.../{bind,unbind,new_id}`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SysfsBindOps;

impl PciBindOps for SysfsBindOps {
    fn write_unbind(&self, unbind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
        append_sysfs(unbind_path, bdf)
    }

    fn write_bind(&self, bind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
        append_sysfs(bind_path, bdf)
    }

    fn write_new_id(
        &self,
        new_id_path: &Path,
        vendor: &str,
        device: &str,
    ) -> Result<(), GpuBindError> {
        append_sysfs(new_id_path, &format!("{vendor} {device}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindRequest<'a> {
    pub os: HostOs,
    pub capabilities: &'a HostCapabilities,
    pub share_gpu: bool,
    pub shared_gpu_id: Option<&'a str>,
    pub sysfs: &'a PciSysfs,
    /// When true, Linux writes the planned bind (and rolls back on failure).
    /// Windows still does not apply (22b). Always false from the Host Agent UI.
    pub apply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuBindAction {
    Planned,
    Bound,
    WouldAssign,
    Skipped,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindStep {
    pub action: String,
    pub driver: String,
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuBindPlan {
    #[serde(serialize_with = "serialize_backend")]
    pub backend: ComputeBackend,
    pub gpu_id: String,
    pub pci_address: Option<String>,
    pub previous_driver: Option<String>,
    pub target_driver: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub steps: Vec<BindStep>,
    pub exclusive: bool,
    pub exclusivity_warning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuBindStatus {
    pub action: GpuBindAction,
    pub backend: Option<String>,
    pub gpu_id: Option<String>,
    pub pci_address: Option<String>,
    pub previous_driver: Option<String>,
    pub target_driver: Option<String>,
    pub steps: Vec<BindStep>,
    pub bound: bool,
    pub rolled_back: bool,
    pub exclusive: bool,
    pub log: String,
    pub hypervisor_invoked: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GpuBindError {
    #[error("{0}")]
    Provision(#[from] ProvisionError),
    #[error("{reason}")]
    NotEligible { reason: String },
    #[error(
        "Pro sessions are not available: this PC needs a second display path (an iGPU or another GPU) so you can keep using it while a guest has the dedicated GPU."
    )]
    NoSecondDisplayPath,
    #[error(
        "Pro sessions are not available: this Linux PC does not report IOMMU, which is needed to assign a GPU to a guest."
    )]
    IommuMissing,
    #[error(
        "a Pro GPU bind needs you to dedicate a GPU (shareGpu); that GPU is exclusive for the session, not a concurrent slice"
    )]
    GpuNotDedicated,
    #[error(
        "a Pro session needs a dedicated GPU id (sharedGpuId); GPU assignment is exclusive for the session, not a concurrent slice"
    )]
    GpuIdRequired,
    #[error(
        "could not find PCI address for GPU {gpu_id} in sysfs; the agent computes this itself (no lspci)"
    )]
    GpuNotFound { gpu_id: String },
    #[error("GPU {gpu_id} matched more than one PCI device: {devices:?}")]
    AmbiguousGpu {
        gpu_id: String,
        devices: Vec<String>,
    },
    #[error(
        "GPU {pci_address} is not in an IOMMU group; not binding so this PC keeps its display path"
    )]
    DeviceNotInIommuGroup { pci_address: String },
    #[error("the {driver} driver control files are missing; not binding the GPU")]
    DriverControlMissing { driver: String },
    #[error("Firecracker has no PCI passthrough; Pro GPU assignment uses QEMU+VFIO or Hyper-V")]
    FirecrackerHasNoPassthrough,
    #[error("tests must not bind live sysfs; pass a mock tree")]
    LiveSysfsForbiddenInTests,
    #[error("{0}")]
    ApplyFailed(ApplyFailure),
    #[error("could not update GPU bind files: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyFailure {
    pub detail: String,
    pub rolled_back: bool,
    pub previous_driver: Option<String>,
}

impl std::fmt::Display for ApplyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.rolled_back {
            write!(
                f,
                "{}; the previous driver was restored so this PC keeps its display path",
                self.detail
            )
        } else {
            write!(
                f,
                "{}; could not restore the previous GPU driver",
                self.detail
            )
        }
    }
}

fn serialize_backend<S: serde::Serializer>(
    backend: &ComputeBackend,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(backend.slug())
}

/// UI entry: never panics; ineligible probes become a refused status.
pub fn report(request: BindRequest<'_>, ops: &dyn PciBindOps) -> GpuBindStatus {
    match assign(request, ops) {
        Ok(status) => status,
        Err(error) => GpuBindStatus::refused(&error),
    }
}

/// Plan, and optionally apply, a Pro GPU assignment. Standard is a skip.
/// Does not invoke a hypervisor.
pub fn assign(
    request: BindRequest<'_>,
    ops: &dyn PciBindOps,
) -> Result<GpuBindStatus, GpuBindError> {
    if request.apply {
        refuse_live_sysfs_in_tests(request.sysfs)?;
    }

    match StubProvisioner::select(request.os, LabTier::Pro) {
        Ok(selected) => {
            if selected.backend() == ComputeBackend::Firecracker {
                return Err(GpuBindError::FirecrackerHasNoPassthrough);
            }
        }
        Err(ProvisionError::UnsupportedHostOs(os)) => {
            return Ok(GpuBindStatus::skipped(
                format!(
                    "hosting is not supported on {os}; this agent does not fall back to Firecracker for GPU assignment"
                ),
            ));
        }
        Err(error) => return Err(error.into()),
    }

    let gpu_id = eligible_gpu_id(
        request.os,
        request.capabilities,
        request.share_gpu,
        request.shared_gpu_id,
    )?;

    match request.os {
        HostOs::Linux => assign_linux(&gpu_id, request, ops),
        HostOs::Windows => Ok(assign_windows(&gpu_id, request.capabilities)),
        HostOs::MacOs | HostOs::Other => Ok(GpuBindStatus::skipped(format!(
            "hosting is not supported on {}; this agent does not fall back to Firecracker for GPU assignment",
            request.os
        ))),
    }
}

/// Plan only (no sysfs writes). Used by tests and the apply path.
pub fn plan_linux_vfio(gpu_id: &str, sysfs: &PciSysfs) -> Result<GpuBindPlan, GpuBindError> {
    let bdf = resolve_pci_address(gpu_id, sysfs)?;
    let device_dir = sysfs.device_dir(&bdf);
    if !device_dir.join("iommu_group").exists() {
        return Err(GpuBindError::DeviceNotInIommuGroup { pci_address: bdf });
    }

    let previous_driver = read_current_driver(&device_dir);
    let (vendor_id, device_id) = read_pci_ids(&device_dir)?;
    let warning = exclusivity_warning(gpu_id);

    let mut steps = Vec::new();
    steps.push(BindStep {
        action: "driverOverride".to_string(),
        driver: VFIO_PCI.to_string(),
        device: bdf.clone(),
    });
    if let Some(driver) = previous_driver.as_deref() {
        if driver != VFIO_PCI {
            steps.push(BindStep {
                action: "unbind".to_string(),
                driver: driver.to_string(),
                device: bdf.clone(),
            });
        }
    }
    steps.push(BindStep {
        action: "newId".to_string(),
        driver: VFIO_PCI.to_string(),
        device: format!("{vendor_id} {device_id}"),
    });
    if previous_driver.as_deref() != Some(VFIO_PCI) {
        steps.push(BindStep {
            action: "bind".to_string(),
            driver: VFIO_PCI.to_string(),
            device: bdf.clone(),
        });
    }

    Ok(GpuBindPlan {
        backend: ComputeBackend::QemuVfio,
        gpu_id: gpu_id.to_string(),
        pci_address: Some(bdf),
        previous_driver,
        target_driver: VFIO_PCI.to_string(),
        vendor_id: Some(vendor_id),
        device_id: Some(device_id),
        steps,
        exclusive: true,
        exclusivity_warning: warning,
    })
}

/// Apply a Linux VFIO plan against injectable driver paths. Rolls back to the
/// previous driver if bind fails. Never a partial bind on ineligible plans —
/// callers must run eligibility first.
pub fn apply_linux_vfio(
    plan: &GpuBindPlan,
    sysfs: &PciSysfs,
    ops: &dyn PciBindOps,
) -> Result<GpuBindStatus, GpuBindError> {
    refuse_live_sysfs_in_tests(sysfs)?;
    if plan.backend == ComputeBackend::Firecracker {
        return Err(GpuBindError::FirecrackerHasNoPassthrough);
    }

    let bdf = plan
        .pci_address
        .as_deref()
        .ok_or_else(|| GpuBindError::GpuNotFound {
            gpu_id: plan.gpu_id.clone(),
        })?;
    let previous = plan.previous_driver.as_deref();

    if previous == Some(VFIO_PCI) {
        return Ok(status_from_plan(
            plan,
            GpuBindAction::Bound,
            true,
            false,
            format!(
                "GPU {} is already bound to vfio-pci at {bdf}. Exclusive for the session. Not a hypervisor boot.",
                plan.gpu_id
            ),
        ));
    }

    require_driver_controls(sysfs, VFIO_PCI)?;
    if let Some(driver) = previous {
        require_driver_controls(sysfs, driver)?;
    }

    let device_dir = sysfs.device_dir(bdf);
    let override_path = device_dir.join("driver_override");

    if let Err(error) = write_trimmed(&override_path, VFIO_PCI) {
        return Err(apply_failed(error.to_string(), false, previous));
    }

    if let Some(driver) = previous {
        let unbind = sysfs.driver_dir(driver).join("unbind");
        if let Err(error) = ops.write_unbind(&unbind, bdf) {
            let _ = write_trimmed(&override_path, "");
            restore_driver_link(&device_dir, &sysfs.driver_dir(driver));
            return Err(apply_failed(error.to_string(), true, previous));
        }
        clear_driver_link(&device_dir);
    }

    if let (Some(vendor), Some(device)) = (&plan.vendor_id, &plan.device_id) {
        let new_id = sysfs.vfio_pci.join("new_id");
        if let Err(error) = ops.write_new_id(&new_id, vendor, device) {
            rollback_linux(sysfs, ops, bdf, previous, &device_dir)?;
            return Err(apply_failed(error.to_string(), true, previous));
        }
    }

    let bind = sysfs.vfio_pci.join("bind");
    if let Err(error) = ops.write_bind(&bind, bdf) {
        let rolled_back = rollback_linux(sysfs, ops, bdf, previous, &device_dir).is_ok();
        return Err(apply_failed(error.to_string(), rolled_back, previous));
    }

    restore_driver_link(&device_dir, &sysfs.vfio_pci);

    Ok(status_from_plan(
        plan,
        GpuBindAction::Bound,
        true,
        false,
        format!(
            "bound GPU {} ({bdf}) to vfio-pci; previous driver was {}. Exclusive for the session. Hypervisor not invoked.",
            plan.gpu_id,
            previous.unwrap_or("none")
        ),
    ))
}

/// Resolve `sharedGpuId` (NVML UUID, test id, or BDF) to a PCI address by
/// walking injectable sysfs. Never shells out to `lspci`.
pub fn resolve_pci_address(gpu_id: &str, sysfs: &PciSysfs) -> Result<String, GpuBindError> {
    let want = gpu_id.trim();
    if want.is_empty() {
        return Err(GpuBindError::GpuIdRequired);
    }

    if let Some(bdf) = parse_bdf(want) {
        if sysfs.device_dir(&bdf).is_dir() {
            return Ok(bdf);
        }
    }

    let entries = match fs::read_dir(&sysfs.pci_devices) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(GpuBindError::GpuNotFound {
                gpu_id: want.to_string(),
            });
        }
        Err(error) => {
            return Err(GpuBindError::Io(error.to_string()));
        }
    };

    let mut matches = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Some(bdf) = parse_bdf(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        if device_matches_gpu_id(&entry.path(), &bdf, want, sysfs) {
            matches.push(bdf);
        }
    }

    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(GpuBindError::GpuNotFound {
            gpu_id: want.to_string(),
        }),
        _ => Err(GpuBindError::AmbiguousGpu {
            gpu_id: want.to_string(),
            devices: matches,
        }),
    }
}

pub fn write_driver_unbind(unbind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
    SysfsBindOps.write_unbind(unbind_path, bdf)
}

pub fn write_driver_bind(bind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
    SysfsBindOps.write_bind(bind_path, bdf)
}

pub fn write_vfio_new_id(
    new_id_path: &Path,
    vendor: &str,
    device: &str,
) -> Result<(), GpuBindError> {
    SysfsBindOps.write_new_id(new_id_path, vendor, device)
}

impl GpuBindStatus {
    fn skipped(log: String) -> Self {
        Self {
            action: GpuBindAction::Skipped,
            backend: None,
            gpu_id: None,
            pci_address: None,
            previous_driver: None,
            target_driver: None,
            steps: Vec::new(),
            bound: false,
            rolled_back: false,
            exclusive: false,
            log,
            hypervisor_invoked: false,
        }
    }

    fn refused(error: &GpuBindError) -> Self {
        Self {
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
            log: error.to_string(),
            hypervisor_invoked: false,
        }
    }
}

fn assign_linux(
    gpu_id: &str,
    request: BindRequest<'_>,
    ops: &dyn PciBindOps,
) -> Result<GpuBindStatus, GpuBindError> {
    let plan = plan_linux_vfio(gpu_id, request.sysfs)?;
    debug_assert_ne!(plan.backend, ComputeBackend::Firecracker);

    if !request.apply {
        let bdf = plan.pci_address.as_deref().unwrap_or("unknown");
        let log = format!(
            "plan: unbind GPU {} from {} then bind vfio-pci at {bdf}. {}. Not applied. Hypervisor not invoked.",
            plan.gpu_id,
            plan.previous_driver.as_deref().unwrap_or("no host driver"),
            plan.exclusivity_warning
        );
        return Ok(status_from_plan(
            &plan,
            GpuBindAction::Planned,
            false,
            false,
            log,
        ));
    }

    apply_linux_vfio(&plan, request.sysfs, ops)
}

fn assign_windows(gpu_id: &str, capabilities: &HostCapabilities) -> GpuBindStatus {
    let warning = exclusivity_warning(gpu_id);
    GpuBindStatus {
        action: GpuBindAction::WouldAssign,
        backend: Some(ComputeBackend::HyperVGpu.slug().to_string()),
        gpu_id: Some(gpu_id.to_string()),
        pci_address: None,
        previous_driver: None,
        target_driver: Some("dda-gpu-p".to_string()),
        steps: vec![BindStep {
            action: "wouldAssign".to_string(),
            driver: "hyperv-gpu".to_string(),
            device: gpu_id.to_string(),
        }],
        bound: false,
        rolled_back: false,
        exclusive: true,
        log: format!(
            "would assign GPU {gpu_id} with Hyper-V DDA/GPU-P. {warning} Live PowerShell assignment is leftover 22b so this agent does not hang. Hypervisor not invoked. ({})",
            capabilities.pro.reason
        ),
        hypervisor_invoked: false,
    }
}

fn eligible_gpu_id(
    os: HostOs,
    capabilities: &HostCapabilities,
    share_gpu: bool,
    shared_gpu_id: Option<&str>,
) -> Result<String, GpuBindError> {
    if !capabilities.second_display_path {
        return Err(GpuBindError::NoSecondDisplayPath);
    }
    if os == HostOs::Linux && capabilities.iommu != IommuStatus::Present {
        return Err(GpuBindError::IommuMissing);
    }
    if !capabilities.pro.offered {
        return Err(GpuBindError::NotEligible {
            reason: capabilities.pro.reason.clone(),
        });
    }
    if !share_gpu {
        return Err(GpuBindError::GpuNotDedicated);
    }
    match shared_gpu_id.map(str::trim) {
        Some(id) if !id.is_empty() => Ok(id.to_string()),
        _ => Err(GpuBindError::GpuIdRequired),
    }
}

fn status_from_plan(
    plan: &GpuBindPlan,
    action: GpuBindAction,
    bound: bool,
    rolled_back: bool,
    log: String,
) -> GpuBindStatus {
    GpuBindStatus {
        action,
        backend: Some(plan.backend.slug().to_string()),
        gpu_id: Some(plan.gpu_id.clone()),
        pci_address: plan.pci_address.clone(),
        previous_driver: plan.previous_driver.clone(),
        target_driver: Some(plan.target_driver.clone()),
        steps: plan.steps.clone(),
        bound,
        rolled_back,
        exclusive: true,
        log,
        hypervisor_invoked: false,
    }
}

fn rollback_linux(
    sysfs: &PciSysfs,
    ops: &dyn PciBindOps,
    bdf: &str,
    previous: Option<&str>,
    device_dir: &Path,
) -> Result<(), GpuBindError> {
    let vfio_unbind = sysfs.vfio_pci.join("unbind");
    let _ = ops.write_unbind(&vfio_unbind, bdf);
    let _ = write_trimmed(&device_dir.join("driver_override"), "");
    clear_driver_link(device_dir);

    if let Some(driver) = previous {
        let bind = sysfs.driver_dir(driver).join("bind");
        ops.write_bind(&bind, bdf)?;
        restore_driver_link(device_dir, &sysfs.driver_dir(driver));
    }
    Ok(())
}

fn apply_failed(detail: String, rolled_back: bool, previous: Option<&str>) -> GpuBindError {
    GpuBindError::ApplyFailed(ApplyFailure {
        detail,
        rolled_back,
        previous_driver: previous.map(str::to_string),
    })
}

fn require_driver_controls(sysfs: &PciSysfs, driver: &str) -> Result<(), GpuBindError> {
    let dir = sysfs.driver_dir(driver);
    let bind = dir.join("bind");
    let unbind = dir.join("unbind");
    if !bind.is_file() || !unbind.is_file() {
        return Err(GpuBindError::DriverControlMissing {
            driver: driver.to_string(),
        });
    }
    Ok(())
}

fn refuse_live_sysfs_in_tests(sysfs: &PciSysfs) -> Result<(), GpuBindError> {
    #[cfg(test)]
    {
        let root = sysfs.root.as_os_str();
        if root.is_empty() || root == "/" {
            return Err(GpuBindError::LiveSysfsForbiddenInTests);
        }
        if sysfs.pci_devices == Path::new("/sys/bus/pci/devices") {
            return Err(GpuBindError::LiveSysfsForbiddenInTests);
        }
    }
    let _ = sysfs;
    Ok(())
}

fn device_matches_gpu_id(device_dir: &Path, bdf: &str, gpu_id: &str, sysfs: &PciSysfs) -> bool {
    let want = gpu_id.trim();
    for rel in ["dcl_gpu_id", "gpu_uuid", "nvidia/gpu_uuid"] {
        if ids_equal(
            &read_trimmed(&device_dir.join(rel)).unwrap_or_default(),
            want,
        ) {
            return true;
        }
    }
    let information = sysfs.nvidia_proc.join(bdf).join("information");
    if let Ok(text) = fs::read_to_string(information) {
        for line in text.lines() {
            if let Some(value) = line.trim().strip_prefix("GPU UUID:") {
                if ids_equal(value.trim(), want) {
                    return true;
                }
            }
        }
    }
    false
}

fn ids_equal(found: &str, want: &str) -> bool {
    !found.is_empty() && found.trim().eq_ignore_ascii_case(want.trim())
}

fn read_current_driver(device_dir: &Path) -> Option<String> {
    let link = device_dir.join("driver");
    if let Ok(target) = fs::read_link(&link) {
        return target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
    }
    read_trimmed(&link).filter(|name| !name.is_empty())
}

fn read_pci_ids(device_dir: &Path) -> Result<(String, String), GpuBindError> {
    Ok((
        read_hex_id(&device_dir.join("vendor"))?,
        read_hex_id(&device_dir.join("device"))?,
    ))
}

fn read_hex_id(path: &Path) -> Result<String, GpuBindError> {
    let raw = fs::read_to_string(path).map_err(|error| GpuBindError::Io(error.to_string()))?;
    let trimmed = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
    if trimmed.len() != 4 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(GpuBindError::Io(format!(
            "invalid PCI id in {}: {raw:?}",
            path.display()
        )));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn parse_bdf(raw: &str) -> Option<String> {
    let s = raw.trim();
    let segments: Vec<&str> = s.split(':').collect();
    match segments.as_slice() {
        [domain, bus, slot_fn] if domain.len() == 4 && bus.len() == 2 => {
            let (slot, func) = slot_fn.split_once('.')?;
            if slot.len() == 2
                && func.len() == 1
                && is_hex(domain)
                && is_hex(bus)
                && is_hex(slot)
                && is_hex(func)
            {
                Some(format!("{domain}:{bus}:{slot}.{func}").to_ascii_lowercase())
            } else {
                None
            }
        }
        [bus, slot_fn] if bus.len() == 2 => parse_bdf(&format!("0000:{bus}:{slot_fn}")),
        _ => None,
    }
}

fn is_hex(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn exclusivity_warning(gpu_id: &str) -> String {
    format!(
        "GPU {gpu_id} is assigned exclusively to the guest for this Pro session; \
         the host loses that GPU until the session ends. CPU and RAM remain a concurrent slice."
    )
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn append_sysfs(path: &Path, line: &str) -> Result<(), GpuBindError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| GpuBindError::Io(error.to_string()))?;
    writeln!(file, "{line}").map_err(|error| GpuBindError::Io(error.to_string()))
}

fn write_trimmed(path: &Path, value: &str) -> Result<(), GpuBindError> {
    fs::write(path, format!("{value}\n")).map_err(|error| GpuBindError::Io(error.to_string()))
}

fn clear_driver_link(device_dir: &Path) {
    let _ = fs::remove_file(device_dir.join("driver"));
}

fn restore_driver_link(device_dir: &Path, driver_dir: &Path) {
    let link = device_dir.join("driver");
    let _ = fs::remove_file(&link);
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(driver_dir, &link);
    }
    #[cfg(not(unix))]
    {
        let name = driver_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = fs::write(&link, format!("{name}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{evaluate, EvaluateInput, KvmAccess, WhpxCapability};
    use crate::models::GpuInfo;
    use crate::provision::Provisioner;
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    const GPU_ID: &str = "GPU-TEST-4070";
    const BDF: &str = "0000:01:00.0";

    struct FailVfioBind {
        inner: SysfsBindOps,
        binds: AtomicUsize,
    }

    impl FailVfioBind {
        fn new() -> Self {
            Self {
                inner: SysfsBindOps,
                binds: AtomicUsize::new(0),
            }
        }
    }

    impl PciBindOps for FailVfioBind {
        fn write_unbind(&self, unbind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
            self.inner.write_unbind(unbind_path, bdf)
        }

        fn write_bind(&self, bind_path: &Path, bdf: &str) -> Result<(), GpuBindError> {
            self.binds.fetch_add(1, Ordering::SeqCst);
            if bind_path
                .parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name == VFIO_PCI)
            {
                return Err(GpuBindError::Io("injected vfio-pci bind failure".into()));
            }
            self.inner.write_bind(bind_path, bdf)
        }

        fn write_new_id(
            &self,
            new_id_path: &Path,
            vendor: &str,
            device: &str,
        ) -> Result<(), GpuBindError> {
            self.inner.write_new_id(new_id_path, vendor, device)
        }
    }

    fn two_gpus() -> [GpuInfo; 2] {
        [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ]
    }

    fn linux_pro_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &two_gpus(),
        })
    }

    fn linux_no_iommu_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &two_gpus(),
        })
    }

    fn linux_one_gpu_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &[nvidia_gpu()],
        })
    }

    fn windows_pro_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Available,
            iommu: IommuStatus::Unknown,
            gpus: &two_gpus(),
        })
    }

    fn windows_no_display_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Available,
            iommu: IommuStatus::Unknown,
            gpus: &[nvidia_gpu()],
        })
    }

    fn macos_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::MacOs,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn mock_gpu_tree(with_iommu_group: bool) -> (TempDir, PciSysfs) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let device = root.join("sys/bus/pci/devices").join(BDF);
        fs::create_dir_all(&device).unwrap();
        fs::write(device.join("vendor"), b"0x10de\n").unwrap();
        fs::write(device.join("device"), b"0x2786\n").unwrap();
        fs::write(device.join("dcl_gpu_id"), format!("{GPU_ID}\n")).unwrap();
        if with_iommu_group {
            fs::create_dir_all(device.join("iommu_group")).unwrap();
            fs::create_dir_all(root.join("sys/kernel/iommu_groups/1")).unwrap();
        }

        let nvidia = root.join("sys/bus/pci/drivers/nvidia");
        let vfio = root.join("sys/bus/pci/drivers").join(VFIO_PCI);
        fs::create_dir_all(&nvidia).unwrap();
        fs::create_dir_all(&vfio).unwrap();
        fs::write(nvidia.join("unbind"), b"").unwrap();
        fs::write(nvidia.join("bind"), b"").unwrap();
        fs::write(vfio.join("unbind"), b"").unwrap();
        fs::write(vfio.join("bind"), b"").unwrap();
        fs::write(vfio.join("new_id"), b"").unwrap();
        restore_driver_link(&device, &nvidia);

        let nvidia_proc = root.join("proc/driver/nvidia/gpus").join(BDF);
        fs::create_dir_all(&nvidia_proc).unwrap();
        fs::write(
            nvidia_proc.join("information"),
            format!("Model:           NVIDIA GeForce RTX 4070\nGPU UUID:        {GPU_ID}\n"),
        )
        .unwrap();

        let sysfs = PciSysfs::from_root(root);
        (dir, sysfs)
    }

    fn request<'a>(
        os: HostOs,
        caps: &'a HostCapabilities,
        sysfs: &'a PciSysfs,
        share_gpu: bool,
        apply: bool,
    ) -> BindRequest<'a> {
        BindRequest {
            os,
            capabilities: caps,
            share_gpu,
            shared_gpu_id: Some(GPU_ID),
            sysfs,
            apply,
        }
    }

    fn control_lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn linux_plan_computes_vfio_from_sysfs_without_lspci() {
        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let status = assign(
            request(HostOs::Linux, &caps, &sysfs, true, false),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Planned);
        assert_eq!(status.backend.as_deref(), Some("qemu-vfio"));
        assert_ne!(status.backend.as_deref(), Some("firecracker"));
        assert_eq!(status.pci_address.as_deref(), Some(BDF));
        assert_eq!(status.previous_driver.as_deref(), Some("nvidia"));
        assert_eq!(status.target_driver.as_deref(), Some(VFIO_PCI));
        assert!(status.exclusive);
        assert!(!status.bound);
        assert!(!status.hypervisor_invoked);
        assert!(status.log.contains("exclusively"));
        assert!(status.log.contains("Not applied"));
        assert!(status.steps.iter().any(|step| step.action == "unbind"));
        assert!(status.steps.iter().any(|step| step.action == "bind"));
        assert!(
            control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty(),
            "plan must not unbind"
        );
    }

    #[test]
    fn linux_bind_success_unbinds_host_driver_then_binds_vfio() {
        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let status = assign(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Bound);
        assert!(status.bound);
        assert!(!status.rolled_back);
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some(VFIO_PCI)
        );
        assert_eq!(
            control_lines(&sysfs.driver_dir("nvidia").join("unbind")),
            vec![BDF]
        );
        assert_eq!(control_lines(&sysfs.vfio_pci.join("bind")), vec![BDF]);
        assert_eq!(
            control_lines(&sysfs.vfio_pci.join("new_id")),
            vec!["10de 2786"]
        );
        assert!(!status.hypervisor_invoked);
    }

    #[test]
    fn linux_bind_fail_rolls_back_to_previous_driver() {
        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let ops = FailVfioBind::new();
        let error = assign(request(HostOs::Linux, &caps, &sysfs, true, true), &ops).unwrap_err();

        match error {
            GpuBindError::ApplyFailed(ref failure) => {
                assert!(failure.rolled_back);
                assert_eq!(failure.previous_driver.as_deref(), Some("nvidia"));
                assert!(error.to_string().contains("restored"));
            }
            other => panic!("expected ApplyFailed, got {other:?}"),
        }
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some("nvidia")
        );
        assert!(
            control_lines(&sysfs.vfio_pci.join("bind")).is_empty(),
            "failed vfio bind must not leave a bind record"
        );
        assert_eq!(
            control_lines(&sysfs.driver_dir("nvidia").join("bind")),
            vec![BDF],
            "rollback must rebind nvidia"
        );
    }

    #[test]
    fn ineligible_linux_without_iommu_does_not_write() {
        let caps = linux_no_iommu_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let error = assign(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap_err();

        assert_eq!(error, GpuBindError::IommuMissing);
        assert!(error.to_string().contains("IOMMU"));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
        assert!(control_lines(&sysfs.vfio_pci.join("bind")).is_empty());
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some("nvidia")
        );
    }

    #[test]
    fn ineligible_without_second_display_does_not_write() {
        let caps = linux_one_gpu_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let error = assign(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap_err();

        assert_eq!(error, GpuBindError::NoSecondDisplayPath);
        assert!(error.to_string().contains("second display path"));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some("nvidia")
        );
    }

    #[test]
    fn missing_iommu_group_on_device_does_not_bind() {
        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(false);
        let error = assign(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap_err();

        assert!(matches!(error, GpuBindError::DeviceNotInIommuGroup { .. }));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some("nvidia")
        );
    }

    #[test]
    fn share_gpu_off_does_not_bind() {
        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let error = assign(
            request(HostOs::Linux, &caps, &sysfs, false, true),
            &SysfsBindOps,
        )
        .unwrap_err();

        assert_eq!(error, GpuBindError::GpuNotDedicated);
        assert!(error.to_string().contains("not a concurrent slice"));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
    }

    #[test]
    fn linux_pro_never_uses_firecracker() {
        let selected = StubProvisioner::select(HostOs::Linux, LabTier::Pro).unwrap();
        assert_eq!(selected.backend(), ComputeBackend::QemuVfio);
        assert_ne!(selected.backend(), ComputeBackend::Firecracker);

        let caps = linux_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let plan = plan_linux_vfio(GPU_ID, &sysfs).unwrap();
        assert_eq!(plan.backend, ComputeBackend::QemuVfio);
        let status = assign(
            request(HostOs::Linux, &caps, &sysfs, true, false),
            &SysfsBindOps,
        )
        .unwrap();
        assert_eq!(status.backend.as_deref(), Some("qemu-vfio"));
    }

    #[test]
    fn windows_would_assign_without_applying() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let planned = assign(
            request(HostOs::Windows, &caps, &sysfs, true, false),
            &SysfsBindOps,
        )
        .unwrap();
        let applied = assign(
            request(HostOs::Windows, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap();

        for status in [planned, applied] {
            assert_eq!(status.action, GpuBindAction::WouldAssign);
            assert_eq!(status.backend.as_deref(), Some("hyperv-gpu"));
            assert!(!status.bound);
            assert!(!status.hypervisor_invoked);
            assert!(status.log.contains("DDA/GPU-P"));
            assert!(status.log.contains("22b"));
            assert!(status.log.contains("exclusively"));
        }
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
    }

    #[test]
    fn windows_ineligible_is_plain_language_and_does_not_bind() {
        let caps = windows_no_display_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let error = assign(
            request(HostOs::Windows, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap_err();

        assert_eq!(error, GpuBindError::NoSecondDisplayPath);
        assert!(error.to_string().contains("second display path"));
        assert!(!error.to_string().contains("PowerShell"));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
    }

    #[test]
    fn macos_skips_and_does_not_pretend_firecracker() {
        let caps = macos_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let status = assign(
            request(HostOs::MacOs, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Skipped);
        assert!(!status.bound);
        assert!(status.log.contains("macOS"));
        assert!(status.log.contains("Firecracker"));
        assert!(control_lines(&sysfs.driver_dir("nvidia").join("unbind")).is_empty());
    }

    #[test]
    fn tests_cannot_apply_against_live_sysfs() {
        let caps = linux_pro_caps();
        let live = PciSysfs::live();
        let error = assign(
            request(HostOs::Linux, &caps, &live, true, true),
            &SysfsBindOps,
        )
        .unwrap_err();
        assert_eq!(error, GpuBindError::LiveSysfsForbiddenInTests);
    }

    #[test]
    fn resolve_pci_address_reads_uuid_from_mock_sysfs() {
        let (_dir, sysfs) = mock_gpu_tree(true);
        assert_eq!(resolve_pci_address(GPU_ID, &sysfs).unwrap(), BDF);
        assert_eq!(resolve_pci_address(BDF, &sysfs).unwrap(), BDF);
        assert!(matches!(
            resolve_pci_address("GPU-MISSING", &sysfs),
            Err(GpuBindError::GpuNotFound { .. })
        ));
    }

    #[test]
    fn report_maps_ineligible_to_refused_without_writes() {
        let caps = linux_no_iommu_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let status = report(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        );
        assert_eq!(status.action, GpuBindAction::Refused);
        assert!(!status.bound);
        assert!(status.log.contains("IOMMU"));
        assert!(control_lines(&sysfs.vfio_pci.join("bind")).is_empty());
    }

    #[test]
    fn injectable_driver_paths_record_bind_unbind_and_new_id() {
        let dir = TempDir::new().unwrap();
        let unbind = dir.path().join("unbind");
        let bind = dir.path().join("bind");
        let new_id = dir.path().join("new_id");
        fs::write(&unbind, b"").unwrap();
        fs::write(&bind, b"").unwrap();
        fs::write(&new_id, b"").unwrap();

        write_driver_unbind(&unbind, BDF).unwrap();
        write_driver_bind(&bind, BDF).unwrap();
        write_vfio_new_id(&new_id, "10de", "2786").unwrap();

        assert_eq!(control_lines(&unbind), vec![BDF]);
        assert_eq!(control_lines(&bind), vec![BDF]);
        assert_eq!(control_lines(&new_id), vec!["10de 2786"]);
    }
}
