// Mirrors the serialized Rust types. None of them carries a token or an id.
export interface SessionStatus {
  logged_in: boolean;
  expired: boolean;
  /** "absent" or "present (len=N)". */
  oauth_token: string;
  client_id: string;
  user_agent: string | null;
}

export interface ScrapeReport {
  scripts_found: number;
  scripts_fetched: number;
  matched_index: number;
  pattern: 'ObjectKey' | 'QueryString';
  elapsed_ms: number;
}

export interface Likes {
  display_name: string | null;
  tracks: { title: string; artist: string | null }[];
  has_more: boolean;
  total_likes: number | null;
  endpoint: string;
  client_id_source: 'Cached' | 'Scraped' | 'Rescraped';
  scrape: ScrapeReport | null;
}
