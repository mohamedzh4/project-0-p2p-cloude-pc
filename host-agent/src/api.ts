import { invoke } from "@tauri-apps/api/core";

import { BYTES_PER_GIB } from "./format";
import {
  isTauriRuntime,
  previewCapabilities,
  previewConfig,
  previewGuestImage,
  previewHardware,
  previewHypervisor,
} from "./preview";
import type {
  AgentStatus,
  GpuBindStatus,
  GuestImageStatus,
  HardwareSnapshot,
  HostCapabilities,
  HostRegistration,
  HypervisorStatus,
  LaunchConfig,
  SharingConfig,
  SharingPresence,
} from "./types";

/** True only when the UI runs in a plain browser during development. */
export const IS_PREVIEW = import.meta.env.DEV && !isTauriRuntime();

let previewStore: SharingConfig = previewConfig;
let previewRegistration: HostRegistration | null = null;
let previewInSession = false;

function previewPresence(sharingEnabled: boolean, inSession: boolean): SharingPresence {
  if (!sharingEnabled) {
    return "paused";
  }
  return inSession ? "inSession" : "idle";
}

function previewStatus(): AgentStatus {
  return {
    presence: previewPresence(previewStore.sharingEnabled, previewInSession),
    sharingEnabled: previewStore.sharingEnabled,
    inSession: previewInSession,
    earningsNote: "0 credits accrued (not paid out)",
  };
}

export function getHardwareInfo(): Promise<HardwareSnapshot> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewHardware);
  }

  return invoke<HardwareSnapshot>("get_hardware_info");
}

export function getHostCapabilities(): Promise<HostCapabilities> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewCapabilities);
  }

  return invoke<HostCapabilities>("get_host_capabilities");
}

export function getHypervisorStatus(): Promise<HypervisorStatus> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewHypervisor);
  }

  return invoke<HypervisorStatus>("get_hypervisor_status");
}

export function getGuestImageStatus(): Promise<GuestImageStatus> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewGuestImage);
  }

  return invoke<GuestImageStatus>("get_guest_image_status");
}

const BYTES_PER_MIB = 1024 * 1024;
const MIN_VCPU = 2;
const MIN_MEMORY_BYTES = 4 * BYTES_PER_GIB;

/** Preview-only mirror of `launch::build` for the sample Linux host. Rust is the source of truth. */
function previewBuildLaunchConfig(config: SharingConfig, labTier: string): LaunchConfig {
  const cores = Math.min(config.sharedLogicalCores, previewHardware.cpu.logicalCores);
  const memoryBytes = Math.min(config.sharedMemoryBytes, previewHardware.memory.totalBytes);
  if (cores < MIN_VCPU) {
    throw new Error(`this PC would lend ${cores} vCPU, but a session needs at least ${MIN_VCPU} vCPU`);
  }
  if (memoryBytes < MIN_MEMORY_BYTES) {
    throw new Error(
      `this PC would lend ${memoryBytes / BYTES_PER_GIB} GiB of RAM, but a session needs at least 4 GiB`,
    );
  }

  if (labTier === "standard") {
    return {
      backend: "firecracker",
      labTier: "standard",
      vcpuCount: cores,
      memoryMib: memoryBytes / BYTES_PER_MIB,
      kernelPath: null,
      rootfsPath: previewGuestImage.path,
      hypervisorInvoked: false,
    };
  }

  if (labTier === "pro") {
    const shareGpu = config.shareGpu && previewHardware.gpus.length > 0;
    if (!shareGpu) {
      throw new Error(
        "a Pro session needs you to dedicate a GPU (shareGpu); that GPU is exclusive for the session, not a concurrent slice",
      );
    }
    const id =
      config.sharedGpuId && previewHardware.gpus.some((gpu) => gpu.id === config.sharedGpuId)
        ? config.sharedGpuId
        : previewHardware.gpus[0]?.id;
    if (!id) {
      throw new Error(
        "a Pro session needs a dedicated GPU id (sharedGpuId); GPU assignment is exclusive for the session, not a concurrent slice",
      );
    }
    return {
      backend: "qemu-vfio",
      labTier: "pro",
      vcpuCount: cores,
      memoryMib: memoryBytes / BYTES_PER_MIB,
      kernelPath: null,
      rootfsPath: null,
      gpu: { id, exclusive: true },
      hypervisorInvoked: false,
    };
  }

  throw new Error(`unknown lab tier ${JSON.stringify(labTier)}; expected "standard" or "pro"`);
}

export function buildLaunchConfig(config: SharingConfig, labTier: string): Promise<LaunchConfig> {
  if (IS_PREVIEW) {
    try {
      return Promise.resolve(previewBuildLaunchConfig(config, labTier));
    } catch (error: unknown) {
      return Promise.reject(error instanceof Error ? error : new Error(String(error)));
    }
  }

  return invoke<LaunchConfig>("build_launch_config", { config, labTier });
}

