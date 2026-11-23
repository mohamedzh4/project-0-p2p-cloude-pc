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
import { AccountPage } from "./pages/AccountPage";
import { AppShell } from "./pages/AppShell";
import { AuthPage } from "./pages/AuthPage";
import { Dashboard } from "./pages/Dashboard";
import { Landing } from "./pages/Landing";
import { SharePage } from "./pages/SharePage";
import { isAppRoute, navigate, useRoute } from "./router";
import { useTheme } from "./theme";
import type { HostKitOffer, HostKitOs } from "./api";
import { mapIntentToLabTier, canRequestSession } from "./intent";
import {
  endedNotice,
  interpretPolledSession,
  interpretSessionError,
  interpretSessionSuccess,
  pollIntervalMs,
} from "./sessionRequest";
import type { AuthMode, Notice, SessionHistoryResponse, SessionResponse, UserPublic } from "./types";

export default function App() {
  const configured = matchmakerUrl();
  const route = useRoute();
  const [theme, setTheme] = useTheme();
  const [mode, setMode] = useState<AuthMode>(route === "login" ? "login" : "signup");
  const [pendingKit, setPendingKit] = useState<HostKitOs | null>(null);
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

  // Keep the auth tab in step with the address, and keep signed-out
  // visitors out of the dashboard pages.
  useEffect(() => {
    if (route === "login" || route === "signup") {
      setMode(route);
      if (token) navigate("sessions");
    } else if (isAppRoute(route) && !token) {
      navigate("login");
    }
  }, [route, token]);

  // A download picked on the landing page before signing in starts once the
  // kit list for the new account has loaded.
  useEffect(() => {
    if (!token || !pendingKit || kits === null) return;
    const os = pendingKit;
    setPendingKit(null);
    if (kits.some((kit) => kit.os === os && kit.available)) {
      void handleKitDownload(os);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, pendingKit, kits]);

  function handleLandingDownload(os: HostKitOs) {
    setPendingKit(os);
    navigate(token ? "share" : "signup");
  }

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
      navigate(pendingKit ? "share" : "sessions");
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
    setPendingKit(null);
    navigate("home");
  }

  if (user && token && isAppRoute(route)) {
    return (
      <AppShell route={route} user={user} onSignOut={handleSignOut}>
        {route === "sessions" ? (
          <Dashboard
            session={session}
            history={history}
            notice={notice}
            busy={busy}
            intent={intent}
            onIntent={setIntent}
            onRequest={() => void handleRequest()}
            onEnd={() => void handleEnd()}
          />
        ) : route === "share" ? (
          <SharePage
            kits={kits}
            busy={kitBusy}
            message={kitMessage}
            onDownload={(os) => void handleKitDownload(os)}
          />
        ) : (
          <AccountPage
            user={user}
            history={history}
            theme={theme}
            onTheme={setTheme}
            onSignOut={handleSignOut}
          />
        )}
      </AppShell>
    );
  }

  if (route === "login" || route === "signup") {
    return (
      <AuthPage
        mode={mode}
        busy={busy}
        email={email}
        password={password}
        error={notice?.kind === "error" && !user ? notice.message : null}
        reason={
          pendingKit
            ? `Sign in and your Host Agent download for ${pendingKit === "windows" ? "Windows" : "Linux"} starts right after.`
            : null
        }
        onMode={setMode}
        onEmail={setEmail}
        onPassword={setPassword}
        onSubmit={() => void handleAuth()}
      />
    );
  }

  return <Landing signedIn={Boolean(user && token)} onDownload={handleLandingDownload} />;
}
