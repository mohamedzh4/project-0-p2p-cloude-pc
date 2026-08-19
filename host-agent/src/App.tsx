import { useEffect, useMemo, useState } from "react";

import {
  getConfigFilePath,
  getHardwareInfo,
  IS_PREVIEW,
  loadSharingConfig,
  saveSharingConfig,
} from "./api";
import { HardwareCard } from "./components/HardwareCard";
import { ResourceSlider } from "./components/ResourceSlider";
import { BYTES_PER_GIB, formatCores, formatGib } from "./format";
import type { HardwareSnapshot, SharingConfig } from "./types";

type SaveState = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; message: string };

export default function App() {
  const [hardware, setHardware] = useState<HardwareSnapshot | null>(null);
  const [config, setConfig] = useState<SharingConfig | null>(null);
  const [savedConfig, setSavedConfig] = useState<SharingConfig | null>(null);
  const [configPath, setConfigPath] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  useEffect(() => {
    let active = true;

    Promise.all([getHardwareInfo(), loadSharingConfig(), getConfigFilePath()])
      .then(([detected, stored, path]) => {
        if (!active) {
          return;
        }

        setHardware(detected);
        setConfig(stored);
        setSavedConfig(stored);
        setConfigPath(path);
      })
      .catch((error: unknown) => {
        if (active) {
          setLoadError(error instanceof Error ? error.message : String(error));
        }
      });

    return () => {
      active = false;
    };
  }, []);

  const maxMemoryGib = hardware ? Math.floor(hardware.memory.totalBytes / BYTES_PER_GIB) : 0;
  const sharedMemoryGib = config ? Math.min(Math.round(config.sharedMemoryBytes / BYTES_PER_GIB), maxMemoryGib) : 0;
  const gpu = hardware?.gpus[0] ?? null;

  const isDirty = useMemo(
    () => JSON.stringify(config) !== JSON.stringify(savedConfig),
    [config, savedConfig],
  );

  function update(patch: Partial<SharingConfig>) {
    setConfig((current) => (current ? { ...current, ...patch } : current));
    setSaveState({ kind: "idle" });
  }

  async function handleSave() {
    if (!config) {
      return;
    }

    setSaveState({ kind: "saving" });

    try {
      // The backend re-clamps and returns what it actually stored.
      const stored = await saveSharingConfig(config);
      setConfig(stored);
      setSavedConfig(stored);
      setSaveState({ kind: "saved" });
    } catch (error: unknown) {
      setSaveState({
        kind: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  if (loadError) {
    return (
      <main className="app">
        <div className="panel">
          <h2>Host agent</h2>
          <p className="status" data-tone="error">
            Could not read this machine&apos;s hardware: {loadError}
          </p>
        </div>
      </main>
    );
  }

  if (!hardware || !config) {
    return (
      <main className="app">
        <div className="panel">
          <p className="status">Detecting hardware…</p>
        </div>
      </main>
    );
  }

  return (
    <main className="app">
      <header className="header reveal" data-reveal={0}>
        <div>
          <h1>Host Agent</h1>
          <p>Choose how much of this machine you are willing to lend out. Nothing is shared until you turn sharing on.</p>
        </div>
        <span className="pill" data-active={config.sharingEnabled}>
          {config.sharingEnabled ? "Sharing on" : "Sharing paused"}
        </span>
      </header>

      {IS_PREVIEW ? (
        <p className="notice reveal" data-reveal={1}>
          Browser preview: these readings are sample data. The packaged agent reads the real machine.
        </p>
      ) : null}

      <section className="panel reveal" data-reveal={1}>
        <h2>Detected hardware</h2>
        <div className="cards">
          <HardwareCard
            title="CPU"
            value={hardware.cpu.model}
            detail={
              hardware.cpu.physicalCores
                ? `${formatCores(hardware.cpu.logicalCores)} · ${hardware.cpu.physicalCores} physical`
                : formatCores(hardware.cpu.logicalCores)
            }
          />
          <HardwareCard title="Memory" value={formatGib(hardware.memory.totalBytes)} detail="Total installed RAM" />
          <HardwareCard
            title="GPU"
            value={gpu ? gpu.model : "No GPU detected"}
            detail={gpu ? (gpu.vramBytes ? `${formatGib(gpu.vramBytes)} VRAM` : "VRAM not reported") : hardware.gpuNote}
            empty={!gpu}
          />
        </div>
      </section>

      <section className="panel reveal" data-reveal={2}>
        <h2>What you share</h2>

        <div className="field">
          <label className="switch">
            <input
              type="checkbox"
              checked={config.sharingEnabled}
              onChange={(event) => update({ sharingEnabled: event.target.checked })}
            />
            Make this machine available for sessions
          </label>
        </div>

        <ResourceSlider
          id="shared-cores"
          label="CPU cores"
          readout={`${config.sharedLogicalCores} of ${hardware.cpu.logicalCores}`}
          min={0}
          max={hardware.cpu.logicalCores}
          value={config.sharedLogicalCores}
          hint="Leave at least one core for your own work."
          onChange={(value) => update({ sharedLogicalCores: value })}
        />

        <ResourceSlider
          id="shared-memory"
          label="Memory"
          readout={`${sharedMemoryGib} of ${maxMemoryGib} GiB`}
          min={0}
          max={maxMemoryGib}
          value={sharedMemoryGib}
          disabled={maxMemoryGib === 0}
          hint="Reserved while a session is running, so keep enough for your own applications."
          onChange={(value) => update({ sharedMemoryBytes: value * BYTES_PER_GIB })}
        />

        <div className="field">
          <label className="switch" data-disabled={!gpu}>
            <input
              type="checkbox"
              checked={config.shareGpu}
              disabled={!gpu}
              onChange={(event) => update({ shareGpu: event.target.checked })}
            />
            {gpu ? `Share the GPU (${gpu.model})` : "Share the GPU — unavailable, no GPU detected"}
          </label>
        </div>
      </section>

      <section className="panel reveal" data-reveal={3}>
        <div className="actions">
          <button type="button" onClick={handleSave} disabled={!isDirty || saveState.kind === "saving"}>
            {saveState.kind === "saving" ? "Saving…" : "Save configuration"}
          </button>

          {savedConfig ? (
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setConfig(savedConfig);
                setSaveState({ kind: "idle" });
              }}
              disabled={!isDirty}
            >
              Discard changes
            </button>
          ) : null}

          {saveState.kind === "error" ? (
            <span className="status" data-tone="error">
              {saveState.message}
            </span>
          ) : (
            <span className="status">
              {isDirty ? "Unsaved changes" : saveState.kind === "saved" ? "Saved" : "Up to date"}
            </span>
          )}
        </div>

        {configPath ? <p className="path">{configPath}</p> : null}
      </section>
    </main>
  );
}
