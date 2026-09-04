import type { Notice, SessionResponse, UserPublic } from "../types";

interface SessionPanelProps {
  user: UserPublic;
  session: SessionResponse | null;
  notice: Notice | null;
  busy: boolean;
  onRequest: () => void;
  onEnd: () => void;
  onSignOut: () => void;
}

function noticeText(notice: Notice): string {
  switch (notice.kind) {
    case "occupied":
      return "Another user already holds the host. End that session (or wait) before requesting again.";
    case "ended":
      return "Session ended. The host is free to request again.";
    case "existing":
      return "You already have an active session. Showing it instead of creating a new one.";
    case "error":
      return notice.message;
  }
}

/** Live desktop vs RFC 2606 `.invalid` placeholders (seed + provision stubs). */
export function isLiveKasmUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return false;
    }
    const labels = parsed.hostname.split(".");
    const tld = labels[labels.length - 1];
    return tld !== "invalid";
  } catch {
    return false;
  }
}

export function SessionPanel({
  user,
  session,
  notice,
  busy,
  onRequest,
  onEnd,
  onSignOut,
}: SessionPanelProps) {
  const live = session ? isLiveKasmUrl(session.connection.kasmUrl) : false;

  return (
    <div className="panel">
      <div className="who">
        <span>
          Signed in as <strong>{user.email}</strong>
        </span>
        <button type="button" className="ghost" disabled={busy} onClick={onSignOut}>
          Sign out
        </button>
      </div>

      {notice ? (
        <p className="notice" data-kind={notice.kind} role="status">
          {noticeText(notice)}
        </p>
      ) : null}

      {session ? (
        <>
          <span className="placeholder-flag">
            {live
              ? "Live session URL — opens on the host PC"
              : "Connection URL is a placeholder, not a live desktop"}
          </span>
          <dl>
            <dt>Session id</dt>
            <dd>{session.sessionId}</dd>
            <dt>Host id</dt>
            <dd>{session.hostId}</dd>
            <dt>Lab tier</dt>
            <dd>{session.labTier}</dd>
            <dt>Started at</dt>
            <dd>{session.startedAt}</dd>
            <dt>kasmUrl</dt>
            <dd>{session.connection.kasmUrl}</dd>
            <dt>kasmToken</dt>
            <dd>{session.connection.kasmToken ?? "(none)"}</dd>
          </dl>
          {live ? (
            <p className="hint">
              Kasm is not embedded here (iframes are usually blocked). Open desktop
              uses a new tab. Paste any token into Kasm if it asks.
            </p>
          ) : (
            <p className="hint">
              Seed and mock-provision URLs use the <code>.invalid</code> TLD. A real
              Kasm URL from <code>docs/runbooks/gate-fc.md</code> or{" "}
              <code>docs/runbooks/gate-whpx.md</code> is required before Open
              desktop appears.
            </p>
          )}
          <div className="row end-row">
            {live ? (
              <a
                className="open-link"
                href={session.connection.kasmUrl}
                target="_blank"
                rel="noreferrer"
              >
                Open desktop
              </a>
            ) : null}
            <button type="button" className="danger" disabled={busy} onClick={onEnd}>
              {busy ? "Working…" : "End session"}
            </button>
          </div>
        </>
      ) : (
        <div className="row">
          <button type="button" disabled={busy} onClick={onRequest}>
            {busy ? "Working…" : "Request session"}
          </button>
        </div>
      )}
    </div>
  );
}
