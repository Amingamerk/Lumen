<script lang="ts">
  import BooksIcon from 'phosphor-svelte/lib/BooksIcon';
  import HouseIcon from 'phosphor-svelte/lib/HouseIcon';
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import type { PlaylistSummary, Route } from '../ipc';
  import { SOURCE_NAMES } from '../sources';
  import BrandMark from './BrandMark.svelte';
  import SourceBadge from './SourceBadge.svelte';

  interface Props {
    playlists: PlaylistSummary[];
    route: Route | undefined;
    onnavigate: (route: Route) => void;
  }

  let { playlists, route, onnavigate }: Props = $props();

  const destinations = [
    { view: 'home', label: 'Home', Icon: HouseIcon },
    { view: 'search', label: 'Search', Icon: MagnifyingGlassIcon },
    { view: 'library', label: 'Library', Icon: BooksIcon },
  ] as const;

  /** Rust owns the route, so the link never changes the page by itself. */
  function open(event: MouseEvent, next: Route) {
    event.preventDefault();
    onnavigate(next);
  }
</script>

<nav class="sidebar surface" aria-label="Main">
  <div class="brand">
    <BrandMark />
  </div>

  <ul class="flex flex-col gap-1">
    {#each destinations as { view, label, Icon }, order (view)}
      {@const current = route?.view === view}
      <li class="rise" style:--order={order}>
        <a
          class="nav-link h-11 gap-3 text-[15px] font-semibold"
          href="#{view}"
          draggable="false"
          aria-current={current ? 'page' : undefined}
          onclick={(event) => open(event, { view })}
        >
          <Icon size={22} weight={current ? 'fill' : 'regular'} aria-hidden="true" />
          {label}
        </a>
      </li>
    {/each}
  </ul>

  <h2 id="playlists-title" class="section-title">Playlists</h2>
  <ul class="scroller min-h-0 flex-1" aria-labelledby="playlists-title">
    {#each playlists as playlist, order (playlist.id)}
      <li class="rise" style:--order={destinations.length + order}>
        <a
          class="nav-link h-10 justify-between gap-3"
          href="#playlist/{playlist.id}"
          draggable="false"
          title={playlist.name}
          aria-label="{playlist.name}, from {SOURCE_NAMES[playlist.source]}"
          aria-current={route?.view === 'playlist' && route.id === playlist.id ? 'page' : undefined}
          onclick={(event) => open(event, { view: 'playlist', id: playlist.id })}
        >
          <span class="min-w-0 truncate">{playlist.name}</span>
          <SourceBadge source={playlist.source} badge={playlist.badge} />
        </a>
      </li>
    {:else}
      <!-- Before the first snapshot arrives the list is unknown, not empty. -->
      {#if route}
        <li class="text-ink-faint px-3 py-2">No playlists yet</li>
      {/if}
    {/each}
  </ul>
</nav>
