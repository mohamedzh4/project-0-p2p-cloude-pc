import { describe, expect, it } from "vitest";

import { ApiError } from "./api";
import { interpretSessionError, interpretSessionSuccess } from "./sessionRequest";
import type { SessionResponse } from "./types";

const session: SessionResponse = {
  sessionId: "11111111-1111-1111-1111-111111111111",
  hostId: "22222222-2222-2222-2222-222222222222",
  labTier: "standard",
  startedAt: "2026-09-04T12:00:00.000Z",
  connection: {
    kasmUrl: "https://kasm.test.example/session",
    kasmToken: "test-kasm-token",
  },
};

describe("interpretSessionSuccess", () => {
  it("treats 201 as a new session with no notice", () => {
    expect(interpretSessionSuccess(201, session)).toEqual({
      session,
      notice: null,
    });
  });

  it("treats 200 as the caller's existing session", () => {
    expect(interpretSessionSuccess(200, session)).toEqual({
      session,
      notice: { kind: "existing" },
    });
  });
});

describe("interpretSessionError", () => {
  it("maps occupied/unavailable 409 to the occupied notice", () => {
    expect(interpretSessionError(new ApiError(409, "Host unavailable"))).toEqual({
      session: null,
      notice: { kind: "occupied" },
    });
  });

  it("maps other ApiError statuses to the detail message", () => {
    expect(interpretSessionError(new ApiError(401, "Not authenticated"))).toEqual({
      session: null,
      notice: { kind: "error", message: "Not authenticated" },
    });
  });

  it("maps unknown failures to a generic request error", () => {
    expect(interpretSessionError(new Error("boom"))).toEqual({
      session: null,
      notice: { kind: "error", message: "Could not request a session" },
    });
  });
});
