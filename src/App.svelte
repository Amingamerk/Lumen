<script lang="ts">
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import { onMount } from 'svelte';
  import MainFrame from './lib/components/MainFrame.svelte';
  import PlayerBar from './lib/components/PlayerBar.svelte';
  import PlaylistPicker from './lib/components/PlaylistPicker.svelte';
  import QueuePanel from './lib/components/QueuePanel.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import Toast from './lib/components/Toast.svelte';
  import { tauriBridge, type QueuePlacement } from './lib/ipc';
  import { shortcutFor } from './lib/shortcuts';
  import { AccountStore } from './lib/stores/account.svelte';
  import { AppStore } from './lib/stores/app.svelte';
  import { CatalogStore } from './lib/stores/catalog.svelte';
  import { LibraryStore } from './lib/stores/library.svelte';
  import { PlayerStore } from './lib/stores/player.svelte';
  import { PlaylistsStore } from './lib/stores/playlists.svelte';
  import { QueueStore } from './lib/stores/queue.svelte';
  import { YoutubeStore } from './lib/stores/youtube.svelte';
  import Home from './routes/Home.svelte';
  import Library from './routes/Library.svelte';
  import Playlist from './routes/Playlist.svelte';
  import Search from './routes/Search.svelte';

  interface Props {
    store?: AppStore;
    catalog?: CatalogStore;
    library?: LibraryStore;
    player?: PlayerStore;
    playlists?: PlaylistsStore;
    queue?: QueueStore;
    account?: AccountStore;
    youtube?: YoutubeStore;
  }

  let {
    store = new AppStore(tauriBridge),
    catalog = new CatalogStore(tauriBridge),
    library = new LibraryStore(tauriBridge),
    player = new PlayerStore(tauriBridge),
    playlists = new PlaylistsStore(tauriBridge),
    queue = new QueueStore(tauriBridge),
    account = new AccountStore(tauriBridge),
    youtube = new YoutubeStore(tauriBridge),
  }: Props = $props();

  const route = $derived(store.navigation?.route);
  const pageKey = $derived(route?.view === 'playlist' ? `playlist/${route.id}` : route?.view);
  const queueOpen = $derived(store.navigation?.queueOpen ?? false);
  const error = $derived(
    store.error ??
      catalog.error ??
      library.error ??
      player.error ??
      playlists.error ??
      queue.error ??
      account.error ??
      youtube.error,
  );

  /** Rust decides whether the playlist exists. */
  const queueTrack = (trackId: string, placement: QueuePlacement) =>
    void player.queueTrack(trackId, placement);
  const openPlaylist = (id: string) => void store.navigate({ view: 'playlist', id });

  /** Makes a playlist, then opens it. */
  async function createAndOpen(name: string) {
    const made = await playlists.create(name);
    if (made) openPlaylist(made.id);
  }

  /** Makes a playlist for the song being chosen for, and puts it in. */
  async function createAndAdd(name: string) {
    const picking = playlists.picking;
    const made = await playlists.create(name);
    if (!made) return;
    if (picking) await playlists.add(made.id, picking.trackId);
    playlists.stopChoosing();
  }

  async function addTo(playlistId: string) {
    const picking = playlists.picking;
    if (picking) await playlists.add(playlistId, picking.trackId);
    playlists.stopChoosing();
  }

  /** Set by Ctrl+F until the search box has taken the focus. */
  let searchFocusWanted = $state(false);

  function onkeydown(event: KeyboardEvent) {
    const shortcut = shortcutFor(event);
    if (!shortcut) return;
    event.preventDefault();
    switch (shortcut.action) {
      case 'toggle':
        void player.togglePlay();
        break;
      case 'next':
        void player.next();
        break;
      case 'previous':
        void player.previous();
        break;
      case 'seek':
        void player.seekBy(shortcut.deltaMs);
        break;
      case 'volume':
        void player.changeVolume(shortcut.delta);
        break;
      case 'focus-search':
        searchFocusWanted = true;
        if (route?.view !== 'search') void store.navigate({ view: 'search' });
        break;
    }
  }

  onMount(() => {
    const connections = [
      store.connect(),
      catalog.connect(),
      library.connect(),
      playlists.connect(),
      player.connect(),
      account.connect(),
      youtube.connect(),
    ];
    return () => {
      for (const connection of connections) void connection.then((disconnect) => disconnect());
    };
  });
</script>

<svelte:window {onkeydown} />

<a class="skip-link" href="#main">Skip to content</a>

<div class="shell" data-queue={queueOpen ? 'open' : undefined}>
  <Sidebar
    playlists={playlists.playlists}
    {route}
    onnavigate={(next) => store.navigate(next)}
    oncreate={(name) => void createAndOpen(name)}
    account={account.status}
    youtube={youtube.status}
    onsignin={() => void account.signIn()}
    ondisconnect={() => void account.disconnect()}
    onresume={() => void youtube.resume()}
  />

  <MainFrame view={route?.view} {pageKey}>
    {#if error}
      <div class="alert" role="alert">
        <WarningCircleIcon size={20} weight="fill" aria-hidden="true" />
        {#if store.navigation}
          <p>That did not work. Try again, or restart Lumen. Details: {error}</p>
        {:else}
          <p>Lumen could not load. Restart Lumen to try again. Details: {error}</p>
        {/if}
      </div>
    {/if}

    {#if route?.view === 'home'}
      <Home
        {catalog}
        {library}
        onopen={openPlaylist}
        onshowall={(part) => {
          library.part = part;
          void store.navigate({ view: 'library' });
        }}
      />
    {:else if route?.view === 'search'}
      <Search
        {catalog}
        {library}
        {playlists}
        account={account.status}
        youtube={youtube.status}
        onsignin={() => void account.signIn()}
        onresume={() => void youtube.resume()}
        onplay={(trackId) => void player.playSearchResult(trackId)}
        onqueue={queueTrack}
        focusWanted={searchFocusWanted}
        onfocused={() => (searchFocusWanted = false)}
      />
    {:else if route?.view === 'library'}
      <Library {playlists} {library} onopen={openPlaylist} onqueue={queueTrack} />
    {:else if route?.view === 'playlist'}
      {@const id = route.id}
      <Playlist {playlists} {library} {id} onqueue={queueTrack} />
    {/if}
  </MainFrame>

  {#if queueOpen}
    <QueuePanel {player} {queue} onclose={() => void store.toggleQueue()} />
  {/if}

  <PlayerBar {player} {library} {queueOpen} ontogglequeue={() => void store.toggleQueue()} />
</div>

{#if playlists.picking}
  <PlaylistPicker
    title={playlists.picking.title}
    playlists={playlists.playlists}
    onchoose={(playlistId) => void addTo(playlistId)}
    oncreate={(name) => void createAndAdd(name)}
    onclose={() => playlists.stopChoosing()}
  />
{/if}

<!-- A new notice is a new element, so it is announced again. -->
{#key player.notice}
  {#if player.notice}
    <Toast message={player.notice.message} ondismiss={() => player.dismissNotice()} />
  {/if}
{/key}
