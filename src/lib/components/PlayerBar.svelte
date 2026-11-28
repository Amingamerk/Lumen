<!-- The player's controls. It shows what Rust says and asks Rust for changes. -->
<script lang="ts">
  import MusicNoteIcon from 'phosphor-svelte/lib/MusicNoteIcon';
  import PauseIcon from 'phosphor-svelte/lib/PauseIcon';
  import PlayIcon from 'phosphor-svelte/lib/PlayIcon';
  import QueueIcon from 'phosphor-svelte/lib/QueueIcon';
  import RepeatIcon from 'phosphor-svelte/lib/RepeatIcon';
  import RepeatOnceIcon from 'phosphor-svelte/lib/RepeatOnceIcon';
  import ShuffleIcon from 'phosphor-svelte/lib/ShuffleIcon';
  import SkipBackIcon from 'phosphor-svelte/lib/SkipBackIcon';
  import SkipForwardIcon from 'phosphor-svelte/lib/SkipForwardIcon';
  import { releasePointerFocus } from '../focus';
  import type { PlaybackStatus, RepeatMode } from '../ipc';
  import type { LibraryStore } from '../stores/library.svelte';
  import type { PlayerStore } from '../stores/player.svelte';
  import CoverArt from './CoverArt.svelte';
  import LikeButton from './LikeButton.svelte';
  import SeekBar from './SeekBar.svelte';
  import VolumeSlider from './VolumeSlider.svelte';

  interface Props {
    player: PlayerStore;
    library: LibraryStore;
    queueOpen: boolean;
    ontogglequeue: () => void;
  }

  let { player, library, queueOpen, ontogglequeue }: Props = $props();

  /** The statuses the controls alone do not tell apart. */
  const WORDS: Partial<Record<PlaybackStatus, string>> = {
    loading: 'Loading…',
    reconnecting: 'Reconnecting…',
    ended: 'Ended',
    error: 'Could not play',
  };

  const REPEAT_LABELS: Record<RepeatMode, string> = {
    off: 'Repeat',
    all: 'Repeat all',
    one: 'Repeat one',
  };

  const current = $derived(player.state?.current ?? null);
  const idle = $derived(current === null);
  const status = $derived(player.state?.status ?? 'idle');
  /** A stream is on its way: there is nothing to seek in yet. */
  const waiting = $derived(status === 'loading' || status === 'reconnecting');
  // While it waits it is about to play, so the button pauses.
  const playing = $derived(status === 'playing' || waiting);
  const seekable = $derived(!idle && !waiting && status !== 'error');
  const canNext = $derived(player.state?.canNext ?? false);
  const canPrevious = $derived(player.state?.canPrevious ?? false);
  const shuffle = $derived(player.state?.shuffle ?? false);
  const repeat = $derived(player.state?.repeat ?? 'off');

  // Asks Rust about the song that plays, once per song.
  $effect(() => {
    const id = current?.track.id;
    if (id) void library.watch([id]);
  });

  /** Runs a button's action and gives the keyboard back after a mouse click. */
  function press(event: MouseEvent, action: () => unknown) {
    releasePointerFocus(event);
    void action();
  }
</script>

<section class="player surface" aria-label="Player">
  <div class="flex min-w-0 items-center gap-3">
    {#if current}
      {@const track = current.track}
      {@const artists = track.artists.join(', ')}
      <div class="now-art">
        <CoverArt title={track.album ?? track.title} trackId={track.hasArt ? track.id : null} />
      </div>
      <div class="min-w-0">
        <p class="truncate text-[15px] font-semibold" title={track.title}>{track.title}</p>
        {#if artists}
          <p class="text-ink-faint truncate text-[13px]" title={artists}>{artists}</p>
        {/if}
        <div class="flex items-center gap-2">
          <p class="text-ink-soft text-[12px]" role="status">{WORDS[status]}</p>
          {#if status === 'error'}
            <button
              type="button"
              class="retry"
              onclick={(event) => press(event, () => player.togglePlay())}
            >
              Try again
            </button>
          {/if}
        </div>
      </div>
      <!-- A local file belongs to no service, so there is nothing to keep. -->
      {#if track.sources.length > 0}
        <LikeButton
          liked={library.isLiked(track.id)}
          title={track.title}
          ontoggle={() => void library.toggle(track.id)}
        />
      {/if}
    {:else}
      <div class="art-slot" aria-hidden="true">
        <MusicNoteIcon size={22} />
      </div>
      <p class="text-ink-soft min-w-0 truncate">Nothing is playing</p>
    {/if}
  </div>

  <div class="flex flex-col items-center gap-2">
    <div class="flex items-center gap-4">
      <button
        type="button"
        class="transport transport-mode"
        disabled={player.state === null}
        aria-label="Shuffle"
        aria-pressed={shuffle}
        title="Shuffle"
        onclick={(event) => press(event, () => player.toggleShuffle())}
      >
        <ShuffleIcon size={18} weight="bold" aria-hidden="true" />
      </button>
      <!-- aria-disabled here and on Next, so losing one does not throw the focus away. -->
      <button
        type="button"
        class="transport"
        disabled={idle}
        aria-disabled={!canPrevious}
        aria-label="Previous"
        onclick={(event) => press(event, () => canPrevious && player.previous())}
      >
        <SkipBackIcon size={20} weight="fill" aria-hidden="true" />
      </button>
      <button
        type="button"
        class="transport transport-play"
        disabled={idle}
        aria-label={playing ? 'Pause' : 'Play'}
        onclick={(event) => press(event, () => player.togglePlay())}
      >
        {#if playing}
          <PauseIcon size={18} weight="fill" aria-hidden="true" />
        {:else}
          <PlayIcon size={18} weight="fill" aria-hidden="true" />
        {/if}
      </button>
      <button
        type="button"
        class="transport"
        disabled={idle}
        aria-disabled={!canNext}
        aria-label="Next"
        onclick={(event) => press(event, () => canNext && player.next())}
      >
        <SkipForwardIcon size={20} weight="fill" aria-hidden="true" />
      </button>
      <button
        type="button"
        class="transport transport-mode"
        disabled={player.state === null}
        aria-label={REPEAT_LABELS[repeat]}
        aria-pressed={repeat !== 'off'}
        title={REPEAT_LABELS[repeat]}
        onclick={(event) => press(event, () => player.cycleRepeat())}
      >
        {#if repeat === 'one'}
          <RepeatOnceIcon size={18} weight="bold" aria-hidden="true" />
        {:else}
          <RepeatIcon size={18} weight="bold" aria-hidden="true" />
        {/if}
      </button>
    </div>
    <!-- A new track drops a half-made drag. -->
    {#key current?.key}
      <SeekBar
        positionMs={player.positionMs}
        durationMs={player.state?.durationMs ?? 0}
        disabled={!seekable}
        onseek={(positionMs) => player.seek(positionMs)}
      />
    {/key}
  </div>

  <div class="flex items-center justify-end gap-3">
    <VolumeSlider
      volume={player.state?.volume ?? 0}
      disabled={player.state === null}
      onvolume={(volume) => player.setVolume(volume)}
    />
    <button
      type="button"
      class="transport"
      aria-label="Queue"
      aria-expanded={queueOpen}
      aria-controls="queue-panel"
      onclick={(event) => press(event, ontogglequeue)}
    >
      <QueueIcon size={20} weight={queueOpen ? 'fill' : 'regular'} aria-hidden="true" />
    </button>
  </div>
</section>
