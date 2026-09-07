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
}

export interface MeshPeerInfo {
  hostname: string;
  aclTags: string[];
  preauthKey: string;
  ephemeral: boolean;
  reusable: boolean;
  expiresAt: string;
  tailscaleUp: string[];
}

export interface SessionMeshInfo {
  loginServer: string;
  exitNode: string | null;
  exitNodeApplied: boolean;
  guest: MeshPeerInfo;
  user: MeshPeerInfo;
}

export interface SessionResponse {
  sessionId: string;
  hostId: string;
  labTier: string;
  startedAt: string;
  connection: ConnectionInfo;
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
  | { kind: "ended"; creditsDebited?: number }
  | { kind: "existing" }
  | { kind: "error"; message: string };
