<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import type { CheckReport, Clients, SessionStatus } from './types';

  let session = $state<SessionStatus | null>(null);
  let message = $state('');
  let busy = $state(false);
  let pasted = $state('');
  let userAgent = $state('');
  let trackUrl = $state('https://music.youtube.com/watch?v=dQw4w9WgXcQ');
  let clients = $state<Clients>('Default');
  let report = $state<CheckReport | null>(null);

  const when = (unixSeconds: number | null) =>
    unixSeconds === null ? 'none' : new Date(unixSeconds * 1000).toLocaleString();

  /** Runs one command and shows its error, if any. */
  async function run(action: () => Promise<string>) {
    busy = true;
    message = '';
    try {
      message = await action();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }

  const openLogin = () =>
    run(async () => {
      await invoke('open_login');
      return 'Log in in the new window. It closes by itself once the session is captured.';
    });

  const captureNow = () =>
    run(async () => {
      await invoke('capture_now');
      return 'Session captured.';
    });

  const clearSession = () =>
    run(async () => {
      await invoke('clear_session');
      report = null;
      return 'Saved session cleared.';
    });

  async function importCookies(cookies: string) {
    await run(async () => {
      await invoke('import_session', { cookies, userAgent });
      return 'Session imported. Delete the exported cookie file if you made one.';
    });
  }

  async function importPasted() {
    const cookies = pasted;
    // Do not keep cookie values in the page.
    pasted = '';
    await importCookies(cookies);
  }

  async function importFile(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget;
    const file = input.files?.[0];
    if (!file) return;
    const cookies = await file.text();
    input.value = '';
    await importCookies(cookies);
  }

  const runCheck = () =>
    run(async () => {
      report = null;
      report = await invoke<CheckReport>('run_check', { trackUrl, clients });
      return '';
    });

  onMount(() => {
    const listeners = [
      listen<SessionStatus | null>('session-changed', (event) => {
        session = event.payload;
      }),
      listen<string>('login-error', (event) => {
        message = event.payload;
      }),
    ];
    // Read once; later changes arrive as events.
    run(async () => {
      session = await invoke<SessionStatus | null>('session_status');
      return '';
    });
    return () => listeners.forEach((listener) => listener.then((unlisten) => unlisten()));
  });
</script>

<main>
  <h1>YouTube Music login spike</h1>

  <section>
    <h2>Saved session</h2>
    {#if session}
      <p class={{ bad: session.expired }}>
        {session.expired ? 'Expired: log in again.' : 'Saved.'}
        Captured {when(session.captured_at)} by
        {session.method === 'LoginWindow' ? 'the login window' : 'cookie import'},
        {session.cookie_count} youtube.com cookies, LOGIN_INFO
        {session.has_login_info ? 'present' : 'missing (yt-dlp will act signed out)'}.
      </p>
      <p class="small">{session.user_agent}</p>
    {:else}
      <p>None.</p>
    {/if}
    <button onclick={clearSession} disabled={busy}>Clear Saved Session</button>
  </section>

  <section>
    <h2>1. Log in</h2>
    <button onclick={openLogin} disabled={busy}>Open YouTube Music Login</button>
    <button onclick={captureNow} disabled={busy}>Capture now</button>
  </section>

  <section>
    <h2>Fallback: import cookies</h2>
    <p class="small">
      Use this if Google blocks the embedded login. Paste a Netscape cookies.txt or the Cookie
      request header of music.youtube.com.
    </p>
    <label>
      User-Agent of that browser (required: run navigator.userAgent in its console)
      <input bind:value={userAgent} spellcheck="false" />
    </label>
    <label>
      Cookies
      <textarea bind:value={pasted} rows="4" spellcheck="false" autocomplete="off"></textarea>
    </label>
    <button onclick={importPasted} disabled={busy || !userAgent.trim() || !pasted.trim()}>
      Import pasted
    </button>
    <label>
      Or load a file
      <input
        type="file"
        accept=".txt"
        onchange={importFile}
        disabled={busy || !userAgent.trim()}
      />
    </label>
  </section>

  <section>
    <h2>2. Test</h2>
    <label>
      Track URL
      <input bind:value={trackUrl} spellcheck="false" />
    </label>
    <label>
      yt-dlp clients
      <select bind:value={clients}>
        <option value="Default">yt-dlp defaults</option>
        <option value="PinnedUserAgent">only clients that use our User-Agent (web_music, web)</option>
      </select>
    </label>
    <button onclick={runCheck} disabled={busy || !session}>
      Test Authenticated Call + yt-dlp
    </button>
  </section>

  {#if busy}<p>Working...</p>{/if}
  {#if message}<p role="status">{message}</p>{/if}

  {#if report}
    <section>
      <h2>Result</h2>
      {#if report.blocked}<p class="bad">Nothing was sent: {report.blocked}</p>{/if}

      {#if report.browse}
        <h3>Library playlists (one authenticated call)</h3>
        {#if 'Err' in report.browse}
          <p class="bad">{report.browse.Err}</p>
        {:else}
          <p>
            {report.browse.Ok.playlists.length} playlists{report.browse.Ok.has_more
              ? ' on the first page, more exist'
              : ''}.
          </p>
          <ul>
            {#each report.browse.Ok.playlists as playlist (playlist.id)}
              <li>{playlist.title} ({playlist.track_count ?? 'no'} tracks listed)</li>
            {/each}
          </ul>
        {/if}
      {/if}

      {#if report.resolve}
        <h3>yt-dlp resolve</h3>
        {#if 'Err' in report.resolve}
          <p class="bad">{report.resolve.Err}</p>
        {:else}
          {@const stream = report.resolve.Ok}
          <dl>
            <dt>Range request</dt>
            <dd>HTTP {stream.range_status}</dd>
            <dt>Client</dt>
            <dd>{stream.client ?? 'unknown'}</dd>
            <dt>Format</dt>
            <dd>
              {stream.format_id}, {stream.codec ?? 'unknown codec'},
              {stream.abr_kbps ?? '?'} kbps, {stream.protocol ?? 'unknown protocol'}
            </dd>
            <dt>available_at</dt>
            <dd>{when(stream.available_at)} (waited {stream.waited_s} s)</dd>
            <dt>Expires</dt>
            <dd>{when(stream.expires_at)}</dd>
            <dt>Resolve time</dt>
            <dd>{stream.resolve_ms} ms</dd>
            <dt>Stream User-Agent equals login</dt>
            <dd>{stream.user_agent_matches_login ? 'yes' : 'no'}</dd>
            <dt>Warnings</dt>
            <dd>{stream.warnings.join('; ') || 'none known'}</dd>
          </dl>
        {/if}
      {/if}
    </section>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font: 14px/1.4 system-ui, sans-serif;
    color: #e6e6e6;
    background: #14161a;
  }
  main {
    padding: 16px;
  }
  h1 {
    font-size: 20px;
  }
  h2 {
    font-size: 15px;
  }
  section {
    margin-bottom: 16px;
    border-top: 1px solid #2c3038;
  }
  label {
    display: block;
    margin: 8px 0;
  }
  textarea,
  label input,
  select {
    display: block;
    box-sizing: border-box;
    width: 100%;
  }
  .small {
    color: #9aa0aa;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .bad {
    color: #ff9a8a;
  }
  dt {
    color: #9aa0aa;
  }
  dd {
    margin: 0 0 4px;
  }
</style>
