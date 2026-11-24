import { useEffect, useMemo, useState } from "react";

import {
  buildLaunchConfig,
  evaluateHostingOffers,
  getAgentStatus,
  getConfigFilePath,
  getDiagnosticsReport,
  getGuestImageStatus,
  getHardwareInfo,
  getHostCapabilities,
  getHypervisorStatus,
  IS_PREVIEW,
  loadHostRegistration,
  loadSharingConfig,
  planGpuBind,
  previewSetInSession,
  registerDeclaredCaps,
  saveSharingConfig,
  setSharingEnabled,
} from "./api";
import { CapabilityPanel } from "./components/CapabilityPanel";
import { PixelIcon } from "./components/PixelIcon";
import { SkyCanvas } from "./components/SkyCanvas";
import { HardwareCard } from "./components/HardwareCard";
import { GpuShareField } from "./components/GpuShareField";
import { ResourceSlider } from "./components/ResourceSlider";
import { BYTES_PER_GIB, formatCores, formatGib, formatGpuLabel } from "./format";
import {
  presenceLabel,
  type AgentStatus,
  type EligibilityReport,
  type GuestImageStatus,
  type HardwareSnapshot,
  type HostCapabilities,
  type HostRegistration,
  type HypervisorStatus,
  type GpuBindStatus,
  type LaunchConfig,
  type SharingConfig,
} from "./types";

type SaveState = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; message: string };

