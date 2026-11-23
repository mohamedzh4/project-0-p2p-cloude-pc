import { useEffect, useState } from "react";

import {
  ApiError,
  currentSession,
  downloadHostKit,
  endSession,
  getSession,
  listHostKits,
  listSessions,
  login,
  matchmakerUrl,
  requestSession,
  signup,
} from "./api";
import { AuthForm } from "./components/AuthForm";
import { SessionPanel } from "./components/SessionPanel";
import { SharePcPanel } from "./components/SharePcPanel";
import type { HostKitOffer, HostKitOs } from "./api";
import { mapIntentToLabTier, canRequestSession } from "./intent";
import {
  creditsSummary,
  endedNotice,
  interpretPolledSession,
  interpretSessionError,
  interpretSessionSuccess,
  pollIntervalMs,
} from "./sessionRequest";
import type { AuthMode, Notice, SessionHistoryResponse, SessionResponse, UserPublic } from "./types";

export default function App() {
  const configured = matchmakerUrl();
  const [mode, setMode] = useState<AuthMode>("signup");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<UserPublic | null>(null);
  const [session, setSession] = useState<SessionResponse | null>(null);
  const [history, setHistory] = useState<SessionHistoryResponse | null>(null);
  const [intent, setIntent] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  const [kits, setKits] = useState<HostKitOffer[] | null>(null);
  const [kitBusy, setKitBusy] = useState<HostKitOs | null>(null);
  const [kitMessage, setKitMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!token) {
      setKits(null);
      return;
    }
    let cancelled = false;
    listHostKits(token)
      .then((result) => {
        if (!cancelled) setKits(result.data.kits);
      })
      .catch(() => {
        if (!cancelled) setKits([]);
      });
    return () => {
      cancelled = true;
    };
  }, [token]);

  async function handleKitDownload(os: HostKitOs) {
    if (!token) {
      return;
    }
    setKitBusy(os);
    setKitMessage(null);
    try {
      const { blob, filename } = await downloadHostKit(token, os);
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = filename;
      document.body.appendChild(link);
      link.click();
      link.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 10000);
    } catch (error) {
      setKitMessage(error instanceof ApiError ? error.message : "Could not download the setup");
    } finally {
      setKitBusy(null);
    }
  }

  // While a session is held, ask the Matchmaker about it: every few seconds
  // while the desktop is starting, less often once it is ready (to notice a
  // host that went offline).
  const heldId = session?.sessionId ?? null;
  const heldInterval = session ? pollIntervalMs(session) : null;
  useEffect(() => {
    if (!token || !heldId || heldInterval == null) {
      return;
    }
    let cancelled = false;
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const polled = (await getSession(token, heldId)).data;
          const stillHeld = polled.status === "provisioning" || polled.status === "active";
          const replacement = stillHeld ? null : await currentSession(token);
          if (cancelled) {
            return;
          }
          const view = interpretPolledSession(polled, replacement);
          setSession(view.session);
          if (view.notice) {
            setNotice(view.notice);
          }
          if (!view.session || view.session.sessionId !== heldId) {
            void loadHistory(token);
          }
        } catch {
          // A missed poll is not an error: the next one will try again.
        }
      })();
    }, heldInterval);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, heldId, heldInterval]);

  if (!configured) {
    return (
      <main className="app">
        <p className="config-error">
          VITE_MATCHMAKER_URL is missing. Copy <code>web/.env.example</code> to{" "}
          <code>web/.env</code> and restart the Vite server.
        </p>
      </main>
    );
  }

  async function loadHistory(accessToken: string) {
    try {
      const result = await listSessions(accessToken);
      setHistory(result.data);
      return result.data;
    } catch {
      setHistory(null);
      return null;
    }
  }

  async function restoreHeldSession(accessToken: string) {
    try {
      const held = await currentSession(accessToken);
      if (held) {
        setSession(held);
        setNotice({ kind: "existing" });
      }
    } catch {
      // The history panel already shows whether the Matchmaker is reachable.
    }
  }

  async function handleAuth() {
    setBusy(true);
    setNotice(null);
    try {
      const result =
        mode === "signup" ? await signup(email, password) : await login(email, password);
      setToken(result.data.accessToken);
      setUser(result.data.user);
      setSession(null);
      setPassword("");
      await loadHistory(result.data.accessToken);
      await restoreHeldSession(result.data.accessToken);
    } catch (error) {
      setNotice({
        kind: "error",
        message: error instanceof ApiError ? error.message : "Could not authenticate",
      });
    } finally {
      setBusy(false);
    }
  }

  async function handleRequest() {
    if (!token || !canRequestSession(intent)) {
      return;
    }
    const labTier = mapIntentToLabTier(intent);
    setBusy(true);
    setNotice(null);
    try {
      const result = await requestSession(token, labTier);
      const view = interpretSessionSuccess(result.status, result.data);
      setSession(view.session);
      setNotice(view.notice);
      await loadHistory(token);
    } catch (error) {
      const view = interpretSessionError(error, labTier);
      setSession(view.session);
      setNotice(view.notice);
    } finally {
      setBusy(false);
    }
  }

  async function handleEnd() {
    if (!token || !session) {
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const result = await endSession(token, session.sessionId);
      setSession(null);
      setNotice(endedNotice(result.data.creditsDebited));
      await loadHistory(token);
    } catch (error) {
      setNotice({
        kind: "error",
        message: error instanceof ApiError ? error.message : "Could not end the session",
      });
    } finally {
      setBusy(false);
    }
  }

  function handleSignOut() {
    setToken(null);
    setUser(null);
    setSession(null);
    setHistory(null);
    setIntent("");
    setNotice(null);
  }

  return (
    <main className="app">
      <header className="reveal">
        <p className="kicker">Distributed Compute Lab</p>
        <h1>Your cloud computer</h1>
        <p className="lede">
          Sign in, describe your work, and we will connect you to an available
          computer. Standard sessions open in your browser; Pro sessions use
          an exclusive host GPU with Sunshine and Moonlight.
        </p>
      </header>

      {notice && !user ? (
        <p className="notice" data-kind={notice.kind} role="status">
          {notice.kind === "error" ? notice.message : null}
        </p>
      ) : null}

      {user && token ? (
        <>
          <SessionPanel
            user={user}
            session={session}
            notice={notice}
            busy={busy}
            intent={intent}
            onIntent={setIntent}
            onRequest={() => void handleRequest()}
            onEnd={() => void handleEnd()}
            onSignOut={handleSignOut}
          />
          <SharePcPanel
            kits={kits}
            busy={kitBusy}
            message={kitMessage}
            onDownload={(os) => void handleKitDownload(os)}
          />
          <section className="panel history">
            <h2>Session history</h2>
            <p className="hint">{creditsSummary(history)}</p>
            {history && history.sessions.length === 0 ? (
              <p className="hint">No sessions yet.</p>
            ) : null}
            {history && history.sessions.length > 0 ? (
              <ul className="history-list">
                {history.sessions.map((item) => (
                  <li key={item.sessionId}>
                    <span className="history-status">
                      {item.status === "failed" ? "did not start" : item.status}
                    </span>
                    <span>
                      {item.labTier}
                      {item.durationSeconds != null ? ` · ${item.durationSeconds}s` : ""}
                      {` · ${item.creditsDebited} credits`}
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
        </>
      ) : (
        <AuthForm
          mode={mode}
          busy={busy}
          email={email}
          password={password}
          onMode={setMode}
          onEmail={setEmail}
          onPassword={setPassword}
          onSubmit={() => void handleAuth()}
        />
      )}
    </main>
  );
}
