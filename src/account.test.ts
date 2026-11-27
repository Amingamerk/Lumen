import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import type { AccountStatus } from './lib/ipc';
import { appStores, fakeBridge } from './lib/testing';

const control = () => screen.findByRole('region', { name: 'YouTube Music' });
const buttons = (region: HTMLElement) =>
  within(region)
    .queryAllByRole('button')
    .map((button) => button.textContent?.trim());

async function openApp(status: AccountStatus) {
  const bridge = fakeBridge();
  bridge.accountStatus = status;
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
    ['expired', 'Expired', ['Sign in again', 'Disconnect']],
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
