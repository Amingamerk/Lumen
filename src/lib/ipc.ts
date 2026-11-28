// The only door to Rust. Types mirror src-tauri/src/state.rs and src-tauri/src/catalog/.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type SourceKind = 'youtube' | 'soundcloud' | 'spotify';

export interface SourceTag {
  source: SourceKind;
  badge: 'YT' | 'SC' | 'SP';
}

/** A local playlist as the sidebar and its page show it. */
export interface PlaylistSummary {
  id: string;
  name: string;
  trackCount: number;
  /** The length of all its entries together. */
  durationMs: number;
  /** The song whose picture covers the playlist, if one of its songs has a picture. */
  coverTrack: string | null;
}

/** A playlist or album as a card or a page header shows it. */
export interface CollectionSummary extends SourceTag {
  id: string;
  kind: 'playlist' | 'album';
  title: string;
  /** A playlist's description, or an album's artists. */
  subtitle: string;
}

export interface TrackRow {
  id: string;
  title: string;
  artists: string[];
  album: string | null;
  durationMs: number;
  /** Whether `lumen://art/` has a picture for this row. See `artUrl`. */
  hasArt: boolean;
  sources: SourceTag[];
}

/** Some consecutive rows of a Collection. The rest stay in Rust. */
export interface TrackSlice {
  total: number;
  /** The position of the first row asked for, counted from 0. */
  offset: number;
  tracks: TrackRow[];
}

/** The shelves on Home, one row of cards for each shelf. A shelf with nothing is empty. */
export interface HomeView {
  /** Songs played, the latest first, each once. */
  recent: TrackRow[];
  /** Liked songs, the newest like first. */
  liked: TrackRow[];
  playlists: PlaylistSummary[];
}

export type SourceFilter = 'all' | 'youtube' | 'soundcloud';

/** The type chip: which kind of result a live search asks for. */
export type SearchKind = 'all' | 'songs' | 'albums' | 'artists' | 'playlists';

export type HitKind = 'song' | 'album' | 'artist' | 'playlist';

/** An album, artist or playlist a search found. Its picture is at `artUrl(id, size)`. */
export interface HitRow {
  id: string;
  kind: HitKind;
  title: string;
  /** The artists of an album or song, or the author of a playlist. */
  by: string[];
  /** What else the service said, such as an album's year or a playlist's size. */
  detail: string | null;
  hasArt: boolean;
}

/** The best match with the songs that play from its card. */
export interface TopView {
  best: HitRow;
  songs: TrackRow[];
}

export type SearchStatus =
  | 'idle'
  | 'loading'
  | 'results'
  | 'noResults'
  | 'offline'
  | 'unexpected'
  | 'blocked';

/** A search and what Rust found for it. */
export interface SearchView {
  /** Grows with every change. */
  revision: number;
  /** The query last submitted. */
  query: string;
  filter: SourceFilter;
  /** The type chip. */
  kind: SearchKind;
  status: SearchStatus;
  top: TopView | null;
  tracks: TrackRow[];
  albums: HitRow[];
  artists: HitRow[];
  playlists: HitRow[];
  collections: CollectionSummary[];
}

export type Route =
  | { view: 'home' }
  | { view: 'search' }
  | { view: 'library' }
  | { view: 'playlist'; id: string };

export interface NavigationState {
  route: Route;
  queueOpen: boolean;
  revision: number;
}

export interface AppSnapshot {
  navigation: NavigationState;
}

// Playback. Mirrors src-tauri/src/audio/state.rs.

export type PlaybackStatus =
  | 'idle'
  | 'loading'
  | 'playing'
  | 'paused'
  | 'reconnecting'
  | 'ended'
  | 'error';

/** The current Queue entry. */
export interface NowPlaying {
  key: number;
  track: TrackRow;
  /** Whether the Recommender picked it. */
  autoplay: boolean;
}

/** Everything about playback but the Queue's rows. */
export interface PlayerState {
  revision: number;
  status: PlaybackStatus;
  current: NowPlaying | null;
  positionMs: number;
  durationMs: number;
  /** From 0 to 100. */
  volume: number;
  canNext: boolean;
  canPrevious: boolean;
  /** How many entries come after the current one. */
  upcoming: number;
  /** How many entries came before it. */
  history: number;
  queueRevision: number;
  /** Why playback failed, in words for the user. Set in the error state only. */
  error: string | null;
  /** The latest queued song that could not be played and was passed over. */
  skipped: Skipped | null;
}

