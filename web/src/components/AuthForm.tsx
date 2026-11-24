import type { FormEvent } from "react";

import { routeHref } from "../router";
import type { AuthMode } from "../types";

interface AuthFormProps {
  mode: AuthMode;
  busy: boolean;
  email: string;
  password: string;
  error?: string | null;
  /** Shown above the form, e.g. why the visitor was sent here. */
  reason?: string | null;
  onMode: (mode: AuthMode) => void;
  onEmail: (value: string) => void;
  onPassword: (value: string) => void;
  onSubmit: () => void;
}

export function AuthForm({
  mode,
  busy,
  email,
  password,
  error,
  reason,
  onMode,
  onEmail,
  onPassword,
  onSubmit,
}: AuthFormProps) {
  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    onSubmit();
  }

  const signup = mode === "signup";

  return (
    <form className="auth-form" onSubmit={handleSubmit}>
      <h1>{signup ? "Create your account" : "Welcome back"}</h1>
      <p className="auth-sub">
        {signup
          ? "One account to use computers and to lend yours."
          : "Log in to start a session or manage your PC."}
      </p>
      {reason ? <p className="auth-reason">{reason}</p> : null}

      <div className="segmented" role="group" aria-label="Account">
        <a
          href={routeHref("signup")}
          aria-current={signup ? "page" : undefined}
          onClick={() => onMode("signup")}
        >
          Sign up
        </a>
        <a
          href={routeHref("login")}
          aria-current={signup ? undefined : "page"}
          onClick={() => onMode("login")}
        >
          Log in
        </a>
      </div>

      <div className="field">
        <label htmlFor="email">Email</label>
        <input
          id="email"
          name="email"
          type="email"
          autoComplete="username"
          required
          disabled={busy}
          value={email}
          onChange={(event) => onEmail(event.target.value)}
        />
      </div>
      <div className="field">
        <label htmlFor="password">Password</label>
        <input
          id="password"
          name="password"
          type="password"
          autoComplete={signup ? "new-password" : "current-password"}
          required
          minLength={signup ? 8 : 1}
          disabled={busy}
          value={password}
          onChange={(event) => onPassword(event.target.value)}
        />
        {signup ? <p className="field-hint">At least 8 characters.</p> : null}
      </div>

      {error ? (
        <p className="notice" data-kind="error" role="alert">
          {error}
        </p>
      ) : null}

      <button type="submit" className="btn btn-rent btn-lg btn-block" disabled={busy}>
        {busy ? "Working…" : signup ? "Create account" : "Log in"}
      </button>
    </form>
  );
}
