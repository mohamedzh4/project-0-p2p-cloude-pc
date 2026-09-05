import {
  INTENT_PRESETS,
  PRO_GPU_EXCLUSIVITY,
  STANDARD_SLICE_HINT,
  canRequestSession,
  gpuExclusivityVisible,
  mapIntentToLabTier,
} from "../intent";
import { isLiveConnectionUrl, sessionLocator, sessionStreamer } from "../connection";
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
  const streamer = session ? sessionStreamer(session) : null;
  const locator = session ? sessionLocator(session) : null;
  const live = isLiveConnectionUrl(locator);
  const mappedTier = mapIntentToLabTier(intent);
  const showExclusivity = gpuExclusivityVisible(mappedTier);
  const canRequest = canRequestSession(intent);
  const isPro = streamer === "sunshine";

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
              ? isPro
                ? "Live Sunshine locator — open with Moonlight, not Kasm"
                : "Live session URL — opens on the host PC"
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
            <dt>streamer</dt>
            <dd>{streamer}</dd>
            {isPro ? (
              <>
                <dt>sunshineUrl</dt>
                <dd>{session.connection.sunshineUrl ?? "(none)"}</dd>
                <dt>moonlight</dt>
                <dd>
                  {session.connection.moonlight
                    ? `${session.connection.moonlight.host} · ${session.connection.moonlight.app}`
                    : "(none)"}
                </dd>
              </>
            ) : (
              <>
                <dt>kasmUrl</dt>
                <dd>{session.connection.kasmUrl ?? "(none)"}</dd>
                <dt>kasmToken</dt>
                <dd>{session.connection.kasmToken ?? "(none)"}</dd>
              </>
            )}
            {session.storage ? (
              <>
                <dt>storage (guest rclone)</dt>
                <dd>
                  {session.storage.endpoint} / {session.storage.bucket} — mount
                  inside the guest only
                </dd>
              </>
            ) : null}
            {session.mesh ? (
              <>
                <dt>guest mesh</dt>
                <dd>
                  {session.mesh.guest.hostname} · {session.mesh.guest.aclTags.join(", ")}
                  {session.mesh.exitNodeApplied && session.mesh.exitNode
                    ? ` · exit-node ${session.mesh.exitNode}`
                    : " · exit-node not applied"}
                </dd>
                <dt>user mesh</dt>
                <dd>
                  {session.mesh.user.hostname} · {session.mesh.user.aclTags.join(", ")}
                </dd>
              </>
            ) : null}
          </dl>
          {session.labTier === "pro" || isPro ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY}
            </p>
          ) : null}
          {live ? (
            <p className="hint">
              {isPro
                ? "Pro streams Sunshine from inside the guest. Use a Moonlight client with the host below. Selkies-GStreamer is a documented browser alternative, not this API. Do not iframe the stream here."
                : "Kasm is not embedded here (iframes are usually blocked). Open desktop uses a new tab. Paste any token into Kasm if it asks."}
            </p>
          ) : (
            <p className="hint">
              {isPro ? (
                <>
                  Stub Sunshine/Moonlight locators use the <code>.invalid</code> TLD.
                  GATE-PRO is operator-declared closed (2026-09-05); a live stream
                  still is not in this repo.
                </>
              ) : (
                <>
                  Seed and mock-provision URLs use the <code>.invalid</code> TLD.
                  GATE-FC / GATE-WHPX are operator-declared closed (2026-09-05);
                  Open desktop stays hidden until a live Kasm URL exists.
                </>
              )}
            </p>
          )}
          <div className="row end-row">
            {live && locator && !isPro ? (
              <a className="open-link" href={locator} target="_blank" rel="noreferrer">
                Open desktop
              </a>
            ) : null}
            {live && locator && isPro ? (
              <a className="open-link" href={locator} target="_blank" rel="noreferrer">
                Open Sunshine URL
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
              {PRO_GPU_EXCLUSIVITY} Streaming is Sunshine in the guest (Moonlight
              client), not Kasm.
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
