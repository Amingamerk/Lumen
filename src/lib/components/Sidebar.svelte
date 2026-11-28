<script lang="ts">
  import BooksIcon from 'phosphor-svelte/lib/BooksIcon';
  import HouseIcon from 'phosphor-svelte/lib/HouseIcon';
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import PlusIcon from 'phosphor-svelte/lib/PlusIcon';
  import { tick } from 'svelte';
  import type { AccountStatus, PlaylistSummary, Route, ServiceStatus } from '../ipc';
  import AccountControl from './AccountControl.svelte';
  import BrandMark from './BrandMark.svelte';

  interface Props {
    /** `null` until Rust has said. */
    playlists: PlaylistSummary[] | null;
    route: Route | undefined;
    onnavigate: (route: Route) => void;
    /** Makes a playlist with this name and opens it. */
    oncreate: (name: string) => void;
    /** `null` until Rust has said. */
    account: AccountStatus | null;
    /** `null` until Rust has said. */
    youtube: ServiceStatus | null;
    onsignin: () => void;
    ondisconnect: () => void;
    onresume: () => void;
  }

  let {
    playlists,
    route,
    onnavigate,
    oncreate,
    account,
    youtube,
    onsignin,
    ondisconnect,
    onresume,
  }: Props = $props();

  const destinations = [
    { view: 'home', label: 'Home', Icon: HouseIcon },
    { view: 'search', label: 'Search', Icon: MagnifyingGlassIcon },
    { view: 'library', label: 'Library', Icon: BooksIcon },
  ] as const;

  let naming = $state(false);
  let name = $state('');
  let box = $state<HTMLInputElement>();

  async function startNaming() {
    naming = true;
    await tick();
    box?.focus();
  }

  function finishNaming(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim()) return;
    oncreate(name);
    name = '';
    naming = false;
  }

  function cancelNaming() {
    name = '';
    naming = false;
  }

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
    <li>
      {#if naming}
        <form class="px-1 py-1" onsubmit={finishNaming}>
          <input
            bind:this={box}
            bind:value={name}
            class="picker-name w-full"
            type="text"
            name="new-playlist"
            aria-label="Name of the new playlist"
            placeholder="Name, then Enter"
            autocomplete="off"
            spellcheck="false"
            onkeydown={(event) => event.key === 'Escape' && cancelNaming()}
            onblur={() => !name.trim() && cancelNaming()}
          />
        </form>
      {:else}
        <button type="button" class="nav-link h-10 w-full gap-3" onclick={startNaming}>
          <PlusIcon size={18} aria-hidden="true" />
          New playlist
        </button>
      {/if}
    </li>
    {#each playlists ?? [] as playlist, order (playlist.id)}
      <li class="rise" style:--order={destinations.length + order}>
        <a
          class="nav-link h-10 justify-between gap-3"
          href="#playlist/{playlist.id}"
          draggable="false"
          title={playlist.name}
          aria-label="{playlist.name}, playlist"
          aria-current={route?.view === 'playlist' && route.id === playlist.id ? 'page' : undefined}
          onclick={(event) => open(event, { view: 'playlist', id: playlist.id })}
        >
          <span class="min-w-0 truncate">{playlist.name}</span>
        </a>
      </li>
    {:else}
      <!-- Before Rust has answered the list is unknown, not empty. -->
      {#if playlists}
        <li class="text-ink-faint px-3 py-2">No playlists yet</li>
      {/if}
    {/each}
  </ul>

  {#if account}
    <AccountControl status={account} {youtube} {onsignin} {ondisconnect} {onresume} />
  {/if}
</nav>
