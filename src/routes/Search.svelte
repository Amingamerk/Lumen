<script lang="ts">
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import { onMount } from 'svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import Shelf from '../lib/components/Shelf.svelte';
  import TrackList from '../lib/components/TrackList.svelte';
  import { countLabel } from '../lib/format';
  import type { SourceFilter } from '../lib/ipc';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  interface Props {
    catalog: CatalogStore;
    onopen: (id: string) => void;
    onplay: (trackId: string) => void;
    /** Ctrl+F asked for the search box. */
    focusWanted: boolean;
    onfocused: () => void;
  }

  let { catalog, onopen, onplay, focusWanted, onfocused }: Props = $props();

  let box = $state<HTMLInputElement>();

  const chips: { value: SourceFilter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'youtube', label: 'YouTube' },
    { value: 'soundcloud', label: 'SoundCloud' },
  ];

  /** What is typed and chosen during this visit. Rust remembers it for the next. */
  let draft = $state.raw<{ query: string; filter: SourceFilter } | null>(null);

  const results = $derived(catalog.search);
  const query = $derived(draft?.query ?? results?.query ?? '');
  const filter = $derived(draft?.filter ?? results?.filter ?? 'all');
  const summary = $derived(
    results
      ? [
          countLabel(results.tracks.length, 'track', 'tracks'),
          countLabel(results.collections.length, 'album or playlist', 'albums and playlists'),
        ].join(', ')
      : '',
  );

  onMount(() => void catalog.openSearch());

  $effect(() => {
    if (!focusWanted || !box) return;
    box.focus();
    box.select();
    onfocused();
  });

  function ask(next: { query: string; filter: SourceFilter }) {
    draft = next;
    void catalog.runSearch(next.query, next.filter);
  }
</script>

<div class="page">
  <h1 class="page-title">Search</h1>

  <form class="mt-8" role="search" onsubmit={(event) => event.preventDefault()}>
    <div class="search-field">
      <MagnifyingGlassIcon size={20} aria-hidden="true" />
      <input
        bind:this={box}
        type="search"
        name="query"
        aria-label="Search tracks, albums and playlists"
        placeholder="Search tracks, albums and playlists…"
        autocomplete="off"
        spellcheck="false"
        enterkeyhint="search"
        value={query}
        oninput={(event) => ask({ query: event.currentTarget.value, filter })}
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
            onchange={() => ask({ query, filter: value })}
          />
          <span translate={value === 'all' ? undefined : 'no'}>{label}</span>
        </label>
      {/each}
    </fieldset>
  </form>

  <p class="sr-only" role="status">{summary}</p>

  {#if results && results.tracks.length === 0 && results.collections.length === 0}
    <EmptyState Icon={MagnifyingGlassIcon} title="No matches for “{results.query.trim()}”">
      {#if results.filter === 'all'}
        Check the spelling, or try fewer words.
      {:else}
        Check the spelling, or choose All to search every source.
      {/if}
    </EmptyState>
  {:else if results}
    <div class="mt-8 flex flex-col gap-9">
      {#if results.tracks.length}
        <section aria-labelledby="search-tracks">
          <h2 id="search-tracks" class="shelf-title mb-3">Tracks</h2>
          <TrackList tracks={results.tracks} labelledby="search-tracks" {onplay} />
        </section>
      {/if}
      {#if results.collections.length}
        <Shelf title="Albums and playlists" items={results.collections} {onopen} />
      {/if}
    </div>
  {/if}
</div>
