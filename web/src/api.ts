import type { EndSessionResponse, SessionResponse, TokenResponse } from "./types";

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
): Promise<{ status: number; data: SessionResponse }> {
  return request<SessionResponse>("/v1/sessions", {
    method: "POST",
    token,
    body: JSON.stringify({ labTier: "standard" }),
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
