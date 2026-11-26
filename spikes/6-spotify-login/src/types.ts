// Mirrors the structs that the Rust commands return. None of them hold a secret.

export interface SessionStatus {
  saved: boolean;
  expired: boolean;
}

export interface LoginResult {
  saved: boolean;
  message: string;
}

export type Route = 'pathfinder' | 'rest';

export interface LikedTrack {
  id: string;
  title: string;
  artists: string[];
  duration_ms: number;
  isrc: string | null;
  isrc_source: 'liked_response' | 'track_metadata' | null;
}

export interface Report {
  route: Route;
  items_returned: number;
  track_count: number;
  skipped: number;
  total_liked: number | null;
  isrc_in_liked_response: number;
  isrc_from_metadata: number;
  tracks_with_isrc: number;
  track_fields: string[];
  tracks: LikedTrack[];
  lookup_error: string | null;
  session_expired: boolean;
  totp_status: string | null;
}
