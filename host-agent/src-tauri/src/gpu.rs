//! GPU detection.
//!
//! Every display device comes from the OS's PCI list: sysfs on Linux,
//! `Win32_VideoController` on Windows. That finds Intel/AMD iGPUs and AMD
//! cards, which the Pro tier needs to tell whether the owner keeps a screen
//! while one card goes to a guest (issue 35). NVIDIA's management library
//! (NVML) then adds the UUID and VRAM for the NVIDIA cards it can read.
//!
//! IDs: on Linux an NVIDIA card keeps its NVML UUID (old configs still
//! match) and other cards use their PCI address, which `gpu_bind` resolves
//! directly. On Windows every card uses its PnP instance ID, because that is
//! what Hyper-V's `Get-PnpDevice -InstanceId` lookup needs.
//!
//! Nothing here fails: a machine without a usable GPU is still a good
//! Standard host, so problems end up in the probe's note.

// Each OS's probe uses only its own half of the helpers below; the tests
// use all of them.
#![cfg_attr(not(test), allow(dead_code))]

use crate::host_capabilities::looks_like_integrated;
use crate::models::{GpuInfo, GpuProbe, GpuVendor};

pub fn probe() -> GpuProbe {
    platform::probe()
}

pub const VENDOR_NVIDIA: u16 = 0x10de;
pub const VENDOR_AMD: u16 = 0x1002;
pub const VENDOR_INTEL: u16 = 0x8086;

/// One display device as the OS lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciDisplay {
    /// `dddd:bb:dd.f`, lowercase. Windows reports no domain, so it is `0000`.
    pub address: Option<String>,
    pub vendor_id: u16,
    pub device_id: u16,
    pub name: Option<String>,
    /// The ID this OS needs later (PCI address on Linux, PnP instance ID on
    /// Windows).
    pub os_id: String,
}

/// One card as NVML reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvmlGpu {
    pub address: Option<String>,
    pub name: String,
    pub vram_bytes: u64,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdScheme {
    /// Prefer the NVML UUID for NVIDIA cards (Linux).
    NvmlUuid,
    /// Always the OS ID (Windows: PnP instance ID for Hyper-V).
    OsId,
}

#[cfg_attr(
    not(any(target_os = "windows", target_os = "linux", test)),
    allow(dead_code)
)]
fn nvidia_gpu(raw_name: &str, vram_bytes: u64, id: impl Into<String>) -> GpuInfo {
    GpuInfo {
        id: id.into(),
        model: normalize_model(raw_name, GpuVendor::Nvidia),
        vendor: GpuVendor::Nvidia,
        vram_bytes: (vram_bytes > 0).then_some(vram_bytes),
        integrated: false,
    }
}

/// NVML UUID when the driver reports one; otherwise a per-index fallback so
/// two unnamed cards are still distinguishable in the config file.
#[cfg_attr(
    not(any(target_os = "windows", target_os = "linux", test)),
    allow(dead_code)
)]
pub fn gpu_identity(uuid: Option<&str>, index: u32) -> String {
    match uuid {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => format!("nvidia:{index}"),
    }
}

fn normalize_model(raw: &str, vendor: GpuVendor) -> String {
    let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");

    if !cleaned.is_empty() {
        return cleaned;
    }

    match vendor {
        GpuVendor::Nvidia => "Unknown NVIDIA GPU".to_string(),
        GpuVendor::Amd => "Unknown AMD GPU".to_string(),
        GpuVendor::Intel => "Unknown Intel GPU".to_string(),
        GpuVendor::Unknown => "Unknown GPU".to_string(),
    }
}

pub fn vendor_of(vendor_id: u16) -> GpuVendor {
    match vendor_id {
        VENDOR_NVIDIA => GpuVendor::Nvidia,
        VENDOR_AMD => GpuVendor::Amd,
        VENDOR_INTEL => GpuVendor::Intel,
        _ => GpuVendor::Unknown,
    }
}

/// Integrated GPUs share the CPU package and drive the owner's screen; they
/// are never offered for dedication. Intel's only discrete line is Arc.
pub fn is_integrated(vendor: GpuVendor, model: &str) -> bool {
    match vendor {
        GpuVendor::Nvidia => false,
        GpuVendor::Intel => !model.to_ascii_lowercase().contains("arc"),
        GpuVendor::Amd | GpuVendor::Unknown => looks_like_integrated(model),
    }
}

