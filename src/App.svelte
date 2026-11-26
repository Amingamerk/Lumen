<script lang="ts">
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import { onMount } from 'svelte';
  import MainFrame from './lib/components/MainFrame.svelte';
  import PlayerBar from './lib/components/PlayerBar.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import { tauriBridge } from './lib/ipc';
  import { AppStore } from './lib/stores/app.svelte';
  import Home from './routes/Home.svelte';
  import Library from './routes/Library.svelte';
  import Playlist from './routes/Playlist.svelte';
  import Search from './routes/Search.svelte';

  let { store = new AppStore(tauriBridge) }: { store?: AppStore } = $props();

  const route = $derived(store.navigation?.route);
  const playlist = $derived(
    route?.view === 'playlist' ? store.playlists.find((p) => p.id === route.id) : undefined,
  );
  const pageKey = $derived(route?.view === 'playlist' ? `playlist/${route.id}` : route?.view);

  onMount(() => {
    const connection = store.connect();
    return () => void connection.then((disconnect) => disconnect());
  });
</script>

<a class="skip-link" href="#main">Skip to content</a>

<div class="shell">
  <Sidebar playlists={store.playlists} {route} onnavigate={(next) => store.navigate(next)} />

  <MainFrame view={route?.view} {pageKey}>
    {#if store.error}
      <div class="alert" role="alert">
        <WarningCircleIcon size={20} weight="fill" aria-hidden="true" />
        {#if store.navigation}
          <p>That did not work. Try again, or restart Lumen. Details: {store.error}</p>
        {:else}
          <p>Lumen could not load. Restart Lumen to try again. Details: {store.error}</p>
        {/if}
      </div>
    {/if}

    {#if route?.view === 'home'}
      <Home />
    {:else if route?.view === 'search'}
      <Search />
    {:else if route?.view === 'library'}
      <Library />
    {:else if playlist}
      <Playlist {playlist} />
    {/if}
  </MainFrame>

  <PlayerBar />
</div>
