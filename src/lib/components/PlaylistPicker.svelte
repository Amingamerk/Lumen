<!-- Chooses the playlist a song goes into, or makes a new one for it. -->
<script lang="ts">
  import PlusIcon from 'phosphor-svelte/lib/PlusIcon';
  import XIcon from 'phosphor-svelte/lib/XIcon';
  import { onMount } from 'svelte';
  import { releasePointerFocus } from '../focus';
  import { countLabel } from '../format';
  import type { PlaylistSummary } from '../ipc';

  interface Props {
    /** The song's title, so the person knows which one. */
    title: string;
    /** `null` until Rust has said. */
    playlists: PlaylistSummary[] | null;
    onchoose: (playlistId: string) => void;
    /** Makes a playlist holding the song. */
    oncreate: (name: string) => void;
    onclose: () => void;
  }

  let { title, playlists, onchoose, oncreate, onclose }: Props = $props();

  let name = $state('');
  let dialog = $state<HTMLElement>();
  const uid = $props.id();

  // The focus goes in, and comes back to the button that opened it.
  onMount(() => {
    const opener = document.activeElement;
    dialog?.querySelector<HTMLElement>('button, input')?.focus();
    return () => {
      if (opener instanceof HTMLElement) opener.focus();
    };
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose();
    }
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (name.trim()) oncreate(name);
  }
</script>

<div class="picker-backdrop" role="presentation" onclick={onclose}>
  <div
    bind:this={dialog}
    class="picker"
    role="dialog"
    aria-modal="true"
    aria-labelledby="{uid}-title"
    tabindex="-1"
    {onkeydown}
    onclick={(event) => event.stopPropagation()}
  >
    <div class="flex items-start justify-between gap-4">
      <h2 id="{uid}-title" class="font-display text-ink min-w-0 text-lg font-semibold break-words">
        Add “{title}” to a playlist
      </h2>
      <button
        type="button"
        class="step flex-none"
        aria-label="Close"
        onclick={(event) => {
          releasePointerFocus(event);
          onclose();
        }}
      >
        <XIcon size={16} aria-hidden="true" />
      </button>
    </div>

    {#if playlists?.length}
      <ul class="picker-list scroller">
        {#each playlists as playlist (playlist.id)}
          <li>
            <button
              type="button"
              class="nav-link h-11 w-full justify-between gap-3"
              onclick={(event) => {
                releasePointerFocus(event);
                onchoose(playlist.id);
              }}
            >
              <span class="min-w-0 truncate">{playlist.name}</span>
              <span class="text-ink-faint flex-none text-[13px] tabular-nums">
                {countLabel(playlist.trackCount, 'track', 'tracks')}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {:else if playlists}
      <p class="text-ink-soft text-[15px]">You have no playlists yet. Make the first one here.</p>
    {/if}

    <form class="flex gap-2" onsubmit={submit}>
      <input
        class="picker-name"
        type="text"
        name="playlist-name"
        aria-label="Name of a new playlist"
        placeholder="New playlist"
        autocomplete="off"
        spellcheck="false"
        bind:value={name}
      />
      <button type="submit" class="chip" disabled={!name.trim()}>
        <PlusIcon size={16} aria-hidden="true" />
        <span class="ml-1">Make and add</span>
      </button>
    </form>
  </div>
</div>
