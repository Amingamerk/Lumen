<script lang="ts">
  import SpeakerHighIcon from 'phosphor-svelte/lib/SpeakerHighIcon';
  import { latestOnly } from '../latest';

  interface Props {
    volume: number;
    disabled: boolean;
    onvolume: (volume: number) => Promise<unknown>;
  }

  let { volume, disabled, onvolume }: Props = $props();

  /** Where the thumb is until Rust has caught up with it. */
  let draft = $state.raw<number | null>(null);

  const shown = $derived(draft ?? volume);

  // One call in flight: a drag must not flood Rust.
  const send = latestOnly(async (value: number) => {
    await onvolume(value);
  });

  async function change(value: number) {
    draft = value;
    await send(value);
    if (draft === value) draft = null;
  }
</script>

<div class="volume">
  <SpeakerHighIcon size={18} aria-hidden="true" />
  <input
    type="range"
    class="slider"
    aria-label="Volume"
    aria-valuetext="{shown}%"
    min="0"
    max="100"
    value={shown}
    {disabled}
    oninput={(event) => void change(event.currentTarget.valueAsNumber)}
  />
</div>
