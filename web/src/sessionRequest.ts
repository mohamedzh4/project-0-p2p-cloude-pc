import { ApiError } from "./api";
import type { LabTier } from "./intent";
import type {
  Notice,
  SessionHistoryResponse,
  SessionResponse,
  SessionStatus,
} from "./types";

export type SessionRequestView = {
  session: SessionResponse | null;
  notice: Notice | null;
};

/** A response without `status` comes from an older server: it is ready. */
export function sessionStatus(session: SessionResponse): SessionStatus {
  return session.status ?? "active";
}

/** How often to ask the Matchmaker about a held session, in milliseconds. */
export function pollIntervalMs(session: SessionResponse): number {
  return sessionStatus(session) === "provisioning" ? 3000 : 10000;
}

/**
 * Decide what to show after polling a held session.
 *
 * `replacement` is the session the user holds now, if any. When a host goes
 * offline the Matchmaker interrupts the session and books another host, so
 * an interrupted session with a replacement means "moved", not "lost".
 */
export function interpretPolledSession(
  polled: SessionResponse,
  replacement: SessionResponse | null,
): SessionRequestView {
  switch (sessionStatus(polled)) {
    case "provisioning":
    case "active":
      return { session: polled, notice: null };
    case "failed":
      return { session: null, notice: { kind: "failed", reason: polled.failureReason } };
    case "interrupted":
      if (replacement && replacement.sessionId !== polled.sessionId) {
        return { session: replacement, notice: { kind: "rematched" } };
      }
      return { session: null, notice: { kind: "interrupted" } };
    case "ended":
      return { session: null, notice: { kind: "ended" } };
  }
}

/** Map POST /v1/sessions success: 201 ready, 202 starting, 200 already held. */
export function interpretSessionSuccess(
  status: number,
  data: SessionResponse,
): SessionRequestView {
  if (status === 200) {
    return { session: data, notice: { kind: "existing" } };
  }
  return { session: data, notice: null };
}

/** Map POST /v1/sessions failures: 409 occupied vs no matching host, vs other errors. */
export function interpretSessionError(
  error: unknown,
  labTier: LabTier = "standard",
): SessionRequestView {
  if (error instanceof ApiError && error.status === 409) {
    if (labTier === "pro") {
      return { session: null, notice: { kind: "unavailable" } };
    }
    return { session: null, notice: { kind: "noHost" } };
  }
  if (error instanceof ApiError && error.status === 502) {
    return { session: null, notice: { kind: "failed", reason: null } };
  }
  return {
    session: null,
    notice: {
      kind: "error",
      message: error instanceof ApiError ? error.message : "Could not request a session",
    },
  };
}

/** POST /v1/sessions body: lab tier only. Matching stays on HostSelector. */
export function sessionBookBody(labTier: LabTier): { labTier: LabTier } {
  return { labTier };
}

/** Restore a held session from GET /v1/sessions without picking a host. */
export function activeSessionLabTier(history: SessionHistoryResponse | null): LabTier | null {
  const active = history?.sessions.find(
    (row) => row.status === "active" || row.status === "provisioning",
  );
  if (!active) {
    return null;
  }
  return active.labTier === "pro" ? "pro" : "standard";
}

export function endedNotice(creditsDebited: number): Notice {
  return { kind: "ended", creditsDebited };
}

export function creditsSummary(history: SessionHistoryResponse | null): string {
  if (!history) {
    return "Could not load credits from Matchmaker yet.";
  }
  return `${history.creditsDebited} credits debited on the internal ledger. Not a card charge.`;
}
