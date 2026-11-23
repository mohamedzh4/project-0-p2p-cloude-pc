//! Pro GPU assignment with rollback (phase 22 + phase 25 / leftover 22b +
//! phase 29 Linux Pro apply + phase 30 Windows Pro Hyper-V apply).
//!
//! Linux: plan and optionally apply a vfio-pci bind against injectable sysfs.
//! Production [`POST /v1/provision`](crate::provision_http) for Linux Pro
//! sets [`BindRequest::apply`] true and writes the kernel vfio-pci sysfs
//! contract (`driver_override`, host `unbind`, vfio-pci `new_id`, vfio-pci
//! `bind`). The Host Agent window always passes [`WINDOW_GPU_BIND_APPLY`]
//! (false) so opening settings does not steal the GPU. Tests inject a temp
//! tree; [`cfg(test)`] refuses live `/sys`. On apply failure the previous
//! host driver is restored.
//!
//! Windows: plan Hyper-V Discrete Device Assignment (DDA) or exclusive GPU-P.
//! Apply runs documented PowerShell/CIM cmdlets through [`HyperVGpuOps`].
//! Production [`POST /v1/provision`](crate::provision_http) for Windows Pro
//! sets [`BindRequest::apply`] true and uses [`ProcessHyperVGpuOps`]
//! (hang-safe timeout). Tests inject a fake runner and never spawn
//! PowerShell. On apply failure, completed steps are undone so the host is
//! not left with a half-assigned GPU. The window still does not run it.
//!
//! The Host Agent computes the PCI address from sysfs; the volunteer does not run `lspci`.
//! After a successful DDA/GPU-P assign, production provision calls `Start-VM`
//! on the existing `dcl-pro-guest` (tests inject the runner). GATE-PRO stays
//! open until a human reports metal.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::host_capabilities::{HostCapabilities, IommuStatus};
use crate::provision::{
    ComputeBackend, HostOs, LabTier, ProvisionError, StubProvisioner,
};

pub const VFIO_PCI: &str = "vfio-pci";

/// The settings window and eligibility card never write bind/unbind/new_id.
/// Opening the agent must not steal the volunteer GPU.
pub const WINDOW_GPU_BIND_APPLY: bool = false;

/// Matchmaker / Host Agent `POST /v1/provision` applies Linux vfio-pci when
/// the capability probe offered Pro.
pub fn linux_pro_provision_should_apply(os: HostOs, lab_tier: LabTier, pro_offered: bool) -> bool {
    os == HostOs::Linux && lab_tier == LabTier::Pro && pro_offered
}

/// Matchmaker / Host Agent `POST /v1/provision` applies Hyper-V DDA (preferred)
/// or exclusive GPU-P when the capability probe offered Pro. The window stays
/// [`WINDOW_GPU_BIND_APPLY`].
pub fn windows_pro_provision_should_apply(os: HostOs, lab_tier: LabTier, pro_offered: bool) -> bool {
    os == HostOs::Windows && lab_tier == LabTier::Pro && pro_offered
}

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
///
/// Kernel ABI (vfio.rst / sysfs-bus-pci): write the BDF
/// (`DDDD:BB:DD.F`) to `unbind`/`bind`, and `vendor device` hex to
/// vfio-pci `new_id`. [`Send`] + [`Sync`] so the provision listener can hold
/// the ops across threads.
pub trait PciBindOps: Send + Sync {
    fn write_unbind(&self, unbind_path: &Path, bdf: &str) -> Result<(), GpuBindError>;
    fn write_bind(&self, bind_path: &Path, bdf: &str) -> Result<(), GpuBindError>;
    fn write_new_id(
        &self,
        new_id_path: &Path,
        vendor: &str,
        device: &str,
    ) -> Result<(), GpuBindError>;
}

/// Default ops: write the kernel sysfs control files (not an append log).
/// Tests pass a temp tree; production passes
/// `/sys/bus/pci/drivers/.../{bind,unbind,new_id}`.
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

/// Default Hyper-V VM name used when attaching a DDA device or GPU-P adapter.
/// The VM must already exist; this module never creates or starts it.
pub const DEFAULT_HYPERV_VM: &str = "dcl-pro-guest";

/// Override [`DEFAULT_HYPERV_VM`]. Empty or unset keeps the default.
pub const HYPERV_VM_ENV: &str = "HOST_AGENT_HYPERV_VM";

/// Operator VM name for Windows Pro apply. The VM must already exist; this
/// agent does not create it. Provision `Start-VM`s it after GPU assign.
pub fn hyperv_vm_name_from_env(lookup: impl Fn(&str) -> Option<String>) -> String {
    lookup(HYPERV_VM_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_HYPERV_VM.to_string())
}

/// Hang-safe timeout for one PowerShell/CIM invocation (phase 22 leftover).
pub const HYPERV_POWERSHELL_TIMEOUT: Duration = Duration::from_secs(30);

/// GPU-P resource fraction for a full exclusive partition.
/// Microsoft `Set-VMGpuPartitionAdapter` values are UInt64; 100_000_000 is the
/// documented full-capacity unit used with min/max/optimal equal.
pub const GPU_P_FULL_PARTITION: u64 = 100_000_000;

pub const DDA_TARGET: &str = "dda";
pub const GPU_P_TARGET: &str = "gpu-p";

/// Documented Microsoft DDA / GPU-P cmdlets. Tests inspect this sequence;
/// production maps each variant to a PowerShell `-Command` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyperVCmdlet {
    DisablePnpDevice { instance_id: String },
    EnablePnpDevice { instance_id: String },
    DismountHostAssignable { location_path: String },
    MountHostAssignable { location_path: String },
    ConfigureVmForDda { vm_name: String },
    AddVmAssignable { location_path: String, vm_name: String },
    RemoveVmAssignable { location_path: String, vm_name: String },
    VerifyVmAssignable { location_path: String, vm_name: String },
    SetHostPartitionCount { instance_path: String, count: u32 },
    AddGpuPartitionAdapter { instance_path: String, vm_name: String },
    SetGpuPartitionExclusive { vm_name: String },
    RemoveGpuPartitionAdapter { vm_name: String },
    VerifyGpuPartition { vm_name: String },
    StartVm { vm_name: String },
    StopVm { vm_name: String },
}

impl HyperVCmdlet {
    pub fn name(&self) -> &'static str {
        match self {
            Self::DisablePnpDevice { .. } => "Disable-PnpDevice",
            Self::EnablePnpDevice { .. } => "Enable-PnpDevice",
            Self::DismountHostAssignable { .. } => "Dismount-VMHostAssignableDevice",
            Self::MountHostAssignable { .. } => "Mount-VMHostAssignableDevice",
            Self::ConfigureVmForDda { .. } => "Set-VM",
            Self::AddVmAssignable { .. } => "Add-VMAssignableDevice",
            Self::RemoveVmAssignable { .. } => "Remove-VMAssignableDevice",
            Self::VerifyVmAssignable { .. } => "Get-VMAssignableDevice",
            Self::SetHostPartitionCount { .. } => "Set-VMHostPartitionableGpu",
            Self::AddGpuPartitionAdapter { .. } => "Add-VMGpuPartitionAdapter",
            Self::SetGpuPartitionExclusive { .. } => "Set-VMGpuPartitionAdapter",
            Self::RemoveGpuPartitionAdapter { .. } => "Remove-VMGpuPartitionAdapter",
            Self::VerifyGpuPartition { .. } => "Get-VMGpuPartitionAdapter",
            Self::StartVm { .. } => "Start-VM",
            Self::StopVm { .. } => "Stop-VM",
        }
    }
}

/// Result of probing PnP + Hyper-V for DDA vs GPU-P eligibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperVDeviceProbe {
    pub instance_id: String,
    pub location_path: Option<String>,
    pub dda_assignable: bool,
    pub partitionable: bool,
    pub partitionable_name: Option<String>,
    pub valid_partition_counts: Vec<u32>,
    pub partition_count: u32,
    /// Windows Server (`ProductType` 2 or 3). Microsoft supports DDA only
    /// there; client Windows (Pro, Enterprise, Education) supports GPU-P.
    pub server_edition: bool,
}

