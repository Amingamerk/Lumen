/**
 * Drops the focus a mouse click left on a button, so Space is play/pause again.
 * A button pressed from the keyboard (`detail` 0) keeps it.
 */
export function releasePointerFocus(event: MouseEvent): void {
  if (event.detail > 0) (event.currentTarget as HTMLElement).blur();
}
