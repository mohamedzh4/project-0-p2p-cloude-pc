//! OS-conditional provisioner contract (stubs).
//!
//! Standard Lab backends are peers chosen by host OS: Linux → QEMU+KVM,
//! Windows → QEMU+WHPX. Pro is a *different* target (QEMU+VFIO or Hyper-V GPU
//! assignment), not the Standard backend with a GPU bolted on. GPU assignment is
//! exclusive for the session; CPU/RAM stay a concurrent slice.
//!
//! The HTTP port in `provision_http` verifies the signed image digest
//! ([`crate::image::verify_image`]) **before** calling [`plan`]. Linux Standard
//! provision may then start QEMU with KVM. Windows Standard may then
//! start QEMU with `-accel whpx` (phase 28). `plan` itself never invokes a
//! hypervisor — it only selects the backend. Do not put QEMU/WHPX
//! details in the session router.

use std::fmt;

use thiserror::Error;

/// Fake origin so a stub connection URL cannot be mistaken for a live Kasm
/// session. Phase 6 may replace this with a mock agent URL; it must not point
/// at a real hypervisor.
pub const STUB_CONNECTION_ORIGIN: &str = "https://provision.invalid";

/// Host OS used for backend selection. Passed in explicitly so tests can cover
/// Linux and Windows on a Mac without compiling those targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    Windows,
    /// Dev-only. Hosting is unsupported; selection fails rather than pretending
    /// this machine can host.
    MacOs,
    Other,
}

impl HostOs {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }

    pub fn supports_hosting(self) -> bool {
        matches!(self, Self::Linux | Self::Windows)
    }
}

impl fmt::Display for HostOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Linux => "Linux",
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Other => "this OS",
        })
    }
}

/// Lab tier. Independent of the Linux/Windows backend split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabTier {
    Standard,
    Pro,
}

impl LabTier {
    pub fn parse(value: &str) -> Result<Self, ProvisionError> {
        match value {
            "standard" => Ok(Self::Standard),
            "pro" => Ok(Self::Pro),
            other => Err(ProvisionError::UnknownLabTier(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Pro => "pro",
        }
    }
}

/// Peer compute backends. None of these is a default the others special-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeBackend {
    QemuKvm,
    QemuWhpx,
    QemuVfio,
    HyperVGpu,
}

impl ComputeBackend {
    pub fn slug(self) -> &'static str {
        match self {
            Self::QemuKvm => "qemu-kvm",
            Self::QemuWhpx => "qemu-whpx",
            Self::QemuVfio => "qemu-vfio",
            Self::HyperVGpu => "hyperv-gpu",
        }
    }

    /// Pro backends assign one whole GPU to the guest. Standard never does.
    pub fn assigns_gpu_exclusively(self) -> bool {
        matches!(self, Self::QemuVfio | Self::HyperVGpu)
    }
}

impl fmt::Display for ComputeBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::QemuKvm => "QEMU+KVM",
            Self::QemuWhpx => "QEMU+WHPX",
            Self::QemuVfio => "QEMU+VFIO",
            Self::HyperVGpu => "Hyper-V GPU assignment",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionRequest {
    pub lab_tier: LabTier,
    /// Required for Pro. Ignored for Standard (CPU/RAM slice only).
    pub shared_gpu_id: Option<String>,
}

impl ProvisionRequest {
    pub fn standard() -> Self {
        Self {
            lab_tier: LabTier::Standard,
            shared_gpu_id: None,
        }
    }

    pub fn pro(shared_gpu_id: impl Into<String>) -> Self {
        Self {
            lab_tier: LabTier::Pro,
            shared_gpu_id: Some(shared_gpu_id.into()),
        }
    }
}

