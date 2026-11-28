import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
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
});
