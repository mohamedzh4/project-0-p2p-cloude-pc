import type { FormEvent } from "react";

import type { AuthMode } from "../types";

interface AuthFormProps {
  mode: AuthMode;
  busy: boolean;
  email: string;
  password: string;
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
  onMode,
  onEmail,
  onPassword,
  onSubmit,
}: AuthFormProps) {
  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    onSubmit();
  }

  return (
    <form className="panel" onSubmit={handleSubmit}>
      <div className="modes" role="tablist" aria-label="Account">
        <button
          type="button"
          className={mode === "signup" ? undefined : "ghost"}
          aria-pressed={mode === "signup"}
          disabled={busy}
          onClick={() => onMode("signup")}
        >
          Sign up
        </button>
        <button
          type="button"
          className={mode === "login" ? undefined : "ghost"}
          aria-pressed={mode === "login"}
          disabled={busy}
          onClick={() => onMode("login")}
        >
          Log in
        </button>
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
          autoComplete={mode === "signup" ? "new-password" : "current-password"}
          required
          minLength={mode === "signup" ? 8 : 1}
          disabled={busy}
          value={password}
          onChange={(event) => onPassword(event.target.value)}
        />
      </div>
      <button type="submit" disabled={busy}>
        {busy ? "Working…" : mode === "signup" ? "Create account" : "Log in"}
      </button>
    </form>
  );
}