/// Stub outcome. Phase 6 should take [`Self::connection_url`] as the session
/// connection until a real agent reports ready. [`Self::hypervisor_invoked`] is
/// always false in this phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionPlan {
    pub backend: ComputeBackend,
    pub lab_tier: LabTier,
    /// True only for Pro: the named GPU is assigned to the guest and the host
    /// loses it for the session. Never a concurrent slice like CPU/RAM.
    pub gpu_exclusive: bool,
    pub shared_gpu_id: Option<String>,
    /// Set on Pro so exclusivity is visible at plan time, not only in UI copy.
    pub exclusivity_warning: Option<String>,
    pub connection_url: String,
    pub hypervisor_invoked: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProvisionError {
    #[error(
        "hosting is not supported on {0}; this agent does not fall back to another backend"
    )]
    UnsupportedHostOs(HostOs),
    #[error(
        "a Pro session needs a dedicated GPU id (sharedGpuId); GPU assignment is exclusive for the session, not a concurrent slice"
    )]
    GpuIdRequired,
    #[error("unknown lab tier {0:?}; expected \"standard\" or \"pro\"")]
    UnknownLabTier(String),
    #[error("provision request for {tier} cannot run on {backend}")]
    TierBackendMismatch {
        tier: &'static str,
        backend: &'static str,
    },
}

/// One stub backend. Linux/Windows Standard and Pro are all first-class
/// implementors — Windows is not a branch inside the Linux path.
pub trait Provisioner {
    fn backend(&self) -> ComputeBackend;
    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QemuKvmStub;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QemuWhpxStub;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QemuVfioStub;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HyperVGpuStub;

impl Provisioner for QemuKvmStub {
    fn backend(&self) -> ComputeBackend {
        ComputeBackend::QemuKvm
    }

    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
        standard_plan(self.backend(), request)
    }
}

impl Provisioner for QemuWhpxStub {
    fn backend(&self) -> ComputeBackend {
        ComputeBackend::QemuWhpx
    }

    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
        standard_plan(self.backend(), request)
    }
}

impl Provisioner for QemuVfioStub {
    fn backend(&self) -> ComputeBackend {
        ComputeBackend::QemuVfio
    }

    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
        pro_plan(self.backend(), request)
    }
}

impl Provisioner for HyperVGpuStub {
    fn backend(&self) -> ComputeBackend {
        ComputeBackend::HyperVGpu
    }

    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
        pro_plan(self.backend(), request)
    }
}

/// Peer enum over the stubs. Selection is a match on `(HostOs, LabTier)`, not
/// a Linux path with a Windows `if`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubProvisioner {
    QemuKvm(QemuKvmStub),
    QemuWhpx(QemuWhpxStub),
    QemuVfio(QemuVfioStub),
    HyperVGpu(HyperVGpuStub),
}

impl StubProvisioner {
    pub fn select(os: HostOs, lab_tier: LabTier) -> Result<Self, ProvisionError> {
        match (os, lab_tier) {
            (HostOs::Linux, LabTier::Standard) => Ok(Self::QemuKvm(QemuKvmStub)),
            (HostOs::Windows, LabTier::Standard) => Ok(Self::QemuWhpx(QemuWhpxStub)),
            (HostOs::Linux, LabTier::Pro) => Ok(Self::QemuVfio(QemuVfioStub)),
            (HostOs::Windows, LabTier::Pro) => Ok(Self::HyperVGpu(HyperVGpuStub)),
            (os, _) => Err(ProvisionError::UnsupportedHostOs(os)),
        }
    }
}

impl Provisioner for StubProvisioner {
    fn backend(&self) -> ComputeBackend {
        match self {
            Self::QemuKvm(inner) => inner.backend(),
            Self::QemuWhpx(inner) => inner.backend(),
            Self::QemuVfio(inner) => inner.backend(),
            Self::HyperVGpu(inner) => inner.backend(),
        }
    }

    fn plan(&self, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
        match self {
            Self::QemuKvm(inner) => inner.plan(request),
            Self::QemuWhpx(inner) => inner.plan(request),
            Self::QemuVfio(inner) => inner.plan(request),
            Self::HyperVGpu(inner) => inner.plan(request),
        }
    }
}

