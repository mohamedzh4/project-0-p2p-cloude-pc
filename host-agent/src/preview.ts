/*
 * Sample data for `npm run dev` in a plain browser, where no Tauri backend is
 * attached. It exists so the UI can be worked on without a Rust toolchain, and
 * is compiled out of production builds by the `import.meta.env.DEV` guard in
 * api.ts. The window always shows a banner in this mode: these numbers describe
 * nobody's real machine. Two GPUs are listed so the exclusive-GPU picker can
 * be exercised without a dual-GPU host.
 */

import { BYTES_PER_GIB } from "./format";
import type { HardwareSnapshot, HostCapabilities, HypervisorStatus, GuestImageStatus, SharingConfig } from "./types";

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export const previewHardware: HardwareSnapshot = {
  cpu: {
    model: "AMD Ryzen 9 7950X 16-Core Processor",
    logicalCores: 32,
    physicalCores: 16,
  },
  memory: { totalBytes: 64 * BYTES_PER_GIB },
  gpus: [
    {
      id: "GPU-TEST-4070",
      model: "NVIDIA GeForce RTX 4070",
      vendor: "nvidia",
      vramBytes: 12 * BYTES_PER_GIB,
    },
    {
      id: "GPU-TEST-3060",
      model: "NVIDIA GeForce RTX 3060",
      vendor: "nvidia",
      vramBytes: 12 * BYTES_PER_GIB,
    },
  ],
  gpuNote: null,
};

/** Sample dual-GPU Linux host so the capability panel can be designed without metal. */
export const previewCapabilities: HostCapabilities = {
  os: "linux",
  kvm: "presentReadWrite",
  whpx: "notApplicable",
  iommu: "present",
  secondDisplayPath: true,
  gpuCount: 2,
  standard: {
    offered: true,
    reason: "This PC can host Standard sessions.",
  },
  pro: {
    offered: true,
    reason:
      "This PC can host Pro sessions when you dedicate a GPU. That GPU is exclusive for the session — not a concurrent slice like CPU or RAM.",
  },
  summary: "This Linux PC can host Standard and Pro sessions.",
};

/** Sample Linux host that already has Firecracker in the user cache (no live fetch). */
export const previewHypervisor: HypervisorStatus = {
  action: "skippedPresent",
  hypervisor: "firecracker",
  version: "v1.16.1",
  path: "/home/host/.cache/dcl-host-agent/hypervisors/firecracker/v1.16.1/firecracker",
  log: "skipped because present; firecracker v1.16.1 at /home/host/.cache/dcl-host-agent/hypervisors/firecracker/v1.16.1/firecracker (sha256 fixture)",
  hypervisorInvoked: false,
};

/** Sample Linux host that already has the signed Standard image (no live fetch). */
export const previewGuestImage: GuestImageStatus = {
  action: "skippedPresent",
  labTier: "standard",
  backend: "firecracker",
  path: "/home/host/.cache/dcl-host-agent/images/standard/rootfs.img",
  digest: "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe",
  log: "skipped because present; standard Firecracker guest image at /home/host/.cache/dcl-host-agent/images/standard/rootfs.img (sha256 6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe)",
  imageBooted: false,
};

export const previewConfig: SharingConfig = {
  version: 2,
  sharingEnabled: false,
  sharedLogicalCores: 16,
  sharedMemoryBytes: 32 * BYTES_PER_GIB,
  shareGpu: false,
  sharedGpuId: "GPU-TEST-4070",
};
