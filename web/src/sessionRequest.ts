import { ApiError } from "./api";
import type { Notice, SessionResponse } from "./types";

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

/** Map POST /v1/sessions failures: 409 occupied/unavailable vs other API errors. */
export function interpretSessionError(error: unknown): SessionRequestView {
  if (error instanceof ApiError && error.status === 409) {
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