export default function App() {
  const [hardware, setHardware] = useState<HardwareSnapshot | null>(null);
  const [capabilities, setCapabilities] = useState<HostCapabilities | null>(null);
  const [hypervisor, setHypervisor] = useState<HypervisorStatus | null>(null);
  const [guestImage, setGuestImage] = useState<GuestImageStatus | null>(null);
  const [standardLaunch, setStandardLaunch] = useState<LaunchConfig | null>(null);
  const [standardLaunchError, setStandardLaunchError] = useState<string | null>(null);
  const [proLaunch, setProLaunch] = useState<LaunchConfig | null>(null);
  const [proLaunchError, setProLaunchError] = useState<string | null>(null);
  const [gpuBind, setGpuBind] = useState<GpuBindStatus | null>(null);
  const [offers, setOffers] = useState<EligibilityReport | null>(null);
  const [config, setConfig] = useState<SharingConfig | null>(null);
  const [savedConfig, setSavedConfig] = useState<SharingConfig | null>(null);
  const [configPath, setConfigPath] = useState("");
  const [registration, setRegistration] = useState<HostRegistration | null>(null);
  const [reportState, setReportState] = useState<"idle" | "copied" | "failed">("idle");

  async function handleCopyReport() {
    try {
      const report = await getDiagnosticsReport();
      await navigator.clipboard.writeText(report);
      setReportState("copied");
    } catch {
      setReportState("failed");
    }
  }
  const [status, setStatus] = useState<AgentStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  const [pauseError, setPauseError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;

    Promise.all([
      getHardwareInfo(),
      getHostCapabilities(),
      getHypervisorStatus(),
      getGuestImageStatus(),
      loadSharingConfig(),
      getConfigFilePath(),
      loadHostRegistration(),
      getAgentStatus(),
    ])
      .then(([detected, hostCapabilities, hypervisorStatus, guestImageStatus, stored, path, storedRegistration, agentStatus]) => {
        if (!active) {
          return;
        }

        setHardware(detected);
        setCapabilities(hostCapabilities);
        setHypervisor(hypervisorStatus);
        setGuestImage(guestImageStatus);
        setConfig(stored);
        setSavedConfig(stored);
        setConfigPath(path);
        setRegistration(storedRegistration);
        setStatus(agentStatus);
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

  useEffect(() => {
    if (!config) {
      return;
    }

    let active = true;

    Promise.allSettled([
      buildLaunchConfig(config, "standard"),
      buildLaunchConfig(config, "pro"),
      planGpuBind(config),
      evaluateHostingOffers(config),
    ]).then(([standard, pro, bind, hosting]) => {
        if (!active) {
          return;
        }

        if (standard.status === "fulfilled") {
          setStandardLaunch(standard.value);
          setStandardLaunchError(null);
        } else {
          setStandardLaunch(null);
          setStandardLaunchError(
            standard.reason instanceof Error ? standard.reason.message : String(standard.reason),
          );
        }

        if (pro.status === "fulfilled") {
          setProLaunch(pro.value);
          setProLaunchError(null);
        } else {
          setProLaunch(null);
          setProLaunchError(pro.reason instanceof Error ? pro.reason.message : String(pro.reason));
        }

        if (bind.status === "fulfilled") {
          setGpuBind(bind.value);
        } else {
          setGpuBind(null);
        }

        if (hosting.status === "fulfilled") {
          setOffers(hosting.value);
        }
      },
    );

    return () => {
      active = false;
    };
  }, [config]);

  useEffect(() => {
    let active = true;
    const id = window.setInterval(() => {
      getAgentStatus()
        .then((next) => {
          if (active) {
            setStatus(next);
          }
        })
        .catch(() => {
          /* occupancy updates are best-effort; pause still writes to disk */
        });
    }, 2000);

    return () => {
      active = false;
      window.clearInterval(id);
    };
  }, []);

  const maxMemoryGib = hardware ? Math.floor(hardware.memory.totalBytes / BYTES_PER_GIB) : 0;
  const sharedMemoryGib = config ? Math.min(Math.round(config.sharedMemoryBytes / BYTES_PER_GIB), maxMemoryGib) : 0;
  const gpus = hardware?.gpus ?? [];
  const primaryGpu = gpus[0] ?? null;
  const presence = status?.presence ?? (config?.sharingEnabled ? "idle" : "paused");

  const isDirty = useMemo(
    () => JSON.stringify(config) !== JSON.stringify(savedConfig),
    [config, savedConfig],
  );

  function update(patch: Partial<SharingConfig>) {
    setConfig((current) => (current ? { ...current, ...patch } : current));
    setSaveState({ kind: "idle" });
  }

  async function handleSharingEnabled(enabled: boolean) {
    if (!config) {
      return;
    }

    const previous = config.sharingEnabled;
    setPauseError(null);
    setConfig((current) => (current ? { ...current, sharingEnabled: enabled } : current));
    setSavedConfig((current) => (current ? { ...current, sharingEnabled: enabled } : current));
    setStatus((current) =>
      current
        ? {
            ...current,
            sharingEnabled: enabled,
            presence: !enabled ? "paused" : current.inSession ? "inSession" : "idle",
          }
        : current,
    );

    try {
      const stored = await setSharingEnabled(enabled);
      setConfig((current) => (current ? { ...current, sharingEnabled: stored.sharingEnabled } : current));
      setSavedConfig((current) =>
        current ? { ...current, sharingEnabled: stored.sharingEnabled } : current,
      );
      setStatus(await getAgentStatus());
    } catch (error: unknown) {
      setConfig((current) => (current ? { ...current, sharingEnabled: previous } : current));
      setSavedConfig((current) => (current ? { ...current, sharingEnabled: previous } : current));
      setPauseError(error instanceof Error ? error.message : String(error));
    }
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
      try {
        const registered = await registerDeclaredCaps();
        setRegistration(registered);
        setStatus(await getAgentStatus());
        setSaveState({ kind: "saved" });
      } catch (error: unknown) {
        setSaveState({
          kind: "error",
          message: `Saved locally, but Matchmaker registration failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        });
      }
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

  if (!hardware || !config || !capabilities || !hypervisor || !guestImage) {
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
      <SkyCanvas />
      <header className="header reveal" data-reveal={0}>
        <div>
          <div className="brand-row">
            <PixelIcon name="cloud" size={22} />
            Blue Sky
          </div>
          <h1>Host Agent</h1>
          <p>Choose how much of this machine you can spare while you keep using it. Pause at any time — that stops Matchmaker pings immediately.</p>
        </div>
        <span className="pill" data-presence={presence === "inSession" ? "in-session" : presence}>
          <PixelIcon name={presence === "paused" ? "pause" : "server"} size={18} />
          {presenceLabel(presence)}
        </span>
      </header>

      {IS_PREVIEW ? (
        <p className="notice reveal" data-reveal={1}>
          Browser preview: these readings are sample data and Matchmaker registration is simulated. The packaged agent reads the real machine, calls the Matchmaker from Rust, and can sit in the system tray.
        </p>
      ) : null}

      <section className="panel reveal" data-reveal={1}>
        <h2>Status</h2>
        <div className="status-board">
          <dl className="status-facts">
            <dt>Presence</dt>
            <dd>{presenceLabel(presence)}</dd>
            <dt>Earnings</dt>
            <dd>{status?.earningsNote ?? "earnings unavailable"}</dd>
          </dl>
          <button
            type="button"
            className={config.sharingEnabled ? "secondary" : undefined}
            onClick={() => void handleSharingEnabled(!config.sharingEnabled)}
          >
            {config.sharingEnabled ? "Pause sharing" : "Resume sharing"}
          </button>
        </div>
        {pauseError ? (
          <p className="status" data-tone="error">
            {pauseError}
          </p>
        ) : (
          <p className="hint">
            Pause writes locally and stops heartbeats on the next tick so this host becomes unselectable
            after it goes stale. It does not end an active guest session. Earnings are credits accrued
            on the Matchmaker ledger and are not paid out.
          </p>
        )}
        {IS_PREVIEW ? (
          <div className="preview-occupancy">
            <button
              type="button"
              className="secondary"
              onClick={() => {
                previewSetInSession(true);
                void getAgentStatus().then(setStatus);
              }}
            >
              Simulate in-session
            </button>
            <button
              type="button"
              className="secondary"
              onClick={() => {
                previewSetInSession(false);
                void getAgentStatus().then(setStatus);
              }}
            >
              Clear simulated session
            </button>
          </div>
        ) : null}
      </section>

      <section className="panel reveal" data-reveal={1}>
        <h2>Detected hardware</h2>
        <div className="cards">
          <HardwareCard
            icon="cpu"
            title="CPU"
            value={hardware.cpu.model}
            detail={
              hardware.cpu.physicalCores
                ? `${formatCores(hardware.cpu.logicalCores)} · ${hardware.cpu.physicalCores} physical`
                : formatCores(hardware.cpu.logicalCores)
            }
          />
          <HardwareCard icon="memory" title="Memory" value={formatGib(hardware.memory.totalBytes)} detail="Total installed RAM" />
          <HardwareCard
            icon="gpu"
            title="GPU"
            value={
              gpus.length === 0
                ? "No GPU detected"
                : gpus.length === 1
                  ? primaryGpu?.model ?? "No GPU detected"
                  : `${gpus.length} GPUs`
            }
            detail={
              gpus.length === 0
                ? hardware.gpuNote
                : gpus
                    .map(
                      (gpu) =>
                        formatGpuLabel(gpu.model, gpu.vramBytes) + (gpu.integrated ? " (integrated)" : ""),
                    )
                    .join(" · ")
            }
            empty={gpus.length === 0}
          />
        </div>
      </section>

      <CapabilityPanel
        capabilities={capabilities}
        hypervisor={hypervisor}
        guestImage={guestImage}
        standardLaunch={standardLaunch}
        standardLaunchError={standardLaunchError}
        proLaunch={proLaunch}
        proLaunchError={proLaunchError}
        gpuBind={gpuBind}
        offers={offers}
      />

      <section className="panel reveal" data-reveal={2}>
        <h2>What you share</h2>

        <div className="field">
          <label className="switch">
            <input
              type="checkbox"
              checked={config.sharingEnabled}
              onChange={(event) => void handleSharingEnabled(event.target.checked)}
            />
            Make this machine available for sessions while I keep using it
          </label>
        </div>

        <ResourceSlider
          id="shared-cores"
          icon="cpu"
          label="CPU cores"
          readout={`${config.sharedLogicalCores} of ${hardware.cpu.logicalCores}`}
          min={0}
          max={hardware.cpu.logicalCores}
          value={config.sharedLogicalCores}
          hint="Leave at least one core for your own work. A session needs at least 2 vCPU — below that, Standard is not offered."
          onChange={(value) => update({ sharedLogicalCores: value })}
        />

        <ResourceSlider
          id="shared-memory"
          icon="memory"
          label="Memory"
          readout={`${sharedMemoryGib} of ${maxMemoryGib} GiB`}
          min={0}
          max={maxMemoryGib}
          value={sharedMemoryGib}
          disabled={maxMemoryGib === 0}
          hint="Reserved while a session is running, so keep enough for your own applications. A session needs at least 4 GiB — below that, Standard is not offered."
          onChange={(value) => update({ sharedMemoryBytes: value * BYTES_PER_GIB })}
        />

        <GpuShareField
          gpus={gpus}
          shareGpu={config.shareGpu}
          sharedGpuId={config.sharedGpuId}
          disabled={!gpus.some((gpu) => !gpu.integrated)}
          onShareChange={(shareGpu) => update({ shareGpu })}
          onGpuChange={(sharedGpuId) => update({ sharedGpuId })}
        />
      </section>

      <section className="panel reveal" data-reveal={3}>
        <div className="actions">
          <button type="button" onClick={() => void handleSave()} disabled={!isDirty || saveState.kind === "saving"}>
            {saveState.kind === "saving"
              ? "Saving…"
              : IS_PREVIEW
                ? "Save configuration"
                : "Save and register"}
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

        <div className="registration">
          {registration ? (
            <>
              <p className="status">
                Matchmaker: {registration.status}
                {registration.status === "pending"
                  ? " — sessions will not land here until an operator approves this host"
                  : null}
              </p>
              <p className="path">{registration.hostId}</p>
              <p className="hint">
                While sharing is on, the packaged agent pings the Matchmaker from Rust every 10
                seconds. Pause sharing to drop out of matching after the host goes stale.
                Occupancy stays on active sessions, not a host flag.
              </p>
              {registration.shareGpu ? (
                <p className="hint gpu-warning">
                  Matchmaker registered GPU {registration.sharedGpuId ?? "unknown"} as exclusive for
                  the session, not a concurrent slice. This PC cannot use that card while a guest is
                  connected.
                </p>
              ) : null}
            </>
          ) : (
            <p className="status">
              Not registered with the Matchmaker yet. Save to declare these caps from Rust.
            </p>
          )}
        </div>

        <div className="registration">
          <button type="button" className="secondary" onClick={() => void handleCopyReport()}>
            Copy report
          </button>
          <p className="hint" role="status">
            {reportState === "copied"
              ? "Report copied. Paste it into your message to support."
              : reportState === "failed"
                ? "Could not copy the report. Try again."
                : "If something fails, copy this report and send it. It contains versions, what this PC can host and the last provision error. It contains no passwords or tokens."}
          </p>
        </div>
      </section>
    </main>
  );
}
