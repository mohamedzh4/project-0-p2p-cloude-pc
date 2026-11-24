export interface UserPublic {
  id: string;
  email: string;
}

export interface TokenResponse {
  accessToken: string;
  tokenType: string;
  user: UserPublic;
}

export interface MoonlightInfo {
  host: string;
  app: string;
}

export interface ConnectionInfo {
  streamer?: string;
  kasmUrl?: string | null;
  kasmToken?: string | null;
  sunshineUrl?: string | null;
  moonlight?: MoonlightInfo | null;
  gpuExclusive?: boolean;
}

export interface StorageMountInfo {
  provider: string;
  endpoint: string;
  region: string;
  bucket: string;
  prefix: string;
  pathStyle: boolean;
  mountInsideGuest: boolean;
  rclone: string;
  rcloneArgs?: string[];
  envAuth?: boolean;
  objects: string[];
  savedAsOf?: string | null;
}

/** The guest's mesh identity, for display. Renters are not on the mesh. */
export interface SessionMeshInfo {
  loginServer: string;
  exitNode: string | null;
  exitNodeApplied: boolean;
  guestHostname: string;
}

/**
 * provisioning: the host is starting the desktop; poll until it changes.
 * active: `connection` is set. failed: `failureReason` says why (not billed).
 * interrupted: the host went away. ended: the user ended it.
 */
export type SessionStatus = "provisioning" | "active" | "ended" | "interrupted" | "failed";

export interface SessionResponse {
  sessionId: string;
  hostId: string;
  labTier: string;
  /** Missing on old servers; treated as `active`. */
  status?: SessionStatus;
  failureReason?: string | null;
  startedAt: string;
  endedAt?: string | null;
  /** Null until the desktop is ready. */
  connection: ConnectionInfo | null;
  storage?: StorageMountInfo;
  mesh?: SessionMeshInfo;
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
  failureReason?: string | null;
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
  /** No Standard host is free right now. */
  | { kind: "noHost" }
  /** No Pro host (exclusive GPU) is free right now. */
  | { kind: "unavailable" }
  /** The host could not start the desktop. Not billed. */
  | { kind: "failed"; reason?: string | null }
  /** The host went offline and no other host was free. */
  | { kind: "interrupted" }
  /** The host went offline and the session moved to another host. */
  | { kind: "rematched" }
  | { kind: "ended"; creditsDebited?: number }
  | { kind: "existing" }
  | { kind: "error"; message: string };
