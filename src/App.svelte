<script lang="ts">
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import { onMount } from 'svelte';
  import MainFrame from './lib/components/MainFrame.svelte';
  import PlayerBar from './lib/components/PlayerBar.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import { tauriBridge } from './lib/ipc';
  import { AppStore } from './lib/stores/app.svelte';
  import { CatalogStore } from './lib/stores/catalog.svelte';
  import Home from './routes/Home.svelte';
  import Library from './routes/Library.svelte';
  import Playlist from './routes/Playlist.svelte';
  import Search from './routes/Search.svelte';

  interface Props {
    store?: AppStore;
    catalog?: CatalogStore;
  }

  let { store = new AppStore(tauriBridge), catalog = new CatalogStore(tauriBridge) }: Props =
    $props();

  const route = $derived(store.navigation?.route);
  const pageKey = $derived(route?.view === 'playlist' ? `playlist/${route.id}` : route?.view);
  const error = $derived(store.error ?? catalog.error);

  /** Cards open albums and playlists alike; Rust decides whether it can. */
  const openCollection = (id: string) => void store.navigate({ view: 'playlist', id });

  onMount(() => {
    const connection = store.connect();
    return () => void connection.then((disconnect) => disconnect());
  });
</script>

<a class="skip-link" href="#main">Skip to content</a>

<div class="shell">
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
      <Search {catalog} onopen={openCollection} />
    {:else if route?.view === 'library'}
      <Library {catalog} onopen={openCollection} />
    {:else if route?.view === 'playlist'}
      <Playlist {catalog} id={route.id} />
    {/if}
  </MainFrame>

  <PlayerBar />
</div>
