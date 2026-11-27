import { describeError } from '../errors';
import type { Bridge, ServiceStatus } from '../ipc';

/** The YouTube gate and session check, as Rust reports them. */
export class YoutubeStore {
  status = $state.raw<ServiceStatus | null>(null);
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    let stop = () => {};
    try {
      stop = await this.#bridge.onYoutubeStatusChanged((status) => this.#apply(status));
      this.#apply(await this.#bridge.getYoutubeStatus());
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stop();
  }

  async resume(): Promise<void> {
    try {
      await this.#bridge.resumeYoutube();
      this.error = null;
    } catch (error) {
      this.error = describeError(error);
    }
  }

  /** Events and answers can overtake each other; the revision tells them apart. */
  #apply(status: ServiceStatus) {
    if (status.revision > (this.status?.revision ?? -1)) this.status = status;
  }
}
