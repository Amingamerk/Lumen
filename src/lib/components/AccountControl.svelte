<script lang="ts">
  import type { AccountStatus, ServiceStatus } from '../ipc';
  import GateNotice from './GateNotice.svelte';

  interface Props {
    status: AccountStatus;
    /** `null` until Rust has said. */
    youtube: ServiceStatus | null;
    onsignin: () => void;
    ondisconnect: () => void;
    onresume: () => void;
  }

  let { status, youtube, onsignin, ondisconnect, onresume }: Props = $props();

  const WORDS: Record<AccountStatus, string> = {
    connected: 'Connected',
    expired: 'Expired',
    disconnected: 'Disconnected',
  };

  const SIGN_IN: Record<AccountStatus, string> = {
    connected: 'Sign in again',
    expired: 'Log in again',
    disconnected: 'Sign in',
  };

  /** Only a stored, unexpired session can be checking or verified. */
  const check = $derived(status === 'connected' ? youtube?.check : undefined);
  const word = $derived(
    check === 'checking' ? 'Checking' : check === 'verified' ? 'Verified' : WORDS[status],
  );
</script>

<section class="account" aria-labelledby="account-title">
  <div class="flex items-baseline justify-between gap-3">
    <h2 id="account-title" class="text-ink-soft font-semibold">YouTube Music</h2>
    <p class="account-status" role="status" data-status={status}>{word}</p>
  </div>
  {#if status === 'expired'}
    <p class="account-note">Session expired, log in again.</p>
  {/if}
  <GateNotice gate={youtube?.gate} {check} {onresume} />
  <div class="mt-2.5 flex flex-wrap gap-1.5">
    <button type="button" class="chip" onclick={onsignin}>{SIGN_IN[status]}</button>
    {#if status !== 'disconnected'}
      <button type="button" class="chip" onclick={ondisconnect}>Disconnect</button>
    {/if}
  </div>
</section>
