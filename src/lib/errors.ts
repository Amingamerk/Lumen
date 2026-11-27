/** Rust commands reject with a string; the Tauri API itself throws errors. */
export function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
