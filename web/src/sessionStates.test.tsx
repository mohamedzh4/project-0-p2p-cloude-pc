import { afterEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { ApiError, currentSession, getSession } from "./api";
import { SessionPanel, noticeText } from "./components/SessionPanel";
import {
  interpretPolledSession,
  interpretSessionError,
  pollIntervalMs,
  sessionStatus,
} from "./sessionRequest";
import type { Notice, SessionResponse } from "./types";

function session(overrides: Partial<SessionResponse> = {}): SessionResponse {
  return {
    sessionId: "11111111-1111-1111-1111-111111111111",
    hostId: "22222222-2222-2222-2222-222222222222",
    labTier: "standard",
    status: "active",
    startedAt: "2026-11-15T12:00:00.000Z",
    connection: { streamer: "kasm", kasmUrl: "https://gw.test.example/s/token/" },
    ...overrides,
  };
}

const starting = session({ status: "provisioning", connection: null });

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("session status", () => {
  it("treats a response without status as ready", () => {
    const { status: _omitted, ...legacy } = session();
    expect(sessionStatus(legacy)).toBe("active");
  });

  it("polls quickly while starting and slowly once ready", () => {
    expect(pollIntervalMs(starting)).toBe(3000);
    expect(pollIntervalMs(session())).toBe(10000);
  });
});

describe("interpretPolledSession", () => {
  it("keeps showing a session that is still starting", () => {
    expect(interpretPolledSession(starting, null)).toEqual({ session: starting, notice: null });
  });

  it("shows the desktop once it is ready", () => {
    const ready = session();
    expect(interpretPolledSession(ready, null)).toEqual({ session: ready, notice: null });
  });

  it("explains a host that failed to start the desktop", () => {
    const failed = session({
      status: "failed",
      connection: null,
      failureReason: "The host could not start the desktop.",
    });
    expect(interpretPolledSession(failed, null)).toEqual({
      session: null,
      notice: { kind: "failed", reason: "The host could not start the desktop." },
    });
  });

  it("moves to the replacement session when the host went offline", () => {
    const replacement = session({ sessionId: "33333333-3333-3333-3333-333333333333" });
    expect(interpretPolledSession(session({ status: "interrupted" }), replacement)).toEqual({
      session: replacement,
      notice: { kind: "rematched" },
    });
  });

  it("says the session was lost when no other host was free", () => {
    expect(interpretPolledSession(session({ status: "interrupted" }), null)).toEqual({
      session: null,
      notice: { kind: "interrupted" },
    });
  });

  it("clears a session that was ended elsewhere", () => {
    expect(interpretPolledSession(session({ status: "ended" }), null)).toEqual({
      session: null,
      notice: { kind: "ended" },
    });
  });
});

describe("booking errors", () => {
  it("maps 409 on Standard to the no-host notice", () => {
    expect(interpretSessionError(new ApiError(409, "Host unavailable"), "standard").notice).toEqual({
      kind: "noHost",
    });
  });

  it("maps 502 to a failed start, not a raw server message", () => {
    expect(interpretSessionError(new ApiError(502, "host provision failed")).notice).toEqual({
      kind: "failed",
      reason: null,
    });
  });
});

describe("notice wording", () => {
  const cases: Array<[Notice, RegExp]> = [
    [{ kind: "noHost" }, /No computer is free right now/],
    [{ kind: "failed", reason: "The host took too long to start the desktop." }, /took too long.*not charged/s],
    [{ kind: "failed" }, /could not start your desktop\. You were not charged/],
    [{ kind: "interrupted" }, /went offline and no other one is free.*Persistent are kept/s],
    [{ kind: "rematched" }, /moved you to another one/],
    [{ kind: "ended", creditsDebited: 3 }, /3 credits/],
    [{ kind: "ended" }, /^Session ended\.$/],
  ];
  it.each(cases)("%j", (notice, pattern) => {
    expect(noticeText(notice)).toMatch(pattern);
  });
});

describe("SessionPanel while the desktop starts", () => {
  const base = {
    user: { id: "user-1", email: "user@test.example" },
    notice: null,
    busy: false,
    intent: "Write code",
    onIntent: vi.fn(),
    onRequest: vi.fn(),
    onEnd: vi.fn(),
    onSignOut: vi.fn(),
  };

  it("shows progress and a cancel button, and no desktop link", () => {
    const html = renderToStaticMarkup(<SessionPanel {...base} session={starting} />);
    expect(html).toContain("Starting your computer");
    expect(html).toContain("You are not charged while it starts");
    expect(html).toContain("Cancel");
    expect(html).not.toContain("Open desktop");
    expect(html).not.toContain("Request session");
  });

  it("shows the desktop link once ready", () => {
    const html = renderToStaticMarkup(<SessionPanel {...base} session={session()} />);
    expect(html).toContain("Open desktop");
    expect(html).not.toContain("Starting your computer");
  });

  it("shows the guest name without any mesh key", () => {
    const html = renderToStaticMarkup(
      <SessionPanel
        {...base}
        session={session({
          mesh: {
            loginServer: "https://hs.test.example",
            exitNode: "dcl-platform-exit",
            exitNodeApplied: true,
            guestHostname: "dcl-guest-abc",
          },
        })}
      />,
    );
    expect(html).toContain("dcl-guest-abc");
    expect(html).not.toContain("user mesh");
    expect(html).not.toContain("tskey");
  });
});

describe("session lookups", () => {
  it("getSession reads one session by id", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(200, starting));
    vi.stubGlobal("fetch", fetchMock);

    const result = await getSession("access-token", starting.sessionId);

    expect(result.data.status).toBe("provisioning");
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`http://matchmaker.test/v1/sessions/${starting.sessionId}`);
    expect(init.method).toBe("GET");
  });

  it("currentSession returns null when the user holds nothing", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(jsonResponse(404, { detail: "Session not found" })),
    );
    await expect(currentSession("access-token")).resolves.toBeNull();
  });

  it("currentSession returns the held session", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(200, starting)));
    await expect(currentSession("access-token")).resolves.toEqual(starting);
  });

  it("currentSession still reports real failures", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(500, {})));
    await expect(currentSession("access-token")).rejects.toBeInstanceOf(ApiError);
  });
});
