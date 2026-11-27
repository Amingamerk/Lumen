<!-- The player's controls. It shows what Rust says and asks Rust for changes. -->
<script lang="ts">
  import MusicNoteIcon from 'phosphor-svelte/lib/MusicNoteIcon';
  import PauseIcon from 'phosphor-svelte/lib/PauseIcon';
  import PlayIcon from 'phosphor-svelte/lib/PlayIcon';
  import QueueIcon from 'phosphor-svelte/lib/QueueIcon';
  import SkipBackIcon from 'phosphor-svelte/lib/SkipBackIcon';
  import SkipForwardIcon from 'phosphor-svelte/lib/SkipForwardIcon';
  import { releasePointerFocus } from '../focus';
  import type { PlayerStore } from '../stores/player.svelte';
  import CoverArt from './CoverArt.svelte';
  import SeekBar from './SeekBar.svelte';
  import VolumeSlider from './VolumeSlider.svelte';

  interface Props {
    player: PlayerStore;
    queueOpen: boolean;
    ontogglequeue: () => void;
  }

  let { player, queueOpen, ontogglequeue }: Props = $props();

  const current = $derived(player.state?.current ?? null);
  const idle = $derived(current === null);
  const playing = $derived(player.state?.status === 'playing');
  const canNext = $derived(player.state?.canNext ?? false);

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
        <CoverArt title={track.album ?? track.title} />
      </div>
      <div class="min-w-0">
        <p class="truncate text-[15px] font-semibold" title={track.title}>{track.title}</p>
        <p class="text-ink-faint truncate text-[13px]" title={artists}>{artists}</p>
      </div>
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
        class="transport"
        disabled={idle}
        aria-label="Previous"
        onclick={(event) => press(event, () => player.previous())}
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
      <!-- aria-disabled, so reaching the end does not throw the focus away. -->
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
    </div>
    <!-- A new track drops a half-made drag. -->
    {#key current?.key}
      <SeekBar
        positionMs={player.positionMs}
        durationMs={player.state?.durationMs ?? 0}
        disabled={idle}
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