/** A queued song that was skipped because it could not be played. */
export interface Skipped {
  /** The Queue entry that was passed over, so it is said once. */
  key: number;
  message: string;
}

/** The clock moving on. It belongs to the state with the same revision. */
export interface PlayerPosition {
  revision: number;
  positionMs: number;
}

/** A list of the library that plays through the Queue. */
export type PlayList = { kind: 'playlist'; id: string } | { kind: 'liked' } | { kind: 'recent' };

export interface QueueItem {
  key: number;
  track: TrackRow;
  autoplay: boolean;
}

/** Some consecutive upcoming entries, or played ones with the latest first. */
export interface QueueSlice {
  queueRevision: number;
  offset: number;
  total: number;
  items: QueueItem[];
}

// The account. Mirrors src-tauri/src/sources/youtube/account.rs. The webview
// never learns more about the session than this.

export type AccountStatus = 'connected' | 'expired' | 'disconnected';

// The gate and the session check. Mirrors
// src-tauri/src/sources/youtube/service.rs.

export type PauseReason =
  | 'botCheck'
  | 'repeatedFailures'
  | 'stateUnreadable'
  | 'stateUnwritable'
  | 'expiryUnsaved';

/** Whether YouTube requests may go out. */
export type GateView =
  | { state: 'ready' }
  | { state: 'coolingDown'; untilMs: number }
  | { state: 'paused'; reason: PauseReason };

/** Whether the stored session has been seen to work. */
export type SessionCheck = 'unchecked' | 'checking' | 'verified' | 'offline' | 'unexpected';

export interface ServiceStatus {
  /** Rises with every change, so an older answer can be dropped. */
  revision: number;
  gate: GateView;
  check: SessionCheck;
}

// Likes and history. Mirrors src-tauri/src/ipc/library.rs.

/** A song was liked or unliked. */
export interface LikesChanged {
  /** Rises with every library event, so an older one can be dropped. */
  revision: number;
  trackId: string;
  liked: boolean;
  likedCount: number;
}

/** A play started, so the songs played changed. */
export interface HistoryChanged {
  revision: number;
  /** How many different songs have been played. */
  playedCount: number;
}

/** What the Library view starts from. */
export interface LibrarySummary {
  /** The revision of the newest event this snapshot already includes. */
  revision: number;
  likedCount: number;
  playedCount: number;
  /** Why the library cannot be used, in words. Its file is left as it is. */
  problem: string | null;
}

// Local playlists. Mirrors src-tauri/src/ipc/playlists.rs.

/** Every playlist as it stands, after a change or as a starting point. */
export interface PlaylistsChanged {
  /** Rises with every library event, so an older one can be dropped. */
  revision: number;
  playlists: PlaylistSummary[];
  /** The playlist whose name or entries changed, if one did. */
  changed: string | null;
}

export interface Bridge {
  getAppState(): Promise<AppSnapshot>;
  navigate(route: Route): Promise<void>;
  getHome(): Promise<HomeView>;
  /** The search Rust remembers, with its results. */
  getSearch(): Promise<SearchView>;
  /** The only thing that sends a search. The outcome arrives as events. */
  submitSearch(query: string, filter: SourceFilter): Promise<void>;
  /** Switches the chip. Sends nothing to a service. The outcome arrives as an event. */
  setSearchFilter(filter: SourceFilter): Promise<void>;
  /**
   * A type chip. Rust shows what that chip already found or makes its one
   * request; the outcome arrives as events.
   */
  setSearchKind(kind: SearchKind): Promise<void>;
  onSearchChanged(handler: (view: SearchView) => void): Promise<() => void>;
  /** Resolves with a function that stops listening. */
  onNavigationChanged(handler: (navigation: NavigationState) => void): Promise<() => void>;

