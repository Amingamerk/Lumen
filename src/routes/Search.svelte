<script lang="ts">
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import { onMount } from 'svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import CoverArt from '../lib/components/CoverArt.svelte';
  import GateNotice from '../lib/components/GateNotice.svelte';
  import HitList from '../lib/components/HitList.svelte';
  import TrackList from '../lib/components/TrackList.svelte';
  import { countLabel, hitLabel } from '../lib/format';
  import type { AccountStatus, SearchKind, ServiceStatus, SourceFilter } from '../lib/ipc';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';
  import type { LibraryStore } from '../lib/stores/library.svelte';
  import type { PlaylistsStore } from '../lib/stores/playlists.svelte';

  interface Props {
    catalog: CatalogStore;
    library: LibraryStore;
    playlists: PlaylistsStore;
    /** `null` until Rust has said. */
    account: AccountStatus | null;
    /** `null` until Rust has said. */
    youtube: ServiceStatus | null;
    onsignin: () => void;
    onresume: () => void;
    /** Plays the chosen result. */
    onplay: (trackId: string) => void;
    /** Ctrl+F asked for the search box. */
    focusWanted: boolean;
    onfocused: () => void;
  }

  let {
    catalog,
    library,
    playlists,
    account,
    youtube,
    onsignin,
    onresume,
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

  const kinds: { value: SearchKind; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'songs', label: 'Songs' },
    { value: 'albums', label: 'Albums' },
    { value: 'artists', label: 'Artists' },
    { value: 'playlists', label: 'Playlists' },
  ];

  /** What is typed and chosen during this visit. Rust remembers what was submitted. */
  let typed = $state.raw<string | null>(null);
  let chosen = $state.raw<SourceFilter | null>(null);
  let chosenKind = $state.raw<SearchKind | null>(null);
  const results = $derived(catalog.search);
  const query = $derived(typed ?? results?.query ?? '');
  const filter = $derived(chosen ?? results?.filter ?? 'all');
  const kind = $derived(chosenKind ?? results?.kind ?? 'all');
  const live = $derived(results !== null && results.filter !== 'soundcloud');

  /** Why YouTube Music cannot be asked. It wins over whatever the search says. */
  const blocked = $derived.by(() => {
    if (!live) return null;
    if (account === 'disconnected' || account === 'expired') return account;
    return youtube && youtube.gate.state !== 'ready' ? 'gate' : null;
  });

  const found = $derived(live && !blocked && results?.status === 'results' ? results : null);

  /** What the list says to a screen reader once it arrives: `2 songs, 1 album`. */
  const summary = $derived(
    found
      ? [
          countLabel(found.tracks.length, 'song', 'songs'),
          countLabel(found.albums.length, 'album', 'albums'),
          countLabel(found.artists.length, 'artist', 'artists'),
          countLabel(found.playlists.length, 'playlist', 'playlists'),
        ]
          .filter((part) => !part.startsWith('0 '))
          .join(', ')
      : '',
  );
  const topLine = $derived(
    found?.top ? [...found.top.best.by, found.top.best.detail].filter(Boolean).join(' · ') : '',
  );

  onMount(() => void catalog.openSearch());

  // Asks Rust which of the songs on show are liked.
  $effect(() => {
    if (found) {
      void library.watch([...found.tracks, ...(found.top?.songs ?? [])].map((track) => track.id));
    }
  });

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

  function chooseKind(next: SearchKind) {
    chosenKind = next;
    void catalog.setSearchKind(next);
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

    {#if live}
      <fieldset class="mt-3 flex flex-wrap gap-2">
        <legend class="sr-only">Type</legend>
        {#each kinds as { value, label } (value)}
          <label class="chip">
            <input
              class="sr-only"
              type="radio"
              name="type"
              {value}
              checked={kind === value}
              onchange={() => chooseKind(value)}
            />
            <span>{label}</span>
          </label>
        {/each}
      </fieldset>
    {/if}
  </form>

  <p class="sr-only" role="status">{summary}</p>

  <!-- Always there, so what appears in it is announced. -->
  <div aria-live="polite">
    {#if results && !live}
      <EmptyState Icon={MagnifyingGlassIcon} title="SoundCloud arrives later">
        Search covers YouTube Music for now. SoundCloud comes in a later version.
      </EmptyState>
    {:else if blocked === 'disconnected'}
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
      {#if found.top}
        {@const top = found.top}
        <section aria-labelledby="search-top">
          <h2 id="search-top" class="shelf-title mb-3">Top result</h2>
          <div class="@container">
            <div class="grid gap-6 @3xl:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
              <div class="bg-deck flex items-center gap-5 rounded-[18px] p-5">
                <div class="size-24 shrink-0 {top.best.kind === 'artist' ? '[&_.art]:rounded-full' : ''}">
                  <CoverArt title={top.best.title} trackId={top.best.hasArt ? top.best.id : null} />
                </div>
                <div class="min-w-0">
                  <h3 class="font-display truncate text-2xl font-semibold" title={top.best.title}>
                    {top.best.title}
                  </h3>
                  <p class="text-ink-faint mt-1 truncate text-[13px]">
                    <span class="text-ink-soft font-semibold">{hitLabel(top.best.kind)}</span>
                    {#if topLine}<span aria-hidden="true"> · </span>{topLine}{/if}
                  </p>
                </div>
              </div>
              {#if top.songs.length}
                <TrackList tracks={top.songs} labelledby="search-top" {onplay} compact />
              {/if}
            </div>
          </div>
        </section>
      {/if}
      {#if found.tracks.length}
        <section aria-labelledby="search-tracks">
          <h2 id="search-tracks" class="shelf-title mb-3">Songs</h2>
          <TrackList
            tracks={found.tracks}
            labelledby="search-tracks"
            {onplay}
            liked={(id) => library.isLiked(id)}
            onlike={(id) => void library.toggle(id)}
            onadd={(row) => playlists.choose({ trackId: row.id, title: row.title })}
          />
        </section>
      {/if}
      {#each [{ id: 'search-albums', title: 'Albums', hits: found.albums }, { id: 'search-artists', title: 'Artists', hits: found.artists }, { id: 'search-playlists', title: 'Playlists', hits: found.playlists }] as shelf (shelf.id)}
        {#if shelf.hits.length}
          <section aria-labelledby={shelf.id}>
            <h2 id={shelf.id} class="shelf-title mb-3">{shelf.title}</h2>
            <HitList hits={shelf.hits} labelledby={shelf.id} />
          </section>
        {/if}
      {/each}
    </div>
  {/if}
</div>