/// `00000000:01:00.0` (NVML) or `0000:01:00.0` (sysfs) → `0000:01:00.0`.
pub fn normalize_address(raw: &str) -> Option<String> {
    let raw = raw.trim().to_ascii_lowercase();
    let (domain, rest) = raw.split_once(':')?;
    let (bus, slot) = rest.split_once(':')?;
    let (device, function) = slot.split_once('.')?;
    let hex = |s: &str, width: usize| {
        (!s.is_empty() && s.len() <= 8 && s.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(s, 16).ok())
            .flatten()
            .map(|v| format!("{v:0width$x}"))
    };
    let domain = hex(domain, 4)?;
    let domain = &domain[domain.len() - 4..];
    Some(format!(
        "{domain}:{}:{}.{}",
        hex(bus, 2)?,
        hex(device, 2)?,
        hex(function, 1)?
    ))
}

/// Same bus, device and function; domains are ignored because Windows does
/// not report one.
fn same_slot(a: &str, b: &str) -> bool {
    match (normalize_address(a), normalize_address(b)) {
        (Some(a), Some(b)) => a[5..] == b[5..],
        _ => false,
    }
}

/// Joins the OS list with NVML. Discrete cards come first so a default pick
/// never lands on the owner's iGPU. When the OS list is empty (it could not
/// be read), the NVML list is used as before.
pub fn merge(pci: &[PciDisplay], nvml: &[NvmlGpu], scheme: IdScheme) -> Vec<GpuInfo> {
    let mut used = vec![false; nvml.len()];
    let mut gpus = Vec::new();

    for dev in pci {
        let vendor = vendor_of(dev.vendor_id);
        let matched = if vendor == GpuVendor::Nvidia {
            let by_slot = dev.address.as_deref().and_then(|addr| {
                nvml.iter().enumerate().position(|(i, gpu)| {
                    !used[i] && gpu.address.as_deref().is_some_and(|a| same_slot(a, addr))
                })
            });
            // No address to compare: pair NVIDIA cards in order.
            by_slot.or_else(|| {
                dev.address
                    .is_none()
                    .then(|| (0..nvml.len()).find(|&i| !used[i] && nvml[i].address.is_none()))
                    .flatten()
            })
        } else {
            None
        };

        let gpu = match matched {
            Some(i) => {
                used[i] = true;
                let id = match scheme {
                    IdScheme::NvmlUuid => nvml[i].id.clone(),
                    IdScheme::OsId => dev.os_id.clone(),
                };
                nvidia_gpu(&nvml[i].name, nvml[i].vram_bytes, id)
            }
            None => {
                let fallback = format!(
                    "{} GPU ({:04x}:{:04x})",
                    vendor_label(vendor),
                    dev.vendor_id,
                    dev.device_id
                );
                let model = normalize_model(dev.name.as_deref().unwrap_or(&fallback), vendor);
                GpuInfo {
                    id: dev.os_id.clone(),
                    integrated: is_integrated(vendor, &model),
                    model,
                    vendor,
                    vram_bytes: None,
                }
            }
        };
        gpus.push(gpu);
    }

    // NVML cards the OS list did not show (or no OS list at all).
    for (i, gpu) in nvml.iter().enumerate() {
        if !used[i] {
            gpus.push(nvidia_gpu(&gpu.name, gpu.vram_bytes, gpu.id.clone()));
        }
    }

    gpus.sort_by_key(|gpu| gpu.integrated);
    gpus
}

fn vendor_label(vendor: GpuVendor) -> &'static str {
    match vendor {
        GpuVendor::Nvidia => "NVIDIA",
        GpuVendor::Amd => "AMD",
        GpuVendor::Intel => "Intel",
        GpuVendor::Unknown => "Unknown",
    }
}

/// Explains an empty result, preferring the NVML reason when there is one.
fn probe_from(gpus: Vec<GpuInfo>, nvml_note: Option<String>) -> GpuProbe {
    if gpus.is_empty() {
        GpuProbe::none(nvml_note.unwrap_or_else(|| "No GPU detected.".to_string()))
    } else {
        GpuProbe::detected(gpus)
    }
}

