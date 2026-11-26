<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import type { LoginResult, Report, Route, SessionStatus } from './types';

  let session = $state<SessionStatus>({ saved: false, expired: false });
  let report = $state.raw<Report | null>(null);
  let message = $state('');
  let pasted = $state('');
  let busy = $state(false);

  const usable = $derived(session.saved && !session.expired);
  const sessionText = $derived(
    !session.saved ? 'none' : session.expired ? 'expired, log in again' : 'saved',
  );

  // The login window runs on this same WebView2 build, so Rust can send its
  // user agent and languages on every request.
  const browser = { userAgent: navigator.userAgent, languages: [...navigator.languages] };

  async function refresh() {
    try {
      session = await invoke<SessionStatus>('session_status');
    } catch (error) {
      message = String(error);
    }
  }

  async function run(action: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await action();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
      await refresh();
    }
  }

  const openLogin = () =>
    run(async () => {
      await invoke('open_login', browser);
      message = 'Log in in the new window. It closes by itself.';
    });

  const savePasted = () =>
    run(async () => {
      const spDc = pasted;
      // Do not keep the cookie in the page any longer than needed.
      pasted = '';
      await invoke('save_manual_session', { spDc, ...browser });
      report = null;
      message = 'Session saved to Credential Manager.';
    });

  const check = (route: Route) =>
    run(async () => {
      report = null;
      message = 'Checking, one paced request at a time...';
      report = await invoke<Report>('run_check', { route });
      message = 'Done.';
    });

  const clearSession = () =>
    run(async () => {
      await invoke('clear_session');
      report = null;
      message = 'Saved session deleted.';
    });

  const minutes = (ms: number) => {
    const seconds = Math.round(ms / 1000);
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
  };

  onMount(() => {
    const unlisten = listen<LoginResult>('spotify-login', ({ payload }) => {
      message = payload.message;
      if (payload.saved) report = null;
      void refresh();
    });
    void refresh();
    return () => void unlisten.then((off) => off());
  });
</script>

<main>
  <h1>Spotify login spike</h1>

  <section>
    <p>Saved session: <strong>{sessionText}</strong></p>
    <button onclick={openLogin} disabled={busy}>Open Spotify Login</button>
    <button onclick={clearSession} disabled={busy}>Clear Saved Session</button>
  </section>

  <section>
    <details>
      <summary>Paste sp_dc by hand instead</summary>
      <p class="note">Only if the login window does not work. The value goes to Credential Manager.</p>
      <input
        type="password"
        aria-label="sp_dc cookie value"
        placeholder="sp_dc value"
        autocomplete="off"
        spellcheck="false"
        bind:value={pasted}
      />
      <button onclick={savePasted} disabled={busy || !pasted}>Save pasted cookie</button>
    </details>
  </section>

  <section>
    <button onclick={() => check('pathfinder')} disabled={busy || !usable}>
      Derive Token + Check Liked Songs &amp; ISRC
    </button>
    <button onclick={() => check('rest')} disabled={busy || !usable}>
      Compare: REST /v1/me/tracks
    </button>
    <p class="note">
      One page of 10 tracks. The first button also asks for the metadata of each track that has no
      ISRC. The REST comparison may answer 429, which pauses every Spotify request for a while.
    </p>
  </section>

  <p aria-live="polite">{message}</p>

  {#if report}
    <section>
      <h2>{report.route === 'rest' ? 'REST /v1/me/tracks' : 'Pathfinder fetchLibraryTracks'}</h2>
      <ul>
        <li>
          Items returned: {report.items_returned} ({report.track_count} tracks, {report.skipped}
          skipped)
        </li>
        <li>Liked songs in total: {report.total_liked ?? 'not reported'}</li>
        <li>ISRC in the liked songs response: {report.isrc_in_liked_response}</li>
        <li>ISRC from track metadata: {report.isrc_from_metadata}</li>
        <li>
          <strong>Tracks with ISRC: {report.tracks_with_isrc} of {report.track_count}</strong>
        </li>
        <li>TOTP warning from Spotify: {report.totp_status ?? 'none'}</li>
        <li>Track fields: {report.track_fields.join(', ') || 'none'}</li>
      </ul>
      {#if report.lookup_error}
        <p class="warn">ISRC lookups stopped: {report.lookup_error}</p>
      {/if}
      <table>
        <thead>
          <tr><th>Title</th><th>Artists</th><th>Length</th><th>ISRC</th><th>From</th></tr>
        </thead>
        <tbody>
          {#each report.tracks as track, index (index)}
            <tr>
              <td>{track.title}</td>
              <td>{track.artists.join(', ')}</td>
              <td>{minutes(track.duration_ms)}</td>
              <td>{track.isrc ?? 'missing'}</td>
              <td>{track.isrc_source?.replace('_', ' ') ?? ''}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
</main>
