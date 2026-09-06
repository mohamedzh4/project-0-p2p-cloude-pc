// Mirrors the serde representations in src-tauri/src/models.rs.

export type GpuVendor = "nvidia" | "unknown";

export interface CpuInfo {
  model: string;
  logicalCores: number;
  physicalCores: number | null;
}

export interface MemoryInfo {
  totalBytes: number;
}

export interface GpuInfo {
  /** NVML UUID, or `nvidia:{index}` if the driver did not report one. */
  id: string;
  model: string;
  vendor: GpuVendor;
  vramBytes: number | null;
}

export interface HardwareSnapshot {
  cpu: CpuInfo;
  memory: MemoryInfo;
  gpus: GpuInfo[];
  /** Why no GPU is listed, when `gpus` is empty. */
  gpuNote: string | null;
}

export type CapabilityOs = "linux" | "windows" | "macos" | "other";
export type KvmAccess = "presentReadWrite" | "presentNotWritable" | "missing" | "notApplicable";
export type WhpxCapability = "available" | "unavailable" | "unknown" | "notApplicable";
export type IommuStatus = "present" | "absent" | "unknown" | "notApplicable";

export interface LabOffer {
  offered: boolean;
  reason: string;
}

/** Hosting eligibility. The agent chooses the backend; the UI never asks. */
export interface HostCapabilities {
  os: CapabilityOs;
  kvm: KvmAccess;
  whpx: WhpxCapability;
  iommu: IommuStatus;
  secondDisplayPath: boolean;
  gpuCount: number;
  standard: LabOffer;
  pro: LabOffer;
  summary: string;
}

export interface EligibilityReport {
  standard: LabOffer;
  pro: LabOffer;
  summary: string;
}

export type HypervisorAction =
  | "skippedPresent"
  | "fetched"
  | "skippedOs"
  | "wouldFetch"
  | "refused"
  | "checksumMismatch"
  | "fetchFailed"
  | "installFailed";

export type HypervisorName = "firecracker" | "qemu" | "none";

/** Locate/fetch log for the OS hypervisor binary. Never invoked. */
export interface HypervisorStatus {
  action: HypervisorAction;
  hypervisor: HypervisorName;
  version: string | null;
  path: string | null;
  log: string;
  hypervisorInvoked: boolean;
}

export type GuestImageAction =
  | "skippedPresent"
  | "fetched"
  | "skippedOs"
  | "wouldFetch"
  | "refused"
  | "rejected"
  | "fetchFailed";

export type GuestImageBackend = "firecracker" | "qemu" | "none";

/** Locate/fetch log for the signed guest image. Never booted. */
export interface GuestImageStatus {
  action: GuestImageAction;
  labTier: string | null;
  backend: GuestImageBackend;
  path: string | null;
  digest: string | null;
  log: string;
  imageBooted: boolean;
}

export type LaunchBackend = "firecracker" | "qemu-whpx" | "qemu-vfio" | "hyperv-gpu";

/** Generated hypervisor document. Never a running VM. */
export interface LaunchConfig {
  backend: LaunchBackend;
  labTier: "standard" | "pro";
  vcpuCount: number;
  memoryMib: number;
  kernelPath: string | null;
  rootfsPath: string | null;
  /** Present only for Pro when a GPU is dedicated. Omitted on Standard. */
  gpu?: {
    id: string;
    exclusive: boolean;
  };
  hypervisorInvoked: boolean;
}

export type GpuBindAction = "planned" | "bound" | "wouldAssign" | "skipped" | "refused";

export interface BindStep {
  action: string;
  driver: string;
  device: string;
}

/** Pro GPU assignment plan. Never a live VFIO/DDA/GPU-P apply from the window. */
export interface GpuBindStatus {
  action: GpuBindAction;
  backend: string | null;
  gpuId: string | null;
  pciAddress: string | null;
  previousDriver: string | null;
  targetDriver: string | null;
  steps: BindStep[];
  bound: boolean;
  rolledBack: boolean;
  exclusive: boolean;
  log: string;
  hypervisorInvoked: boolean;
}

export interface SharingConfig {
  version: number;
  sharingEnabled: boolean;
  sharedLogicalCores: number;
  sharedMemoryBytes: number;
  shareGpu: boolean;
  /** Which card to dedicate. Null when no supported GPU is present. */
  sharedGpuId: string | null;
}

/** Last Matchmaker host-inventory response persisted by Rust. */
export interface HostRegistration {
  hostId: string;
  displayName: string;
  labTier: string;
  status: string;
  sharedLogicalCores: number;
  sharedMemoryBytes: number;
  shareGpu: boolean;
  sharedGpuId: string | null;
}

export type SharingPresence = "paused" | "idle" | "inSession";

export interface AgentStatus {
  presence: SharingPresence;
  sharingEnabled: boolean;
  inSession: boolean;
  earningsNote: string;
}

export function presenceLabel(presence: SharingPresence): string {
  switch (presence) {
    case "paused":
      return "Paused";
    case "idle":
      return "Idle";
    case "inSession":
      return "In session";
  }
}
