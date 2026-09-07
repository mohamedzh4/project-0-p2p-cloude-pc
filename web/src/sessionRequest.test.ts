import { describe, expect, it } from "vitest";

import { ApiError } from "./api";
import {
  activeSessionLabTier,
  creditsSummary,
  endedNotice,
  interpretSessionError,
  interpretSessionSuccess,
  sessionBookBody,
} from "./sessionRequest";
import type { SessionHistoryResponse, SessionResponse } from "./types";

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
  it("maps occupied/unavailable 409 to the occupied notice for Standard", () => {
    expect(interpretSessionError(new ApiError(409, "Host unavailable"))).toEqual({
      session: null,
      notice: { kind: "occupied" },
    });
    expect(interpretSessionError(new ApiError(409, "Host unavailable"), "standard")).toEqual({
      session: null,
      notice: { kind: "occupied" },
    });
  });

  it("maps Pro 409 to unavailable (no GPU host, or that GPU already assigned)", () => {
    expect(interpretSessionError(new ApiError(409, "Host unavailable"), "pro")).toEqual({
      session: null,
      notice: { kind: "unavailable" },
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

describe("sessionBookBody", () => {
  it("posts only labTier — matching stays automatic (no hostId)", () => {
    expect(sessionBookBody("standard")).toEqual({ labTier: "standard" });
    expect(sessionBookBody("pro")).toEqual({ labTier: "pro" });
    expect(sessionBookBody("standard")).not.toHaveProperty("hostId");
    expect(Object.keys(sessionBookBody("pro"))).toEqual(["labTier"]);
  });
});

describe("activeSessionLabTier", () => {
  const history = (status: string, labTier = "standard"): SessionHistoryResponse => ({
    creditsDebited: 3,
    sessions: [
      {
        sessionId: "11111111-1111-1111-1111-111111111111",
        hostId: "22222222-2222-2222-2222-222222222222",
        labTier,
        status,
        startedAt: "2026-09-07T12:00:00.000Z",
        endedAt: status === "active" ? null : "2026-09-07T12:02:00.000Z",
        durationSeconds: status === "active" ? null : 120,
        creditsDebited: status === "active" ? 0 : 2,
      },
    ],
  });

  it("returns the lab tier of an active Matchmaker session", () => {
    expect(activeSessionLabTier(history("active", "pro"))).toBe("pro");
    expect(activeSessionLabTier(history("active", "standard"))).toBe("standard");
  });

  it("does not invent a session when history is ended-only or missing", () => {
    expect(activeSessionLabTier(history("ended"))).toBeNull();
    expect(activeSessionLabTier(history("interrupted"))).toBeNull();
    expect(activeSessionLabTier(null)).toBeNull();
    expect(activeSessionLabTier({ sessions: [], creditsDebited: 0 })).toBeNull();
  });
});

describe("endedNotice / creditsSummary", () => {
  it("surfaces ledger credits from the Matchmaker end response", () => {
    expect(endedNotice(4)).toEqual({ kind: "ended", creditsDebited: 4 });
  });

  it("summarizes lifetime credits from GET /v1/sessions, not a mock", () => {
    expect(creditsSummary(null)).toBe("Could not load credits from Matchmaker yet.");
    expect(creditsSummary({ sessions: [], creditsDebited: 9 })).toBe(
      "9 credits debited on the internal ledger. Not a card charge.",
    );
  });
});
