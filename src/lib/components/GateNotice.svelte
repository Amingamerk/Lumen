<script lang="ts">
  import { formatTotalDuration } from '../format';
  import type { GateView, PauseReason, SessionCheck } from '../ipc';

  interface Props {
    gate: GateView | undefined;
    check: SessionCheck | undefined;
    onresume: () => void;
  }

  let { gate, check, onresume }: Props = $props();

  const PAUSED: Record<PauseReason, string> = {
    botCheck: 'YouTube Music asked for a bot check, so Lumen paused it.',
    repeatedFailures: 'Three YouTube Music requests failed in a row, so Lumen paused it.',
    stateUnreadable: 'Lumen could not read its saved YouTube pause, so it paused to be safe.',
    stateUnwritable: 'Lumen could not save its YouTube pause, so it paused to be safe.',
    expiryUnsaved: 'Lumen could not save that the session expired, so it paused YouTube Music.',
  };

  const PROBLEMS: Partial<Record<SessionCheck, string>> = {
    offline: 'Could not reach YouTube Music. Check your internet connection.',
    unexpected: 'YouTube Music gave an unexpected answer. Three failures in a row pause it.',
  };

  // The date too: a long cool-down can end days from now.
  const endFormat = new Intl.DateTimeFormat('en-GB', { dateStyle: 'medium', timeStyle: 'short' });

  /** The wait as it is when the status arrives; Rust says when it is over. */
  function waiting(untilMs: number): string {
    const end = new Date(untilMs);
    if (Number.isNaN(end.getTime())) return 'The end of the wait is too far off to show.';
    const left = formatTotalDuration(untilMs - Date.now());
    return `Waiting about ${left}, until ${endFormat.format(end)}.`;
  }

  const problem = $derived(check && PROBLEMS[check]);
</script>

<!-- Always there, so a notice that appears is announced. -->
<div aria-live="polite">
  {#if gate?.state === 'paused'}
    <div class="gate-notice" data-blocked>
      <p>{PAUSED[gate.reason]}</p>
      <p>Check your account in a browser first, then resume.</p>
      <button type="button" class="chip" onclick={onresume}>Resume</button>
    </div>
  {:else if gate?.state === 'coolingDown'}
    <div class="gate-notice" data-blocked>
      <p>YouTube Music asked Lumen to slow down.</p>
      <p>{waiting(gate.untilMs)}</p>
    </div>
  {:else if problem}
    <p class="gate-notice">{problem}</p>
  {/if}
</div>
