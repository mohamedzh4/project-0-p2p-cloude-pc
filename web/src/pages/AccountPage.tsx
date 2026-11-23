import { Moon, Monitor, Sun } from "lucide-react";

import type { ThemeChoice } from "../theme";
import type { SessionHistoryResponse, UserPublic } from "../types";

interface AccountPageProps {
  user: UserPublic;
  history: SessionHistoryResponse | null;
  theme: ThemeChoice;
  onTheme: (choice: ThemeChoice) => void;
  onSignOut: () => void;
}

const THEMES: { value: ThemeChoice; label: string; icon: typeof Moon }[] = [
  { value: "night", label: "Night", icon: Moon },
  { value: "day", label: "Day", icon: Sun },
  { value: "system", label: "Match my device", icon: Monitor },
];

export function AccountPage({ user, history, theme, onTheme, onSignOut }: AccountPageProps) {
  return (
    <>
      <div className="page-head">
        <h1>Account</h1>
        <p>Your sign-in details and how this site looks to you.</p>
      </div>

      <div className="settings">
        <section className="card setting" aria-labelledby="profile-title">
          <div className="setting-label">
            <h2 id="profile-title">Profile</h2>
            <p>The account you use for sessions and for lending your PC.</p>
          </div>
          <dl className="setting-body kv">
            <dt>Email</dt>
            <dd>{user.email}</dd>
            <dt>Account ID</dt>
            <dd className="mono-soft">{user.id}</dd>
            <dt>Credits used</dt>
            <dd>{history ? history.creditsDebited : "—"}</dd>
          </dl>
        </section>

        <section className="card setting" aria-labelledby="look-title">
          <div className="setting-label">
            <h2 id="look-title">Appearance</h2>
            <p>Saved in this browser only.</p>
          </div>
          <div className="setting-body theme-picker" role="radiogroup" aria-label="Theme">
            {THEMES.map(({ value, label, icon: Icon }) => (
              <label key={value} className="theme-option" data-theme-option={value}>
                <input
                  type="radio"
                  name="theme"
                  value={value}
                  checked={theme === value}
                  onChange={() => onTheme(value)}
                />
                <span className="theme-swatch" aria-hidden="true">
                  <Icon size={18} />
                </span>
                {label}
              </label>
            ))}
          </div>
        </section>

        <section className="card setting" aria-labelledby="security-title">
          <div className="setting-label">
            <h2 id="security-title">Sign-in</h2>
            <p>For safety, your sign-in is kept in memory and ends when you close the tab.</p>
          </div>
          <div className="setting-body">
            <p className="hint">Changing your password or email isn't available yet.</p>
            <button type="button" className="btn btn-outline" onClick={onSignOut}>
              Sign out
            </button>
          </div>
        </section>
      </div>
    </>
  );
}
