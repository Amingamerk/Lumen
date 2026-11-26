// Mirrors of the Rust types in src-tauri/src.

export type SessionStatus = {
  method: 'LoginWindow' | 'Import';
  captured_at: number;
  cookie_count: number;
  has_login_info: boolean;
  expired: boolean;
  user_agent: string;
};

export type Playlist = { id: string; title: string; track_count: number | null };

export type PlaylistPage = { playlists: Playlist[]; has_more: boolean };

export type ResolveReport = {
  format_id: string;
  codec: string | null;
  abr_kbps: number | null;
  protocol: string | null;
  client: string | null;
  available_at: number | null;
  waited_s: number;
  expires_at: number | null;
  resolve_ms: number;
  user_agent_matches_login: boolean;
  range_status: number;
  warnings: string[];
};

export type Step<T> = { Ok: T } | { Err: string };

export type CheckReport = {
  blocked: string | null;
  browse: Step<PlaylistPage> | null;
  resolve: Step<ResolveReport> | null;
};

export type Clients = 'Default' | 'PinnedUserAgent';
