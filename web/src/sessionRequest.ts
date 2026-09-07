import { ApiError } from "./api";
import type { LabTier } from "./intent";
import type { Notice, SessionHistoryResponse, SessionResponse } from "./types";

export type SessionRequestView = {
  session: SessionResponse | null;
  notice: Notice | null;
};

/** Map POST /v1/sessions success: 201 new, 200 already held by this user. */
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
    return { session: null, notice: { kind: "occupied" } };
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
  const active = history?.sessions.find((row) => row.status === "active");
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
