import { afterEach, expect, test } from "bun:test";
import { claudeRecording, loadReplay, parseCast, RECORDING_END } from "./replay";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

test("parses asciinema v2 (absolute times) and v3 (intervals), output events only", () => {
  const v2 =
    '{"version":2,"width":80,"height":24}\n[0.5,"o","a"]\n[0.75,"i","x"]\n[1.25,"o","b"]\n';
  expect(parseCast(v2)).toEqual([
    { at: 500, data: "a" },
    { at: 1250, data: "b" },
  ]);
  const v3 =
    '{"version":3,"term":{"cols":80,"rows":24}}\n# a comment\n[0.5,"o","a"]\n[0.25,"r","80x24"]\n[0.5,"o","b"]';
  expect(parseCast(v3)).toEqual([
    { at: 500, data: "a" },
    { at: 1250, data: "b" },
  ]);
  expect(() => parseCast('{"version":1}')).toThrow("not an asciinema v2/v3 cast");
  expect(() => parseCast("")).toThrow("not an asciinema v2/v3 cast");
});

test("loads a cast from a URL, or the generated recording without one", async () => {
  const urls: string[] = [];
  globalThis.fetch = (async (url: string) => {
    urls.push(url);
    return url === "/rec.cast"
      ? new Response('{"version":2}\n[0,"o","hi"]')
      : new Response("", { status: 404 });
  }) as unknown as typeof fetch;
  expect(await loadReplay("/rec.cast")).toEqual([{ at: 0, data: "hi" }]);
  await expect(loadReplay("/missing.cast")).rejects.toThrow("cannot fetch /missing.cast: 404");
  expect(await loadReplay(null)).toEqual(claudeRecording());
  expect(urls).toEqual(["/rec.cast", "/missing.cast"]);
});

test("the generated recording looks like a Claude Code session", () => {
  const events = claudeRecording();
  const all = events.map((e) => e.data).join("");
  const seconds = (events.at(-1)?.at ?? 0) / 1000;
  // Deterministic, ordered in time, about 20 s long.
  expect(claudeRecording()).toEqual(events);
  expect(events.every((e, i) => i === 0 || e.at >= (events[i - 1]?.at ?? 0))).toBe(true);
  expect(seconds).toBeGreaterThan(15);
  expect(seconds).toBeLessThan(25);
  // Tens of KB/s: redraws of the whole dynamic region dominate.
  const rate = new TextEncoder().encode(all).length / seconds;
  expect(rate).toBeGreaterThan(20_000);
  expect(rate).toBeLessThan(200_000);
  // Spinner redraws at 12.5 Hz, synchronized output, erase-line + cursor-up, SGR colors.
  expect(events.filter((e) => e.data.includes("esc to interrupt")).length).toBeGreaterThan(200);
  expect(all).toContain("\x1b[?2026h");
  expect(all).toContain("\x1b[2K\x1b[1A");
  expect(all).toContain("\x1b[38;2;");
  expect(all).toContain("\x1b[48;2;");
  expect(all).toContain("\x1b[38;5;");
  expect(all).toContain("╭");
  // Long tool output arrives in PTY-sized chunks; the recording ends with a known line.
  expect(Math.max(...events.map((e) => e.data.length))).toBe(4096);
  expect(events.at(-1)?.data.endsWith(`${RECORDING_END}\r\n`)).toBe(true);
});