/// Phase 6 entry point: choose the OS-conditional backend and return a plan.
/// Never starts a hypervisor, Kasm, Sunshine, or VFIO. Linux Standard
/// invoke lives in [`crate::qemu_kvm`] via the provision HTTP port.
/// Windows Standard invoke lives in [`crate::qemu_whpx`]. Linux Pro vfio-pci apply
/// and Windows Pro Hyper-V DDA/GPU-P apply live in [`crate::gpu_bind`] via
/// the same HTTP port (not `plan()`).
pub fn plan(os: HostOs, request: &ProvisionRequest) -> Result<ProvisionPlan, ProvisionError> {
    StubProvisioner::select(os, request.lab_tier)?.plan(request)
}

fn standard_plan(
    backend: ComputeBackend,
    request: &ProvisionRequest,
) -> Result<ProvisionPlan, ProvisionError> {
    if request.lab_tier != LabTier::Standard {
        return Err(ProvisionError::TierBackendMismatch {
            tier: request.lab_tier.as_str(),
            backend: backend.slug(),
        });
    }

    Ok(ProvisionPlan {
        backend,
        lab_tier: LabTier::Standard,
        gpu_exclusive: false,
        shared_gpu_id: None,
        exclusivity_warning: None,
        connection_url: stub_connection_url(backend),
        hypervisor_invoked: false,
    })
}

fn pro_plan(
    backend: ComputeBackend,
    request: &ProvisionRequest,
) -> Result<ProvisionPlan, ProvisionError> {
    if request.lab_tier != LabTier::Pro {
        return Err(ProvisionError::TierBackendMismatch {
            tier: request.lab_tier.as_str(),
            backend: backend.slug(),
        });
    }

    let gpu_id = dedicated_gpu_id(request.shared_gpu_id.as_deref())?;

    Ok(ProvisionPlan {
        backend,
        lab_tier: LabTier::Pro,
        gpu_exclusive: true,
        shared_gpu_id: Some(gpu_id.to_string()),
        exclusivity_warning: Some(exclusivity_warning(gpu_id)),
        connection_url: stub_connection_url(backend),
        hypervisor_invoked: false,
    })
}

fn dedicated_gpu_id(shared_gpu_id: Option<&str>) -> Result<&str, ProvisionError> {
    match shared_gpu_id.map(str::trim) {
        Some(id) if !id.is_empty() => Ok(id),
        _ => Err(ProvisionError::GpuIdRequired),
    }
}

fn exclusivity_warning(gpu_id: &str) -> String {
    format!(
        "GPU {gpu_id} is assigned exclusively to the guest for this Pro session; \
         the host loses that GPU until the session ends. CPU and RAM remain a concurrent slice."
    )
}

