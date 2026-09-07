//! Inbound provision HTTP port. Verifies the signed image digest
//! ([`crate::image::verify_image`]) **before** calling [`crate::provision::plan`].
//! Linux Standard then starts Firecracker when the probe offered it and the
//! binary, kernel, and verified rootfs are present. Other backends stay stubs.
//! The Matchmaker POSTs here after host selection. The webview must not `fetch`
//! this URL (CSP allows no remote origins).

use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::agent_status::Occupancy;
use crate::eligibility::{self, EligibilityRequest};
use crate::firecracker::{self, FirecrackerError, FirecrackerRuntime};
use crate::host_capabilities::HostCapabilities;
use crate::image::{
    trust_from_env, verify_image, GuestImageStatus, ImageError, ImageManifest, ImageTrust,
};
use crate::launch::{self, LaunchError, LaunchRequest};
use crate::models::{HardwareSnapshot, SharingConfig};
use crate::provision::{plan, HostOs, LabTier, ProvisionError, ProvisionPlan, ProvisionRequest};
use crate::storage::GuestRcloneMount;

const PROVISION_PATH: &str = "/v1/provision";
const BODY_MAX: usize = 16 * 1024;
const STREAMER_KASM: &str = "kasm";
const STREAMER_SUNSHINE: &str = "sunshine";
const STUB_SUNSHINE_URL: &str = "https://provision.invalid/sunshine";
const STUB_MOONLIGHT_HOST: &str = "provision.invalid";
const STUB_MOONLIGHT_APP: &str = "Desktop";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProvisionListenError {
    #[error("HOST_BOOTSTRAP_TOKEN is not set. The provision listener does not hardcode a token.")]
    MissingToken,
    #[error("HOST_AGENT_LISTEN_ADDR must be host:port (or \"off\" to disable)")]
    InvalidAddr,
    #[error("could not bind the provision listener at {addr}: {detail}")]
    Bind { addr: String, detail: String },
    #[error(transparent)]
    Image(#[from] ImageError),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanHttpError {
    #[error(transparent)]
    Provision(#[from] ProvisionError),
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("{0}")]
    NotOffered(String),
    #[error("{0}")]
    Launch(#[from] LaunchError),
    #[error("{0}")]
    Firecracker(#[from] FirecrackerError),
}

/// Probe + spare CPU/RAM/disk used to refuse a stub provision instead of
/// half-planning an unoffered tier. Owned so the listener thread can hold it.
#[derive(Debug, Clone)]
pub struct ProvisionGate {
    pub capabilities: HostCapabilities,
    pub hardware: HardwareSnapshot,
    pub sharing: Option<SharingConfig>,
    pub spare_disk_bytes: Option<u64>,
    /// When set on Linux Standard, InstanceStart runs (or provision fails).
    pub firecracker: Option<FirecrackerBoot>,
}

/// Inputs for a real Firecracker invoke. Tests inject [`FirecrackerRuntime`].
#[derive(Clone)]
pub struct FirecrackerBoot {
    pub binary: PathBuf,
    pub image_cache_dir: PathBuf,
    pub guest_image: GuestImageStatus,
    pub kernel_override: Option<PathBuf>,
    pub arch: String,
    pub runtime: Arc<dyn FirecrackerRuntime + Send + Sync>,
}

impl std::fmt::Debug for FirecrackerBoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirecrackerBoot")
            .field("binary", &self.binary)
            .field("image_cache_dir", &self.image_cache_dir)
            .field("guest_image", &self.guest_image)
            .field("kernel_override", &self.kernel_override)
            .field("arch", &self.arch)
            .field("runtime", &"FirecrackerRuntime")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ListenConfig {
    pub addr: String,
    token: String,
    pub image_trust: ImageTrust,
}

impl ListenConfig {
    pub fn new(addr: impl Into<String>, token: impl Into<String>, image_trust: ImageTrust) -> Self {
        Self {
            addr: addr.into(),
            token: token.into(),
            image_trust,
        }
    }
}

impl std::fmt::Debug for ListenConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenConfig")
            .field("addr", &self.addr)
            .field("token", &"[redacted]")
            .field("image_trust", &self.image_trust)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionHttpRequest {
    pub lab_tier: String,
    pub shared_gpu_id: Option<String>,
    pub image_digest: String,
    pub image_signature: String,
    /// renterd S3 locators the **guest** should rclone. Never mount on this host OS.
    #[serde(default)]
    pub storage_endpoint: Option<String>,
    #[serde(default)]
    pub storage_bucket: Option<String>,
    #[serde(default)]
    pub storage_prefix: Option<String>,
    #[serde(default)]
    pub storage_region: Option<String>,
    /// Guest mesh join hint (no preauth). Host Agent must not `tailscale up` these.
    #[serde(default)]
    pub guest_mesh: Option<GuestMeshHint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestMeshHint {
    pub login_server: Option<String>,
    pub hostname: Option<String>,
    pub exit_node: Option<String>,
    #[serde(default)]
    pub acl_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoonlightHint {
    pub host: String,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionHttpResponse {
    pub connection_url: String,
    pub hypervisor_invoked: bool,
    pub gpu_exclusive: bool,
    pub streamer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunshine_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moonlight: Option<MoonlightHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guest_rclone: Option<GuestRclonePlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestRclonePlan {
    pub args: Vec<String>,
    pub mount_point: String,
    pub remote: String,
    pub env_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProvisionOutput {
    pub status: u16,
    pub body: String,
}

/// Env: `HOST_AGENT_LISTEN_ADDR=127.0.0.1:7842` (or `off`). Token and image
/// trust are required if listening (fail fast; no hardcoded key).
pub fn listen_config_from_env<F>(
    mut lookup: F,
) -> Result<Option<ListenConfig>, ProvisionListenError>
where
    F: FnMut(&str) -> Option<String>,
{
    let addr = nonempty(lookup("HOST_AGENT_LISTEN_ADDR"));
    let Some(addr) = addr else {
        return Ok(None);
    };
    if addr.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    if addr.parse::<SocketAddr>().is_err() {
        return Err(ProvisionListenError::InvalidAddr);
    }
    let token =
        nonempty(lookup("HOST_BOOTSTRAP_TOKEN")).ok_or(ProvisionListenError::MissingToken)?;
    let image_trust = trust_from_env(|key| lookup(key))?;
    Ok(Some(ListenConfig::new(addr, token, image_trust)))
}

/// Verify the advertised image, then select the OS-conditional backend.
/// Linux Standard with [`ProvisionGate::firecracker`] starts Firecracker
/// (InstanceStart) or fails clearly. Digest mismatch never boots.
pub fn plan_http(
    os: HostOs,
    request: &ProvisionHttpRequest,
    trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
) -> Result<ProvisionHttpResponse, PlanHttpError> {
    let advertised = ImageManifest::parse(&request.image_digest, &request.image_signature)?;
    verify_image(trust, &advertised, None)?;
    let lab_tier = LabTier::parse(&request.lab_tier)?;
    if let Some(gate) = gate {
        refuse_unoffered_tier(lab_tier, request, gate)?;
    }
    // Locators are for a guest image. This path never rclone-mounts on the
    // volunteer host OS and never `tailscale up`s guestMesh (architecture §8/§9).
    let guest_rclone = guest_rclone_from_request(request);
    ignore_guest_mesh_on_host(request.guest_mesh.as_ref());
    let mut planned = plan(
        os,
        &ProvisionRequest {
            lab_tier,
            shared_gpu_id: request.shared_gpu_id.clone(),
        },
    )?;
    if os == HostOs::Linux && lab_tier == LabTier::Standard {
        if let Some(boot) = gate.and_then(|gate| gate.firecracker.as_ref()) {
            let gate = gate.expect("firecracker boot requires a gate");
            planned = invoke_linux_standard(os, request, gate, boot, planned)?;
        }
    }
    Ok(http_response_from_plan(&planned, guest_rclone))
}

fn invoke_linux_standard(
    os: HostOs,
    request: &ProvisionHttpRequest,
    gate: &ProvisionGate,
    boot: &FirecrackerBoot,
    mut planned: ProvisionPlan,
) -> Result<ProvisionPlan, PlanHttpError> {
    let sharing = sharing_for_gate(gate, request);
    let launch = launch::build(LaunchRequest {
        os,
        lab_tier: LabTier::Standard,
        hardware: &gate.hardware,
        sharing: &sharing,
        image_cache_dir: Some(boot.image_cache_dir.as_path()),
        guest_image: Some(&boot.guest_image),
        capabilities: Some(&gate.capabilities),
        spare_disk_bytes: gate.spare_disk_bytes,
        kernel_override: boot.kernel_override.as_deref(),
    })?;
    let kernel_env = boot
        .kernel_override
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    firecracker::boot_from_launch(
        os,
        LabTier::Standard,
        &gate.capabilities,
        &launch,
        &boot.binary,
        |key| {
            if key == firecracker::KERNEL_ENV {
                kernel_env.clone()
            } else {
                None
            }
        },
        &boot.image_cache_dir,
        boot.arch.as_str(),
        boot.runtime.as_ref(),
    )?;
    planned.hypervisor_invoked = true;
    Ok(planned)
}

fn refuse_unoffered_tier(
    lab_tier: LabTier,
    request: &ProvisionHttpRequest,
    gate: &ProvisionGate,
) -> Result<(), PlanHttpError> {
    let sharing = sharing_for_gate(gate, request);
    let report = eligibility::evaluate(EligibilityRequest {
        capabilities: &gate.capabilities,
        hardware: &gate.hardware,
        sharing: &sharing,
        spare_disk_bytes: gate.spare_disk_bytes,
        gpu_bind: None,
    });
    let offer = eligibility::offer_for_tier(&report, lab_tier);
    if !offer.offered {
        return Err(PlanHttpError::NotOffered(offer.reason.clone()));
    }
    Ok(())
}

fn sharing_for_gate(gate: &ProvisionGate, request: &ProvisionHttpRequest) -> SharingConfig {
    gate.sharing.clone().unwrap_or_else(|| SharingConfig {
        version: crate::models::CONFIG_VERSION,
        sharing_enabled: true,
        shared_logical_cores: gate.hardware.cpu.logical_cores,
        shared_memory_bytes: gate.hardware.memory.total_bytes,
        share_gpu: request
            .shared_gpu_id
            .as_deref()
            .is_some_and(|id| !id.is_empty()),
        shared_gpu_id: request.shared_gpu_id.clone(),
    })
}

fn ignore_guest_mesh_on_host(hint: Option<&GuestMeshHint>) {
    // Accepted on the wire for a future guest image. Never `tailscale up` here.
    if let Some(mesh) = hint {
        let _ = (
            mesh.login_server.as_deref(),
            mesh.hostname.as_deref(),
            mesh.exit_node.as_deref(),
            mesh.acl_tags.len(),
        );
    }
}

fn guest_rclone_from_request(request: &ProvisionHttpRequest) -> Option<GuestRclonePlan> {
    let endpoint = nonempty(request.storage_endpoint.clone())?;
    let bucket = nonempty(request.storage_bucket.clone())?;
    let prefix = request
        .storage_prefix
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let region =
        nonempty(request.storage_region.clone()).unwrap_or_else(|| "us-east-1".to_string());
    let mount = GuestRcloneMount::from_locators(endpoint, bucket, prefix, region, None).ok()?;
    Some(GuestRclonePlan {
        args: mount.rclone_args(),
        mount_point: mount.mount_point.clone(),
        remote: mount.remote(),
        env_auth: true,
    })
}

fn http_response_from_plan(
    planned: &ProvisionPlan,
    guest_rclone: Option<GuestRclonePlan>,
) -> ProvisionHttpResponse {
    if planned.lab_tier == LabTier::Pro {
        ProvisionHttpResponse {
            connection_url: planned.connection_url.clone(),
            hypervisor_invoked: planned.hypervisor_invoked,
            gpu_exclusive: planned.gpu_exclusive,
            streamer: STREAMER_SUNSHINE.to_string(),
            sunshine_url: Some(STUB_SUNSHINE_URL.to_string()),
            moonlight: Some(MoonlightHint {
                host: STUB_MOONLIGHT_HOST.to_string(),
                app: STUB_MOONLIGHT_APP.to_string(),
            }),
            guest_rclone,
        }
    } else {
        ProvisionHttpResponse {
            connection_url: planned.connection_url.clone(),
            hypervisor_invoked: planned.hypervisor_invoked,
            gpu_exclusive: planned.gpu_exclusive,
            streamer: STREAMER_KASM.to_string(),
            sunshine_url: None,
            moonlight: None,
            guest_rclone,
        }
    }
}

pub fn respond(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
    image_trust: &ImageTrust,
) -> HttpProvisionOutput {
    respond_with_gate(
        method,
        path,
        authorization,
        body,
        expected_token,
        os,
        image_trust,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn respond_with_gate(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
    image_trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
) -> HttpProvisionOutput {
    let path = path.split('?').next().unwrap_or(path);
    if !method.eq_ignore_ascii_case("POST") {
        return text(405, "method not allowed");
    }
    if path != PROVISION_PATH {
        return text(404, "not found");
    }
    if !bearer_matches(authorization, expected_token) {
        return text(401, "Not authenticated");
    }
    if body.len() > BODY_MAX {
        return text(413, "payload too large");
    }

    let parsed: ProvisionHttpRequest = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return text(400, "invalid provision request"),
    };

    match plan_http(os, &parsed, image_trust, gate) {
        Ok(planned) => {
            let json = serde_json::to_string(&planned).unwrap_or_else(|_| {
                "{\"connectionUrl\":\"\",\"hypervisorInvoked\":false,\"gpuExclusive\":false,\"streamer\":\"kasm\"}"
                    .to_string()
            });
            HttpProvisionOutput {
                status: 200,
                body: json,
            }
        }
        Err(error) => text(400, &error.to_string()),
    }
}

/// Same as [`respond`], then records local occupancy on HTTP 200 so the tray
/// can show in-session. Pause still wins in [`crate::agent_status::presence`].
#[allow(clippy::too_many_arguments)]
pub fn respond_and_track(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
    image_trust: &ImageTrust,
) -> HttpProvisionOutput {
    respond_and_track_with_gate(
        method,
        path,
        authorization,
        body,
        expected_token,
        os,
        occupancy,
        image_trust,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn respond_and_track_with_gate(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
    image_trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
) -> HttpProvisionOutput {
    let output = respond_with_gate(
        method,
        path,
        authorization,
        body,
        expected_token,
        os,
        image_trust,
        gate,
    );
    if output.status == 200 {
        occupancy.mark_in_session();
    }
    output
}

pub fn serve(
    config: &ListenConfig,
    os: HostOs,
    occupancy: &Occupancy,
    gate: Option<ProvisionGate>,
) -> Result<(), ProvisionListenError> {
    let server = Server::http(&config.addr).map_err(|error| ProvisionListenError::Bind {
        addr: config.addr.clone(),
        detail: error.to_string(),
    })?;
    for request in server.incoming_requests() {
        let _ = dispatch(
            request,
            &config.token,
            os,
            occupancy,
            &config.image_trust,
            gate.as_ref(),
        );
    }
    Ok(())
}

fn dispatch(
    mut request: Request,
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
    image_trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
) -> std::io::Result<()> {
    let method = method_name(request.method());
    let url = request.url().to_string();
    let authorization = header_value(&request, "Authorization");
    let mut body = Vec::new();
    let _ = request
        .as_reader()
        .take(BODY_MAX as u64 + 1)
        .read_to_end(&mut body);

    let output = respond_and_track_with_gate(
        &method,
        &url,
        authorization.as_deref(),
        &body,
        expected_token,
        os,
        occupancy,
        image_trust,
        gate,
    );
    let mut response =
        Response::from_string(output.body).with_status_code(StatusCode(output.status));
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]) {
        response = response.with_header(header);
    }
    request.respond(response)
}

fn method_name(method: &Method) -> String {
    match method {
        Method::Get => "GET".into(),
        Method::Head => "HEAD".into(),
        Method::Post => "POST".into(),
        Method::Put => "PUT".into(),
        Method::Delete => "DELETE".into(),
        Method::Connect => "CONNECT".into(),
        Method::Options => "OPTIONS".into(),
        Method::Trace => "TRACE".into(),
        Method::Patch => "PATCH".into(),
        Method::NonStandard(value) => value.as_str().to_string(),
    }
}

fn header_value(request: &Request, name: &'static str) -> Option<String> {
    request.headers().iter().find_map(|header| {
        if header.field.equiv(name) {
            Some(header.value.as_str().to_string())
        } else {
            None
        }
    })
}

fn bearer_matches(authorization: Option<&str>, expected: &str) -> bool {
    let Some(header) = authorization else {
        return false;
    };
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .unwrap_or("");
    if token.len() != expected.len() {
        return false;
    }
    token
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn text(status: u16, message: &str) -> HttpProvisionOutput {
    HttpProvisionOutput {
        status,
        body: serde_json::json!({ "detail": message }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::tests::{signed_fixture, FIXTURE_PRO_BYTES, FIXTURE_STANDARD_BYTES};
    use crate::image::Sha256Digest;
    use crate::provision::STUB_CONNECTION_ORIGIN;
    use std::thread;

    const TOKEN: &str = "test-host-bootstrap-token";

    fn unsigned_standard_body() -> Vec<u8> {
        serde_json::json!({ "labTier": "standard", "sharedGpuId": null })
            .to_string()
            .into_bytes()
    }

    fn signed_standard() -> (ImageTrust, Vec<u8>) {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "sharedGpuId": null,
            "imageDigest": manifest.digest_hex(),
            "imageSignature": manifest.signature_base64(),
        })
        .to_string()
        .into_bytes();
        (trust, body)
    }

    #[test]
    fn linux_standard_returns_stub_url_without_invoking_a_hypervisor() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(body["gpuExclusive"], false);
        assert_eq!(body["streamer"], "kasm");
        assert!(body.get("sunshineUrl").is_none());
        assert!(body.get("moonlight").is_none());
    }

    #[test]
    fn windows_standard_returns_a_peer_stub_url() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Windows,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/qemu-whpx")
        );
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(body["streamer"], "kasm");
    }

    fn signed_pro() -> (ImageTrust, Vec<u8>) {
        let (trust, manifest) = signed_fixture(FIXTURE_PRO_BYTES);
        let body = serde_json::json!({
            "labTier": "pro",
            "sharedGpuId": "GPU-TEST-4070",
            "imageDigest": manifest.digest_hex(),
            "imageSignature": manifest.signature_base64(),
        })
        .to_string()
        .into_bytes();
        (trust, body)
    }

    #[test]
    fn linux_pro_returns_sunshine_not_kasm() {
        let (trust, body) = signed_pro();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/qemu-vfio")
        );
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(body["gpuExclusive"], true);
        assert_eq!(body["streamer"], "sunshine");
        assert_eq!(body["sunshineUrl"], STUB_SUNSHINE_URL);
        assert_eq!(body["moonlight"]["host"], STUB_MOONLIGHT_HOST);
        assert_eq!(body["moonlight"]["app"], STUB_MOONLIGHT_APP);
    }

    #[test]
    fn windows_pro_returns_hyperv_gpu_with_sunshine() {
        let (trust, body) = signed_pro();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Windows,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/hyperv-gpu")
        );
        assert_eq!(body["streamer"], "sunshine");
        assert_eq!(body["gpuExclusive"], true);
        assert_eq!(body["sunshineUrl"], STUB_SUNSHINE_URL);
    }

    #[test]
    fn macos_does_not_pretend_to_host() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::MacOs,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("macOS"));
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert!(body.get("connectionUrl").is_none());
    }

    #[test]
    fn missing_image_fields_refuse_the_stub_boot() {
        let (trust, _) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &unsigned_standard_body(),
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("invalid provision request"));
    }

    #[test]
    fn mismatched_signature_refuses_the_stub_boot() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let other = signed_fixture(FIXTURE_PRO_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "imageDigest": manifest.digest_hex(),
            "imageSignature": other.1.signature_base64(),
        })
        .to_string()
        .into_bytes();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("signature does not verify"));
    }

    #[test]
    fn advertised_digest_must_match_local_standby() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let other = signed_fixture(FIXTURE_PRO_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "imageDigest": other.1.digest_hex(),
            "imageSignature": other.1.signature_base64(),
        })
        .to_string()
        .into_bytes();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("digest does not match"));
    }

    #[test]
    fn missing_bearer_is_unauthorized() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            None,
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 401);
    }

    #[test]
    fn wrong_bearer_is_unauthorized() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/provision",
            Some("Bearer other-token"),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 401);
    }

    #[test]
    fn other_paths_are_not_found() {
        let (trust, body) = signed_standard();
        let output = respond(
            "POST",
            "/v1/hosts",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 404);
    }

    #[test]
    fn listen_config_skips_when_addr_is_unset() {
        let result = listen_config_from_env(|key| match key {
            "HOST_BOOTSTRAP_TOKEN" => Some(TOKEN.to_string()),
            _ => None,
        })
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn listen_config_fails_fast_without_a_token() {
        let error = listen_config_from_env(|key| match key {
            "HOST_AGENT_LISTEN_ADDR" => Some("127.0.0.1:7842".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ProvisionListenError::MissingToken);
        assert!(!error.to_string().contains(TOKEN));
    }

    #[test]
    fn listen_config_fails_fast_without_an_image_public_key() {
        let error = listen_config_from_env(|key| match key {
            "HOST_AGENT_LISTEN_ADDR" => Some("127.0.0.1:7842".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some(TOKEN.to_string()),
            "IMAGE_LOCAL_DIGEST" => Some(Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(
            error,
            ProvisionListenError::Image(ImageError::MissingPublicKey)
        );
        assert!(error.to_string().contains("IMAGE_SIGNING_PUBLIC_KEY"));
    }

    #[test]
    fn listen_config_debug_redacts_the_token() {
        let (trust, _) = signed_standard();
        let config = ListenConfig::new("127.0.0.1:7842", TOKEN, trust);
        let debug = format!("{config:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(TOKEN));
    }

    #[test]
    fn http_bind_round_trip_stays_a_stub() {
        let (trust, body) = signed_standard();
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server
            .server_addr()
            .to_ip()
            .expect("provision test listens on TCP");
        let trust_for_server = trust.clone();
        thread::spawn(move || {
            let occupancy = Occupancy::new();
            let request = server.recv().unwrap();
            dispatch(
                request,
                TOKEN,
                HostOs::Linux,
                &occupancy,
                &trust_for_server,
                None,
            )
            .unwrap();
        });

        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let response = agent
            .post(&format!("http://{addr}/v1/provision"))
            .set("Authorization", &format!("Bearer {TOKEN}"))
            .send_json(payload)
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.into_json().unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
    }

    #[test]
    fn successful_provision_marks_local_occupancy() {
        let (trust, body) = signed_standard();
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
        );
        assert_eq!(output.status, 200);
        assert!(occupancy.is_in_session());
    }

    #[test]
    fn rejected_provision_does_not_mark_occupancy() {
        let (trust, body) = signed_standard();
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            None,
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
        );
        assert_eq!(output.status, 401);
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn image_mismatch_does_not_mark_occupancy() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        let other = signed_fixture(FIXTURE_PRO_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "imageDigest": manifest.digest_hex(),
            "imageSignature": other.1.signature_base64(),
        })
        .to_string()
        .into_bytes();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn macos_reject_does_not_mark_occupancy() {
        let (trust, body) = signed_standard();
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::MacOs,
            &occupancy,
            &trust,
        );
        assert_eq!(output.status, 400);
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn storage_endpoint_is_accepted_and_does_not_mount_on_the_host() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "sharedGpuId": null,
            "imageDigest": manifest.digest_hex(),
            "imageSignature": manifest.signature_base64(),
            "storageEndpoint": "http://renterd.test.invalid",
        })
        .to_string()
        .into_bytes();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
        assert!(body.get("rclone").is_none());
        assert!(body.get("guestRclone").is_none());
        assert!(body.get("secretAccessKey").is_none());
    }

    #[test]
    fn storage_locators_plan_guest_rclone_without_mounting_or_secrets() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let body = serde_json::json!({
            "labTier": "standard",
            "sharedGpuId": null,
            "imageDigest": manifest.digest_hex(),
            "imageSignature": manifest.signature_base64(),
            "storageEndpoint": "http://renterd.test.invalid",
            "storageBucket": "dcl-example",
            "storagePrefix": "files/",
            "storageRegion": "us-east-1",
            "guestMesh": {
                "loginServer": "http://headscale.test.invalid",
                "hostname": "dcl-guest-example",
                "exitNode": "dcl-platform-exit",
                "aclTags": ["tag:guest"]
            }
        })
        .to_string()
        .into_bytes();
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        let plan = body["guestRclone"].as_object().expect("guest rclone plan");
        let args = plan["args"].as_array().expect("rclone args");
        let args: Vec<&str> = args.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(args[0], "mount");
        assert_eq!(args[1], ":s3:dcl-example/files");
        assert!(args.contains(&"--s3-env-auth"));
        assert!(args.contains(&"--s3-force-path-style"));
        assert_eq!(plan["envAuth"], true);
        assert_eq!(plan["mountPoint"], "/home/user/Persistent");
        assert!(body.get("secretAccessKey").is_none());
        assert!(body.get("accessKeyId").is_none());
        assert!(!output.body.contains("AWS_SECRET_ACCESS_KEY"));
        assert!(!output.body.contains("preauthKey"));
    }

    #[test]
    fn provision_http_never_spawns_rclone_or_guest_tailscale() {
        let source = include_str!("provision_http.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("provision_http.rs should have a test module");
        assert!(!production.contains("Command::new(\"rclone\")"));
        assert!(!production.contains("Command::new(\"tailscale\")"));
        assert!(!production.contains("Command::new(\"firecracker\")"));
        assert!(!production.contains("S3_SECRET_KEY"));
        assert!(!production.contains("S3_ACCESS_KEY"));
        assert!(!production.contains("preauthKey"));
    }

    fn linux_kvm_gate() -> ProvisionGate {
        use crate::host_capabilities::{
            evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
        };
        use crate::models::BYTES_PER_GIB;
        use crate::test_support::{nvidia_gpu, nvidia_gpu_with, snapshot};

        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        ProvisionGate {
            capabilities: evaluate(EvaluateInput {
                os: HostOs::Linux,
                kvm: KvmAccess::PresentReadWrite,
                whpx: WhpxCapability::NotApplicable,
                iommu: IommuStatus::Present,
                gpus: &gpus,
            }),
            hardware: snapshot(16, 32 * BYTES_PER_GIB, gpus.to_vec()),
            sharing: None,
            spare_disk_bytes: Some(64 * BYTES_PER_GIB),
            firecracker: None,
        }
    }

    fn linux_no_kvm_gate() -> ProvisionGate {
        use crate::host_capabilities::{
            evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
        };
        use crate::models::BYTES_PER_GIB;
        use crate::test_support::{nvidia_gpu, nvidia_gpu_with, snapshot};

        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        ProvisionGate {
            capabilities: evaluate(EvaluateInput {
                os: HostOs::Linux,
                kvm: KvmAccess::Missing,
                whpx: WhpxCapability::NotApplicable,
                iommu: IommuStatus::Present,
                gpus: &gpus,
            }),
            hardware: snapshot(16, 32 * BYTES_PER_GIB, gpus.to_vec()),
            sharing: None,
            spare_disk_bytes: Some(64 * BYTES_PER_GIB),
            firecracker: None,
        }
    }

    #[test]
    fn gated_linux_without_kvm_refuses_without_a_stub_url_or_occupancy() {
        let (trust, body) = signed_standard();
        let occupancy = Occupancy::new();
        let gate = linux_no_kvm_gate();
        let output = respond_and_track_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("hardware virtualization"));
        assert!(!output.body.contains("connectionUrl"));
        assert!(!occupancy.is_in_session());
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert!(body.get("connectionUrl").is_none());
        assert_eq!(
            body["detail"].as_str().unwrap().contains("Firecracker"),
            false
        );
    }

    #[test]
    fn gated_insufficient_ram_is_a_clean_rejection() {
        use crate::models::{BYTES_PER_GIB, CONFIG_VERSION};

        let (trust, body) = signed_standard();
        let mut gate = linux_kvm_gate();
        gate.sharing = Some(SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: 4,
            shared_memory_bytes: 2 * BYTES_PER_GIB,
            share_gpu: false,
            shared_gpu_id: None,
        });
        let occupancy = Occupancy::new();
        let output = respond_and_track_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("needs at least 4 GiB"));
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn gated_capable_linux_still_returns_the_stub_without_invoking() {
        let (trust, body) = signed_standard();
        let gate = linux_kvm_gate();
        let output = respond_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
    }

    fn guest_ready(path: &std::path::Path) -> crate::image::GuestImageStatus {
        crate::image::GuestImageStatus {
            action: crate::image::ImageEnsureAction::SkippedPresent,
            lab_tier: Some("standard".to_string()),
            backend: crate::image::ImageBackend::Firecracker,
            path: Some(path.to_string_lossy().into_owned()),
            digest: None,
            log: "test".to_string(),
            image_booted: false,
        }
    }

    fn boot_tree() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("bin").join("firecracker");
        let rootfs = dir.path().join("standard").join("rootfs.img");
        let kernel = dir.path().join("standard").join("vmlinux");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fake-firecracker").unwrap();
        std::fs::write(&rootfs, b"fake-rootfs").unwrap();
        std::fs::write(&kernel, b"fake-vmlinux").unwrap();
        (dir, binary, rootfs)
    }

    #[test]
    fn linux_standard_with_boot_inputs_invokes_and_keeps_the_placeholder_locator() {
        use crate::firecracker::{RecordingFirecracker, API_PATH_ORDER, API_SOCK_ARG, ENABLE_PCI_ARG};
        use std::sync::Arc;

        let (trust, body) = signed_standard();
        let (tree, binary, rootfs) = boot_tree();
        let recorder = Arc::new(RecordingFirecracker::succeeding());
        let mut gate = linux_kvm_gate();
        gate.firecracker = Some(FirecrackerBoot {
            binary: binary.clone(),
            image_cache_dir: tree.path().to_path_buf(),
            guest_image: guest_ready(&rootfs),
            kernel_override: None,
            arch: "x86_64".to_string(),
            runtime: recorder.clone(),
        });
        let occupancy = Occupancy::new();
        let output = respond_and_track_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(body["hypervisorInvoked"], true);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
        assert_eq!(body["streamer"], "kasm");
        assert!(occupancy.is_in_session());

        let spawns = recorder.spawn_calls();
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].0, binary);
        assert_eq!(spawns[0].1[0], API_SOCK_ARG);
        assert_eq!(spawns[0].1[2], ENABLE_PCI_ARG);
        let puts = recorder.put_calls();
        let paths: Vec<&str> = puts.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, API_PATH_ORDER);
        let machine: serde_json::Value = serde_json::from_str(&puts[2].1).unwrap();
        assert_eq!(machine["vcpu_count"], 16);
        assert_eq!(machine["mem_size_mib"], 32768);
    }

    #[test]
    fn linux_standard_boot_without_kernel_is_400_not_a_successful_stub() {
        use crate::firecracker::RecordingFirecracker;
        use std::sync::Arc;

        let (trust, body) = signed_standard();
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("bin").join("firecracker");
        let rootfs = dir.path().join("standard").join("rootfs.img");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(rootfs.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fake-firecracker").unwrap();
        std::fs::write(&rootfs, b"fake-rootfs").unwrap();
        let recorder = Arc::new(RecordingFirecracker::succeeding());
        let mut gate = linux_kvm_gate();
        gate.firecracker = Some(FirecrackerBoot {
            binary,
            image_cache_dir: dir.path().to_path_buf(),
            guest_image: guest_ready(&rootfs),
            kernel_override: None,
            arch: "x86_64".to_string(),
            runtime: recorder.clone(),
        });
        let occupancy = Occupancy::new();
        let output = respond_and_track_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Linux,
            &occupancy,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("FIRECRACKER_KERNEL"));
        assert!(!output.body.contains("connectionUrl"));
        assert!(!occupancy.is_in_session());
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn windows_standard_ignores_firecracker_boot_inputs() {
        use crate::firecracker::RecordingFirecracker;
        use std::sync::Arc;

        let (trust, body) = signed_standard();
        let (tree, binary, rootfs) = boot_tree();
        let recorder = Arc::new(RecordingFirecracker::succeeding());
        let mut gate = linux_kvm_gate();
        gate.firecracker = Some(FirecrackerBoot {
            binary,
            image_cache_dir: tree.path().to_path_buf(),
            guest_image: guest_ready(&rootfs),
            kernel_override: None,
            arch: "x86_64".to_string(),
            runtime: recorder.clone(),
        });
        let output = respond_with_gate(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &body,
            TOKEN,
            HostOs::Windows,
            &trust,
            Some(&gate),
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/qemu-whpx")
        );
        assert!(recorder.spawn_calls().is_empty());
    }
}