impl HyperVDeviceProbe {
    /// Every method this GPU supports, the one this edition supports first
    /// (issue 36): DDA then GPU-P on Windows Server, GPU-P then DDA on client
    /// Windows. Apply tries them in order and moves on only after a clean
    /// rollback.
    pub fn methods(&self) -> Result<Vec<HyperVMethod>, GpuBindError> {
        let dda = if !self.dda_assignable {
            Err("it is not DDA-assignable".to_string())
        } else if self.location_path.as_deref().unwrap_or("").is_empty() {
            Err("DDA needs a PCI location path (DEVPKEY_Device_LocationPaths); none was returned".to_string())
        } else {
            Ok(HyperVMethod::Dda)
        };
        let gpu_p = if !self.partitionable {
            Err("it is not GPU-P partitionable".to_string())
        } else if !self.valid_partition_counts.contains(&1) {
            Err("GPU-P cannot be exclusive on this GPU: partition count 1 is not in ValidPartitionCounts, so the host would keep a slice".to_string())
        } else if self.partitionable_name.as_deref().unwrap_or("").is_empty() {
            Err("GPU-P needs the partitionable GPU Name (InstancePath); none was returned".to_string())
        } else {
            Ok(HyperVMethod::GpuP)
        };

        let ordered = if self.server_edition {
            [dda, gpu_p]
        } else {
            [gpu_p, dda]
        };
        let mut methods = Vec::new();
        let mut reasons = Vec::new();
        for candidate in ordered {
            match candidate {
                Ok(method) => methods.push(method),
                Err(reason) => reasons.push(reason),
            }
        }
        if methods.is_empty() {
            return Err(GpuBindError::HyperVNotAssignable {
                reason: format!("this GPU cannot be assigned: {}", reasons.join("; ")),
            });
        }
        Ok(methods)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperVMethod {
    Dda,
    GpuP,
}

impl HyperVMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dda => "DDA",
            Self::GpuP => "GPU-P",
        }
    }
}

/// Injectable Hyper-V / PnP runner. Production spawns PowerShell; tests record
/// cmdlets and never touch a live hypervisor. [`Send`] + [`Sync`] so the
/// provision listener can hold the ops across threads.
pub trait HyperVGpuOps: Send + Sync {
    fn probe_device(&self, gpu_id: &str) -> Result<HyperVDeviceProbe, GpuBindError>;
    fn invoke(&self, cmdlet: &HyperVCmdlet) -> Result<String, GpuBindError>;
    fn is_live(&self) -> bool {
        false
    }
}

/// Production runner: `powershell.exe -NoProfile -NonInteractive` with a
/// hang-safe timeout. Tests must not use this — [`is_live`] is true and apply
/// refuses it under `cfg(test)`.
#[derive(Debug, Clone, Copy)]
pub struct ProcessHyperVGpuOps {
    pub timeout: Duration,
}

impl Default for ProcessHyperVGpuOps {
    fn default() -> Self {
        Self {
            timeout: HYPERV_POWERSHELL_TIMEOUT,
        }
    }
}

impl HyperVGpuOps for ProcessHyperVGpuOps {
    fn probe_device(&self, gpu_id: &str) -> Result<HyperVDeviceProbe, GpuBindError> {
        refuse_live_hyperv_in_tests(self)?;
        let stdout = run_powershell(&probe_powershell(gpu_id), self.timeout)?;
        parse_hyperv_probe(&stdout)
    }

    fn invoke(&self, cmdlet: &HyperVCmdlet) -> Result<String, GpuBindError> {
        refuse_live_hyperv_in_tests(self)?;
        run_powershell(&powershell_command(cmdlet), self.timeout)
    }

    fn is_live(&self) -> bool {
        true
    }
}

