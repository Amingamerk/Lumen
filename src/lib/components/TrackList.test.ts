import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { tracks } from '../testing';
import TrackList from './TrackList.svelte';

describe('TrackList', () => {
  it('asks for a picture only for the rows that have one, by track id', () => {
    const rows = tracks().slice(0, 2);
    const { container } = render(TrackList, {
      tracks: rows,
      labelledby: 'heading',
      onplay: () => {},
    });

    const pictures = [...container.querySelectorAll('img')].map((img) => img.getAttribute('src'));
    expect(rows.map((row) => row.hasArt)).toEqual([true, false]);
    expect(pictures).toEqual([`http://lumen.localhost/art/64/${rows[0].id}`]);
  });

  it('shows no length for a song whose length is not known yet', () => {
    const [known, unknown] = tracks().slice(0, 2).map((row, at) => ({
      ...row,
      durationMs: at === 0 ? 201_000 : 0,
    }));
    const { container } = render(TrackList, {
      tracks: [known, unknown],
      labelledby: 'heading',
      onplay: () => {},
    });

    const lines = [...container.querySelectorAll('.track-meta')].map((p) => p.textContent?.trim() ?? '');
    expect(lines[0].endsWith(' · 3:21')).toBe(true);
    expect(lines[1]).not.toMatch(/\d:\d\d$/);
    expect(lines[1]).not.toMatch(/ · $/);
  });

  it('queues a song next or last from its own buttons, and keeps the play label', async () => {
    const [row] = tracks();
    const onqueue = vi.fn();
    const onplay = vi.fn();
    render(TrackList, { tracks: [row], labelledby: 'heading', onplay, onqueue });

    await fireEvent.click(screen.getByRole('button', { name: `Queue ${row.title} to play next` }));
    await fireEvent.click(screen.getByRole('button', { name: `Add ${row.title} to the queue` }));

    expect(onqueue.mock.calls).toEqual([
      [row.id, 'next'],
      [row.id, 'last'],
    ]);
    expect(onplay).not.toHaveBeenCalled();
    expect(
      screen.getByRole('button', { name: `Play ${row.title} by ${row.artists.join(', ')}` }),
    ).toBeTruthy();
  });

  it('has no queue buttons without a handler', () => {
    render(TrackList, { tracks: tracks().slice(0, 1), labelledby: 'heading', onplay: () => {} });

    expect(screen.queryByRole('button', { name: /to the queue$/ })).toBeNull();
  });
});
