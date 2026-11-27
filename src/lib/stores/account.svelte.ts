import { describeError } from '../errors';
import type { AccountStatus, Bridge } from '../ipc';

/** Whether YouTube Music is connected, as Rust reports it. */
export class AccountStore {
  status = $state.raw<AccountStatus | null>(null);
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  // An event that beats the first answer is newer than that answer.
  #changes = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    const listeners: (() => void)[] = [];
    try {
      listeners.push(await this.#bridge.onAccountChanged((status) => this.#apply(status)));
      listeners.push(await this.#bridge.onSignInFailed((reason) => (this.error = reason)));
      const changes = this.#changes;
      const status = await this.#bridge.getAccountStatus();
      if (changes === this.#changes) this.status = status;
    } catch (error) {
      this.error = describeError(error);
    }
    return () => {
      for (const stop of listeners) stop();
    };
  }

  signIn(): Promise<void> {
    return this.#send(() => this.#bridge.signIn());
  }

  disconnect(): Promise<void> {
    return this.#send(() => this.#bridge.disconnect());
  }

  async #send(command: () => Promise<void>): Promise<void> {
    try {
      await command();
      this.error = null;
    } catch (error) {
      this.error = describeError(error);
    }
  }

  #apply(status: AccountStatus) {
    this.#changes += 1;
    this.status = status;
    if (status === 'connected') this.error = null;
  }
}
