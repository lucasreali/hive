/**
 * WCAG 2.2 contrast (28.6): the ratio between two colours and whether text in one colour on
 * the other passes level AA. The theme file's checks (28.16) reuse these.
 */

/** An opaque sRGB colour, each channel 0 to 255. */
export type Rgb = readonly [number, number, number];

/** AA's minimum for normal text (WCAG 1.4.3); large text and non-text marks need 3 (1.4.11). */
export const AA_TEXT = 4.5;
export const AA_LARGE = 3;

/**
 * `#rgb`, `#rrggbb` or `#rrggbbaa` as its channels and alpha (0 to 1); null for anything else.
 */
export function parseHex(hex: string): { rgb: Rgb; alpha: number } | null {
  const digits = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(hex.trim())?.[1];
  if (!digits) return null;
  const full = digits.length === 3 ? [...digits].map((d) => d + d).join("") : digits;
  const byte = (i: number) => Number.parseInt(full.slice(i, i + 2), 16);
  return { rgb: [byte(0), byte(2), byte(4)], alpha: full.length === 8 ? byte(6) / 255 : 1 };
}

/** `top` at `alpha` over the opaque `bottom`, as the browser composites it. */
export function over(top: Rgb, alpha: number, bottom: Rgb): Rgb {
  const mix = (i: 0 | 1 | 2) => Math.round(top[i] * alpha + bottom[i] * (1 - alpha));
  return [mix(0), mix(1), mix(2)];
}

/** `rgb` as `#rrggbb`. */
export function toHex(rgb: Rgb): string {
  return `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

/** WCAG's relative luminance, 0 (black) to 1 (white). */
export function luminance([r, g, b]: Rgb): number {
  const linear = (channel: number) => {
    const c = channel / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** The contrast ratio of two colours, 1 (the same) to 21 (black on white), either order. */
export function contrast(a: Rgb, b: Rgb): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

/**
 * Whether `text` (a hex colour, translucent ones composited over `background`) on the opaque
 * `background` passes AA: 4.5, or 3 for `large` text and non-text marks. A colour that is not
 * hex never passes, so a typo in a theme is caught rather than waved through.
 */
export function passesAA(text: string, background: string, large = false): boolean {
  const fg = parseHex(text);
  const bg = parseHex(background);
  if (!fg || !bg || bg.alpha !== 1) return false;
  return contrast(over(fg.rgb, fg.alpha, bg.rgb), bg.rgb) >= (large ? AA_LARGE : AA_TEXT);
}
