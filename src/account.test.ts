import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import type { AccountStatus, PauseReason, ServiceStatus } from './lib/ipc';
import { appStores, fakeBridge, serviceStatus } from './lib/testing';

const control = () => screen.findByRole('region', { name: 'YouTube Music' });
const buttons = (region: HTMLElement) =>
  within(region)
    .queryAllByRole('button')
    .map((button) => button.textContent?.trim());

const word = (region: HTMLElement) =>
  within(region).getByRole('status').textContent?.trim();
const text = (region: HTMLElement) => region.textContent?.replace(/\s+/g, ' ') ?? '';

async function openApp(status: AccountStatus, youtube?: Partial<ServiceStatus>) {
  const bridge = fakeBridge();
  bridge.accountStatus = status;
  if (youtube) bridge.youtubeStatus = serviceStatus(youtube);
  const view = render(App, { props: appStores(bridge) });
  return { ...view, bridge, region: await control() };
}

describe('account control', () => {
  it('sits in the sidebar', async () => {
    const { region } = await openApp('disconnected');

    expect(screen.getByRole('navigation', { name: 'Main' }).contains(region)).toBe(true);
  });

  it.each([
    ['disconnected', 'Disconnected', ['Sign in']],
    ['connected', 'Connected', ['Sign in again', 'Disconnect']],
    ['expired', 'Expired', ['Log in again', 'Disconnect']],
  ] as const)('shows %s and what can be done about it', async (status, word, actions) => {
    const { region } = await openApp(status);

    expect(within(region).getByRole('status').textContent?.trim()).toBe(word);
    expect(buttons(region)).toEqual(actions);
  });

  it('asks Rust to sign in and shows connected once Rust says so', async () => {
    const { bridge, region } = await openApp('disconnected');

    await fireEvent.click(within(region).getByRole('button', { name: 'Sign in' }));
    expect(bridge.commands).toEqual([['signIn']]);
    expect(within(region).getByRole('status').textContent?.trim()).toBe('Disconnected');

    bridge.emitAccount('connected');
    await waitFor(() =>
      expect(within(region).getByRole('status').textContent?.trim()).toBe('Connected'),
    );
  });

  it('asks Rust to disconnect', async () => {
    const { bridge, region } = await openApp('connected');

    await fireEvent.click(within(region).getByRole('button', { name: 'Disconnect' }));

    expect(bridge.commands).toEqual([['disconnect']]);
  });

  it('shows an alert when a sign-in fails', async () => {
    const { bridge } = await openApp('disconnected');

    bridge.emitSignInFailed('Lumen could not save the sign-in.');

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('Lumen could not save the sign-in.');
  });

  it('stops listening when the app closes', async () => {
    const { bridge, unmount } = await openApp('connected');

    unmount();

    await waitFor(() => expect(bridge.accountListeners).toBe(0));
  });
});

