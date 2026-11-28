<!-- Two buttons on a song row: play it after the current song, or add it to the end of the queue. -->
<script lang="ts">
  import ListPlusIcon from 'phosphor-svelte/lib/ListPlusIcon';
  import SkipForwardIcon from 'phosphor-svelte/lib/SkipForwardIcon';
  import { releasePointerFocus } from '../focus';
  import type { QueuePlacement } from '../ipc';

  interface Props {
    title: string;
    onqueue: (placement: QueuePlacement) => void;
  }

  let { title, onqueue }: Props = $props();
</script>

<button
  type="button"
  class="row-action"
  aria-label="Queue {title} to play next"
  title="Play next"
  onclick={(event) => {
    releasePointerFocus(event);
    onqueue('next');
  }}
>
  <SkipForwardIcon size={18} aria-hidden="true" />
</button>
<button
  type="button"
  class="row-action"
  aria-label="Add {title} to the queue"
  title="Add to queue"
  onclick={(event) => {
    releasePointerFocus(event);
    onqueue('last');
  }}
>
  <ListPlusIcon size={18} aria-hidden="true" />
</button>
