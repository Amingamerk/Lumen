// Placeholder covers, drawn locally until `lumen://art/` serves real thumbnails.

/** How many tones and light positions app.css styles. */
const TONES = 6;
const LIGHTS = 4;

/** The same title always gets the same cover. */
export function placeholderArt(title: string): { initials: string; tone: number; light: number } {
  const initials = title
    .split(/\s+/)
    .flatMap((word) => word.match(/[\p{L}\p{N}]/u)?.[0] ?? [])
    .slice(0, 2)
    .join('')
    .toUpperCase();

  const bits = hash(title);
  return { initials, tone: (bits >>> 8) % TONES, light: (bits >>> 20) % LIGHTS };
}

/** FNV-1a with a final mix, so that similar titles still land far apart. */
function hash(text: string): number {
  let value = 0x811c9dc5;
  for (const char of text) {
    value = Math.imul(value ^ (char.codePointAt(0) ?? 0), 0x01000193);
  }
  value = Math.imul(value ^ (value >>> 16), 0x85ebca6b);
  value = Math.imul(value ^ (value >>> 13), 0xc2b2ae35);
  return (value ^ (value >>> 16)) >>> 0;
}
