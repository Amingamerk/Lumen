// jsdom has no layout, so it has no ResizeObserver. Tests resize by scrolling.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
