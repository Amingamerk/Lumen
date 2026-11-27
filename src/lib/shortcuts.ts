// The app's keys. docs/PROJECT.md section 5 lists them.

const SEEK_MS = 5000;
const VOLUME_STEP = 5;

export type Shortcut =
  | { action: 'toggle' | 'next' | 'previous' | 'focus-search' | 'swallow' }
  | { action: 'seek'; deltaMs: number }
  | { action: 'volume'; delta: number };

/** Inputs that are pressed, not typed in. */
const NOT_TEXT = ['button', 'checkbox', 'color', 'file', 'image', 'radio', 'range', 'reset', 'submit'];

function typesText(target: Element): boolean {
  if (target.closest('[contenteditable]:not([contenteditable="false"])')) return true;
  if (target instanceof HTMLTextAreaElement) return true;
  return target instanceof HTMLInputElement && !NOT_TEXT.includes(target.type);
}

/** Whether Space already means something to the element. A slider has no use for it. */
function usesSpace(target: Element): boolean {
  if (target instanceof HTMLInputElement) return target.type !== 'range';
  return typesText(target) || target.matches('button, select, summary');
}

function usesArrows(target: Element): boolean {
  return typesText(target) || target.matches('input[type="range"], select');
}

/** What a key press asks for. The caller prevents the default of any answer. */
export function shortcutFor(event: KeyboardEvent): Shortcut | null {
  const { key, ctrlKey: ctrl, shiftKey: shift, repeat } = event;
  // AltGr arrives as Ctrl+Alt, so Alt always means someone else's key.
  if (event.defaultPrevented || event.isComposing || event.altKey || event.metaKey) return null;
  const target = event.target instanceof Element ? event.target : document.body;
  const once = (action: 'toggle' | 'next' | 'previous'): Shortcut => ({
    action: repeat ? 'swallow' : action,
  });

  if (key === ' ') return ctrl || shift || usesSpace(target) ? null : once('toggle');
  // Ctrl+F also keeps WebView2's find bar away, so it works everywhere.
  if (key.toLowerCase() === 'f') return ctrl && !shift ? { action: 'focus-search' } : null;
  if (ctrl === shift || usesArrows(target)) return null;
  if (key === 'ArrowLeft' || key === 'ArrowRight') {
    const forward = key === 'ArrowRight';
    if (ctrl) return once(forward ? 'next' : 'previous');
    return { action: 'seek', deltaMs: forward ? SEEK_MS : -SEEK_MS };
  }
  if (ctrl && (key === 'ArrowUp' || key === 'ArrowDown')) {
    return { action: 'volume', delta: key === 'ArrowUp' ? VOLUME_STEP : -VOLUME_STEP };
  }
  return null;
}