  getPlayerState(): Promise<PlayerState>;
  /** Up to `limit` upcoming entries, starting at `offset`. */
  getQueue(offset: number, limit: number): Promise<QueueSlice>;
  /** Up to `limit` entries played before the current one, the latest first. */
  getQueueHistory(offset: number, limit: number): Promise<QueueSlice>;
  // The answer to each of these arrives as an event.
  playSearchResult(trackId: string): Promise<void>;
  togglePlay(): Promise<void>;
  nextTrack(): Promise<void>;
  previousTrack(): Promise<void>;
  seek(positionMs: number): Promise<void>;
  seekBy(deltaMs: number): Promise<void>;
  setVolume(volume: number): Promise<void>;
  changeVolume(delta: number): Promise<void>;
  moveQueueEntry(key: number, before: number | null): Promise<void>;
  toggleQueue(): Promise<void>;
  onPlayerChanged(handler: (state: PlayerState) => void): Promise<() => void>;
  onPlayerPosition(handler: (position: PlayerPosition) => void): Promise<() => void>;

  getLibrarySummary(): Promise<LibrarySummary>;
  // The answer to each of these arrives as an event.
  likeTrack(trackId: string): Promise<void>;
  unlikeTrack(trackId: string): Promise<void>;
  /** Which of these songs are liked. */
  getLikedIds(trackIds: string[]): Promise<string[]>;
  /** Up to `limit` liked songs from `offset`, the newest like first. */
  getLikedSlice(offset: number, limit: number): Promise<TrackSlice>;
  /** Up to `limit` played songs from `offset`, the one played last first. */
  getRecentSlice(offset: number, limit: number): Promise<TrackSlice>;
  /**
   * Queues a whole list and plays from one of its rows, named by its place and
   * its song. The answer arrives as an event.
   */
  playList(list: PlayList, position: number, trackId: string): Promise<void>;
  onLikesChanged(handler: (change: LikesChanged) => void): Promise<() => void>;
  onHistoryChanged(handler: (change: HistoryChanged) => void): Promise<() => void>;

  getPlaylists(): Promise<PlaylistsChanged>;
  /** Up to `limit` entries of a playlist from `offset`, in its order. */
  getPlaylistSlice(id: string, offset: number, limit: number): Promise<TrackSlice>;
  // The answer to each of these arrives as an event.
  createPlaylist(name: string): Promise<PlaylistSummary>;
  renamePlaylist(id: string, name: string): Promise<void>;
  deletePlaylist(id: string): Promise<void>;
  addToPlaylist(id: string, trackId: string): Promise<void>;
  /** Takes out the entry at `position`, if it still holds the song. */
  removeFromPlaylist(id: string, position: number, trackId: string): Promise<void>;
  onPlaylistsChanged(handler: (change: PlaylistsChanged) => void): Promise<() => void>;

  getAccountStatus(): Promise<AccountStatus>;
  /** Opens the sign-in window, or focuses it. The outcome arrives as an event. */
  signIn(): Promise<void>;
  disconnect(): Promise<void>;
  onAccountChanged(handler: (status: AccountStatus) => void): Promise<() => void>;
  /** Why a sign-in window could not be read or its session saved. */
  onSignInFailed(handler: (reason: string) => void): Promise<() => void>;

  getYoutubeStatus(): Promise<ServiceStatus>;
  /** Ends a pause. Sends nothing to YouTube. The outcome arrives as an event. */
  resumeYoutube(): Promise<void>;
  onYoutubeStatusChanged(handler: (status: ServiceStatus) => void): Promise<() => void>;
}

/** Must match NAVIGATION_CHANGED in src-tauri/src/ipc/mod.rs. */
const NAVIGATION_CHANGED = 'navigation-changed';
/** Must match the names in src-tauri/src/ipc/player.rs. */
const PLAYER_CHANGED = 'player-changed';
const PLAYER_POSITION = 'player-position';
/** Must match the names in src-tauri/src/ipc/library.rs. */
const LIKES_CHANGED = 'likes-changed';
const HISTORY_CHANGED = 'history-changed';
/** Must match the name in src-tauri/src/ipc/playlists.rs. */
const PLAYLISTS_CHANGED = 'playlists-changed';
/** Must match the names in src-tauri/src/ipc/account.rs. */
const ACCOUNT_CHANGED = 'account-changed';
const SIGN_IN_FAILED = 'sign-in-failed';
/** Must match the name in src-tauri/src/ipc/search.rs. */
const SEARCH_CHANGED = 'search-changed';
/** Must match the name in src-tauri/src/ipc/youtube.rs. */
const YOUTUBE_STATUS_CHANGED = 'youtube-status-changed';

