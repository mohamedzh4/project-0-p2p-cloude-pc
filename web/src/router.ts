import { useEffect, useState } from "react";

/**
 * Hash routes keep the static nginx config unchanged and keep the sign-in
 * token in memory: moving between pages never reloads the app.
 */
export type Route = "home" | "login" | "signup" | "sessions" | "share" | "account";

const PATHS: Record<Route, string> = {
  home: "#/",
  login: "#/login",
  signup: "#/signup",
  sessions: "#/app",
  share: "#/app/share",
  account: "#/app/account",
};

export function routeHref(route: Route): string {
  return PATHS[route];
}

export function parseRoute(hash: string): Route {
  const found = (Object.keys(PATHS) as Route[]).find((route) => PATHS[route] === hash);
  return found ?? "home";
}

export function navigate(route: Route) {
  if (window.location.hash !== PATHS[route]) {
    window.location.hash = PATHS[route];
  }
}

export function isAppRoute(route: Route): boolean {
  return route === "sessions" || route === "share" || route === "account";
}

export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parseRoute(window.location.hash));
  useEffect(() => {
    const onChange = () => {
      setRoute(parseRoute(window.location.hash));
      window.scrollTo({ top: 0 });
    };
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
