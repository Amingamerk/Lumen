// Mirrors the serde types in src-tauri/src.
export type View = 'home' | 'list';

export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  durationMs: number;
}

export interface TrackSlice {
  total: number;
  tracks: Track[];
}
