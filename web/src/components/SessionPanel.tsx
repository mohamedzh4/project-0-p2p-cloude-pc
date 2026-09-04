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
      return "Another user already holds the seeded host. End that session (or wait) before requesting again.";
    case "ended":
      return "Session ended. The host is free to request again.";
    case "existing":
      return "You already have an active session. Showing it instead of creating a new one.";
    case "error":
      return notice.message;
  }
}

/** Placeholder from matchmaker/.env.example — not a desktop on the Phase 0 host. */
export function isLiveKasmUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return false;
    }
    return parsed.hostname !== "example.invalid";
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
              ? "Phase 0 session — opens on the host PC"
              : "Seed is still a placeholder URL"}
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
              Set <code>SEED_HOST_KASM_URL</code> in <code>matchmaker/.env</code> to
              the live session on the Phase 0 host, then run{" "}
              <code>python -m matchmaker.seed</code> and request again.
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