export const tauriBridge: Bridge = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  navigate: (route) => invoke('navigate', { route }),
  getHome: () => invoke<HomeView>('get_home'),
  getSearch: () => invoke<SearchView>('get_search'),
  submitSearch: (query, filter) => invoke('submit_search', { query, filter }),
  setSearchFilter: (filter) => invoke('set_search_filter', { filter }),
  setSearchKind: (kind) => invoke('set_search_kind', { kind }),
  onSearchChanged: (handler) =>
    listen<SearchView>(SEARCH_CHANGED, (event) => handler(event.payload)),
  onNavigationChanged: (handler) =>
    listen<NavigationState>(NAVIGATION_CHANGED, (event) => handler(event.payload)),
  getPlayerState: () => invoke<PlayerState>('get_player_state'),
  getQueue: (offset, limit) => invoke<QueueSlice>('get_queue', { offset, limit }),
  getQueueHistory: (offset, limit) =>
    invoke<QueueSlice>('get_queue_history', { offset, limit }),
  playSearchResult: (trackId) => invoke('play_search_result', { trackId }),
  togglePlay: () => invoke('toggle_play'),
  nextTrack: () => invoke('next_track'),
  previousTrack: () => invoke('previous_track'),
  seek: (positionMs) => invoke('seek', { positionMs }),
  seekBy: (deltaMs) => invoke('seek_by', { deltaMs }),
  setVolume: (volume) => invoke('set_volume', { volume }),
  changeVolume: (delta) => invoke('change_volume', { delta }),
  moveQueueEntry: (key, before) => invoke('move_queue_entry', { key, before }),
  toggleQueue: () => invoke('toggle_queue'),
  onPlayerChanged: (handler) =>
    listen<PlayerState>(PLAYER_CHANGED, (event) => handler(event.payload)),
  onPlayerPosition: (handler) =>
    listen<PlayerPosition>(PLAYER_POSITION, (event) => handler(event.payload)),
  getLibrarySummary: () => invoke<LibrarySummary>('get_library_summary'),
  likeTrack: (trackId) => invoke('like_track', { trackId }),
  unlikeTrack: (trackId) => invoke('unlike_track', { trackId }),
  getLikedIds: (trackIds) => invoke<string[]>('get_liked_ids', { trackIds }),
  getLikedSlice: (offset, limit) => invoke<TrackSlice>('get_liked_slice', { offset, limit }),
  getRecentSlice: (offset, limit) => invoke<TrackSlice>('get_recent_slice', { offset, limit }),
  playList: (list, position, trackId) => invoke('play_list', { list, position, trackId }),
  onLikesChanged: (handler) =>
    listen<LikesChanged>(LIKES_CHANGED, (event) => handler(event.payload)),
  onHistoryChanged: (handler) =>
    listen<HistoryChanged>(HISTORY_CHANGED, (event) => handler(event.payload)),
  getPlaylists: () => invoke<PlaylistsChanged>('get_playlists'),
  getPlaylistSlice: (id, offset, limit) =>
    invoke<TrackSlice>('get_playlist_slice', { id, offset, limit }),
  createPlaylist: (name) => invoke<PlaylistSummary>('create_playlist', { name }),
  renamePlaylist: (id, name) => invoke('rename_playlist', { id, name }),
  deletePlaylist: (id) => invoke('delete_playlist', { id }),
  addToPlaylist: (id, trackId) => invoke('add_to_playlist', { id, trackId }),
  removeFromPlaylist: (id, position, trackId) =>
    invoke('remove_from_playlist', { id, position, trackId }),
  onPlaylistsChanged: (handler) =>
    listen<PlaylistsChanged>(PLAYLISTS_CHANGED, (event) => handler(event.payload)),
  getAccountStatus: () => invoke<AccountStatus>('get_account_status'),
  signIn: () => invoke('sign_in'),
  disconnect: () => invoke('disconnect'),
  onAccountChanged: (handler) =>
    listen<AccountStatus>(ACCOUNT_CHANGED, (event) => handler(event.payload)),
  onSignInFailed: (handler) =>
    listen<string>(SIGN_IN_FAILED, (event) => handler(event.payload)),
  getYoutubeStatus: () => invoke<ServiceStatus>('get_youtube_status'),
  resumeYoutube: () => invoke('resume_youtube'),
  onYoutubeStatusChanged: (handler) =>
    listen<ServiceStatus>(YOUTUBE_STATUS_CHANGED, (event) => handler(event.payload)),
};
