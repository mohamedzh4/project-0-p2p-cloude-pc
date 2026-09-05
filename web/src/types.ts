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

export interface StorageMountInfo {
  provider: string;
  endpoint: string;
  region: string;
  bucket: string;
  prefix: string;
  pathStyle: boolean;
  accessKeyId: string;
  secretAccessKey: string;
  mountInsideGuest: boolean;
  rclone: string;
}

export interface SessionResponse {
  sessionId: string;
  hostId: string;
  labTier: string;
  startedAt: string;
  connection: ConnectionInfo;
  storage?: StorageMountInfo;
}

export interface EndSessionResponse {
  sessionId: string;
  status: string;
  endedAt: string;
  creditsDebited: number;
}

export interface SessionHistoryItem {
  sessionId: string;
  hostId: string;
  labTier: string;
  status: string;
  startedAt: string;
  endedAt: string | null;
  durationSeconds: number | null;
  creditsDebited: number;
}

export interface SessionHistoryResponse {
  sessions: SessionHistoryItem[];
  creditsDebited: number;
}

export type AuthMode = "signup" | "login";

export type Notice =
  | { kind: "occupied" }
  | { kind: "unavailable" }
  | { kind: "ended" }
  | { kind: "existing" }
  | { kind: "error"; message: string };