function previewPlanGpuBind(config: SharingConfig): GpuBindStatus {
  if (!config.shareGpu || previewHardware.gpus.length === 0) {
    return {
      action: "refused",
      backend: null,
      gpuId: null,
      pciAddress: null,
      previousDriver: null,
      targetDriver: null,
      steps: [],
      bound: false,
      rolledBack: false,
      exclusive: false,
      log: "a Pro GPU bind needs you to dedicate a GPU (shareGpu); that GPU is exclusive for the session, not a concurrent slice",
      hypervisorInvoked: false,
    };
  }

  const id =
    config.sharedGpuId && previewHardware.gpus.some((gpu) => gpu.id === config.sharedGpuId)
      ? config.sharedGpuId
      : previewHardware.gpus[0]?.id;
  if (!id) {
    return {
      action: "refused",
      backend: null,
      gpuId: null,
      pciAddress: null,
      previousDriver: null,
      targetDriver: null,
      steps: [],
      bound: false,
      rolledBack: false,
      exclusive: false,
      log: "a Pro session needs a dedicated GPU id (sharedGpuId); GPU assignment is exclusive for the session, not a concurrent slice",
      hypervisorInvoked: false,
    };
  }

  const pciAddress = id === "GPU-TEST-3060" ? "0000:02:00.0" : "0000:01:00.0";
  return {
    action: "planned",
    backend: "qemu-vfio",
    gpuId: id,
    pciAddress,
    previousDriver: "nvidia",
    targetDriver: "vfio-pci",
    steps: [
      { action: "unbind", driver: "nvidia", device: pciAddress },
      { action: "bind", driver: "vfio-pci", device: pciAddress },
    ],
    bound: false,
    rolledBack: false,
    exclusive: true,
    log: `plan: unbind GPU ${id} from nvidia then bind vfio-pci at ${pciAddress}. GPU ${id} is assigned exclusively to the guest for this Pro session; the host loses that GPU until the session ends. CPU and RAM remain a concurrent slice. Not applied. Hypervisor not invoked.`,
    hypervisorInvoked: false,
  };
}

export function planGpuBind(config: SharingConfig): Promise<GpuBindStatus> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewPlanGpuBind(config));
  }

  return invoke<GpuBindStatus>("plan_gpu_bind", { config });
}

export function loadSharingConfig(): Promise<SharingConfig> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewStore);
  }

  return invoke<SharingConfig>("load_sharing_config");
}

/** Returns the config as it was stored, which may be clamped by the backend. */
export function saveSharingConfig(config: SharingConfig): Promise<SharingConfig> {
  if (IS_PREVIEW) {
    previewStore = config;
    return Promise.resolve(config);
  }

  return invoke<SharingConfig>("save_sharing_config", { config });
}

export function getConfigFilePath(): Promise<string> {
  if (IS_PREVIEW) {
    return Promise.resolve("(browser preview — nothing is written to disk)");
  }

  return invoke<string>("get_config_file_path");
}

export function loadHostRegistration(): Promise<HostRegistration | null> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewRegistration);
  }

  return invoke<HostRegistration | null>("load_host_registration");
}

/**
 * Registers the on-disk clamped caps with the Matchmaker from Rust.
 * The webview never fetches the Matchmaker (CSP allows no remote origins).
 */
export function registerDeclaredCaps(): Promise<HostRegistration> {
  if (IS_PREVIEW) {
    previewRegistration = {
      hostId: "00000000-0000-4000-8000-000000000001",
      status: "pending",
      displayName: "preview-host",
      labTier: "standard",
      sharedLogicalCores: previewStore.sharedLogicalCores,
      sharedMemoryBytes: previewStore.sharedMemoryBytes,
      shareGpu: previewStore.shareGpu,
      sharedGpuId: previewStore.sharedGpuId,
    };
    return Promise.resolve(previewRegistration);
  }

  return invoke<HostRegistration>("register_declared_caps");
}

export function getAgentStatus(): Promise<AgentStatus> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewStatus());
  }

  return invoke<AgentStatus>("get_agent_status");
}

/** Immediate pause/resume. Writes `sharingEnabled` in Rust; does not send slider drafts. */
export function setSharingEnabled(enabled: boolean): Promise<SharingConfig> {
  if (IS_PREVIEW) {
    previewStore = { ...previewStore, sharingEnabled: enabled };
    return Promise.resolve(previewStore);
  }

  return invoke<SharingConfig>("set_sharing_enabled", { enabled });
}

/** Browser-only: pretends stub provision already ran so the idle/in-session pill can be checked. */
export function previewSetInSession(inSession: boolean): void {
  if (IS_PREVIEW) {
    previewInSession = inSession;
  }
}
