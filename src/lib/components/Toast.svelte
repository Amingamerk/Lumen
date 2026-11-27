<!-- A notice over the app: what went wrong, until it is dismissed or over. -->
<script lang="ts">
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import XIcon from 'phosphor-svelte/lib/XIcon';
  import { releasePointerFocus } from '../focus';

  interface Props {
    message: string;
    ondismiss: () => void;
  }

  let { message, ondismiss }: Props = $props();
</script>

<div class="toast">
  <div class="toast-message" role="alert">
    <WarningCircleIcon size={20} weight="fill" aria-hidden="true" />
    <p>{message}</p>
  </div>
  <button
    type="button"
    class="step"
    aria-label="Dismiss"
    onclick={(event) => {
      releasePointerFocus(event);
      ondismiss();
    }}
  >
    <XIcon size={16} aria-hidden="true" />
  </button>
</div>
