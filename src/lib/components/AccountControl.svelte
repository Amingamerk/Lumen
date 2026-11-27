<script lang="ts">
  import type { AccountStatus } from '../ipc';

  interface Props {
    status: AccountStatus;
    onsignin: () => void;
    ondisconnect: () => void;
  }

  let { status, onsignin, ondisconnect }: Props = $props();

  const WORDS: Record<AccountStatus, string> = {
    connected: 'Connected',
    expired: 'Expired',
    disconnected: 'Disconnected',
  };
</script>

<section class="account" aria-labelledby="account-title">
  <div class="flex items-baseline justify-between gap-3">
    <h2 id="account-title" class="text-ink-soft font-semibold">YouTube Music</h2>
    <p class="account-status" role="status" data-status={status}>{WORDS[status]}</p>
  </div>
  <div class="mt-2.5 flex flex-wrap gap-1.5">
    <button type="button" class="chip" onclick={onsignin}>
      {status === 'disconnected' ? 'Sign in' : 'Sign in again'}
    </button>
    {#if status !== 'disconnected'}
      <button type="button" class="chip" onclick={ondisconnect}>Disconnect</button>
    {/if}
  </div>
</section>
