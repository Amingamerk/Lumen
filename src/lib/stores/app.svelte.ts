import { describeError } from '../errors';
import type { Bridge, NavigationState, Route } from '../ipc';

/** A read-only mirror of the state Rust owns. It changes only on what Rust sends. */
export class AppStore {
  navigation = $state.raw<NavigationState | null>(null);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    let disconnect = () => {};
    try {
      // Listen first, so a change made while the snapshot is in flight still arrives.
      disconnect = await this.#bridge.onNavigationChanged((navigation) => this.#apply(navigation));
      const snapshot = await this.#bridge.getAppState();
      this.#apply(snapshot.navigation);
    } catch (error) {
      this.error = describeError(error);
    }
    return disconnect;
  }

  navigate(route: Route): Promise<void> {
    return this.#send(() => this.#bridge.navigate(route));
  }

  toggleQueue(): Promise<void> {
    return this.#send(() => this.#bridge.toggleQueue());
  }

  async #send(command: () => Promise<void>): Promise<void> {
    try {
      await command();
      this.error = null;
    } catch (error) {
      this.error = describeError(error);
    }
  }

  #apply(next: NavigationState) {
    if (this.navigation && next.revision < this.navigation.revision) return;
    this.navigation = next;
  }
}
