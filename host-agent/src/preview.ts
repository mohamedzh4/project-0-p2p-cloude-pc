/*
 * Sample data for `npm run dev` in a plain browser, where no Tauri backend is
 * attached. It exists so the UI can be worked on without a Rust toolchain, and
 * is compiled out of production builds by the `import.meta.env.DEV` guard in
 * api.ts. The window always shows a banner in this mode: these numbers describe
 * nobody's real machine. Two GPUs are listed so the exclusive-GPU picker can
 * be exercised without a dual-GPU host.
 */

import { BYTES_PER_GIB } from "./format";
import type { HardwareSnapshot, HostCapabilities, SharingConfig } from "./types";

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

export const previewConfig: SharingConfig = {
  version: 2,
  sharingEnabled: false,
  sharedLogicalCores: 16,
  sharedMemoryBytes: 32 * BYTES_PER_GIB,
  shareGpu: false,
  sharedGpuId: "GPU-TEST-4070",
};