fn stub_connection_url(backend: ComputeBackend) -> String {
    format!("{STUB_CONNECTION_ORIGIN}/{}", backend.slug())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPU_ID: &str = "GPU-TEST-4070";

    #[test]
    fn linux_standard_selects_qemu_kvm() {
        let selected = StubProvisioner::select(HostOs::Linux, LabTier::Standard).unwrap();
        let planned = selected.plan(&ProvisionRequest::standard()).unwrap();

        assert_eq!(selected.backend(), ComputeBackend::QemuKvm);
        assert_eq!(planned.backend, ComputeBackend::QemuKvm);
        assert_eq!(planned.lab_tier, LabTier::Standard);
        assert!(!planned.gpu_exclusive);
        assert!(planned.shared_gpu_id.is_none());
        assert!(planned.exclusivity_warning.is_none());
        assert!(!planned.hypervisor_invoked);
        assert_eq!(
            planned.connection_url,
            "https://provision.invalid/qemu-kvm"
        );
    }

    #[test]
    fn windows_standard_selects_qemu_whpx() {
        let selected = StubProvisioner::select(HostOs::Windows, LabTier::Standard).unwrap();
        let planned = selected.plan(&ProvisionRequest::standard()).unwrap();

        assert_eq!(selected.backend(), ComputeBackend::QemuWhpx);
        assert_eq!(planned.backend, ComputeBackend::QemuWhpx);
        assert!(!planned.gpu_exclusive);
        assert!(!planned.hypervisor_invoked);
        assert_eq!(
            planned.connection_url,
            "https://provision.invalid/qemu-whpx"
        );
    }

    #[test]
    fn standard_backends_are_peers_not_a_windows_special_case() {
        let linux = StubProvisioner::select(HostOs::Linux, LabTier::Standard).unwrap();
        let windows = StubProvisioner::select(HostOs::Windows, LabTier::Standard).unwrap();

        assert_eq!(linux.backend(), ComputeBackend::QemuKvm);
        assert_eq!(windows.backend(), ComputeBackend::QemuWhpx);
        assert_ne!(linux.backend(), windows.backend());
        assert!(!linux.backend().assigns_gpu_exclusively());
        assert!(!windows.backend().assigns_gpu_exclusively());
    }

    #[test]
    fn linux_pro_is_qemu_vfio_not_standard_qemu_kvm() {
        let standard = StubProvisioner::select(HostOs::Linux, LabTier::Standard).unwrap();
        let pro = StubProvisioner::select(HostOs::Linux, LabTier::Pro).unwrap();
        let planned = pro.plan(&ProvisionRequest::pro(GPU_ID)).unwrap();

        assert_eq!(standard.backend(), ComputeBackend::QemuKvm);
        assert_eq!(pro.backend(), ComputeBackend::QemuVfio);
        assert_ne!(pro.backend(), ComputeBackend::QemuKvm);
        assert!(pro.backend().assigns_gpu_exclusively());
        assert_eq!(planned.backend, ComputeBackend::QemuVfio);
        assert!(planned.gpu_exclusive);
        assert_eq!(planned.shared_gpu_id.as_deref(), Some(GPU_ID));
        assert!(planned
            .exclusivity_warning
            .as_deref()
            .is_some_and(|warning| {
                warning.contains(GPU_ID) && warning.contains("exclusively")
            }));
        assert!(!planned.hypervisor_invoked);
    }

    #[test]
    fn windows_pro_is_hyperv_gpu_not_standard_qemu_whpx() {
        let standard = StubProvisioner::select(HostOs::Windows, LabTier::Standard).unwrap();
        let pro = StubProvisioner::select(HostOs::Windows, LabTier::Pro).unwrap();
        let planned = pro.plan(&ProvisionRequest::pro(GPU_ID)).unwrap();

        assert_eq!(standard.backend(), ComputeBackend::QemuWhpx);
        assert_eq!(pro.backend(), ComputeBackend::HyperVGpu);
        assert!(pro.backend().assigns_gpu_exclusively());
        assert_eq!(planned.backend, ComputeBackend::HyperVGpu);
        assert!(planned.gpu_exclusive);
        assert_eq!(planned.shared_gpu_id.as_deref(), Some(GPU_ID));
        assert!(!planned.hypervisor_invoked);
    }

    #[test]
    fn pro_refuses_to_proceed_without_a_gpu_id() {
        let request = ProvisionRequest {
            lab_tier: LabTier::Pro,
            shared_gpu_id: None,
        };

        assert_eq!(
            plan(HostOs::Linux, &request),
            Err(ProvisionError::GpuIdRequired)
        );
        assert_eq!(
            plan(HostOs::Windows, &request),
            Err(ProvisionError::GpuIdRequired)
        );
    }

    #[test]
    fn pro_refuses_a_blank_gpu_id() {
        for blank in ["", "   ", "\t"] {
            let request = ProvisionRequest::pro(blank);
            assert_eq!(
                plan(HostOs::Linux, &request),
                Err(ProvisionError::GpuIdRequired),
                "blank id {blank:?} must not count as a dedicated GPU"
            );
        }
    }

    #[test]
    fn standard_ignores_a_gpu_id_and_stays_a_cpu_ram_slice() {
        let request = ProvisionRequest {
            lab_tier: LabTier::Standard,
            shared_gpu_id: Some(GPU_ID.to_string()),
        };
        let planned = plan(HostOs::Linux, &request).unwrap();

        assert_eq!(planned.backend, ComputeBackend::QemuKvm);
        assert!(!planned.gpu_exclusive);
        assert!(planned.shared_gpu_id.is_none());
        assert!(planned.exclusivity_warning.is_none());
    }

    #[test]
    fn macos_does_not_pretend_to_be_a_linux_host() {
        let error = StubProvisioner::select(HostOs::MacOs, LabTier::Standard).unwrap_err();

        assert_eq!(error, ProvisionError::UnsupportedHostOs(HostOs::MacOs));
        assert!(
            !error.to_string().is_empty() && error.to_string().contains("macOS"),
            "unsupported OS must fail clearly: {error}"
        );
        assert!(plan(HostOs::MacOs, &ProvisionRequest::standard()).is_err());
        assert!(plan(HostOs::MacOs, &ProvisionRequest::pro(GPU_ID)).is_err());
    }

    #[test]
    fn other_os_is_unsupported_for_both_tiers() {
        assert_eq!(
            StubProvisioner::select(HostOs::Other, LabTier::Standard),
            Err(ProvisionError::UnsupportedHostOs(HostOs::Other))
        );
        assert_eq!(
            StubProvisioner::select(HostOs::Other, LabTier::Pro),
            Err(ProvisionError::UnsupportedHostOs(HostOs::Other))
        );
    }

    #[test]
    fn plan_never_invokes_a_hypervisor() {
        let cases = [
            plan(HostOs::Linux, &ProvisionRequest::standard()).unwrap(),
            plan(HostOs::Windows, &ProvisionRequest::standard()).unwrap(),
            plan(HostOs::Linux, &ProvisionRequest::pro(GPU_ID)).unwrap(),
            plan(HostOs::Windows, &ProvisionRequest::pro(GPU_ID)).unwrap(),
        ];

        for planned in cases {
            assert!(
                !planned.hypervisor_invoked,
                "{:?} must stay a stub",
                planned.backend
            );
            assert!(planned.connection_url.starts_with(STUB_CONNECTION_ORIGIN));
        }
    }

    #[test]
    fn current_host_os_matches_the_compile_target() {
        let os = HostOs::current();
        #[cfg(target_os = "linux")]
        assert_eq!(os, HostOs::Linux);
        #[cfg(target_os = "windows")]
        assert_eq!(os, HostOs::Windows);
        #[cfg(target_os = "macos")]
        {
            assert_eq!(os, HostOs::MacOs);
            assert!(!os.supports_hosting());
        }
    }

    #[test]
    fn parses_known_lab_tiers() {
        assert_eq!(LabTier::parse("standard"), Ok(LabTier::Standard));
        assert_eq!(LabTier::parse("pro"), Ok(LabTier::Pro));
        assert!(matches!(
            LabTier::parse("gamer"),
            Err(ProvisionError::UnknownLabTier(value)) if value == "gamer"
        ));
    }

    #[test]
    fn public_plan_entry_matches_select() {
        let via_plan = plan(HostOs::Windows, &ProvisionRequest::pro(GPU_ID)).unwrap();
        let via_select = StubProvisioner::select(HostOs::Windows, LabTier::Pro)
            .unwrap()
            .plan(&ProvisionRequest::pro(GPU_ID))
            .unwrap();

        assert_eq!(via_plan, via_select);
        assert_eq!(via_plan.backend, ComputeBackend::HyperVGpu);
        assert!(via_plan.gpu_exclusive);
    }
}
