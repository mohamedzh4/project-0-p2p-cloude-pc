import {
  INTENT_PRESETS,
  PRO_GPU_EXCLUSIVITY,
  STANDARD_SLICE_HINT,
  canRequestSession,
  gpuExclusivityVisible,
  mapIntentToLabTier,
} from "../intent";
import {
  isLiveConnectionUrl,
  sessionDesktopLink,
  sessionLocator,
  sessionMoonlightClient,
  sessionStreamer,
} from "../connection";
import { sessionStatus } from "../sessionRequest";
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

export function noticeText(notice: Notice): string {
  switch (notice.kind) {
    case "noHost":
      return "No computer is free right now. Every host is in use or offline. Try again in a few minutes.";
    case "unavailable":
      return "No Pro host is free. Either none is approved and live, or that GPU is already assigned. Pro is exclusive for the session, not a concurrent slice.";
    case "failed":
      return `The computer could not start your desktop${
        notice.reason ? `: ${notice.reason}` : "."
      } You were not charged. Request again to try another host.`;
    case "interrupted":
      return "The computer hosting your session went offline and no other one is free. Files you saved in Persistent are kept. Request a new session when you are ready.";
    case "rematched":
      return "The computer hosting your session went offline, so we moved you to another one. Files you saved in Persistent are kept; anything else on the old desktop is gone.";
    case "ended":
      return notice.creditsDebited != null
        ? `Session ended. ${notice.creditsDebited} credits debited on the internal ledger (not a card charge).`
        : "Session ended.";
    case "existing":
      return "You already have a session. Showing it instead of creating a new one.";
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
  const desktopLink = session ? sessionDesktopLink(session) : null;
  const moonlight = session ? sessionMoonlightClient(session) : null;
  const mappedTier = mapIntentToLabTier(intent);
  const showExclusivity = gpuExclusivityVisible(mappedTier);
  const canRequest = canRequestSession(intent);
  const isPro = streamer === "sunshine";
  const starting = session ? sessionStatus(session) === "provisioning" : false;

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

      {session && starting ? (
        <>
          <p className="starting" role="status" aria-live="polite">
            <span className="spinner" aria-hidden="true" />
            Starting your computer. A host is booting your desktop; this usually
            takes one to three minutes. This page updates on its own.
          </p>
          <p className="hint">
            You are not charged while it starts. You can cancel at any time.
          </p>
          <div className="row end-row">
            <button type="button" className="danger" disabled={busy} onClick={onEnd}>
              {busy ? "Working…" : "Cancel"}
            </button>
          </div>
        </>
      ) : session ? (
        <>
          <span className="placeholder-flag" data-live={live ? "true" : "false"}>
            {live
              ? isPro
                ? "Live Sunshine locator — desktop is Moonlight, not Kasm"
                : "Live browser desktop — protected by your session link"
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
                <dd>{session.connection?.sunshineUrl ?? "(none)"}</dd>
                <dt>moonlight</dt>
                <dd>
                  {session.connection?.moonlight
                    ? `${session.connection.moonlight.host} · ${session.connection.moonlight.app}`
                    : "(none)"}
                </dd>
              </>
            ) : (
              <>
                <dt>kasmUrl</dt>
                <dd>{session.connection?.kasmUrl ?? "(none)"}</dd>
                <dt>kasmToken</dt>
                <dd>{session.connection?.kasmToken ?? "(none)"}</dd>
              </>
            )}
            {session.storage ? (
              <>
                <dt>storage (guest rclone)</dt>
                <dd>
                  {session.storage.endpoint} / {session.storage.bucket} — mount
                  inside the guest only
                </dd>
                <dt>saved as of</dt>
                <dd>
                  {session.storage.savedAsOf
                    ? session.storage.savedAsOf
                    : "updates inside the guest after rclone flush"}
                </dd>
              </>
            ) : null}
            {session.mesh ? (
              <>
                <dt>guest mesh</dt>
                <dd>
                  {session.mesh.guestHostname}
                  {session.mesh.exitNodeApplied && session.mesh.exitNode
                    ? ` · exit-node ${session.mesh.exitNode}`
                    : " · exit-node not applied"}
                </dd>
              </>
            ) : null}
          </dl>
          {session.labTier === "pro" || isPro ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY}
            </p>
          ) : null}
          {moonlight ? (
            <p className="moonlight" role="status">
              Moonlight client: host <strong>{moonlight.host}</strong> · app{" "}
              {moonlight.app}. That is the live Pro desktop, not a browser iframe.
            </p>
          ) : null}
          {live ? (
            <p className="hint">
              {isPro
                ? "Sunshine's HTTPS page is the guest web UI. Use Moonlight for the desktop. Selkies-GStreamer is a documented browser alternative, not this API. Do not iframe the stream here."
                : "Open desktop launches the VM's XFCE desktop in a new browser tab. The short-lived DCL session link handles access; there is no second desktop login."}
            </p>
          ) : (
            <p className="hint">
              {isPro ? (
                <>
                  Stub Sunshine/Moonlight locators use the <code>.invalid</code> TLD.
                  Open desktop stays hidden. Hardware is implemented in code, not
                  yet run on physical hardware.
                </>
              ) : (
                <>
                  Seed and mock-provision URLs use the <code>.invalid</code> TLD.
                  Open desktop stays hidden until Matchmaker returns a live Kasm
                  locator. Hardware is implemented in code, not yet run on
                  physical hardware.
                </>
              )}
            </p>
          )}
          <div className="row end-row">
            {desktopLink ? (
              <a
                className="open-link"
                href={desktopLink.href}
                target="_blank"
                rel="noreferrer"
              >
                {desktopLink.label}
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
              {showExclusivity ? " — exclusive GPU" : " — CPU/RAM slice"}. Matchmaker
              auto-matches an approved live host — this harness does not list hosts.
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
          <p className="hint">
            Host inventory is register + heartbeat on Matchmaker (then manual
            approve). Request session posts <code>labTier</code> only.
          </p>
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