pub struct BindRequest<'a> {
    pub os: HostOs,
    pub capabilities: &'a HostCapabilities,
    pub share_gpu: bool,
    pub shared_gpu_id: Option<&'a str>,
    pub sysfs: &'a PciSysfs,
    /// When true, Linux writes the planned bind and Windows runs Hyper-V
    /// assignment (both roll back on failure). The Host Agent window always
    /// passes [`WINDOW_GPU_BIND_APPLY`] (`false`). Linux Pro and Windows Pro
    /// provision pass `true`.
    pub apply: bool,
    /// Windows apply path. Plan-only (`apply: false`) does not invoke it, so
    /// the window never spawns PowerShell. Tests pass a fake runner.
    /// Production provision uses [`ProcessHyperVGpuOps`].
    pub hyperv: Option<&'a dyn HyperVGpuOps>,
    /// Hyper-V VM that receives the device. Default [`DEFAULT_HYPERV_VM`].
    /// The VM is not created or started here.
    pub hyperv_vm_name: Option<&'a str>,
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
    #[error("tests must not bind live sysfs; pass a mock tree")]
    LiveSysfsForbiddenInTests,
    #[error("tests must not spawn PowerShell for Hyper-V; inject a fake runner")]
    LiveHyperVForbiddenInTests,
    #[error("Windows Pro GPU apply needs an injectable Hyper-V runner")]
    HyperVRunnerRequired,
    #[error("Hyper-V DDA/GPU-P assignment only runs on Windows")]
    HyperVOnlyOnWindows,
    #[error("Hyper-V PowerShell timed out after {timeout_secs}s; the command was killed so this agent does not hang")]
    HyperVTimeout { timeout_secs: u64 },
    #[error("Pro sessions are not available: {reason}")]
    HyperVNotAssignable { reason: String },
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
        Ok(_) => {}
        Err(ProvisionError::UnsupportedHostOs(os)) => {
            return Ok(GpuBindStatus::skipped(
                format!(
                    "hosting is not supported on {os}; this agent does not fall back to another backend for GPU assignment"
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
        HostOs::Windows => assign_windows(&gpu_id, request),
        HostOs::MacOs | HostOs::Other => Ok(GpuBindStatus::skipped(format!(
            "hosting is not supported on {}; this agent does not fall back to another backend for GPU assignment",
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

/// Apply a Linux VFIO plan against injectable driver paths. Sequence matches
/// kernel vfio.rst + PCI `new_id` (driver_override, host unbind, vfio-pci
/// `new_id` as `vendor device`, vfio-pci bind). Rolls back to the previous
/// driver if a mid-step fails. Never a partial bind on ineligible plans —
/// callers must run eligibility first.
pub fn apply_linux_vfio(
    plan: &GpuBindPlan,
    sysfs: &PciSysfs,
    ops: &dyn PciBindOps,
) -> Result<GpuBindStatus, GpuBindError> {
    refuse_live_sysfs_in_tests(sysfs)?;

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

fn assign_windows(
    gpu_id: &str,
    request: BindRequest<'_>,
) -> Result<GpuBindStatus, GpuBindError> {
    let vm_name = request
        .hyperv_vm_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_HYPERV_VM);
    let warning = exclusivity_warning(gpu_id);

    if !request.apply {
        let plan = plan_windows_dda(gpu_id, vm_name, None);
        let log = format!(
            "plan: Hyper-V DDA for GPU {gpu_id} on VM {vm_name} (Disable-PnpDevice, Dismount-VMHostAssignableDevice -Force, Add-VMAssignableDevice). {warning} Not applied. Hypervisor not invoked. Apply uses DDA first on Windows Server and GPU-P first on client Windows, falling back to the other after a clean rollback."
        );
        return Ok(status_from_plan(
            &plan,
            GpuBindAction::Planned,
            false,
            false,
            log,
        ));
    }

    let ops = request.hyperv.ok_or(GpuBindError::HyperVRunnerRequired)?;
    refuse_live_hyperv_in_tests(ops)?;

    let probe = ops.probe_device(gpu_id)?;
    let mut failures = Vec::new();
    for method in probe.methods()? {
        let attempt = match method {
            HyperVMethod::Dda => apply_windows_dda(gpu_id, vm_name, &probe, ops, &warning),
            HyperVMethod::GpuP => apply_windows_gpu_p(gpu_id, vm_name, &probe, ops, &warning),
        };
        match attempt {
            Ok(mut status) => {
                if !failures.is_empty() {
                    status.log = format!("{} Tried first: {}.", status.log, failures.join("; "));
                }
                return Ok(status);
            }
            // Clean rollback: the GPU is back on the host, so the other
            // method can start from scratch.
            Err(GpuBindError::ApplyFailed(failure)) if failure.rolled_back => {
                failures.push(format!("{} failed ({})", method.label(), failure.detail));
            }
            // Anything else (rollback failed, timeout) leaves the GPU in an
            // unknown state: stop here rather than stack a second method on it.
            Err(error) => return Err(error),
        }
    }
    Err(apply_failed(failures.join("; "), true, Some("pnp")))
}

fn plan_windows_dda(gpu_id: &str, vm_name: &str, location_path: Option<&str>) -> GpuBindPlan {
    let device = location_path.unwrap_or(gpu_id).to_string();
    GpuBindPlan {
        backend: ComputeBackend::HyperVGpu,
        gpu_id: gpu_id.to_string(),
        pci_address: location_path.map(str::to_string),
        previous_driver: Some("pnp".to_string()),
        target_driver: DDA_TARGET.to_string(),
        vendor_id: None,
        device_id: None,
        steps: vec![
            BindStep {
                action: "configureVm".to_string(),
                driver: DDA_TARGET.to_string(),
                device: vm_name.to_string(),
            },
            BindStep {
                action: "disablePnp".to_string(),
                driver: DDA_TARGET.to_string(),
                device: gpu_id.to_string(),
            },
            BindStep {
                action: "dismount".to_string(),
                driver: DDA_TARGET.to_string(),
                device: device.clone(),
            },
            BindStep {
                action: "addAssignable".to_string(),
                driver: DDA_TARGET.to_string(),
                device,
            },
            BindStep {
                action: "verify".to_string(),
                driver: DDA_TARGET.to_string(),
                device: vm_name.to_string(),
            },
        ],
        exclusive: true,
        exclusivity_warning: exclusivity_warning(gpu_id),
    }
}

fn plan_windows_gpu_p(gpu_id: &str, vm_name: &str, instance_path: &str) -> GpuBindPlan {
    GpuBindPlan {
        backend: ComputeBackend::HyperVGpu,
        gpu_id: gpu_id.to_string(),
        pci_address: Some(instance_path.to_string()),
        previous_driver: Some("pnp".to_string()),
        target_driver: GPU_P_TARGET.to_string(),
        vendor_id: None,
        device_id: None,
        steps: vec![
            BindStep {
                action: "setPartitionCount".to_string(),
                driver: GPU_P_TARGET.to_string(),
                device: instance_path.to_string(),
            },
            BindStep {
                action: "addGpuPartition".to_string(),
                driver: GPU_P_TARGET.to_string(),
                device: vm_name.to_string(),
            },
            BindStep {
                action: "setGpuPartitionExclusive".to_string(),
                driver: GPU_P_TARGET.to_string(),
                device: format!("{GPU_P_FULL_PARTITION}"),
            },
            BindStep {
                action: "verify".to_string(),
                driver: GPU_P_TARGET.to_string(),
                device: vm_name.to_string(),
            },
        ],
        exclusive: true,
        exclusivity_warning: exclusivity_warning(gpu_id),
    }
}

fn apply_windows_dda(
    gpu_id: &str,
    vm_name: &str,
    probe: &HyperVDeviceProbe,
    ops: &dyn HyperVGpuOps,
    warning: &str,
) -> Result<GpuBindStatus, GpuBindError> {
    let location_path = probe.location_path.as_deref().ok_or_else(|| {
        GpuBindError::HyperVNotAssignable {
            reason: "DDA needs a PCI location path".to_string(),
        }
    })?;
    let plan = plan_windows_dda(gpu_id, vm_name, Some(location_path));
    let instance_id = probe.instance_id.as_str();
    let previous = Some("pnp");

    let fail = |detail: String, rolled_back: bool| -> GpuBindError {
        apply_failed(detail, rolled_back, previous)
    };

    if let Err(error) = ops.invoke(&HyperVCmdlet::ConfigureVmForDda {
        vm_name: vm_name.to_string(),
    }) {
        return Err(fail(error.to_string(), false));
    }

    if let Err(error) = ops.invoke(&HyperVCmdlet::DisablePnpDevice {
        instance_id: instance_id.to_string(),
    }) {
        return Err(fail(error.to_string(), false));
    }

    if let Err(error) = ops.invoke(&HyperVCmdlet::DismountHostAssignable {
        location_path: location_path.to_string(),
    }) {
        let rolled_back = rollback_windows_dda(ops, instance_id, location_path, vm_name, true, false, false);
        return Err(fail(error.to_string(), rolled_back));
    }

    if let Err(error) = ops.invoke(&HyperVCmdlet::AddVmAssignable {
        location_path: location_path.to_string(),
        vm_name: vm_name.to_string(),
    }) {
        let rolled_back = rollback_windows_dda(ops, instance_id, location_path, vm_name, true, true, false);
        return Err(fail(error.to_string(), rolled_back));
    }

    match ops.invoke(&HyperVCmdlet::VerifyVmAssignable {
        location_path: location_path.to_string(),
        vm_name: vm_name.to_string(),
    }) {
        Ok(output) if assignment_verified(&output, location_path) => {}
        Ok(_) => {
            let rolled_back = rollback_windows_dda(ops, instance_id, location_path, vm_name, true, true, true);
            return Err(fail(
                "Get-VMAssignableDevice did not confirm the GPU; not claiming assigned".to_string(),
                rolled_back,
            ));
        }
        Err(error) => {
            let rolled_back = rollback_windows_dda(ops, instance_id, location_path, vm_name, true, true, true);
            return Err(fail(error.to_string(), rolled_back));
        }
    }

    if let Err(error) = start_existing_vm(ops, vm_name) {
        let _ = stop_existing_vm(ops, vm_name);
        let rolled_back = rollback_windows_dda(ops, instance_id, location_path, vm_name, true, true, true);
        return Err(fail(error.to_string(), rolled_back));
    }

    Ok(status_from_plan(
        &plan,
        GpuBindAction::Bound,
        true,
        false,
        format!(
            "assigned GPU {gpu_id} with Hyper-V DDA ({location_path}) to VM {vm_name} and Start-VM. {warning} QEMU was not spawned."
        ),
    ))
}

fn apply_windows_gpu_p(
    gpu_id: &str,
    vm_name: &str,
    probe: &HyperVDeviceProbe,
    ops: &dyn HyperVGpuOps,
    warning: &str,
) -> Result<GpuBindStatus, GpuBindError> {
    let instance_path = probe.partitionable_name.as_deref().ok_or_else(|| {
        GpuBindError::HyperVNotAssignable {
            reason: "GPU-P needs Get-VMHostPartitionableGpu Name".to_string(),
        }
    })?;
    let plan = plan_windows_gpu_p(gpu_id, vm_name, instance_path);
    let previous = Some("pnp");
    let previous_count = probe.partition_count;
    let fail = |detail: String, rolled_back: bool| -> GpuBindError {
        apply_failed(detail, rolled_back, previous)
    };

    if let Err(error) = ops.invoke(&HyperVCmdlet::SetHostPartitionCount {
        instance_path: instance_path.to_string(),
        count: 1,
    }) {
        return Err(fail(error.to_string(), false));
    }
    let count_set = previous_count != 1;

    if let Err(error) = ops.invoke(&HyperVCmdlet::AddGpuPartitionAdapter {
        instance_path: instance_path.to_string(),
        vm_name: vm_name.to_string(),
    }) {
        let rolled_back =
            rollback_windows_gpu_p(ops, instance_path, vm_name, previous_count, count_set, false);
        return Err(fail(error.to_string(), rolled_back));
    }

    if let Err(error) = ops.invoke(&HyperVCmdlet::SetGpuPartitionExclusive {
        vm_name: vm_name.to_string(),
    }) {
        let rolled_back =
            rollback_windows_gpu_p(ops, instance_path, vm_name, previous_count, count_set, true);
        return Err(fail(error.to_string(), rolled_back));
    }

    match ops.invoke(&HyperVCmdlet::VerifyGpuPartition {
        vm_name: vm_name.to_string(),
    }) {
        Ok(output) if assignment_verified(&output, instance_path) || assignment_verified(&output, vm_name) => {}
        Ok(_) => {
            let rolled_back =
                rollback_windows_gpu_p(ops, instance_path, vm_name, previous_count, count_set, true);
            return Err(fail(
                "Get-VMGpuPartitionAdapter did not confirm the GPU partition; not claiming assigned".to_string(),
                rolled_back,
            ));
        }
        Err(error) => {
            let rolled_back =
                rollback_windows_gpu_p(ops, instance_path, vm_name, previous_count, count_set, true);
            return Err(fail(error.to_string(), rolled_back));
        }
    }

    if let Err(error) = start_existing_vm(ops, vm_name) {
        let _ = stop_existing_vm(ops, vm_name);
        let rolled_back =
            rollback_windows_gpu_p(ops, instance_path, vm_name, previous_count, count_set, true);
        return Err(fail(error.to_string(), rolled_back));
    }

    Ok(status_from_plan(
        &plan,
        GpuBindAction::Bound,
        true,
        false,
        format!(
            "assigned GPU {gpu_id} with exclusive Hyper-V GPU-P (PartitionCount 1, full 100000000 resource fractions) to VM {vm_name} and Start-VM. {warning} QEMU was not spawned."
        ),
    ))
}

fn start_existing_vm(ops: &dyn HyperVGpuOps, vm_name: &str) -> Result<(), GpuBindError> {
    ops.invoke(&HyperVCmdlet::StartVm {
        vm_name: vm_name.to_string(),
    })?;
    Ok(())
}

/// Power off `dcl-pro-guest` (or `HOST_AGENT_HYPERV_VM`) during teardown.
pub fn stop_existing_vm(ops: &dyn HyperVGpuOps, vm_name: &str) -> Result<(), GpuBindError> {
    ops.invoke(&HyperVCmdlet::StopVm {
        vm_name: vm_name.to_string(),
    })?;
    Ok(())
}

fn rollback_windows_dda(
    ops: &dyn HyperVGpuOps,
    instance_id: &str,
    location_path: &str,
    vm_name: &str,
    pnp_disabled: bool,
    dismounted: bool,
    added: bool,
) -> bool {
    let mut ok = true;
    if added {
        if ops
            .invoke(&HyperVCmdlet::RemoveVmAssignable {
                location_path: location_path.to_string(),
                vm_name: vm_name.to_string(),
            })
            .is_err()
        {
            ok = false;
        }
    }
    if dismounted {
        if ops
            .invoke(&HyperVCmdlet::MountHostAssignable {
                location_path: location_path.to_string(),
            })
            .is_err()
        {
            ok = false;
        }
    }
    if pnp_disabled {
        if ops
            .invoke(&HyperVCmdlet::EnablePnpDevice {
                instance_id: instance_id.to_string(),
            })
            .is_err()
        {
            ok = false;
        }
    }
    ok
}

fn rollback_windows_gpu_p(
    ops: &dyn HyperVGpuOps,
    instance_path: &str,
    vm_name: &str,
    previous_count: u32,
    count_set: bool,
    adapter_added: bool,
) -> bool {
    let mut ok = true;
    if adapter_added {
        if ops
            .invoke(&HyperVCmdlet::RemoveGpuPartitionAdapter {
                vm_name: vm_name.to_string(),
            })
            .is_err()
        {
            ok = false;
        }
    }
    if count_set {
        if ops
            .invoke(&HyperVCmdlet::SetHostPartitionCount {
                instance_path: instance_path.to_string(),
                count: previous_count.max(1),
            })
            .is_err()
        {
            ok = false;
        }
    }
    ok
}

fn assignment_verified(output: &str, token: &str) -> bool {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("not assigned") || lower.contains("notassigned") {
        return false;
    }
    lower.contains(&token.to_ascii_lowercase()) || lower.contains("assigned")
}

fn refuse_live_hyperv_in_tests(ops: &dyn HyperVGpuOps) -> Result<(), GpuBindError> {
    #[cfg(test)]
    {
        if ops.is_live() {
            return Err(GpuBindError::LiveHyperVForbiddenInTests);
        }
    }
    let _ = ops;
    Ok(())
}

/// PowerShell for [`HyperVCmdlet`]. Cmdlet names and flags come from Microsoft
/// Hyper-V / PnP docs (DDA deploy graphics; GPU-P partition-assign).
pub fn powershell_command(cmdlet: &HyperVCmdlet) -> String {
    match cmdlet {
        HyperVCmdlet::DisablePnpDevice { instance_id } => format!(
            "$ErrorActionPreference = 'Stop'; Disable-PnpDevice -InstanceId {} -Confirm:$false",
            ps_quote(instance_id)
        ),
        HyperVCmdlet::EnablePnpDevice { instance_id } => format!(
            "$ErrorActionPreference = 'Stop'; Enable-PnpDevice -InstanceId {} -Confirm:$false",
            ps_quote(instance_id)
        ),
        HyperVCmdlet::DismountHostAssignable { location_path } => format!(
            "$ErrorActionPreference = 'Stop'; Dismount-VMHostAssignableDevice -Force -LocationPath {}",
            ps_quote(location_path)
        ),
        HyperVCmdlet::MountHostAssignable { location_path } => format!(
            "$ErrorActionPreference = 'Stop'; Mount-VMHostAssignableDevice -LocationPath {}",
            ps_quote(location_path)
        ),
        HyperVCmdlet::ConfigureVmForDda { vm_name } => {
            let vm = ps_quote(vm_name);
            format!(
                "$ErrorActionPreference = 'Stop'; \
Set-VM -Name {vm} -AutomaticStopAction TurnOff; \
Set-VM -GuestControlledCacheTypes $true -VMName {vm}; \
Set-VM -LowMemoryMappedIoSpace 3Gb -VMName {vm}; \
Set-VM -HighMemoryMappedIoSpace 33280Mb -VMName {vm}"
            )
        }
        HyperVCmdlet::AddVmAssignable {
            location_path,
            vm_name,
        } => format!(
            "$ErrorActionPreference = 'Stop'; Add-VMAssignableDevice -LocationPath {} -VMName {}",
            ps_quote(location_path),
            ps_quote(vm_name)
        ),
        HyperVCmdlet::RemoveVmAssignable {
            location_path,
            vm_name,
        } => format!(
            "$ErrorActionPreference = 'Stop'; Remove-VMAssignableDevice -LocationPath {} -VMName {}",
            ps_quote(location_path),
            ps_quote(vm_name)
        ),
        HyperVCmdlet::VerifyVmAssignable {
            location_path,
            vm_name,
        } => format!(
            "$ErrorActionPreference = 'Stop'; $dev = Get-VMAssignableDevice -LocationPath {} -VMName {}; if (-not $dev) {{ throw 'Get-VMAssignableDevice returned no device; not assigned' }}; $dev | ConvertTo-Json -Compress",
            ps_quote(location_path),
            ps_quote(vm_name)
        ),
        HyperVCmdlet::SetHostPartitionCount {
            instance_path,
            count,
        } => format!(
            "$ErrorActionPreference = 'Stop'; if (Get-Command Set-VMHostPartitionableGpu -ErrorAction SilentlyContinue) {{ Set-VMHostPartitionableGpu -Name {0} -PartitionCount {count} }} else {{ Set-VMPartitionableGpu -Name {0} -PartitionCount {count} }}",
            ps_quote(instance_path)
        ),
        HyperVCmdlet::AddGpuPartitionAdapter {
            instance_path,
            vm_name,
        } => format!(
            "$ErrorActionPreference = 'Stop'; Add-VMGpuPartitionAdapter -VMName {} -InstancePath {}",
            ps_quote(vm_name),
            ps_quote(instance_path)
        ),
        HyperVCmdlet::SetGpuPartitionExclusive { vm_name } => {
            let vm = ps_quote(vm_name);
            let full = GPU_P_FULL_PARTITION;
            format!(
                "$ErrorActionPreference = 'Stop'; \
Set-VMGpuPartitionAdapter -VMName {vm} \
-MinPartitionVRAM {full} -MaxPartitionVRAM {full} -OptimalPartitionVRAM {full} \
-MinPartitionEncode {full} -MaxPartitionEncode {full} -OptimalPartitionEncode {full} \
-MinPartitionDecode {full} -MaxPartitionDecode {full} -OptimalPartitionDecode {full} \
-MinPartitionCompute {full} -MaxPartitionCompute {full} -OptimalPartitionCompute {full}"
            )
        }
        HyperVCmdlet::RemoveGpuPartitionAdapter { vm_name } => format!(
            "$ErrorActionPreference = 'Stop'; Remove-VMGpuPartitionAdapter -VMName {}",
            ps_quote(vm_name)
        ),
        HyperVCmdlet::VerifyGpuPartition { vm_name } => format!(
            "$ErrorActionPreference = 'Stop'; $dev = Get-VMGpuPartitionAdapter -VMName {}; if (-not $dev) {{ throw 'Get-VMGpuPartitionAdapter returned no adapter; not assigned' }}; $dev | ConvertTo-Json -Compress",
            ps_quote(vm_name)
        ),
        HyperVCmdlet::StartVm { vm_name } => format!(
            "$ErrorActionPreference = 'Stop'; Start-VM -Name {}",
            ps_quote(vm_name)
        ),
        HyperVCmdlet::StopVm { vm_name } => format!(
            "$ErrorActionPreference = 'Stop'; Stop-VM -Name {} -Force -TurnOff",
            ps_quote(vm_name)
        ),
    }
}

/// Probe script: PnP InstanceId + DEVPKEY_Device_LocationPaths (DDA) and
/// Get-VMHostPartitionableGpu (GPU-P), per Microsoft DDA and GPU-P docs.
pub fn probe_powershell(gpu_id: &str) -> String {
    let id = ps_quote(gpu_id);
    format!(
        "$ErrorActionPreference = 'Stop'; \
$gpuId = {id}; \
$dev = $null; \
try {{ $dev = Get-PnpDevice -InstanceId $gpuId -ErrorAction Stop }} catch {{}}; \
if (-not $dev) {{ \
  $dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object {{ $_.InstanceId -eq $gpuId -or $_.FriendlyName -like ('*' + $gpuId + '*') }} | Select-Object -First 1 \
}}; \
if (-not $dev) {{ throw \"PnP display device not found for $gpuId\" }}; \
$instanceId = $dev.InstanceId; \
$locationPath = $null; \
try {{ \
  $prop = Get-PnpDeviceProperty -InstanceId $instanceId -KeyName 'DEVPKEY_Device_LocationPaths'; \
  if ($prop -and $prop.Data -and $prop.Data.Count -gt 0) {{ $locationPath = [string]$prop.Data[0] }} \
}} catch {{}}; \
$server = $false; \
try {{ $server = ((Get-CimInstance Win32_OperatingSystem).ProductType -ne 1) }} catch {{}}; \
$partitionable = $false; $partName = $null; $counts = @(); $partCount = 0; \
$key = $instanceId.Replace('\\', '#'); \
try {{ \
  $all = if (Get-Command Get-VMHostPartitionableGpu -ErrorAction SilentlyContinue) {{ Get-VMHostPartitionableGpu }} else {{ Get-VMPartitionableGpu }}; \
  $gpu = $all | Where-Object {{ $_.Name -like ('*' + $key + '*') -or $_.Name -eq $gpuId }} | Select-Object -First 1; \
  if ($gpu) {{ $partitionable = $true; $partName = [string]$gpu.Name; $counts = @($gpu.ValidPartitionCounts); $partCount = [int]$gpu.PartitionCount }} \
}} catch {{}}; \
$dda = [bool]($locationPath); \
[ordered]@{{ instanceId = $instanceId; locationPath = $locationPath; ddaAssignable = $dda; partitionable = $partitionable; partitionableName = $partName; validPartitionCounts = $counts; partitionCount = $partCount; serverEdition = $server }} | ConvertTo-Json -Compress"
    )
}

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn parse_hyperv_probe(stdout: &str) -> Result<HyperVDeviceProbe, GpuBindError> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        GpuBindError::Io(format!("Hyper-V probe JSON was not usable: {error}"))
    })?;
    let instance_id = json_string(&value, "instanceId").ok_or_else(|| GpuBindError::Io(
        "Hyper-V probe JSON missing instanceId".to_string(),
    ))?;
    let location_path = json_string(&value, "locationPath").filter(|s| !s.is_empty());
    let dda_assignable = json_bool(&value, "ddaAssignable").unwrap_or(location_path.is_some());
    let partitionable = json_bool(&value, "partitionable").unwrap_or(false);
    let partitionable_name = json_string(&value, "partitionableName").filter(|s| !s.is_empty());
    let valid_partition_counts = json_u32_list(&value, "validPartitionCounts");
    let partition_count = json_u32(&value, "partitionCount").unwrap_or(0);
    let server_edition = json_bool(&value, "serverEdition").unwrap_or(false);
    Ok(HyperVDeviceProbe {
        instance_id,
        location_path,
        dda_assignable,
        partitionable,
        partitionable_name,
        valid_partition_counts,
        partition_count,
        server_edition,
    })
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Null) | None => None,
        Some(other) => Some(other.to_string().trim_matches('"').to_string()),
    }
}

fn json_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(serde_json::Value::as_bool)
}

fn json_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    value.get(key).and_then(serde_json::Value::as_u64).map(|n| n as u32)
}

