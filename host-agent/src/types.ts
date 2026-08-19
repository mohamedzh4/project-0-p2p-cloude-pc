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

export interface SharingConfig {
  version: number;
  sharingEnabled: boolean;
  sharedLogicalCores: number;
  sharedMemoryBytes: number;
  shareGpu: boolean;
}
