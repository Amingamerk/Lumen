import { describe, expect, it } from 'vitest';
import { latestOnly } from './latest';
import { deferred } from './testing';

describe('latestOnly', () => {
  it('sends one at a time and ends on the newest', async () => {
    const sent: number[] = [];
    const first = deferred<unknown>();
    const send = latestOnly(async (value: number) => {
      sent.push(value);
      if (value === 1) await first.promise;
    });

    const done = [send(1), send(2), send(3)];
    expect(sent).toEqual([1]);
    first.resolve(null);
    await Promise.all(done);

    expect(sent).toEqual([1, 3]);
  });

  it('sends again once idle, and after a failure', async () => {
    const sent: number[] = [];
    const send = latestOnly(async (value: number) => {
      sent.push(value);
      if (value === 1) throw new Error('no');
    });

    await expect(send(1)).rejects.toThrow('no');
    await send(2);

    expect(sent).toEqual([1, 2]);
  });
});
