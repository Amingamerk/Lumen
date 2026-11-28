<!-- A heart that likes or unlikes a song. The state is Rust's; this shows it. -->
<script lang="ts">
  import HeartIcon from 'phosphor-svelte/lib/HeartIcon';
  import { releasePointerFocus } from '../focus';

  interface Props {
    liked: boolean;
    /** The song's title, so a screen reader knows which one. */
    title: string;
    ontoggle: () => void;
  }

  let { liked, title, ontoggle }: Props = $props();
</script>

<button
  type="button"
  class="like"
  aria-pressed={liked}
  aria-label="Like {title}"
  onclick={(event) => {
    releasePointerFocus(event);
    ontoggle();
  }}
>
  <HeartIcon size={18} weight={liked ? 'fill' : 'regular'} aria-hidden="true" />
</button>
