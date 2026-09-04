import {
  INTENT_PRESETS,
  PRO_GPU_EXCLUSIVITY,
  STANDARD_SLICE_HINT,
  canRequestSession,
  gpuExclusivityVisible,
  mapIntentToLabTier,
} from "../intent";
import type { Notice, SessionResponse, UserPublic } from "../types";

interface SessionPanelProps {
  user: UserPublic;
  session: SessionResponse | null;
  notice: Notice | null;
  busy: boolean;
  intent: string;
  onIntent: (value: string) => void;
  onRequest: () => void;
  onEnd: () => void;
  onSignOut: () => void;
}

function noticeText(notice: Notice): string {
  switch (notice.kind) {
    case "occupied":
      return "Another user already holds the host. End that session (or wait) before requesting again.";
    case "unavailable":
      return "No Pro host is free. Either none is approved and live, or that GPU is already assigned. Pro is exclusive for the session, not a concurrent slice.";
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
  intent,
  onIntent,
  onRequest,
  onSignOut,
  onEnd,
}: SessionPanelProps) {
  const live = session ? isLiveKasmUrl(session.connection.kasmUrl) : false;
  const mappedTier = mapIntentToLabTier(intent);
  const showExclusivity = gpuExclusivityVisible(mappedTier);
  const canRequest = canRequestSession(intent);

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
          {session.labTier === "pro" ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY}
            </p>
          ) : null}
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
        <form
          className="intent-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (canRequest && !busy) {
              onRequest();
            }
          }}
        >
          <div className="field">
            <label htmlFor="lab-intent">What do you want to do in your lab today?</label>
            <textarea
              id="lab-intent"
              name="intent"
              rows={3}
              disabled={busy}
              value={intent}
              onChange={(event) => onIntent(event.target.value)}
              placeholder="e.g. write code, or play a game with a dedicated GPU"
            />
          </div>
          <div className="intent-presets" role="group" aria-label="Example intents">
            {INTENT_PRESETS.map((preset) => (
              <button
                key={preset.label}
                type="button"
                className="ghost"
                aria-pressed={intent === preset.text}
                disabled={busy}
                onClick={() => onIntent(preset.text)}
              >
                {preset.label}
              </button>
            ))}
          </div>
          {canRequest ? (
            <p className="tier-preview">
              Maps to <strong>{mappedTier}</strong>
              {showExclusivity ? " — exclusive GPU" : " — CPU/RAM slice"}.
            </p>
          ) : (
            <p className="tier-preview">Answer the prompt to choose a lab tier.</p>
          )}
          {showExclusivity ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY}
            </p>
          ) : canRequest ? (
            <p className="hint">{STANDARD_SLICE_HINT}</p>
          ) : null}
          <div className="row">
            <button type="submit" disabled={busy || !canRequest}>
              {busy ? "Working…" : "Request session"}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
