import { afterEach, describe, expect, it } from 'vitest';
import { shortcutFor } from './shortcuts';

afterEach(() => (document.body.innerHTML = ''));

function element(html: string): Element {
  document.body.innerHTML = html;
  return document.body.firstElementChild as Element;
}

/** What a key press on `target` does. */
function press(init: KeyboardEventInit, target: Element = document.body) {
  let seen: ReturnType<typeof shortcutFor> = null;
  target.addEventListener('keydown', (event) => (seen = shortcutFor(event as KeyboardEvent)), {
    once: true,
  });
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
  return seen;
}

const space = { key: ' ' };
const find = { key: 'f', ctrlKey: true };
const ctrl = (key: string) => ({ key, ctrlKey: true });
const shift = (key: string) => ({ key, shiftKey: true });

describe('shortcuts', () => {
  it.each([
    [space, { action: 'toggle' }],
    [find, { action: 'focus-search' }],
    [{ key: 'F', ctrlKey: true }, { action: 'focus-search' }],
    [ctrl('ArrowLeft'), { action: 'previous' }],
    [ctrl('ArrowRight'), { action: 'next' }],
    [shift('ArrowLeft'), { action: 'seek', deltaMs: -5000 }],
    [shift('ArrowRight'), { action: 'seek', deltaMs: 5000 }],
    [ctrl('ArrowUp'), { action: 'volume', delta: 5 }],
    [ctrl('ArrowDown'), { action: 'volume', delta: -5 }],
  ])('maps %o', (init, shortcut) => {
    expect(press(init)).toEqual(shortcut);
  });

  it.each([
    '<input type="text">',
    '<input type="search">',
    '<textarea></textarea>',
    '<div contenteditable="true"></div>',
  ])('leaves %s alone, but for Ctrl+F', (html) => {
    const field = element(html);

    for (const init of [space, ctrl('ArrowLeft'), shift('ArrowRight'), ctrl('ArrowUp')]) {
      expect(press(init, field)).toBeNull();
    }
    expect(press(find, field)).toEqual({ action: 'focus-search' });
  });

  it('leaves buttons and sliders their keys', () => {
    for (const html of [
      '<button></button>',
      '<input type="radio">',
      '<input type="checkbox">',
      '<select></select>',
    ]) {
      expect(press(space, element(html))).toBeNull();
    }
    const range = element('<input type="range">');
    for (const init of [ctrl('ArrowLeft'), shift('ArrowRight'), ctrl('ArrowDown')]) {
      expect(press(init, range)).toBeNull();
    }
    expect(press(space, range)).toEqual({ action: 'toggle' });
    expect(press(ctrl('ArrowRight'), element('<button></button>'))).toEqual({ action: 'next' });
  });

  it('ignores other modifiers, composing and handled keys', () => {
    for (const init of [
      { ...space, altKey: true },
      { ...space, metaKey: true },
      { ...space, ctrlKey: true },
      { ...space, shiftKey: true },
      { ...find, altKey: true },
      { ...find, shiftKey: true },
      { ...ctrl('ArrowLeft'), shiftKey: true },
      { ...space, isComposing: true },
      { key: 'ArrowLeft' },
      { key: 'ArrowUp', shiftKey: true },
      { key: 'f' },
    ]) {
      expect(press(init)).toBeNull();
    }
    const handled = element('<div></div>');
    handled.addEventListener('keydown', (event) => event.preventDefault());
    expect(press(space, handled)).toBeNull();
  });

  it('swallows a held key that must act once, and repeats the others', () => {
    for (const init of [space, ctrl('ArrowLeft'), ctrl('ArrowRight')]) {
      expect(press({ ...init, repeat: true })).toEqual({ action: 'swallow' });
    }
    expect(press({ ...shift('ArrowRight'), repeat: true })).toEqual({
      action: 'seek',
      deltaMs: 5000,
    });
    expect(press({ ...ctrl('ArrowUp'), repeat: true })).toEqual({ action: 'volume', delta: 5 });
  });
});
