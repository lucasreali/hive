import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  AA_LARGE,
  AA_TEXT,
  contrast,
  luminance,
  over,
  parseHex,
  passesAA,
  toHex,
} from "./contrast";

test("hex colours parse in their three forms and nothing else", () => {
  expect(parseHex("#fff")).toEqual({ rgb: [255, 255, 255], alpha: 1 });
  expect(parseHex(" #282C33 ")).toEqual({ rgb: [40, 44, 51], alpha: 1 });
  expect(parseHex("#85beff40")).toEqual({ rgb: [133, 190, 255], alpha: 64 / 255 });
  for (const bad of ["", "#", "#ff", "#ffff", "#fffff", "#fffffff", "282c33", "#gggggg", "red"]) {
    expect(parseHex(bad)).toBeNull();
  }
  expect(toHex([40, 44, 51])).toBe("#282c33");
  expect(toHex([0, 10, 255])).toBe("#000aff");
});

test("luminance and contrast follow WCAG 2.2", () => {
  expect(luminance([0, 0, 0])).toBe(0);
  expect(luminance([255, 255, 255])).toBe(1);
  // Each channel's weight, and the linear part below 0.04045.
  expect(luminance([255, 0, 0])).toBeCloseTo(0.2126, 4);
  expect(luminance([0, 255, 0])).toBeCloseTo(0.7152, 4);
  expect(luminance([0, 0, 255])).toBeCloseTo(0.0722, 4);
  expect(luminance([10, 10, 10])).toBeCloseTo(10 / 255 / 12.92, 6);
  expect(luminance([11, 11, 11])).toBeCloseTo(((11 / 255 + 0.055) / 1.055) ** 2.4, 6);
  expect(contrast([0, 0, 0], [255, 255, 255])).toBe(21);
  expect(contrast([255, 255, 255], [0, 0, 0])).toBe(21);
  expect(contrast([40, 44, 51], [40, 44, 51])).toBe(1);
  // Known pairs: #767676 on white is the classic 4.54.
  expect(contrast([118, 118, 118], [255, 255, 255])).toBeCloseTo(4.54, 2);
});

test("a translucent colour is composited over what is under it", () => {
  expect(over([255, 255, 255], 1, [0, 0, 0])).toEqual([255, 255, 255]);
  expect(over([255, 255, 255], 0, [0, 0, 0])).toEqual([0, 0, 0]);
  expect(over([255, 0, 100], 0.5, [0, 255, 0])).toEqual([128, 128, 50]);
});

test("AA is 4.5 for text and 3 for large text and marks, at its ends", () => {
  expect(AA_TEXT).toBe(4.5);
  expect(AA_LARGE).toBe(3);
  // #767676 (4.54) passes on white, #777777 (4.48) does not.
  expect(passesAA("#767676", "#ffffff")).toBe(true);
  expect(passesAA("#777777", "#ffffff")).toBe(false);
  expect(passesAA("#777777", "#ffffff", true)).toBe(true);
  // #949494 is 3.03 on white, #959595 2.99.
  expect(passesAA("#949494", "#ffffff", true)).toBe(true);
  expect(passesAA("#959595", "#ffffff", true)).toBe(false);
  // Exactly at the threshold passes: black on a grey whose ratio is 3 or 4.5.
  const grey = (ratio: number) => {
    // luminance = ratio * 0.05 - 0.05 for black text; find the channel by search.
    for (let c = 0; c < 256; c++) if (contrast([c, c, c], [0, 0, 0]) >= ratio) return c;
    return 255;
  };
  const at = grey(AA_TEXT);
  expect(passesAA("#000", toHex([at, at, at]))).toBe(true);
  expect(passesAA("#000", toHex([at - 1, at - 1, at - 1]))).toBe(false);
  // Translucent text counts as it shows: half-transparent black on white is mid grey.
  expect(passesAA("#00000080", "#ffffff")).toBe(false);
  expect(passesAA("#000000ff", "#ffffff")).toBe(true);
  // What cannot be judged fails: a name, a translucent background.
  expect(passesAA("black", "#ffffff")).toBe(false);
  expect(passesAA("#000000", "white")).toBe(false);
  expect(passesAA("#000000", "#ffffff80")).toBe(false);
});

// 28.6: the colour-blind diff colours, read from the style sheet, pass AA in both themes.
const css = readFileSync(join(import.meta.dir, "styles.css"), "utf8");

/** The custom properties of the rule whose selector is exactly `selector`. */
function tokens(selector: string): Record<string, string> {
  const start = css.indexOf(`\n${selector} {\n`);
  if (start < 0) throw new Error(`no rule ${selector}`);
  const body = css.slice(start, css.indexOf("\n}", start));
  return Object.fromEntries(
    [...body.matchAll(/^\s*(--[\w-]+):\s*([^;]+);/gm)].map((m) => [m[1], m[2].trim()]),
  );
}

const THEMES = {
  "one-dark": { ...tokens(":root"), ...tokens(':root[data-diff-colors="color-blind"]') },
  "one-light": {
    ...tokens(":root"),
    ...tokens(':root[data-theme="one-light"]'),
    ...tokens(':root[data-theme="one-light"][data-diff-colors="color-blind"]'),
  },
};
const SURFACES = ["--bg", "--panel", "--hover", "--selected"];
const COLORS = ["--diff-added", "--diff-modified", "--diff-removed", "--diff-renamed"];

/** `tints`, each over the last, over `base`: what a line of the diff shows. */
function layered(base: string, ...tints: string[]): string {
  let rgb = parseHex(base)?.rgb ?? ([0, 0, 0] as const);
  for (const tint of tints) {
    const top = parseHex(tint);
    if (!top) throw new Error(`not hex: ${tint}`);
    rgb = over(top.rgb, top.alpha, rgb);
  }
  return toHex(rgb);
}

for (const [theme, t] of Object.entries(THEMES)) {
  test(`the colour-blind diff colours pass AA as text on every surface, in ${theme} (28.6)`, () => {
    for (const color of COLORS) {
      for (const surface of SURFACES) {
        expect({ color, surface, passes: passesAA(t[color], t[surface]) }).toEqual({
          color,
          surface,
          passes: true,
        });
      }
    }
  });

  test(`the text passes AA on the colour-blind diff's line tints, in ${theme} (28.6)`, () => {
    const lines = [
      layered(t["--bg"], t["--diff-added-line"]),
      layered(t["--bg"], t["--diff-added-line"], t["--diff-added-text"]),
      layered(t["--bg"], t["--diff-removed-line"]),
      layered(t["--bg"], t["--diff-removed-line"], t["--diff-removed-text"]),
    ];
    for (const line of lines) {
      expect({ line, passes: passesAA(t["--text"], line) }).toEqual({ line, passes: true });
    }
    // The tints are the line colours' own, so a line reads as its marker.
    expect(t["--diff-added-line"].slice(0, 7)).toBe(t["--diff-added"]);
    expect(t["--diff-removed-text"].slice(0, 7)).toBe(t["--diff-removed"]);
  });
}

test("the colour-blind diff colours use neither red nor green (28.6)", () => {
  for (const t of Object.values(THEMES)) {
    const [r, g, b] = parseHex(t["--diff-added"])?.rgb ?? [0, 0, 0];
    // Added is a blue: blue the strongest channel.
    expect(b).toBeGreaterThan(Math.max(r, g));
    // Removed is an orange: red over green over blue.
    const [rr, rg, rb] = parseHex(t["--diff-removed"])?.rgb ?? [0, 0, 0];
    expect(rr).toBeGreaterThan(rg);
    expect(rg).toBeGreaterThan(rb);
  }
});
