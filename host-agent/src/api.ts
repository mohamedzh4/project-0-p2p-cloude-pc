import { invoke } from "@tauri-apps/api/core";

import { isTauriRuntime, previewConfig, previewHardware } from "./preview";
import type { HardwareSnapshot, SharingConfig } from "./types";

/** True only when the UI runs in a plain browser during development. */
export const IS_PREVIEW = import.meta.env.DEV && !isTauriRuntime();

let previewStore: SharingConfig = previewConfig;

export function getHardwareInfo(): Promise<HardwareSnapshot> {
  if (IS_PREVIEW) {
    return Promise.resolve(previewHardware);
  }

  return invoke<HardwareSnapshot>("get_hardware_info");
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
