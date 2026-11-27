import { describe, expect, it } from 'vitest';
import type { QueueSlice } from '../ipc';
import { deferred, fakeBridge, queueSlice } from '../testing';
import { QueueStore } from './queue.svelte';

describe('queue store', () => {
  it('asks only for the window', async () => {
    const bridge = fakeBridge();
    const queue = new QueueStore(bridge);

    await queue.load(40, 25);

    expect(bridge.queueRequests).toEqual([{ offset: 40, limit: 25 }]);
    expect(queue.slice?.offset).toBe(40);
  });

  it('keeps the latest answer', async () => {
    const bridge = fakeBridge();
    const queue = new QueueStore(bridge);
    const slow = deferred<QueueSlice>();
    bridge.answerQueue = (offset, limit) =>
      offset === 0 ? slow.promise : queueSlice(offset, limit, 90);

    const first = queue.load(0, 10);
    await queue.load(20, 10);
    slow.resolve(queueSlice(0, 10, 90));
    await first;

    expect(queue.slice?.offset).toBe(20);
  });

  it('says why a window could not be fetched, until one is', async () => {
    const bridge = fakeBridge();
    const queue = new QueueStore(bridge);
    bridge.answerQueue = () => {
      throw 'queue unavailable';
    };

    await queue.load(0, 10);
    expect(queue.error).toBe('queue unavailable');

    bridge.answerQueue = (offset, limit) => queueSlice(offset, limit, 3);
    await queue.load(0, 10);
    expect(queue.error).toBeNull();
  });
});