/// Linux: every PCI device of class `0x03xxxx` under `root/sys/bus/pci/devices`.
/// Names come from the `pci.ids` database when the distro ships one.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn list_linux_displays(root: &std::path::Path) -> Vec<PciDisplay> {
    use std::fs;

    let read = |path: std::path::PathBuf| fs::read_to_string(path).ok();
    let hex16 = |text: Option<String>| {
        text.and_then(|t| u16::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok())
    };

    let Ok(entries) = fs::read_dir(root.join("sys/bus/pci/devices")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        let Some(class) = read(dir.join("class")) else {
            continue;
        };
        if !class.trim().to_ascii_lowercase().starts_with("0x03") {
            continue;
        }
        let Some(address) = normalize_address(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let (Some(vendor_id), Some(device_id)) = (
            hex16(read(dir.join("vendor"))),
            hex16(read(dir.join("device"))),
        ) else {
            continue;
        };
        found.push(PciDisplay {
            os_id: address.clone(),
            address: Some(address),
            vendor_id,
            device_id,
            name: None,
        });
    }
    found.sort_by(|a, b| a.address.cmp(&b.address));

    if !found.is_empty() {
        let ids = [
            "usr/share/hwdata/pci.ids",
            "usr/share/misc/pci.ids",
            "usr/share/pci.ids",
        ]
        .iter()
        .find_map(|rel| read(root.join(rel)));
        if let Some(ids) = ids {
            for dev in &mut found {
                dev.name = pci_ids_name(&ids, dev.vendor_id, dev.device_id);
            }
        }
    }
    found
}

/// `Vendor Device` from a `pci.ids` file, e.g. "Intel Raptor Lake-S GT1
/// [UHD Graphics 770]". Short vendor names keep the UI label readable.
pub fn pci_ids_name(db: &str, vendor_id: u16, device_id: u16) -> Option<String> {
    let vendor_key = format!("{vendor_id:04x}  ");
    let device_key = format!("\t{device_id:04x}  ");
    let mut in_vendor = false;
    for line in db.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                return None;
            }
            in_vendor = line.starts_with(&vendor_key);
            continue;
        }
        if in_vendor && line.starts_with(&device_key) {
            let device = line[device_key.len()..].trim();
            let vendor = vendor_label(vendor_of(vendor_id));
            let vendor = if vendor == "Unknown" { "" } else { vendor };
            return Some(format!("{vendor} {device}").trim().to_string());
        }
    }
    None
}

/// Windows: the PowerShell that lists display adapters as JSON.
pub const WINDOWS_LIST_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
$list = @(Get-CimInstance Win32_VideoController | Where-Object { $_.PNPDeviceID -like 'PCI\\*' } | ForEach-Object { \
  $loc = $null; \
  try { $loc = [string](Get-PnpDeviceProperty -InstanceId $_.PNPDeviceID -KeyName 'DEVPKEY_Device_LocationInfo' -ErrorAction Stop).Data } catch {}; \
  [ordered]@{ name = [string]$_.Name; instanceId = [string]$_.PNPDeviceID; location = $loc } \
}); \
ConvertTo-Json -Compress -InputObject $list";

/// Parses [`WINDOWS_LIST_SCRIPT`] output. Vendor/device come from the
/// instance ID (`PCI\VEN_10DE&DEV_2786&...`), the slot from the location
/// text (`PCI bus 1, device 0, function 0`).
pub fn parse_windows_displays(json: &str) -> Vec<PciDisplay> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        name: Option<String>,
        instance_id: String,
        location: Option<String>,
    }
    let rows: Vec<Row> = match serde_json::from_str::<serde_json::Value>(json.trim()) {
        Ok(serde_json::Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        Ok(item @ serde_json::Value::Object(_)) => {
            serde_json::from_value(item).ok().into_iter().collect()
        }
        _ => Vec::new(),
    };

    let field = |id: &str, key: &str| {
        let upper = id.to_ascii_uppercase();
        let start = upper.find(key)? + key.len();
        u16::from_str_radix(upper.get(start..start + 4)?, 16).ok()
    };

    rows.into_iter()
        .filter_map(|row| {
            let vendor_id = field(&row.instance_id, "VEN_")?;
            let device_id = field(&row.instance_id, "DEV_")?;
            Some(PciDisplay {
                address: row.location.as_deref().and_then(windows_location_address),
                vendor_id,
                device_id,
                name: row.name.filter(|n| !n.trim().is_empty()),
                os_id: row.instance_id,
            })
        })
        .collect()
}

