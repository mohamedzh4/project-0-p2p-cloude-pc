export interface UserPublic {
  id: string;
  email: string;
}

export interface TokenResponse {
  accessToken: string;
  tokenType: string;
  user: UserPublic;
}

export interface ConnectionInfo {
  kasmUrl: string;
  kasmToken: string | null;
}

export interface SessionResponse {
  sessionId: string;
  hostId: string;
  labTier: string;
  startedAt: string;
  connection: ConnectionInfo;
}

export interface EndSessionResponse {
  sessionId: string;
  status: string;
  endedAt: string;
}

export type AuthMode = "signup" | "login";

export type Notice =
  | { kind: "occupied" }
  | { kind: "ended" }
  | { kind: "existing" }
  | { kind: "error"; message: string };
