import { PixelIcon } from "./PixelIcon";

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
  /** Kept for callers; the signed-in user now shows in the app bar. */
  user?: UserPublic;
  session: SessionResponse | null;
  notice: Notice | null;
  busy: boolean;
  intent: string;
  onIntent: (value: string) => void;
  onRequest: () => void;
  onEnd: () => void;
  onSignOut?: () => void;
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

function TierIcon({ tier }: { tier: string }) {
  return tier === "pro" ? <PixelIcon name="gamepad" size={18} /> : <PixelIcon name="window" size={18} />;
}

export function SessionPanel({
  session,
  notice,
  busy,
  intent,
  onIntent,
  onRequest,
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
    <section className="card session-card" aria-labelledby="session-title">
      {notice ? (
        <p className="notice" data-kind={notice.kind} role="status">
          {noticeText(notice)}
        </p>
      ) : null}

      {session && starting ? (
        <div className="session-starting">
          <div className="session-head">
            <span className="tier-chip" data-tier={session.labTier}>
              <TierIcon tier={session.labTier} />
              {session.labTier === "pro" ? "Pro" : "Standard"}
            </span>
          </div>
          <h2 id="session-title">Starting your computer</h2>
          <p className="starting" role="status" aria-live="polite">
            <PixelIcon name="spark" size={18} className="spin" />
            Starting your computer. A host is booting your desktop; this usually takes one to three
            minutes. This page updates on its own.
          </p>
          <div className="boot-track" aria-hidden="true">
            <span />
          </div>
          <p className="hint">You are not charged while it starts. You can cancel at any time.</p>
          <div className="row">
            <button type="button" className="btn btn-danger" disabled={busy} onClick={onEnd}>
              {busy ? "Working…" : "Cancel"}
            </button>
          </div>
        </div>
      ) : session ? (
        <div className="session-live">
          <div className="session-head">
            <span className="tier-chip" data-tier={session.labTier}>
              <TierIcon tier={session.labTier} />
              {session.labTier === "pro" ? "Pro" : "Standard"}
            </span>
            <span className="placeholder-flag" data-live={live ? "true" : "false"}>
              {live
                ? isPro
                  ? "Live Sunshine locator — desktop is Moonlight, not Kasm"
                  : "Live browser desktop — protected by your session link"
                : "Connection URL is a placeholder, not a live desktop"}
            </span>
          </div>
          <h2 id="session-title">Your computer is ready</h2>
          <p className="hint">
            {live
              ? isPro
                ? "Sunshine's HTTPS page is the guest web UI. Use Moonlight for the desktop. Do not iframe the stream here."
                : "Open desktop launches the desktop in a new browser tab. Your session link handles access; there is no second desktop login."
              : "This host returned a placeholder address, so there is no desktop to open yet."}
          </p>

          {session.labTier === "pro" || isPro ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY}
            </p>
          ) : null}
          {moonlight ? (
            <p className="moonlight" role="status">
              Moonlight client: host <strong>{moonlight.host}</strong> · app {moonlight.app}. That is the
              live Pro desktop, not a browser iframe.
            </p>
          ) : null}

          <div className="row">
            {desktopLink ? (
              <a className="open-link" href={desktopLink.href} target="_blank" rel="noreferrer">
                {desktopLink.label}
              </a>
            ) : null}
            <button type="button" className="btn btn-danger" disabled={busy} onClick={onEnd}>
              {busy ? "Working…" : "End session"}
            </button>
          </div>

          <details className="conn-details">
            <summary>Connection details</summary>
            <dl>
              <dt>Session</dt>
              <dd>{session.sessionId}</dd>
              <dt>Host</dt>
              <dd>{session.hostId}</dd>
              <dt>Started</dt>
              <dd>{session.startedAt}</dd>
              <dt>Streamer</dt>
              <dd>{streamer}</dd>
              {isPro ? (
                <>
                  <dt>Sunshine URL</dt>
                  <dd>{session.connection?.sunshineUrl ?? "(none)"}</dd>
                  <dt>Moonlight</dt>
                  <dd>
                    {session.connection?.moonlight
                      ? `${session.connection.moonlight.host} · ${session.connection.moonlight.app}`
                      : "(none)"}
                  </dd>
                </>
              ) : (
                <>
                  <dt>Desktop URL</dt>
                  <dd>{session.connection?.kasmUrl ?? "(none)"}</dd>
                  <dt>Desktop token</dt>
                  <dd>{session.connection?.kasmToken ?? "(none)"}</dd>
                </>
              )}
              {session.storage ? (
                <>
                  <dt>Storage</dt>
                  <dd>
                    {session.storage.endpoint} / {session.storage.bucket} — mounted inside the desktop
                  </dd>
                  <dt>Saved as of</dt>
                  <dd>
                    {session.storage.savedAsOf
                      ? session.storage.savedAsOf
                      : "updates inside the guest after rclone flush"}
                  </dd>
                </>
              ) : null}
              {session.mesh ? (
                <>
                  <dt>Desktop network name</dt>
                  <dd>
                    {session.mesh.guestHostname}
                    {session.mesh.exitNodeApplied && session.mesh.exitNode
                      ? ` · exit-node ${session.mesh.exitNode}`
                      : " · exit-node not applied"}
                  </dd>
                </>
              ) : null}
            </dl>
          </details>
        </div>
      ) : (
        <form
          className="composer"
          onSubmit={(event) => {
            event.preventDefault();
            if (canRequest && !busy) {
              onRequest();
            }
          }}
        >
          <h2 id="session-title">New session</h2>
          <label htmlFor="lab-intent" className="composer-label">
            What do you want to do today?
          </label>
          <textarea
            id="lab-intent"
            name="intent"
            rows={3}
            disabled={busy}
            value={intent}
            onChange={(event) => onIntent(event.target.value)}
            placeholder="Write code, edit a video, play a game with a dedicated GPU…"
          />
          <div className="chips" role="group" aria-label="Examples">
            {INTENT_PRESETS.map((preset) => (
              <button
                key={preset.label}
                type="button"
                className="chip"
                aria-pressed={intent === preset.text}
                disabled={busy}
                onClick={() => onIntent(preset.text)}
              >
                {preset.label}
              </button>
            ))}
          </div>

          <div className="tier-preview" data-tier={canRequest ? mappedTier : "none"}>
            {canRequest ? (
              <>
                <TierIcon tier={mappedTier} />
                <p>
                  <strong>{mappedTier === "pro" ? "Pro" : "Standard"}</strong>
                  {showExclusivity ? " — exclusive GPU. " : " — CPU and memory slice. "}
                  Matchmaker auto-matches you with a free, approved computer.
                </p>
              </>
            ) : (
              <p>Describe your work and we'll choose Standard or Pro for you.</p>
            )}
          </div>
          {showExclusivity ? (
            <p className="exclusivity" role="status">
              {PRO_GPU_EXCLUSIVITY} You open it with Moonlight.
            </p>
          ) : canRequest ? (
            <p className="hint">{STANDARD_SLICE_HINT}</p>
          ) : null}

          <div className="row">
            <button type="submit" className="btn btn-rent btn-lg" disabled={busy || !canRequest}>
              {busy ? "Working…" : "Request session"}
            </button>
          </div>
        </form>
      )}
    </section>
  );
}