/// `PCI bus 1, device 0, function 0` → `0000:01:00.0`.
pub fn windows_location_address(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let number = |key: &str| -> Option<u32> {
        let start = lower.find(key)? + key.len();
        let digits: String = lower[start..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    };
    Some(format!(
        "0000:{:02x}:{:02x}.{:x}",
        number("bus")?,
        number("device")?,
        number("function")?
    ))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod platform {
    use super::{gpu_identity, normalize_address, NvmlGpu};
    use crate::models::GpuProbe;
    use nvml_wrapper::error::NvmlError;
    use nvml_wrapper::Nvml;

    pub fn probe() -> GpuProbe {
        let (nvml, note) = nvml_gpus();
        #[cfg(target_os = "linux")]
        let (pci, scheme) = (
            super::list_linux_displays(std::path::Path::new("/")),
            super::IdScheme::NvmlUuid,
        );
        #[cfg(target_os = "windows")]
        let (pci, scheme) = (windows_displays(), super::IdScheme::OsId);
        super::probe_from(super::merge(&pci, &nvml, scheme), note)
    }

    #[cfg(target_os = "windows")]
    fn windows_displays() -> Vec<super::PciDisplay> {
        match crate::gpu_bind::run_powershell(
            super::WINDOWS_LIST_SCRIPT,
            std::time::Duration::from_secs(20),
        ) {
            Ok(json) => super::parse_windows_displays(&json),
            Err(_) => Vec::new(),
        }
    }

    fn nvml_gpus() -> (Vec<NvmlGpu>, Option<String>) {
        let nvml = match Nvml::init() {
            Ok(nvml) => nvml,
            Err(NvmlError::LibloadingError(_)) | Err(NvmlError::LibraryNotFound) => {
                return (Vec::new(), Some("No GPU detected. (No NVIDIA management library found; install the NVIDIA driver if this machine has an NVIDIA GPU.)".to_string()));
            }
            Err(NvmlError::DriverNotLoaded) => {
                return (
                    Vec::new(),
                    Some("No GPU detected. (The NVIDIA driver is not loaded.)".to_string()),
                );
            }
            Err(NvmlError::NoPermission) => {
                return (Vec::new(), Some("Access to the NVIDIA driver was denied. Run the agent with permission to query the GPU.".to_string()));
            }
            Err(error) => {
                return (
                    Vec::new(),
                    Some(format!("NVIDIA GPU detection failed: {error}")),
                );
            }
        };
        let count = match nvml.device_count() {
            Ok(count) => count,
            Err(error) => {
                return (
                    Vec::new(),
                    Some(format!("Could not count NVIDIA GPUs: {error}")),
                );
            }
        };

        let mut gpus = Vec::new();
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                // One unreadable device should not hide the others.
                continue;
            };
            gpus.push(NvmlGpu {
                address: device
                    .pci_info()
                    .ok()
                    .and_then(|info| normalize_address(&info.bus_id)),
                name: device.name().unwrap_or_default(),
                vram_bytes: device.memory_info().map(|memory| memory.total).unwrap_or(0),
                id: gpu_identity(device.uuid().ok().as_deref(), index),
            });
        }
        (gpus, None)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use crate::models::GpuProbe;

    pub fn probe() -> GpuProbe {
        GpuProbe::none("GPU detection supports Windows and Linux only.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pci(address: &str, vendor_id: u16, device_id: u16, name: Option<&str>) -> PciDisplay {
        PciDisplay {
            address: Some(address.to_string()),
            vendor_id,
            device_id,
            name: name.map(str::to_string),
            os_id: address.to_string(),
        }
    }

    fn nv(address: &str, name: &str, id: &str) -> NvmlGpu {
        NvmlGpu {
            address: Some(address.to_string()),
            name: name.to_string(),
            vram_bytes: 12 << 30,
            id: id.to_string(),
        }
    }

    #[test]
    fn keeps_the_reported_nvidia_model_name() {
        let gpu = nvidia_gpu("NVIDIA GeForce RTX 4070", 12_884_901_888, "GPU-4070");

        assert_eq!(gpu.model, "NVIDIA GeForce RTX 4070");
        assert_eq!(gpu.id, "GPU-4070");
        assert_eq!(gpu.vendor, GpuVendor::Nvidia);
        assert_eq!(gpu.vram_bytes, Some(12_884_901_888));
        assert!(!gpu.integrated);
    }

    #[test]
    fn treats_zero_vram_as_unknown() {
        assert_eq!(
            nvidia_gpu("NVIDIA GeForce RTX 4070", 0, "GPU-4070").vram_bytes,
            None
        );
    }

    #[test]
    fn falls_back_when_the_driver_reports_no_name() {
        assert_eq!(nvidia_gpu("   ", 0, "nvidia:0").model, "Unknown NVIDIA GPU");
        assert_eq!(normalize_model("", GpuVendor::Unknown), "Unknown GPU");
    }

    #[test]
    fn collapses_padded_model_names() {
        assert_eq!(
            normalize_model("  NVIDIA   RTX  A4000 \n", GpuVendor::Nvidia),
            "NVIDIA RTX A4000"
        );
    }

    #[test]
    fn probe_never_panics_and_explains_an_empty_result() {
        let probe = probe();

        if probe.gpus.is_empty() {
            assert!(probe.note.is_some(), "an empty probe must say why");
        } else {
            let mut ids: Vec<&str> = probe.gpus.iter().map(|gpu| gpu.id.as_str()).collect();
            assert!(probe.gpus.iter().all(|gpu| !gpu.model.is_empty()));
            assert!(ids.iter().all(|id| !id.is_empty()));
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), probe.gpus.len(), "each GPU needs a unique id");
        }
    }

    #[test]
    fn prefers_the_driver_uuid_over_an_index_fallback() {
        assert_eq!(gpu_identity(Some("GPU-ABCDEF"), 0), "GPU-ABCDEF");
        assert_eq!(gpu_identity(Some("  GPU-ABCDEF  "), 0), "GPU-ABCDEF");
    }

    #[test]
    fn enumerates_multiple_gpus_with_distinct_fallback_ids() {
        assert_eq!(gpu_identity(None, 0), "nvidia:0");
        assert_eq!(gpu_identity(Some(""), 1), "nvidia:1");
        assert_eq!(gpu_identity(Some("   "), 2), "nvidia:2");
    }

    // Issue 35: the common Pro host. Before, only the NVIDIA card showed up
    // and Pro was refused for lack of a second display path.
    #[test]
    fn linux_igpu_plus_nvidia_lists_both_with_the_card_first() {
        let gpus = merge(
            &[
                pci(
                    "0000:00:02.0",
                    VENDOR_INTEL,
                    0xa780,
                    Some("Intel Raptor Lake-S GT1 [UHD Graphics 770]"),
                ),
                pci("0000:01:00.0", VENDOR_NVIDIA, 0x2786, None),
            ],
            &[nv(
                "00000000:01:00.0",
                "NVIDIA GeForce RTX 4070",
                "GPU-4070",
            )],
            IdScheme::NvmlUuid,
        );

        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].id, "GPU-4070", "existing configs keep matching");
        assert_eq!(gpus[0].model, "NVIDIA GeForce RTX 4070");
        assert_eq!(gpus[0].vram_bytes, Some(12 << 30));
        assert!(!gpus[0].integrated);
        assert_eq!(gpus[1].vendor, GpuVendor::Intel);
        assert_eq!(gpus[1].id, "0000:00:02.0");
        assert!(gpus[1].integrated);
        assert!(crate::host_capabilities::has_second_display_path(&gpus));
    }

    #[test]
    fn amd_card_without_nvidia_driver_is_found_by_pci_address() {
        let gpus = merge(
            &[pci(
                "0000:03:00.0",
                VENDOR_AMD,
                0x73bf,
                Some("AMD Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]"),
            )],
            &[],
            IdScheme::NvmlUuid,
        );
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].vendor, GpuVendor::Amd);
        assert_eq!(gpus[0].id, "0000:03:00.0");
        assert!(!gpus[0].integrated);
    }

    #[test]
    fn nvidia_card_without_nvml_still_appears() {
        let gpus = merge(
            &[pci("0000:01:00.0", VENDOR_NVIDIA, 0x2786, None)],
            &[],
            IdScheme::NvmlUuid,
        );
        assert_eq!(gpus[0].model, "NVIDIA GPU (10de:2786)");
        assert_eq!(gpus[0].id, "0000:01:00.0");
    }

    #[test]
    fn without_an_os_list_nvml_alone_is_used() {
        let gpus = merge(
            &[],
            &[nv(
                "00000000:01:00.0",
                "NVIDIA GeForce RTX 4070",
                "GPU-4070",
            )],
            IdScheme::NvmlUuid,
        );
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].id, "GPU-4070");
    }

    // On Windows Hyper-V looks the GPU up by PnP instance ID; the NVML UUID
    // was never found there.
    #[test]
    fn windows_ids_are_pnp_instance_ids_with_nvml_details() {
        let json = r#"[
          {"name":"Intel(R) UHD Graphics 770","instanceId":"PCI\\VEN_8086&DEV_A780&SUBSYS_00000000&REV_04\\3&11583659&0&10","location":"PCI bus 0, device 2, function 0"},
          {"name":"NVIDIA GeForce RTX 4070","instanceId":"PCI\\VEN_10DE&DEV_2786&SUBSYS_00000000&REV_A1\\4&2B1C&0&0008","location":"PCI bus 1, device 0, function 0"}
        ]"#;
        let pci = parse_windows_displays(json);
        assert_eq!(pci.len(), 2);
        assert_eq!(pci[1].address.as_deref(), Some("0000:01:00.0"));

        let gpus = merge(
            &pci,
            &[nv(
                "00000000:01:00.0",
                "NVIDIA GeForce RTX 4070",
                "GPU-4070",
            )],
            IdScheme::OsId,
        );
        assert_eq!(
            gpus[0].id,
            r"PCI\VEN_10DE&DEV_2786&SUBSYS_00000000&REV_A1\4&2B1C&0&0008"
        );
        assert_eq!(gpus[0].vram_bytes, Some(12 << 30));
        assert_eq!(gpus[1].model, "Intel(R) UHD Graphics 770");
        assert!(gpus[1].integrated);
    }

    #[test]
    fn windows_single_adapter_json_is_an_object_not_an_array() {
        let json = r#"{"name":"AMD Radeon(TM) Graphics","instanceId":"PCI\\VEN_1002&DEV_164E&SUBSYS_1\\3&1","location":null}"#;
        let gpus = merge(&parse_windows_displays(json), &[], IdScheme::OsId);
        assert_eq!(gpus.len(), 1);
        assert!(gpus[0].integrated, "Radeon(TM) Graphics is an APU");
        assert!(parse_windows_displays("").is_empty());
        assert!(parse_windows_displays("not json").is_empty());
    }

    #[test]
    fn intel_arc_counts_as_discrete() {
        assert!(!is_integrated(GpuVendor::Intel, "Intel DG2 [Arc A770]"));
        assert!(is_integrated(GpuVendor::Intel, "Intel GPU (8086:a780)"));
        assert!(!is_integrated(
            GpuVendor::Amd,
            "AMD Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]"
        ));
    }

    #[test]
    fn addresses_normalize_across_nvml_sysfs_and_windows() {
        assert_eq!(
            normalize_address("00000000:01:00.0").as_deref(),
            Some("0000:01:00.0")
        );
        assert_eq!(
            normalize_address("0000:0A:00.1").as_deref(),
            Some("0000:0a:00.1")
        );
        assert_eq!(normalize_address("garbage"), None);
        assert_eq!(
            windows_location_address("PCI bus 10, device 0, function 1").as_deref(),
            Some("0000:0a:00.1")
        );
        assert_eq!(windows_location_address("Unknown"), None);
    }

    #[test]
    fn reads_linux_display_devices_and_names_from_a_fake_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let add = |bdf: &str, class: &str, vendor: &str, device: &str| {
            let d = root.join("sys/bus/pci/devices").join(bdf);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("class"), format!("{class}\n")).unwrap();
            std::fs::write(d.join("vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(d.join("device"), format!("{device}\n")).unwrap();
        };
        add("0000:00:02.0", "0x030000", "0x8086", "0xa780");
        add("0000:01:00.0", "0x030000", "0x10de", "0x2786");
        add("0000:01:00.1", "0x040300", "0x10de", "0x22bc"); // the card's audio
        add("0000:02:00.0", "0x010802", "0x144d", "0xa808"); // NVMe
        std::fs::create_dir_all(root.join("usr/share/hwdata")).unwrap();
        std::fs::write(
            root.join("usr/share/hwdata/pci.ids"),
            "# comment\n8086  Intel Corporation\n\ta780  Raptor Lake-S GT1 [UHD Graphics 770]\n10de  NVIDIA Corporation\n\t2786  AD104 [GeForce RTX 4070]\n",
        )
        .unwrap();

        let found = list_linux_displays(root);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].address.as_deref(), Some("0000:00:02.0"));
        assert_eq!(
            found[0].name.as_deref(),
            Some("Intel Raptor Lake-S GT1 [UHD Graphics 770]")
        );
        assert_eq!(
            found[1].name.as_deref(),
            Some("NVIDIA AD104 [GeForce RTX 4070]")
        );
        assert!(list_linux_displays(&root.join("missing")).is_empty());
    }
}
