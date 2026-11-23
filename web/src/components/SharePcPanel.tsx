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

const STEPS: Record<HostKitOs, string[]> = {
  windows: [
    "Unzip the folder and double-click setup.cmd, then click Yes.",
    "If Windows says “Windows protected your PC”, click “More info”, then “Run anyway”.",
    "Setup downloads Host Agent and the desktop disk (about 1.3 GB).",
  ],
  linux: [
    "Ubuntu, Debian or Arch: unzip the folder and run bash linux-host-setup.sh in it.",
    "Setup downloads Host Agent and the desktop disk (about 1.3 GB).",
  ],
};

/** Lend this PC: download a setup kit made for the signed-in account. */
export function SharePcPanel({ kits, busy, message, onDownload }: SharePcPanelProps) {
  const offered = (kits ?? []).filter((kit) => kit.available);
  return (
    <section className="panel share-pc">
      <h2>Share my PC</h2>
      <p className="hint">
        Lend your computer's spare CPU and memory and earn credits. The setup is
        made for your account; do not send it to someone else.
      </p>
      {kits !== null && offered.length === 0 ? (
        <p className="hint">Downloads are not available yet. Try again later.</p>
      ) : null}
      {offered.map((kit) => (
        <div className="share-pc-os" key={kit.os}>
          <button type="button" disabled={busy !== null} onClick={() => onDownload(kit.os)}>
            {busy === kit.os ? "Preparing…" : LABELS[kit.os]}
          </button>
          <ol className="hint">
            {STEPS[kit.os].map((step) => (
              <li key={step}>{step}</li>
            ))}
            <li>Open Host Agent, choose what to share, and click “Save and register”.</li>
          </ol>
        </div>
      ))}
      <p className="hint">Your PC is offered to renters once the operator approves it.</p>
      {message ? (
        <p className="notice" data-kind="error" role="status">
          {message}
        </p>
      ) : null}
    </section>
  );
}
