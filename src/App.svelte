<script lang="ts">
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import { onMount } from 'svelte';
  import MainFrame from './lib/components/MainFrame.svelte';
  import PlayerBar from './lib/components/PlayerBar.svelte';
  import QueuePanel from './lib/components/QueuePanel.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import { tauriBridge } from './lib/ipc';
  import { AppStore } from './lib/stores/app.svelte';
  import { CatalogStore } from './lib/stores/catalog.svelte';
  import { PlayerStore } from './lib/stores/player.svelte';
  import { QueueStore } from './lib/stores/queue.svelte';
  import Home from './routes/Home.svelte';
  import Library from './routes/Library.svelte';
  import Playlist from './routes/Playlist.svelte';
  import Search from './routes/Search.svelte';

  interface Props {
    store?: AppStore;
    catalog?: CatalogStore;
    player?: PlayerStore;
    queue?: QueueStore;
  }

  let {
    store = new AppStore(tauriBridge),
    catalog = new CatalogStore(tauriBridge),
    player = new PlayerStore(tauriBridge),
    queue = new QueueStore(tauriBridge),
  }: Props = $props();

  const route = $derived(store.navigation?.route);
  const pageKey = $derived(route?.view === 'playlist' ? `playlist/${route.id}` : route?.view);
  const queueOpen = $derived(store.navigation?.queueOpen ?? false);
  const error = $derived(store.error ?? catalog.error ?? player.error ?? queue.error);

  /** Cards open albums and playlists alike; Rust decides whether it can. */
  const openCollection = (id: string) => void store.navigate({ view: 'playlist', id });

  onMount(() => {
    const connections = [store.connect(), player.connect()];
    return () => {
      for (const connection of connections) void connection.then((disconnect) => disconnect());
    };
  });
</script>

<a class="skip-link" href="#main">Skip to content</a>

<div class="shell" data-queue={queueOpen ? 'open' : undefined}>
  <Sidebar playlists={store.playlists} {route} onnavigate={(next) => store.navigate(next)} />

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
      <Home {catalog} onopen={openCollection} />
    {:else if route?.view === 'search'}
      <Search
        {catalog}
        onopen={openCollection}
        onplay={(trackId) => void player.playSearchResult(trackId)}
      />
    {:else if route?.view === 'library'}
      <Library {catalog} onopen={openCollection} />
    {:else if route?.view === 'playlist'}
      {@const id = route.id}
      <Playlist {catalog} {id} onplay={(index) => void player.playCollection(id, index)} />
    {/if}
  </MainFrame>

  {#if queueOpen}
    <QueuePanel {player} {queue} onclose={() => void store.toggleQueue()} />
  {/if}

  <PlayerBar {player} {queueOpen} ontogglequeue={() => void store.toggleQueue()} />
</div>
