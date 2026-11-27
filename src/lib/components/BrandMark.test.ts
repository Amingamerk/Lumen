import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import BrandMark from './BrandMark.svelte';

const wordmark = () => screen.getByRole<HTMLImageElement>('img', { name: 'Lumen' });

describe('BrandMark', () => {
  it('shows the wordmark as an image named Lumen', () => {
    render(BrandMark);

    expect(wordmark()).toBeTruthy();
  });

  it('loads the wordmark from the bundle, not from the network', () => {
    render(BrandMark);

    expect(wordmark().getAttribute('src')).toMatch(/^\/.*lumen-logo[^/]*\.svg$/);
  });

  it('reserves the box before the file loads, at the logo aspect ratio', () => {
    render(BrandMark);

    // The artwork's viewBox is 1398 x 465.
    const { width, height } = wordmark();
    expect([width, height]).toEqual([120, 40]);
  });
});
