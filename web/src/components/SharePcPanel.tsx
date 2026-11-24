import type { HostKitOffer, HostKitOs } from "../api";

export interface SharePcPanelProps {
  kits: HostKitOffer[] | null;
  busy: HostKitOs | null;
  message: string | null;
  onDownload: (os: HostKitOs) => void;
}

const LABELS: Record<HostKitOs, string> = {
  windows: "Download for Windows",
  linux: "Download for Linux",
};

const TITLES: Record<HostKitOs, string> = {
  windows: "Windows 10 or 11",
  linux: "Ubuntu, Debian or Arch",
};

const STEPS: Record<HostKitOs, string[]> = {
  windows: [
    "Unzip the folder and double-click setup.cmd, then click Yes.",
    "If Windows says “Windows protected your PC”, click “More info”, then “Run anyway”.",
    "Setup downloads Host Agent and the desktop disk (about 1.3 GB).",
  ],
  linux: [
    "Unzip the folder and run bash linux-host-setup.sh in it.",
    "Setup downloads Host Agent and the desktop disk (about 1.3 GB).",
  ],
};

/** Lend this PC: download a setup kit made for the signed-in account. */
export function SharePcPanel({ kits, busy, message, onDownload }: SharePcPanelProps) {
  const offered = (kits ?? []).filter((kit) => kit.available);
  return (
    <section className="share-pc">
      {kits === null ? <p className="hint">Checking which downloads are ready…</p> : null}
      {kits !== null && offered.length === 0 ? (
        <p className="notice" data-kind="noHost">
          Downloads are not available yet. Try again later.
        </p>
      ) : null}
      <div className="kit-grid">
        {offered.map((kit) => (
          <article className="card kit" key={kit.os}>
            <h3>{TITLES[kit.os]}</h3>
            <ol className="kit-steps">
              {STEPS[kit.os].map((step) => (
                <li key={step}>{step}</li>
              ))}
              <li>Open Host Agent, choose what to share, and click “Save and register”.</li>
            </ol>
            <button
              type="button"
              className="btn btn-share btn-block"
              disabled={busy !== null}
              onClick={() => onDownload(kit.os)}
            >
              {busy === kit.os ? "Preparing…" : LABELS[kit.os]}
            </button>
          </article>
        ))}
      </div>
      {message ? (
        <p className="notice" data-kind="error" role="status">
          {message}
        </p>
      ) : null}
    </section>
  );
}
