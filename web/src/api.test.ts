import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, listSessions, matchmakerUrl, requestSession } from "./api";

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const sessionBody = {
  sessionId: "11111111-1111-1111-1111-111111111111",
  hostId: "22222222-2222-2222-2222-222222222222",
  labTier: "standard",
  startedAt: "2026-09-04T12:00:00.000Z",
  connection: {
    kasmUrl: "https://kasm.test.example/session",
    kasmToken: "test-kasm-token",
  },
};

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("matchmakerUrl", () => {
  it("strips a trailing slash from the configured base", () => {
    expect(matchmakerUrl()).toBe("http://matchmaker.test");
  });
});

describe("requestSession error mapping", () => {
  it("returns 201 and the session body for a new booking", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(201, sessionBody));
    vi.stubGlobal("fetch", fetchMock);

    await expect(requestSession("access-token", "standard")).resolves.toEqual({
      status: 201,
      data: sessionBody,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://matchmaker.test/v1/sessions");
    expect(init.method).toBe("POST");
    expect(init.body).toBe(JSON.stringify({ labTier: "standard" }));
    const headers = new Headers(init.headers);
    expect(headers.get("Authorization")).toBe("Bearer access-token");
  });

  it("returns 200 when the caller already holds a session", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(200, sessionBody)));

    await expect(requestSession("access-token", "standard")).resolves.toEqual({
      status: 200,
      data: sessionBody,
    });
  });

  it("posts the mapped labTier, including pro", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(409, { detail: "Host unavailable" }));
    vi.stubGlobal("fetch", fetchMock);

    const error = await requestSession("access-token", "pro").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 409, message: "Host unavailable" });
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
      body: JSON.stringify({ labTier: "pro" }),
    });
  });

  it("throws ApiError 409 with the FastAPI detail string", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(jsonResponse(409, { detail: "Host unavailable" })),
    );

    const error = await requestSession("access-token", "standard").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 409, message: "Host unavailable" });
  });

  it("maps FastAPI validation arrays to the first msg", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        jsonResponse(422, {
          detail: [{ loc: ["body", "labTier"], msg: "Field required", type: "missing" }],
        }),
      ),
    );

    const error = await requestSession("access-token", "standard").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 422, message: "Field required" });
  });

  it("falls back to status text when the body has no detail", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(500, { error: "nope" })));

    const error = await requestSession("access-token", "standard").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 500, message: "Request failed (500)" });
  });

  it("maps a network failure to an unreachable Matchmaker error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));

    const error = await requestSession("access-token", "standard").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      status: 0,
      message: "Matchmaker is unreachable. Is it running?",
    });
  });
});

describe("listSessions", () => {
  it("gets camelCase history and lifetime creditsDebited", async () => {
    const historyBody = {
      sessions: [
        {
          sessionId: "11111111-1111-1111-1111-111111111111",
          hostId: "22222222-2222-2222-2222-222222222222",
          labTier: "standard",
          status: "ended",
          startedAt: "2026-09-04T12:00:00.000Z",
          endedAt: "2026-09-04T12:02:00.000Z",
          durationSeconds: 120,
          creditsDebited: 2,
        },
      ],
      creditsDebited: 2,
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(200, historyBody));
    vi.stubGlobal("fetch", fetchMock);

    await expect(listSessions("access-token")).resolves.toEqual({
      status: 200,
      data: historyBody,
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://matchmaker.test/v1/sessions");
    expect(init.method).toBe("GET");
    const headers = new Headers(init.headers);
    expect(headers.get("Authorization")).toBe("Bearer access-token");
  });
});
