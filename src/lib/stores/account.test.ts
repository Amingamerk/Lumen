import { describe, expect, it, vi } from 'vitest';
import { deferred, fakeBridge } from '../testing';
import { AccountStore } from './account.svelte';

async function connected(bridge = fakeBridge()) {
  const account = new AccountStore(bridge);
  const disconnect = await account.connect();
  return { account, bridge, disconnect };
}

describe('account store', () => {
  it('listens before it asks, then shows what Rust answers', async () => {
    const bridge = fakeBridge();
    bridge.accountStatus = 'expired';

    const { account } = await connected(bridge);

    expect(bridge.calls).toEqual(['onAccountChanged', 'onSignInFailed', 'getAccountStatus']);
    expect(account.status).toBe('expired');
  });

  it('follows every change Rust announces, until it stops mirroring', async () => {
    const { account, bridge, disconnect } = await connected();

    bridge.emitAccount('connected');
    expect(account.status).toBe('connected');

    disconnect();
    expect(bridge.accountListeners).toBe(0);
  });

  it('keeps a change that arrived while the first answer was on its way', async () => {
    const bridge = fakeBridge();
    const answer = deferred<undefined>();
    bridge.beforeAccountStatus = () => answer.promise;
    const account = new AccountStore(bridge);
    const connecting = account.connect();
    await vi.waitFor(() => expect(bridge.calls).toContain('getAccountStatus'));

    bridge.emitAccount('connected');
    answer.resolve(undefined);
    await connecting;

    expect(account.status).toBe('connected');
  });

  it('sends sign in and disconnect to Rust and waits for its word', async () => {
    const { account, bridge } = await connected();

    await account.signIn();
    await account.disconnect();

    expect(bridge.commands).toEqual([['signIn'], ['disconnect']]);
    expect(account.status).toBe('disconnected');
  });

  it('says why a command failed, until one works', async () => {
    const { account, bridge } = await connected();
    bridge.failCommands = 'Windows Credential Manager failed';

    await account.disconnect();
    expect(account.error).toBe('Windows Credential Manager failed');

    bridge.failCommands = null;
    await account.signIn();
    expect(account.error).toBeNull();
  });

  it('says why a sign-in failed, until the account connects', async () => {
    const { account, bridge } = await connected();

    bridge.emitAccount('disconnected');
    bridge.emitSignInFailed('Lumen could not save the sign-in.');
    expect(account.error).toBe('Lumen could not save the sign-in.');

    bridge.emitAccount('connected');
    expect(account.error).toBeNull();
  });
});
