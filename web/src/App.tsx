import { useState } from "react";

import { ApiError, endSession, listSessions, login, matchmakerUrl, requestSession, signup } from "./api";
import { AuthForm } from "./components/AuthForm";
import { SessionPanel } from "./components/SessionPanel";
import { mapIntentToLabTier, canRequestSession } from "./intent";
import { interpretSessionError, interpretSessionSuccess } from "./sessionRequest";
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
    } catch {
      setHistory(null);
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
      await endSession(token, session.sessionId);
      setSession(null);
      setNotice({ kind: "ended" });
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
        <p className="kicker">Matchmaker harness</p>
        <h1>Lab session</h1>
        <p className="lede">
          Sign in, say what you want to do, and book a session. Intent maps to
          Standard (CPU/RAM slice) or Pro (exclusive GPU). Open desktop appears
          only for a live Kasm URL — seed and mock-provision use <code>.invalid</code>{" "}
          placeholders. This is not the product UI.
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
          <section className="panel history">
            <h2>Session history</h2>
            <p className="hint">
              {history
                ? `${history.creditsDebited} credits debited on the internal ledger. Not a card charge.`
                : "Could not load credits yet."}
            </p>
            {history && history.sessions.length === 0 ? (
              <p className="hint">No sessions yet.</p>
            ) : null}
            {history && history.sessions.length > 0 ? (
              <ul className="history-list">
                {history.sessions.map((item) => (
                  <li key={item.sessionId}>
                    <span className="history-status">{item.status}</span>
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