describe('youtube notices', () => {
  const NOW = new Date(2026, 10, 27, 14, 30);
  const until = (date: Date) => ({ state: 'coolingDown', untilMs: date.getTime() }) as const;
  const paused = (reason: PauseReason) => ({ state: 'paused', reason }) as const;

  afterEach(() => vi.useRealTimers());

  it('shows Connected while unchecked and Verified once Rust says so', async () => {
    const { bridge, region } = await openApp('connected');
    expect(word(region)).toBe('Connected');

    bridge.emitYoutube(serviceStatus({ revision: 1, check: 'checking' }));
    await waitFor(() => expect(word(region)).toBe('Checking'));

    bridge.emitYoutube(serviceStatus({ revision: 2, check: 'verified' }));
    await waitFor(() => expect(word(region)).toBe('Verified'));
  });

  it.each(['disconnected', 'expired'] as const)('never calls a %s account verified', async (status) => {
    const { region } = await openApp(status, { check: 'verified' });

    expect(word(region)?.toLowerCase()).toBe(status);
  });

  it('asks an expired account to log in again, and does only that', async () => {
    const { bridge, region } = await openApp('connected');
    expect(text(region)).not.toContain('log in again');

    bridge.emitAccount('expired');
    await waitFor(() => expect(text(region)).toContain('Session expired, log in again'));
    await fireEvent.click(within(region).getByRole('button', { name: 'Log in again' }));

    expect(bridge.commands).toEqual([['signIn']]);
  });

  it('gives the wait and the end of a cool-down, with no button to cut it short', async () => {
    vi.useFakeTimers({ toFake: ['Date'], now: NOW });

    const { region } = await openApp('connected', { gate: until(new Date(2026, 10, 27, 14, 32)) });

    await waitFor(() =>
      expect(text(region)).toContain('Waiting about 2 min, until 27 Nov 2026, 14:32'),
    );
    expect(buttons(region)).toEqual(['Sign in again', 'Disconnect']);
  });

  it('shows the date of a cool-down that ends on a later day', async () => {
    vi.useFakeTimers({ toFake: ['Date'], now: NOW });

    const { region } = await openApp('connected', { gate: until(new Date(2027, 0, 2, 9, 5)) });

    await waitFor(() => expect(text(region)).toContain('until 2 Jan 2027, 09:05'));
  });

  it('copes with an end too far off to be a date', async () => {
    // What Rust sends for an absurdly long Retry-After.
    const gate = { state: 'coolingDown', untilMs: 18_446_744_073_709_552_000 } as const;

    const { region } = await openApp('connected', { gate });

    await waitFor(() => expect(text(region)).toContain('too far off to show'));
  });

  it.each([
    ['botCheck', 'bot check'],
    ['repeatedFailures', 'failed in a row'],
    ['stateUnreadable', 'could not read'],
    ['stateUnwritable', 'could not save its YouTube'],
    ['expiryUnsaved', 'session expired'],
  ] as const)('explains the %s pause and asks to check the account first', async (reason, why) => {
    const { region } = await openApp('connected', { gate: paused(reason) });

    await waitFor(() => expect(text(region)).toContain(why));
    expect(text(region)).toContain('Check your account in a browser first');
  });

  it('resumes only when the Resume button is pressed', async () => {
    const { bridge, region } = await openApp('connected', { gate: paused('botCheck') });
    const resume = await within(region).findByRole('button', { name: 'Resume' });
    expect(bridge.commands).toEqual([]);

    await fireEvent.click(resume);

    expect(bridge.commands).toEqual([['resumeYoutube']]);
    // The notice stays until Rust says the pause is over.
    expect(within(region).queryByRole('button', { name: 'Resume' })).not.toBeNull();
    bridge.emitYoutube(serviceStatus({ revision: 2 }));
    await waitFor(() =>
      expect(within(region).queryByRole('button', { name: 'Resume' })).toBeNull(),
    );
    expect(text(region)).not.toContain('Check your account');
  });

  it('says something different for a cool-down, a pause and being offline', async () => {
    const { bridge, region } = await openApp('connected');
    const quiet = text(region);
    const seen = new Set<string>();
    const states: Partial<ServiceStatus>[] = [
      { gate: until(new Date(Date.now() + 60_000)) },
      { gate: paused('repeatedFailures') },
      { check: 'offline' },
      { check: 'unexpected' },
    ];

    for (const [at, state] of states.entries()) {
      const before = text(region);
      bridge.emitYoutube(serviceStatus({ revision: at + 1, ...state }));
      await waitFor(() => expect(text(region)).not.toBe(before));
      seen.add(text(region));
    }

    expect(seen.size).toBe(states.length);
    expect(seen).not.toContain(quiet);
    expect(text(region)).toContain('unexpected answer');
  });

  it('says that YouTube Music cannot be reached', async () => {
    const { region } = await openApp('connected', { check: 'offline' });

    await waitFor(() => expect(text(region)).toContain('Could not reach YouTube Music'));
  });

  it('shows an alert when Resume fails', async () => {
    const { bridge, region } = await openApp('connected', { gate: paused('botCheck') });
    bridge.failCommands = 'the window failed';

    await fireEvent.click(await within(region).findByRole('button', { name: 'Resume' }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('the window failed');
  });

  it('stops listening when the app closes', async () => {
    const { bridge, unmount } = await openApp('connected');
    await waitFor(() => expect(bridge.youtubeListeners).toBe(1));

    unmount();

    await waitFor(() => expect(bridge.youtubeListeners).toBe(0));
  });
});
