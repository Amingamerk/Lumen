<!-- Where the track is. A drag stays here until it is let go; then Rust seeks. -->
<script lang="ts">
  import { formatDuration } from '../format';

  interface Props {
    positionMs: number;
    durationMs: number;
    disabled: boolean;
    onseek: (positionMs: number) => Promise<unknown>;
  }

  let { positionMs, durationMs, disabled, onseek }: Props = $props();

  /** Where the thumb is being held, which ticks must not move. */
  let draft = $state.raw<number | null>(null);

  const shown = $derived(draft ?? positionMs);

  async function release(target: number) {
    draft = target;
    await onseek(target);
    // A drag begun meanwhile keeps its own place.
    if (draft === target) draft = null;
  }
</script>

<div class="seek">
  <span>{formatDuration(shown)}</span>
  <input
    type="range"
    class="slider"
    aria-label="Seek"
    aria-valuetext="{formatDuration(shown)} of {formatDuration(durationMs)}"
    min="0"
    max={durationMs}
    step="1000"
    value={shown}
    {disabled}
    oninput={(event) => (draft = event.currentTarget.valueAsNumber)}
    onchange={(event) => void release(event.currentTarget.valueAsNumber)}
    onpointercancel={() => (draft = null)}
    onblur={() => (draft = null)}
  />
  <span>{formatDuration(durationMs)}</span>
</div>
