import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { artUrl } from '../art';
import CoverArt from './CoverArt.svelte';

const ID = '0b9d3f0e-5c5e-5a53-8f6a-3c1f6a7d2b11';

describe('CoverArt', () => {
  it('shows only the placeholder for a track with no picture', () => {
    const { container } = render(CoverArt, { title: 'Low Tide' });

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('.art')?.textContent?.trim()).toBe('LT');
  });

  it('asks for the picture through lumen, by track id and size', () => {
    const { container } = render(CoverArt, { title: 'Low Tide', trackId: ID, size: 300 });

    const picture = container.querySelector('img');
    expect(picture?.getAttribute('src')).toBe(artUrl(ID, 300));
    expect(picture?.getAttribute('src')).toBe(`http://lumen.localhost/art/300/${ID}`);
    expect(picture?.getAttribute('loading')).toBe('lazy');
    // Rows are small: 64 px is the default.
    const row = render(CoverArt, { title: 'Low Tide', trackId: ID });
    expect(row.container.querySelector('img')?.getAttribute('src')).toBe(artUrl(ID, 64));
  });

  it('keeps the placeholder underneath and fades the picture in once it has loaded', async () => {
    const { container } = render(CoverArt, { title: 'Low Tide', trackId: ID });
    const picture = container.querySelector('img') as HTMLImageElement;
    expect(picture.hasAttribute('data-loaded')).toBe(false);

    await fireEvent.load(picture);

    expect(picture.hasAttribute('data-loaded')).toBe(true);
    expect(container.querySelector('.art > span')?.textContent?.trim()).toBe('LT');
  });

  it('falls back to the placeholder when the picture fails', async () => {
    const { container } = render(CoverArt, { title: 'Low Tide', trackId: ID });

    await fireEvent.error(container.querySelector('img') as HTMLImageElement);

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('.art > span')?.textContent?.trim()).toBe('LT');
  });
});
