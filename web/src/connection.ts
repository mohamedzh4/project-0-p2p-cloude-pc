import type { SessionResponse } from "./types";

export type Streamer = "kasm" | "sunshine";

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
    const labels = parsed.hostname.split(".");
    const tld = labels[labels.length - 1];
    return tld !== "invalid";
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
