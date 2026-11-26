<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import type { Likes, SessionStatus } from './lib/types';

  let session = $state<SessionStatus | null>(null);
  let likes = $state<Likes | null>(null);
  let message = $state('');
  let pasted = $state('');
  let busy = $state(false);

  async function run(action: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await action();
    } catch (error) {
      message = String(error);
    } finally {
      busy = false;
    }
  }

  const openLogin = () => run(() => invoke('open_login'));

  const fetchLikes = () =>
    run(async () => {
      likes = null;
      likes = await invoke<Likes>('fetch_likes');
    });

  const clearSession = () =>
    run(async () => {
      likes = null;
      session = await invoke<SessionStatus>('clear_session');
      message = 'Saved session cleared.';
    });

  function importToken(event: SubmitEvent) {
    event.preventDefault();
    const token = pasted;
    // The pasted value does not stay in the page.
    pasted = '';
    run(async () => {
      session = await invoke<SessionStatus>('import_token', { token });
      message = 'Token saved to Credential Manager.';
    });
  }

  onMount(() => {
    const listeners = [
      listen<SessionStatus>('session-changed', (event) => (session = event.payload)),
      listen<string>('login-status', (event) => (message = event.payload)),
    ];
    run(async () => {
      session = await invoke<SessionStatus>('session_status');
    });
    return () => listeners.forEach((listener) => listener.then((unlisten) => unlisten()));
  });
</script>

<main>
  <h1>SoundCloud login spike</h1>

  <div class="actions">
    <button onclick={openLogin} disabled={busy}>Open SoundCloud Login</button>
    <button onclick={fetchLikes} disabled={busy || !session?.logged_in}>
      Scrape client_id + Fetch Likes
    </button>
    <button onclick={clearSession} disabled={busy}>Clear Saved Session</button>
  </div>

  <p class="message" role="status">{busy ? 'Working...' : message}</p>

  {#if session}
    <dl>
      <dt>Session</dt>
      <dd>
        {#if session.expired}
          expired, sign in again
        {:else if session.logged_in}
          saved
        {:else}
          none
        {/if}
      </dd>
      <dt>oauth_token</dt>
      <dd>{session.oauth_token}</dd>
      <dt>client_id</dt>
      <dd>{session.client_id}</dd>
      <dt>User agent</dt>
      <dd>{session.user_agent ?? 'not captured'}</dd>
    </dl>
  {/if}

  <details>
    <summary>Paste an oauth_token by hand (debugging fallback)</summary>
    <form onsubmit={importToken}>
      <input
        type="password"
        bind:value={pasted}
        aria-label="oauth_token"
        placeholder="oauth_token cookie value"
        autocomplete="off"
        spellcheck="false"
      />
      <button type="submit" disabled={busy || pasted.trim() === ''}>Save Token</button>
    </form>
  </details>

  {#if likes}
    <h2>Likes for {likes.display_name ?? '(name unavailable)'}</h2>
    <p>
      {likes.tracks.length} tracks returned{likes.has_more ? ', more available' : ''}
      {#if likes.total_likes !== null}({likes.total_likes} likes on the profile){/if}
    </p>
    <p>
      Path {likes.endpoint}, client_id {likes.client_id_source.toLowerCase()}.
      {#if likes.scrape}
        Scrape: bundle {likes.scrape.matched_index} matched ({likes.scrape.pattern}),
        {likes.scrape.scripts_fetched} of {likes.scrape.scripts_found} fetched in
        {likes.scrape.elapsed_ms} ms.
      {/if}
    </p>
    <ol>
      {#each likes.tracks as track, index (index)}
        <li>{track.title}{track.artist ? ` - ${track.artist}` : ''}</li>
      {/each}
    </ol>
  {/if}
</main>

<style>
  main {
    max-width: 760px;
    margin: 0 auto;
    padding: 16px 24px;
  }

  h1 {
    font-size: 1.4rem;
  }

  .actions,
  form {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  form {
    margin-top: 8px;
  }

  input {
    flex: 1;
  }

  .message {
    min-height: 1.5em;
    color: #ffb86b;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
  }

  dt {
    color: #9a9aa8;
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
