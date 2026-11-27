<script lang="ts">
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import { onMount } from 'svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import GateNotice from '../lib/components/GateNotice.svelte';
  import Shelf from '../lib/components/Shelf.svelte';
  import TrackList from '../lib/components/TrackList.svelte';
  import { countLabel } from '../lib/format';
  import type { AccountStatus, ServiceStatus, SourceFilter } from '../lib/ipc';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  interface Props {
    catalog: CatalogStore;
    /** `null` until Rust has said. */
    account: AccountStatus | null;
    /** `null` until Rust has said. */
    youtube: ServiceStatus | null;
    onsignin: () => void;
    onresume: () => void;
    onopen: (id: string) => void;
    /** Plays the chosen result. */
    onplay: (trackId: string) => void;
    /** Ctrl+F asked for the search box. */
    focusWanted: boolean;
    onfocused: () => void;
  }

  let {
    catalog,
    account,
    youtube,
    onsignin,
    onresume,
    onopen,
    onplay,
    focusWanted,
    onfocused,
  }: Props = $props();

  let box = $state<HTMLInputElement>();

  const chips: { value: SourceFilter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'youtube', label: 'YouTube' },
    { value: 'soundcloud', label: 'SoundCloud' },
  ];

  /** What is typed and chosen during this visit. Rust remembers what was submitted. */
  let typed = $state.raw<string | null>(null);
  let chosen = $state.raw<SourceFilter | null>(null);
  const results = $derived(catalog.search);
  const query = $derived(typed ?? results?.query ?? '');
  const filter = $derived(chosen ?? results?.filter ?? 'all');
  const live = $derived(results !== null && results.filter !== 'soundcloud');

  /** Why YouTube Music cannot be asked. It wins over whatever the search says. */
  const blocked = $derived.by(() => {
    if (!live) return null;
    if (account === 'disconnected' || account === 'expired') return account;
    return youtube && youtube.gate.state !== 'ready' ? 'gate' : null;
  });

  const found = $derived(!blocked && results?.status === 'results' ? results : null);

  const summary = $derived.by(() => {
    if (!found) return '';
    if (live) return countLabel(found.tracks.length, 'song', 'songs');
    return [
      countLabel(found.tracks.length, 'track', 'tracks'),
      countLabel(found.collections.length, 'album or playlist', 'albums and playlists'),
    ].join(', ');
  });

  onMount(() => void catalog.openSearch());

  $effect(() => {
    if (!focusWanted || !box) return;
    box.focus();
    box.select();
    onfocused();
  });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void catalog.submitSearch(query, filter);
  }

  function choose(next: SourceFilter) {
    chosen = next;
    void catalog.setSearchFilter(next);
  }
</script>

{#snippet action(label: string, run: () => void)}
  <button type="button" class="chip mt-4" onclick={run}>{label}</button>
{/snippet}

<div class="page">
  <h1 class="page-title">Search</h1>

  <form class="mt-8" role="search" onsubmit={submit}>
    <div class="search-field">
      <MagnifyingGlassIcon size={20} aria-hidden="true" />
      <input
        bind:this={box}
        type="search"
        name="query"
        aria-label="Search for songs"
        placeholder="Search for songs, then press Enter…"
        autocomplete="off"
        spellcheck="false"
        enterkeyhint="search"
        value={query}
        oninput={(event) => (typed = event.currentTarget.value)}
      />
    </div>

    <fieldset class="mt-4 flex flex-wrap gap-2">
      <legend class="sr-only">Source</legend>
      {#each chips as { value, label } (value)}
        <label class="chip">
          <input
            class="sr-only"
            type="radio"
            name="source"
            {value}
            checked={filter === value}
            onchange={() => choose(value)}
          />
          <span translate={value === 'all' ? undefined : 'no'}>{label}</span>
        </label>
      {/each}
    </fieldset>
  </form>

  <p class="sr-only" role="status">{summary}</p>

  <!-- Always there, so what appears in it is announced. -->
  <div aria-live="polite">
    {#if blocked === 'disconnected'}
      <EmptyState Icon={MagnifyingGlassIcon} title="Sign in to search YouTube Music">
        Lumen searches with your own account.
        <br />
        {@render action('Sign in', onsignin)}
      </EmptyState>
    {:else if blocked === 'expired'}
      <EmptyState Icon={MagnifyingGlassIcon} title="Your session expired">
        Log in again to search YouTube Music.
        <br />
        {@render action('Log in again', onsignin)}
      </EmptyState>
    {:else if blocked === 'gate'}
      <div class="mt-8 max-w-xl">
        <GateNotice gate={youtube?.gate} check={undefined} {onresume} />
      </div>
    {:else if results?.status === 'idle' || results?.status === 'blocked'}
      <EmptyState Icon={MagnifyingGlassIcon} title="Press Enter to search">
        Type a song or an artist.
      </EmptyState>
    {:else if results?.status === 'loading'}
      <EmptyState Icon={MagnifyingGlassIcon} title="Searching YouTube Music…">
        This takes a moment.
      </EmptyState>
    {:else if results?.status === 'noResults'}
      <EmptyState Icon={MagnifyingGlassIcon} title="No matches for “{results.query.trim()}”">
        Check the spelling, or try fewer words.
      </EmptyState>
    {:else if results?.status === 'offline'}
      <EmptyState Icon={MagnifyingGlassIcon} title="Could not reach YouTube Music">
        Check your internet connection.
        <br />
        {@render action('Try again', () => void catalog.submitSearch(results.query, results.filter))}
      </EmptyState>
    {:else if results?.status === 'unexpected'}
      <EmptyState Icon={MagnifyingGlassIcon} title="YouTube Music gave an unexpected answer">
        Three in a row pause YouTube Music.
        <br />
        {@render action('Try again', () => void catalog.submitSearch(results.query, results.filter))}
      </EmptyState>
    {/if}
  </div>

  {#if found}
    <div class="mt-8 flex flex-col gap-9">
      {#if found.tracks.length}
        <section aria-labelledby="search-tracks">
          <h2 id="search-tracks" class="shelf-title mb-3">{live ? 'Songs' : 'Tracks'}</h2>
          <TrackList tracks={found.tracks} labelledby="search-tracks" {onplay} />
        </section>
      {/if}
      {#if found.collections.length}
        <Shelf title="Albums and playlists" items={found.collections} {onopen} />
      {/if}
    </div>
  {/if}
</div>
