import { useState } from "react";

import { ApiError, endSession, login, matchmakerUrl, requestSession, signup } from "./api";
import { AuthForm } from "./components/AuthForm";
import { SessionPanel } from "./components/SessionPanel";
import { interpretSessionError, interpretSessionSuccess } from "./sessionRequest";
import type { AuthMode, Notice, SessionResponse, UserPublic } from "./types";

export default function App() {
  const configured = matchmakerUrl();
  const [mode, setMode] = useState<AuthMode>("signup");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<UserPublic | null>(null);
  const [session, setSession] = useState<SessionResponse | null>(null);
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
    if (!token) {
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const result = await requestSession(token);
      const view = interpretSessionSuccess(result.status, result.data);
      setSession(view.session);
      setNotice(view.notice);
    } catch (error) {
      const view = interpretSessionError(error);
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
    setNotice(null);
  }

  return (
    <main className="app">
      <header className="reveal">
        <p className="kicker">Matchmaker harness</p>
        <h1>Standard Lab loop</h1>
        <p className="lede">
          Sign in, request the one seeded host, open the Phase 0 Kasm desktop, then
          end the session. This is not the product UI.
        </p>
      </header>

      {notice && !user ? (
        <p className="notice" data-kind={notice.kind} role="status">
          {notice.kind === "error" ? notice.message : null}
        </p>
      ) : null}

      {user && token ? (
        <SessionPanel
          user={user}
          session={session}
          notice={notice}
          busy={busy}
          onRequest={() => void handleRequest()}
          onEnd={() => void handleEnd()}
          onSignOut={handleSignOut}
        />
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
