import type { LabTier } from "./intent";
import type { EndSessionResponse, SessionHistoryResponse, SessionResponse, TokenResponse } from "./types";

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export function matchmakerUrl(): string | null {
  const raw = import.meta.env.VITE_MATCHMAKER_URL;
  if (typeof raw !== "string" || raw.trim() === "") {
    return null;
  }
  return raw.replace(/\/$/, "");
}

function detailMessage(body: unknown, fallback: string): string {
  if (body && typeof body === "object" && "detail" in body) {
    const detail = (body as { detail: unknown }).detail;
    if (typeof detail === "string") {
      return detail;
    }
    if (Array.isArray(detail) && detail.length > 0) {
      const first = detail[0] as { msg?: string };
      if (typeof first?.msg === "string") {
        return first.msg;
      }
    }
  }
  return fallback;
}

async function parseBody(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) {
    return null;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

async function request<T>(
  path: string,
  init: RequestInit & { token?: string },
): Promise<{ status: number; data: T }> {
  const base = matchmakerUrl();
  if (!base) {
    throw new ApiError(0, "VITE_MATCHMAKER_URL is required");
  }

  const headers = new Headers(init.headers);
  headers.set("Accept", "application/json");
  if (init.body) {
    headers.set("Content-Type", "application/json");
  }
  if (init.token) {
    headers.set("Authorization", `Bearer ${init.token}`);
  }

  let response: Response;
  try {
    response = await fetch(`${base}${path}`, { ...init, headers });
  } catch {
    throw new ApiError(0, "Matchmaker is unreachable. Is it running?");
  }

  const body = await parseBody(response);
  if (!response.ok) {
    throw new ApiError(
      response.status,
      detailMessage(body, `Request failed (${response.status})`),
    );
  }
  return { status: response.status, data: body as T };
}

export function signup(email: string, password: string): Promise<{ status: number; data: TokenResponse }> {
  return request<TokenResponse>("/v1/auth/signup", {
    method: "POST",
    body: JSON.stringify({ email, password }),
  });
}

export function login(email: string, password: string): Promise<{ status: number; data: TokenResponse }> {
  return request<TokenResponse>("/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email, password }),
  });
}

export function requestSession(
  token: string,
  labTier: LabTier,
): Promise<{ status: number; data: SessionResponse }> {
  return request<SessionResponse>("/v1/sessions", {
    method: "POST",
    token,
    body: JSON.stringify({ labTier }),
  });
}

export function endSession(
  token: string,
  sessionId: string,
): Promise<{ status: number; data: EndSessionResponse }> {
  return request<EndSessionResponse>(`/v1/sessions/${sessionId}/end`, {
    method: "POST",
    token,
  });
}

export function listSessions(
  token: string,
): Promise<{ status: number; data: SessionHistoryResponse }> {
  return request<SessionHistoryResponse>("/v1/sessions", {
    method: "GET",
    token,
  });
}

/** One session in its current state. Poll this while it is provisioning. */
export function getSession(
  token: string,
  sessionId: string,
): Promise<{ status: number; data: SessionResponse }> {
  return request<SessionResponse>(`/v1/sessions/${sessionId}`, {
    method: "GET",
    token,
  });
}

/** The session this user holds now (starting or ready), or null. */
export async function currentSession(token: string): Promise<SessionResponse | null> {
  try {
    const result = await request<SessionResponse>("/v1/sessions/current", {
      method: "GET",
      token,
    });
    return result.data;
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export type HostKitOs = "windows" | "linux";

export interface HostKitOffer {
  os: HostKitOs;
  available: boolean;
}

/** Which setup kits the server can make right now. */
export function listHostKits(
  token: string,
): Promise<{ status: number; data: { kits: HostKitOffer[] } }> {
  return request<{ kits: HostKitOffer[] }>("/v1/host-kits", { method: "GET", token });
}

/**
 * Download a setup kit made for this user: the setup script plus a
 * single-use registration code. Returns the zip and its file name.
 */
export async function downloadHostKit(
  token: string,
  os: HostKitOs,
): Promise<{ blob: Blob; filename: string }> {
  const base = matchmakerUrl();
  if (!base) {
    throw new ApiError(0, "VITE_MATCHMAKER_URL is required");
  }
  let response: Response;
  try {
    response = await fetch(`${base}/v1/host-kits`, {
      method: "POST",
      headers: {
        Accept: "application/zip",
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ os }),
    });
  } catch {
    throw new ApiError(0, "Matchmaker is unreachable. Is it running?");
  }
  if (!response.ok) {
    const body = await parseBody(response);
    throw new ApiError(response.status, detailMessage(body, `Download failed (${response.status})`));
  }
  return { blob: await response.blob(), filename: kitFilename(response, os) };
}

export function kitFilename(response: Response, os: HostKitOs): string {
  const header = response.headers.get("Content-Disposition") ?? "";
  const match = /filename="([^"]+)"/.exec(header);
  return match?.[1] ?? `dcl-host-setup-${os}.zip`;
}
