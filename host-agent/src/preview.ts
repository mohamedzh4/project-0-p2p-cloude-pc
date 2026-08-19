/*
 * Sample data for `npm run dev` in a plain browser, where no Tauri backend is
 * attached. It exists so the UI can be worked on without a Rust toolchain, and
 * is compiled out of production builds by the `import.meta.env.DEV` guard in
 * api.ts. The window always shows a banner in this mode: these numbers describe
 * nobody's real machine.
 */

import { BYTES_PER_GIB } from "./format";
import type { HardwareSnapshot, SharingConfig } from "./types";

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
      model: "NVIDIA GeForce RTX 4070",
      vendor: "nvidia",
      vramBytes: 12 * BYTES_PER_GIB,
    },
  ],
  gpuNote: null,
};

export const previewConfig: SharingConfig = {
  version: 1,
  sharingEnabled: false,
  sharedLogicalCores: 16,
  sharedMemoryBytes: 32 * BYTES_PER_GIB,
  shareGpu: false,
};
