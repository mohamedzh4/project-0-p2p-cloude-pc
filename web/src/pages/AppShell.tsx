import { LogOut } from "lucide-react";
import type { ReactNode } from "react";

import { Brand } from "../components/Brand";
import { routeHref, type Route } from "../router";
import type { UserPublic } from "../types";

interface AppShellProps {
  route: Route;
  user: UserPublic;
  onSignOut: () => void;
  children: ReactNode;
}

const TABS: { route: Route; label: string }[] = [
  { route: "sessions", label: "Sessions" },
  { route: "share", label: "Share my PC" },
  { route: "account", label: "Account" },
];

export function AppShell({ route, user, onSignOut, children }: AppShellProps) {
  const initial = user.email.slice(0, 1).toUpperCase();
  return (
    <div className="shell">
      <header className="topbar">
        <Brand />
        <nav className="pillnav" aria-label="Dashboard">
          {TABS.map((tab) => (
            <a key={tab.route} href={routeHref(tab.route)} aria-current={route === tab.route ? "page" : undefined}>
              {tab.label}
            </a>
          ))}
        </nav>
        <div className="topbar-actions">
          <a className="avatar" href={routeHref("account")} title={user.email} aria-label="Account">
            {initial}
          </a>
          <button type="button" className="btn btn-quiet btn-icon" onClick={onSignOut} aria-label="Sign out" title="Sign out">
            <LogOut size={17} aria-hidden="true" />
          </button>
        </div>
      </header>
      <main className="shell-main">{children}</main>
    </div>
  );
}
