import type { SessionResponse } from "./types";

export type Streamer = "kasm" | "sunshine";

export type SessionDesktopLink = {
  href: string;
  label: string;
  kind: "kasm" | "sunshine";
};

export type MoonlightClient = {
  host: string;
  app: string;
};

/** RFC 2606 `.invalid` (seed `example.invalid`, stub `provision.invalid`). */
export function hostnameIsPlaceholder(host: string | null | undefined): boolean {
  if (!host) {
    return true;
  }
  const normalized = host.trim().toLowerCase().replace(/\.$/, "");
  if (!normalized) {
    return true;
  }
  return normalized === "invalid" || normalized.endsWith(".invalid");
}

/** Live desktop vs RFC 2606 `.invalid` placeholders (seed + provision stubs). */
export function isLiveConnectionUrl(url: string | null | undefined): boolean {
  if (!url) {
    return false;
  }
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return false;
    }
    return !hostnameIsPlaceholder(parsed.hostname);
  } catch {
    return false;
  }
}

/** @deprecated Prefer isLiveConnectionUrl — kept for existing harness tests. */
export function isLiveKasmUrl(url: string): boolean {
  return isLiveConnectionUrl(url);
}

export function sessionStreamer(session: SessionResponse): Streamer {
  if (session.labTier === "pro" || session.connection.streamer === "sunshine") {
    return "sunshine";
  }
  return "kasm";
}

export function sessionLocator(session: SessionResponse): string | null {
  if (sessionStreamer(session) === "sunshine") {
    return session.connection.sunshineUrl ?? null;
  }
  return session.connection.kasmUrl ?? null;
}

/**
 * Clickable locator for a held session. Standard opens Kasm in a new tab.
 * Pro opens the Sunshine web UI (Moonlight is the desktop client — see
 * `sessionMoonlightClient`). `.invalid` stubs stay hidden.
 */
export function sessionDesktopLink(session: SessionResponse): SessionDesktopLink | null {
  const locator = sessionLocator(session);
  if (!locator || !isLiveConnectionUrl(locator)) {
    return null;
  }
  if (sessionStreamer(session) === "sunshine") {
    return { href: locator, label: "Open Sunshine web UI", kind: "sunshine" };
  }
  return { href: locator, label: "Open desktop", kind: "kasm" };
}

/** Moonlight host/app when Pro JSON has a non-placeholder host. */
export function sessionMoonlightClient(session: SessionResponse): MoonlightClient | null {
  if (sessionStreamer(session) !== "sunshine") {
    return null;
  }
  const moonlight = session.connection.moonlight;
  if (!moonlight?.host || hostnameIsPlaceholder(moonlight.host)) {
    return null;
  }
  return { host: moonlight.host, app: moonlight.app || "Desktop" };
}