fn json_u32_list(value: &serde_json::Value, key: &str) -> Vec<u32> {
    match value.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_u64().map(|n| n as u32))
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn run_powershell(script: &str, timeout: Duration) -> Result<String, GpuBindError> {
    if cfg!(not(windows)) {
        return Err(GpuBindError::HyperVOnlyOnWindows);
    }
    #[cfg(test)]
    {
        let _ = script;
        let _ = timeout;
        return Err(GpuBindError::LiveHyperVForbiddenInTests);
    }
    #[cfg(not(test))]
    {
        spawn_powershell(script, timeout)
    }
}

#[cfg(not(test))]
fn spawn_powershell(script: &str, timeout: Duration) -> Result<String, GpuBindError> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GpuBindError::Io(error.to_string()))?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GpuBindError::HyperVTimeout {
                    timeout_secs: timeout.as_secs(),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(GpuBindError::Io(error.to_string())),
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = stdout_pipe.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = stderr_pipe.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }

    if status.success() {
        Ok(stdout)
    } else {
        let detail = if stderr.trim().is_empty() {
            format!("powershell.exe exit {status}")
        } else {
            stderr.trim().to_string()
        };
        Err(GpuBindError::Io(detail))
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

/// Restore the previous host driver after a Pro session (teardown).
pub fn release_linux_vfio(
    sysfs: &PciSysfs,
    ops: &dyn PciBindOps,
    bdf: &str,
    previous_driver: Option<&str>,
) -> Result<(), GpuBindError> {
    refuse_live_sysfs_in_tests(sysfs)?;
    let device_dir = sysfs.device_dir(bdf);
    rollback_linux(sysfs, ops, bdf, previous_driver, &device_dir)
}

/// Stop the Pro VM and undo DDA/GPU-P so the volunteer display path returns.
pub fn teardown_windows_pro(
    ops: &dyn HyperVGpuOps,
    vm_name: &str,
    target_driver: &str,
    instance_id: &str,
    pci_or_path: &str,
) {
    let _ = stop_existing_vm(ops, vm_name);
    if target_driver == DDA_TARGET {
        let _ = rollback_windows_dda(
            ops,
            instance_id,
            pci_or_path,
            vm_name,
            true,
            true,
            true,
        );
    } else if target_driver == GPU_P_TARGET {
        let _ = rollback_windows_gpu_p(ops, pci_or_path, vm_name, 0, true, true);
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
    // Kernel sysfs bind/unbind/new_id are write-once commands. O_APPEND and
    // O_CREAT are not the documented ABI and can fail on live /sys (no seek,
    // cannot create). Mock trees create the files first.
    let mut file = OpenOptions::new()
        .write(true)
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
pub(crate) const TEST_GPU_ID: &str = "GPU-TEST-4070";
#[cfg(test)]
pub(crate) const TEST_GPU_BDF: &str = "0000:01:00.0";

/// Mock PCI/sysfs tree for crate tests (provision HTTP + gpu_bind). Never `/sys`.
#[cfg(test)]
pub(crate) fn mock_linux_vfio_sysfs() -> (tempfile::TempDir, PciSysfs) {
    mock_linux_vfio_sysfs_with_iommu(true)
}

#[cfg(test)]
pub(crate) fn mock_linux_vfio_sysfs_with_iommu(with_iommu_group: bool) -> (tempfile::TempDir, PciSysfs) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let device = root.join("sys/bus/pci/devices").join(TEST_GPU_BDF);
    fs::create_dir_all(&device).unwrap();
    fs::write(device.join("vendor"), b"0x10de\n").unwrap();
    fs::write(device.join("device"), b"0x2786\n").unwrap();
    fs::write(device.join("dcl_gpu_id"), format!("{TEST_GPU_ID}\n")).unwrap();
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

    let nvidia_proc = root.join("proc/driver/nvidia/gpus").join(TEST_GPU_BDF);
    fs::create_dir_all(&nvidia_proc).unwrap();
    fs::write(
        nvidia_proc.join("information"),
        format!("Model:           NVIDIA GeForce RTX 4070\nGPU UUID:        {TEST_GPU_ID}\n"),
    )
    .unwrap();

    let sysfs = PciSysfs::from_root(root);
    (dir, sysfs)
}

#[cfg(test)]
pub(crate) const TEST_HYPERV_LOCATION_PATH: &str = "PCIROOT(0)#PCI(0100)#PCI(0000)";
#[cfg(test)]
pub(crate) const TEST_HYPERV_INSTANCE_ID: &str =
    r"PCI\VEN_10DE&DEV_2786&SUBSYS_0000&REV_A1\4&1234&0&0000";
#[cfg(test)]
pub(crate) const TEST_HYPERV_PARTITION_NAME: &str =
    r"\\?\PCI#VEN_10DE&DEV_2786&SUBSYS_0000&REV_A1#4&1234&0&0000#{064092b3-625e-43bf-9eb5-dc845897dd59}";

/// Fake Hyper-V runner for crate tests. Never `powershell.exe`.
#[cfg(test)]
pub(crate) struct RecordingHyperV {
    probe: HyperVDeviceProbe,
    fail_on: Vec<&'static str>,
    log: Mutex<Vec<String>>,
    pnp_disabled: Mutex<bool>,
    dismounted: Mutex<bool>,
    assigned: Mutex<bool>,
    partition_assigned: Mutex<bool>,
}

#[cfg(test)]
impl RecordingHyperV {
    pub(crate) fn dda() -> Self {
        Self::new(HyperVDeviceProbe {
            instance_id: TEST_HYPERV_INSTANCE_ID.to_string(),
            location_path: Some(TEST_HYPERV_LOCATION_PATH.to_string()),
            dda_assignable: true,
            partitionable: false,
            partitionable_name: None,
            valid_partition_counts: Vec::new(),
            partition_count: 0,
            server_edition: true,
        })
    }

    pub(crate) fn gpu_p() -> Self {
        Self::new(HyperVDeviceProbe {
            instance_id: TEST_HYPERV_INSTANCE_ID.to_string(),
            location_path: None,
            dda_assignable: false,
            partitionable: true,
            partitionable_name: Some(TEST_HYPERV_PARTITION_NAME.to_string()),
            valid_partition_counts: vec![1, 2, 4],
            partition_count: 4,
            server_edition: false,
        })
    }

    /// A GPU that offers both, as most do: the edition decides the order.
    pub(crate) fn both(server_edition: bool) -> Self {
        Self::new(HyperVDeviceProbe {
            instance_id: TEST_HYPERV_INSTANCE_ID.to_string(),
            location_path: Some(TEST_HYPERV_LOCATION_PATH.to_string()),
            dda_assignable: true,
            partitionable: true,
            partitionable_name: Some(TEST_HYPERV_PARTITION_NAME.to_string()),
            valid_partition_counts: vec![1, 2, 4],
            partition_count: 4,
            server_edition,
        })
    }

    fn new(probe: HyperVDeviceProbe) -> Self {
        Self {
            probe,
            fail_on: Vec::new(),
            log: Mutex::new(Vec::new()),
            pnp_disabled: Mutex::new(false),
            dismounted: Mutex::new(false),
            assigned: Mutex::new(false),
            partition_assigned: Mutex::new(false),
        }
    }

    pub(crate) fn fail_on(mut self, cmdlet: &'static str) -> Self {
        self.fail_on.push(cmdlet);
        self
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.log.lock().expect("hyperv log").clone()
    }

    pub(crate) fn leftover_assigned(&self) -> bool {
        *self.assigned.lock().expect("assigned")
            || *self.dismounted.lock().expect("dismounted")
            || *self.pnp_disabled.lock().expect("pnp")
            || *self.partition_assigned.lock().expect("partition")
    }

    pub(crate) fn dda_assigned(&self) -> bool {
        *self.assigned.lock().expect("assigned")
    }

    pub(crate) fn gpu_p_assigned(&self) -> bool {
        *self.partition_assigned.lock().expect("partition")
    }
}

#[cfg(test)]
impl HyperVGpuOps for RecordingHyperV {
    fn probe_device(&self, gpu_id: &str) -> Result<HyperVDeviceProbe, GpuBindError> {
        if gpu_id != TEST_GPU_ID {
            return Err(GpuBindError::GpuNotFound {
                gpu_id: gpu_id.to_string(),
            });
        }
        Ok(self.probe.clone())
    }

    fn invoke(&self, cmdlet: &HyperVCmdlet) -> Result<String, GpuBindError> {
        self.log
            .lock()
            .expect("hyperv log")
            .push(cmdlet.name().to_string());
        if self.fail_on.contains(&cmdlet.name()) {
            return Err(GpuBindError::Io(format!(
                "injected {} failure",
                cmdlet.name()
            )));
        }
        match cmdlet {
            HyperVCmdlet::DisablePnpDevice { .. } => {
                *self.pnp_disabled.lock().expect("pnp") = true;
            }
            HyperVCmdlet::EnablePnpDevice { .. } => {
                *self.pnp_disabled.lock().expect("pnp") = false;
            }
            HyperVCmdlet::DismountHostAssignable { .. } => {
                *self.dismounted.lock().expect("dismounted") = true;
            }
            HyperVCmdlet::MountHostAssignable { .. } => {
                *self.dismounted.lock().expect("dismounted") = false;
            }
            HyperVCmdlet::AddVmAssignable { .. } => {
                *self.assigned.lock().expect("assigned") = true;
            }
            HyperVCmdlet::RemoveVmAssignable { .. } => {
                *self.assigned.lock().expect("assigned") = false;
            }
            HyperVCmdlet::VerifyVmAssignable { .. } => {
                if !*self.assigned.lock().expect("assigned") {
                    return Err(GpuBindError::Io(
                        "Get-VMAssignableDevice returned no device; not assigned".into(),
                    ));
                }
                return Ok(TEST_HYPERV_LOCATION_PATH.to_string());
            }
            HyperVCmdlet::AddGpuPartitionAdapter { .. } => {
                *self.partition_assigned.lock().expect("partition") = true;
            }
            HyperVCmdlet::RemoveGpuPartitionAdapter { .. } => {
                *self.partition_assigned.lock().expect("partition") = false;
            }
            HyperVCmdlet::VerifyGpuPartition { .. } => {
                if !*self.partition_assigned.lock().expect("partition") {
                    return Err(GpuBindError::Io(
                        "Get-VMGpuPartitionAdapter returned no adapter; not assigned".into(),
                    ));
                }
                return Ok(format!("assigned {TEST_HYPERV_PARTITION_NAME}"));
            }
            _ => {}
        }
        Ok(String::new())
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
    const LOCATION_PATH: &str = TEST_HYPERV_LOCATION_PATH;
    const INSTANCE_ID: &str = TEST_HYPERV_INSTANCE_ID;

    #[test]
    fn hyperv_vm_name_defaults_and_overrides_from_env() {
        assert_eq!(hyperv_vm_name_from_env(|_| None), DEFAULT_HYPERV_VM);
        assert_eq!(
            hyperv_vm_name_from_env(|key| (key == HYPERV_VM_ENV).then(|| "  lab-guest  ".into())),
            "lab-guest"
        );
        assert_eq!(
            hyperv_vm_name_from_env(|key| (key == HYPERV_VM_ENV).then(|| "   ".into())),
            DEFAULT_HYPERV_VM
        );
    }

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

    fn windows_request<'a>(
        caps: &'a HostCapabilities,
        sysfs: &'a PciSysfs,
        apply: bool,
        hyperv: Option<&'a dyn HyperVGpuOps>,
    ) -> BindRequest<'a> {
        BindRequest {
            os: HostOs::Windows,
            capabilities: caps,
            share_gpu: true,
            shared_gpu_id: Some(GPU_ID),
            sysfs,
            apply,
            hyperv,
            hyperv_vm_name: Some(DEFAULT_HYPERV_VM),
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
        mock_linux_vfio_sysfs_with_iommu(with_iommu_group)
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
            hyperv: None,
            hyperv_vm_name: Some(DEFAULT_HYPERV_VM),
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
    fn linux_pro_uses_qemu_vfio() {
        let selected = StubProvisioner::select(HostOs::Linux, LabTier::Pro).unwrap();
        assert_eq!(selected.backend(), ComputeBackend::QemuVfio);

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
    fn windows_plan_lists_dda_cmdlets_without_applying() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::dda();
        let status = assign(
            windows_request(&caps, &sysfs, false, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Planned);
        assert_eq!(status.backend.as_deref(), Some("hyperv-gpu"));
        assert_eq!(status.target_driver.as_deref(), Some(DDA_TARGET));
        assert!(!status.bound);
        assert!(!status.hypervisor_invoked);
        assert!(status.log.contains("Not applied"));
        assert!(status.log.contains("exclusively"));
        assert!(status.steps.iter().any(|step| step.action == "disablePnp"));
        assert!(status.steps.iter().any(|step| step.action == "dismount"));
        assert!(status.steps.iter().any(|step| step.action == "addAssignable"));
        assert!(
            runner.names().is_empty(),
            "plan must not invoke PowerShell/CIM"
        );
        assert!(!runner.leftover_assigned());
    }

    #[test]
    fn windows_dda_success_records_assign_sequence() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::dda();
        let status = assign(
            windows_request(&caps, &sysfs, true, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Bound);
        assert!(status.bound);
        assert!(!status.rolled_back);
        assert_eq!(status.target_driver.as_deref(), Some(DDA_TARGET));
        assert_eq!(status.pci_address.as_deref(), Some(LOCATION_PATH));
        assert!(!status.hypervisor_invoked);
        assert!(status.log.contains("DDA"));
        assert!(status.log.contains("exclusively"));
        assert_eq!(
            runner.names(),
            vec![
                "Set-VM",
                "Disable-PnpDevice",
                "Dismount-VMHostAssignableDevice",
                "Add-VMAssignableDevice",
                "Get-VMAssignableDevice",
                "Start-VM",
            ]
        );
        assert!(
            runner.dda_assigned(),
            "success must leave the DDA assignment in place"
        );
    }

    #[test]
    fn windows_dda_mid_step_failure_rolls_back_cleanly() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::dda().fail_on("Add-VMAssignableDevice");
        let error = assign(
            windows_request(&caps, &sysfs, true, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap_err();

        match error {
            GpuBindError::ApplyFailed(ref failure) => {
                assert!(failure.rolled_back);
                assert_eq!(failure.previous_driver.as_deref(), Some("pnp"));
                assert!(error.to_string().contains("restored"));
            }
            other => panic!("expected ApplyFailed, got {other:?}"),
        }
        let names = runner.names();
        assert_eq!(names[0], "Set-VM");
        assert_eq!(names[1], "Disable-PnpDevice");
        assert_eq!(names[2], "Dismount-VMHostAssignableDevice");
        assert_eq!(names[3], "Add-VMAssignableDevice");
        assert!(names.contains(&"Mount-VMHostAssignableDevice".to_string()));
        assert!(names.contains(&"Enable-PnpDevice".to_string()));
        assert!(
            !names.contains(&"Get-VMAssignableDevice".to_string()),
            "verify must not run after a failed add"
        );
        assert!(
            !runner.leftover_assigned(),
            "rollback must not leave a half-assigned GPU"
        );
    }

    #[test]
    fn windows_gpu_p_success_assigns_exclusive_partition() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::gpu_p();
        let status = assign(
            windows_request(&caps, &sysfs, true, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Bound);
        assert!(status.bound);
        assert_eq!(status.target_driver.as_deref(), Some(GPU_P_TARGET));
        assert!(status.log.contains("GPU-P"));
        assert_eq!(runner.names(), GPU_P_SEQUENCE.to_vec());
        assert!(runner.gpu_p_assigned());
    }

    const GPU_P_SEQUENCE: [&str; 5] = [
        "Set-VMHostPartitionableGpu",
        "Add-VMGpuPartitionAdapter",
        "Set-VMGpuPartitionAdapter",
        "Get-VMGpuPartitionAdapter",
        "Start-VM",
    ];

    // Issue 36: client Windows supports GPU-P, not DDA, so a GPU that
    // offers both must go GPU-P first and never touch DDA cmdlets.
    #[test]
    fn client_windows_tries_gpu_p_first() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::both(false);
        let status = assign(windows_request(&caps, &sysfs, true, Some(&runner)), &SysfsBindOps)
            .unwrap();

        assert_eq!(status.target_driver.as_deref(), Some(GPU_P_TARGET));
        assert_eq!(runner.names(), GPU_P_SEQUENCE.to_vec());
        assert!(!status.log.contains("Tried first"));
    }

    #[test]
    fn windows_server_tries_dda_first() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::both(true);
        let status = assign(windows_request(&caps, &sysfs, true, Some(&runner)), &SysfsBindOps)
            .unwrap();

        assert_eq!(status.target_driver.as_deref(), Some(DDA_TARGET));
        assert!(runner.dda_assigned());
        assert!(!runner.names().iter().any(|n| n.contains("Partition")));
    }

    #[test]
    fn a_cleanly_rolled_back_method_falls_back_to_the_other() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::both(false).fail_on("Add-VMGpuPartitionAdapter");
        let status = assign(windows_request(&caps, &sysfs, true, Some(&runner)), &SysfsBindOps)
            .unwrap();

        assert_eq!(status.target_driver.as_deref(), Some(DDA_TARGET));
        assert!(runner.dda_assigned());
        assert!(!runner.gpu_p_assigned());
        assert!(
            status.log.contains("Tried first: GPU-P failed (") && status.log.contains("injected Add-VMGpuPartitionAdapter failure"),
            "{}",
            status.log
        );
    }

    #[test]
    fn both_methods_failing_reports_both_and_leaves_nothing_assigned() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::both(false)
            .fail_on("Add-VMGpuPartitionAdapter")
            .fail_on("Dismount-VMHostAssignableDevice");
        let error = assign(windows_request(&caps, &sysfs, true, Some(&runner)), &SysfsBindOps)
            .unwrap_err();

        let GpuBindError::ApplyFailed(failure) = &error else {
            panic!("expected ApplyFailed, got {error:?}");
        };
        assert!(failure.rolled_back);
        assert!(failure.detail.contains("GPU-P failed"), "{}", failure.detail);
        assert!(failure.detail.contains("DDA failed"), "{}", failure.detail);
        assert!(!runner.leftover_assigned());
    }

    #[test]
    fn a_failed_rollback_does_not_try_the_other_method() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        // Set-VMGpuPartitionAdapter fails, then removing the adapter fails:
        // the GPU is half-assigned, so DDA must not run on top of it.
        let runner = RecordingHyperV::both(false)
            .fail_on("Set-VMGpuPartitionAdapter")
            .fail_on("Remove-VMGpuPartitionAdapter");
        let error = assign(windows_request(&caps, &sysfs, true, Some(&runner)), &SysfsBindOps)
            .unwrap_err();

        assert!(matches!(&error, GpuBindError::ApplyFailed(f) if !f.rolled_back), "{error:?}");
        assert!(!runner.names().iter().any(|n| n == "Dismount-VMHostAssignableDevice"));
    }

    #[test]
    fn methods_explain_why_neither_is_possible() {
        let probe = HyperVDeviceProbe {
            instance_id: "x".into(),
            location_path: None,
            dda_assignable: false,
            partitionable: true,
            partitionable_name: Some("n".into()),
            valid_partition_counts: vec![2, 4],
            partition_count: 4,
            server_edition: false,
        };
        let reason = probe.methods().unwrap_err().to_string();
        assert!(reason.contains("partition count 1"), "{reason}");
        assert!(reason.contains("not DDA-assignable"), "{reason}");
    }

    #[test]
    fn probe_finds_gpu_p_by_name_and_reports_the_edition() {
        let probe = probe_powershell(GPU_ID);
        // Partitionable GPU names use '#' where the instance id has '\'.
        assert!(probe.contains("$instanceId.Replace('\\', '#')"), "{probe}");
        assert!(probe.contains("Get-VMPartitionableGpu"), "older Windows 11 / 10 name");
        assert!(probe.contains("ProductType -ne 1"));
        assert!(probe.contains("serverEdition = $server"));

        let set = powershell_command(&HyperVCmdlet::SetHostPartitionCount {
            instance_path: "n".into(),
            count: 1,
        });
        assert!(set.contains("Set-VMHostPartitionableGpu -Name 'n' -PartitionCount 1"));
        assert!(set.contains("Set-VMPartitionableGpu -Name 'n' -PartitionCount 1"));

        let parsed = parse_hyperv_probe(
            r#"{"instanceId":"PCI-VEN_10DE","locationPath":null,"ddaAssignable":false,"partitionable":true,"partitionableName":"n","validPartitionCounts":[1],"partitionCount":1,"serverEdition":true}"#,
        )
        .unwrap();
        assert!(parsed.server_edition);
        assert!(!parse_hyperv_probe(r#"{"instanceId":"x"}"#).unwrap().server_edition);
    }

    #[test]
    fn windows_gpu_p_mid_step_failure_rolls_back_cleanly() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::gpu_p().fail_on("Set-VMGpuPartitionAdapter");
        let error = assign(
            windows_request(&caps, &sysfs, true, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap_err();

        match error {
            GpuBindError::ApplyFailed(ref failure) => assert!(failure.rolled_back),
            other => panic!("expected ApplyFailed, got {other:?}"),
        }
        let names = runner.names();
        assert!(names.contains(&"Add-VMGpuPartitionAdapter".to_string()));
        assert!(names.contains(&"Remove-VMGpuPartitionAdapter".to_string()));
        assert!(names.contains(&"Set-VMHostPartitionableGpu".to_string()));
        assert!(!runner.leftover_assigned());
    }

    #[test]
    fn windows_apply_without_runner_does_not_claim_assigned() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let error = assign(
            windows_request(&caps, &sysfs, true, None),
            &SysfsBindOps,
        )
        .unwrap_err();
        assert_eq!(error, GpuBindError::HyperVRunnerRequired);
    }

    #[test]
    fn windows_tests_cannot_spawn_live_powershell() {
        let caps = windows_pro_caps();
        let (_dir, sysfs) = mock_gpu_tree(true);
        let live = ProcessHyperVGpuOps::default();
        let error = assign(
            windows_request(&caps, &sysfs, true, Some(&live)),
            &SysfsBindOps,
        )
        .unwrap_err();
        assert_eq!(error, GpuBindError::LiveHyperVForbiddenInTests);
    }

    #[test]
    fn hyperv_powershell_uses_documented_dda_and_gpu_p_cmdlets() {
        let disable = powershell_command(&HyperVCmdlet::DisablePnpDevice {
            instance_id: INSTANCE_ID.to_string(),
        });
        assert!(disable.contains("Disable-PnpDevice"));
        assert!(disable.contains("-Confirm:$false"));

        let dismount = powershell_command(&HyperVCmdlet::DismountHostAssignable {
            location_path: LOCATION_PATH.to_string(),
        });
        assert!(dismount.contains("Dismount-VMHostAssignableDevice"));
        assert!(dismount.contains("-Force"));
        assert!(dismount.contains("-LocationPath"));

        let add = powershell_command(&HyperVCmdlet::AddVmAssignable {
            location_path: LOCATION_PATH.to_string(),
            vm_name: DEFAULT_HYPERV_VM.to_string(),
        });
        assert!(add.contains("Add-VMAssignableDevice"));
        assert!(add.contains("-VMName"));

        let set_part = powershell_command(&HyperVCmdlet::SetGpuPartitionExclusive {
            vm_name: DEFAULT_HYPERV_VM.to_string(),
        });
        assert!(set_part.contains("Set-VMGpuPartitionAdapter"));
        assert!(set_part.contains("MinPartitionVRAM"));
        assert!(set_part.contains(&GPU_P_FULL_PARTITION.to_string()));

        let probe = probe_powershell(GPU_ID);
        assert!(probe.contains("Get-PnpDevice"));
        assert!(probe.contains("DEVPKEY_Device_LocationPaths"));
        assert!(probe.contains("Get-VMHostPartitionableGpu"));
        assert!(!probe.contains("nvidia-smi"));
    }

    #[test]
    fn hyperv_timeout_explains_hang_safe_kill() {
        let error = GpuBindError::HyperVTimeout { timeout_secs: 30 };
        let text = error.to_string();
        assert!(text.contains("timed out after 30s"));
        assert!(text.contains("killed"));
        assert!(!text.to_ascii_lowercase().contains("assigned to"));
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
    fn macos_skips_gpu_assignment() {
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
        assert!(status.log.contains("does not fall back"));
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

    #[test]
    fn window_apply_is_false_linux_pro_provision_apply_is_true_when_offered() {
        assert!(!WINDOW_GPU_BIND_APPLY);
        assert!(linux_pro_provision_should_apply(
            HostOs::Linux,
            LabTier::Pro,
            true
        ));
        assert!(
            !linux_pro_provision_should_apply(HostOs::Linux, LabTier::Pro, false),
            "must not apply when the probe did not offer Pro"
        );
        assert!(!linux_pro_provision_should_apply(
            HostOs::Linux,
            LabTier::Standard,
            true
        ));
        assert!(!linux_pro_provision_should_apply(
            HostOs::Windows,
            LabTier::Pro,
            true
        ));
        assert!(!linux_pro_provision_should_apply(
            HostOs::MacOs,
            LabTier::Pro,
            true
        ));
        let source = include_str!("commands.rs");
        assert!(
            source.contains("WINDOW_GPU_BIND_APPLY"),
            "the window command must stay plan-only"
        );
        assert!(
            !source.contains("apply: true"),
            "the window must never pass apply: true"
        );
    }

    #[test]
    fn given_linux_pro_probe_apply_true_issues_unbind_new_id_and_bind() {
        let caps = linux_pro_caps();
        assert!(caps.pro.offered);
        assert!(caps.second_display_path);
        assert_eq!(caps.iommu, IommuStatus::Present);
        assert!(linux_pro_provision_should_apply(
            HostOs::Linux,
            LabTier::Pro,
            caps.pro.offered
        ));

        let (_dir, sysfs) = mock_gpu_tree(true);
        let status = assign(
            request(HostOs::Linux, &caps, &sysfs, true, true),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Bound);
        assert!(status.bound);
        assert!(status.exclusive);
        assert!(!status.hypervisor_invoked);
        assert_eq!(
            control_lines(&sysfs.driver_dir("nvidia").join("unbind")),
            vec![BDF]
        );
        assert_eq!(
            control_lines(&sysfs.vfio_pci.join("new_id")),
            vec!["10de 2786"]
        );
        assert_eq!(control_lines(&sysfs.vfio_pci.join("bind")), vec![BDF]);
        assert_eq!(
            read_current_driver(&sysfs.device_dir(BDF)).as_deref(),
            Some(VFIO_PCI)
        );
    }

    #[test]
    fn window_apply_is_false_windows_pro_provision_apply_is_true_when_offered() {
        assert!(!WINDOW_GPU_BIND_APPLY);
        assert!(windows_pro_provision_should_apply(
            HostOs::Windows,
            LabTier::Pro,
            true
        ));
        assert!(
            !windows_pro_provision_should_apply(HostOs::Windows, LabTier::Pro, false),
            "must not apply when the probe did not offer Pro"
        );
        assert!(!windows_pro_provision_should_apply(
            HostOs::Windows,
            LabTier::Standard,
            true
        ));
        assert!(!windows_pro_provision_should_apply(
            HostOs::Linux,
            LabTier::Pro,
            true
        ));
        assert!(!windows_pro_provision_should_apply(
            HostOs::MacOs,
            LabTier::Pro,
            true
        ));
        let source = include_str!("commands.rs");
        assert!(
            source.contains("WINDOW_GPU_BIND_APPLY"),
            "the window command must stay plan-only"
        );
        assert!(
            !source.contains("apply: true"),
            "the window must never pass apply: true"
        );
        assert!(
            source.contains("hyperv: None"),
            "the window must not spawn PowerShell"
        );
    }

    #[test]
    fn given_windows_pro_probe_apply_true_issues_dda_cmdlets() {
        let caps = windows_pro_caps();
        assert!(caps.pro.offered);
        assert!(caps.second_display_path);
        assert!(windows_pro_provision_should_apply(
            HostOs::Windows,
            LabTier::Pro,
            caps.pro.offered
        ));

        let (_dir, sysfs) = mock_gpu_tree(true);
        let runner = RecordingHyperV::dda();
        let status = assign(
            windows_request(&caps, &sysfs, true, Some(&runner)),
            &SysfsBindOps,
        )
        .unwrap();

        assert_eq!(status.action, GpuBindAction::Bound);
        assert!(status.bound);
        assert!(status.exclusive);
        assert!(!status.hypervisor_invoked);
        assert_eq!(status.target_driver.as_deref(), Some(DDA_TARGET));
        assert_eq!(
            runner.names(),
            vec![
                "Set-VM",
                "Disable-PnpDevice",
                "Dismount-VMHostAssignableDevice",
                "Add-VMAssignableDevice",
                "Get-VMAssignableDevice",
                "Start-VM",
            ]
        );
        assert!(runner.dda_assigned());
        assert!(status.log.to_ascii_lowercase().contains("start-vm"));
    }
}
