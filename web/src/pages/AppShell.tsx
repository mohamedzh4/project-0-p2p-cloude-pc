import { PixelIcon } from "../components/PixelIcon";
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
        <nav className="navbox" aria-label="Dashboard">
          <span className="navbox-links navbox-links-keep">
            {TABS.map((tab) => (
              <a key={tab.route} href={routeHref(tab.route)} aria-current={route === tab.route ? "page" : undefined}>
                {tab.label}
              </a>
            ))}
          </span>
          <a className="avatar" href={routeHref("account")} title={user.email} aria-label="Account">
            {initial}
          </a>
          <button type="button" className="navbox-icon" onClick={onSignOut} aria-label="Sign out" title="Sign out">
            <PixelIcon name="logout" size={18} />
          </button>
        </nav>
      </header>
      <main className="shell-main">{children}</main>
    </div>
  );
}
