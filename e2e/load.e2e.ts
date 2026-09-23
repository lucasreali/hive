import { existsSync, writeFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

// Load test (1.11, #28): 20 terminals replay a Claude Code recording at the same time while
// the focused one is typed into. `HIVE_LOAD_CAST=<url>` replays an asciinema capture served by
// Vite (e.g. `/target/claude.cast`) instead of the generated recording.
const TERMINALS = 20;
const CWD = "/home/user/projects/shop/.claude/worktrees/fix-login";
const TYPING_MS = 12_000;
const KEY_INTERVAL_MS = 100; // a fast typist, 10 keys/s

// Pass thresholds.
// - Input: 50 ms from key press to the frame that shows the echo is where typing starts to feel
//   laggy; local terminals sit around 10-40 ms. p99 must stay under 100 ms (RAIL response
//   budget). Not the single worst key: on a shared machine one sample is at the mercy of the OS
//   scheduler (stalls of 400 ms were seen with no long task and frames stalled too).
// - Frames: the 95th-percentile frame interval must keep at least 30 fps.
// - Long tasks: none may block the main thread for 100 ms or more (RAIL response budget).
const MAX_P95_INPUT_MS = 50;
const MAX_P99_INPUT_MS = 100;
const MAX_P95_FRAME_MS = 1000 / 30;
const MAX_LONG_TASK_MS = 100;

// WebGL through ANGLE on the system GL, not Chromium's default SwiftShader, which saturates the
// main thread with a single terminal and says nothing about a real GPU. Inside WSL that GL is
// Mesa's d3d12 driver on the Windows GPU; elsewhere whatever Mesa finds (llvmpipe without a GPU,
// which competes with the page for CPU and can fail the thresholds).
const WSL_LIB = "/usr/lib/wsl/lib";
const wsl = existsSync("/dev/dxg") && existsSync(WSL_LIB);
const path = [process.env.LD_LIBRARY_PATH, WSL_LIB].filter(Boolean).join(":");
test.use({
  launchOptions: {
    args: ["--ignore-gpu-blocklist", "--use-gl=angle", "--use-angle=gl"],
    env: wsl ? { ...process.env, GALLIUM_DRIVER: "d3d12", LD_LIBRARY_PATH: path } : undefined,
  },
});

const percentile = (values: number[], p: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] ?? NaN;
};
const round = (n: number) => Math.round(n * 10) / 10;

test("load: 20 terminals replay Claude Code output; the focused one stays fluid", async ({
  page,
}) => {
  test.setTimeout(120_000);
  const cast = process.env.HIVE_LOAD_CAST;
  await page.goto(`/?mock=load${cast ? `&cast=${encodeURIComponent(cast)}` : ""}`);
  await expect(page.getByRole("navigation", { name: "Projects" })).toBeVisible();

  const ids = await page.evaluate(
    async ({ count, cwd }) => {
      const url = "/src/terminals.ts";
      const { openTerminal } = await import(/* @vite-ignore */ url);
      const ids: number[] = [];
      for (let i = 0; i < count; i++) ids.push(await openTerminal(cwd));
      return ids;
    },
    { count: TERMINALS, cwd: CWD },
  );
  const focused = ids.at(-1) as number;
  await expect(page.getByRole("tab")).toHaveCount(TERMINALS);

  // Measures once all 20 are running (the mock starts them over 0.5-2.5 s) and the renderer's
  // glyph atlas is warm: the user types while the agents work.
  await page.waitForTimeout(3000);

  // Instruments the page: key press times, echo marks (OSC 7777) timed to the next frame,
  // every frame interval and every long task.
  await page.evaluate(async (id) => {
    const url = "/src/terminals.ts";
    const { terminal } = await import(/* @vite-ignore */ url);
    const probe = { keys: [] as number[], input: [] as number[], frames: [] as number[] };
    const long: number[] = [];
    Object.assign(window, { probe, long });
    document.addEventListener("keydown", (e) => probe.keys.push(e.timeStamp), true);
    terminal(id).parser.registerOscHandler(7777, () => {
      const pressed = probe.keys.shift() ?? NaN;
      requestAnimationFrame(() => probe.input.push(performance.now() - pressed));
      return true;
    });
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) long.push(entry.duration);
    }).observe({ type: "longtask" });
    let last = performance.now();
    const frame = (now: number) => {
      probe.frames.push(now - last);
      last = now;
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }, focused);

  const keys = Math.floor(TYPING_MS / KEY_INTERVAL_MS);
  for (let i = 0; i < keys; i++) {
    await page.keyboard.press(String.fromCharCode(97 + (i % 26)));
    await page.waitForTimeout(KEY_INTERVAL_MS);
  }
  await page.waitForTimeout(500);
  const { input, frames, long } = await page.evaluate(() => {
    const w = window as unknown as {
      probe: { input: number[]; frames: number[] };
      long: number[];
    };
    return { input: [...w.probe.input], frames: w.probe.frames.slice(1), long: [...w.long] };
  });

  // Once every replay has ended, each hidden terminal holds exactly what a terminal of the
  // same size shows after parsing the whole recording at once: nothing was dropped.
  const check = await page.evaluate(
    async ({ ids, cast }) => {
      const load = (url: string) => import(/* @vite-ignore */ url);
      const { terminal } = await load("/src/terminals.ts");
      const { loadReplay } = await load("/src/transport/replay.ts");
      const events: { at: number; data: string }[] = await loadReplay(cast);
      type Buffer = {
        length: number;
        getLine(y: number): { translateToString(t: boolean): string };
      };
      const text = (term: { buffer: { active: Buffer } }) => {
        const buffer = term.buffer.active;
        const lines: string[] = [];
        for (let y = 0; y < buffer.length; y++) {
          lines.push(buffer.getLine(y).translateToString(true));
        }
        return lines.join("\n").trimEnd();
      };
      const first = terminal(ids[0]);
      const reference = new first.constructor({
        cols: first.cols,
        rows: first.rows,
        scrollback: first.options.scrollback,
      });
      await new Promise<void>((resolve) =>
        reference.write(`mock$ ${events.map((e) => e.data).join("")}`, resolve),
      );
      const expected = text(reference);
      const duration = (events.at(-1)?.at ?? 0) + 500 + 100 * 20 + 2000;
      const deadline = performance.now() + duration;
      let differing = ids;
      while (performance.now() < deadline) {
        differing = ids.filter((id: number) => text(terminal(id)) !== expected);
        if (differing.length === 0) break;
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
      const bytes = new TextEncoder().encode(events.map((e) => e.data).join("")).length;
      const gl = document.createElement("canvas").getContext("webgl2");
      const info = gl?.getExtension("WEBGL_debug_renderer_info");
      const renderer = info ? String(gl?.getParameter(info.UNMASKED_RENDERER_WEBGL)) : "none";
      return { differing, tail: expected.split("\n").at(-1), bytes, renderer };
    },
    { ids: ids.slice(0, -1), cast: cast ?? null },
  );

  const results = {
    terminals: TERMINALS,
    recording: cast ?? "generated",
    recordingBytes: check.bytes,
    renderer: check.renderer,
    keys,
    echoes: input.length,
    inputMs: {
      p50: round(percentile(input, 50)),
      p95: round(percentile(input, 95)),
      p99: round(percentile(input, 99)),
    },
    inputMaxMs: round(Math.max(...input)),
    frameMs: { p50: round(percentile(frames, 50)), p95: round(percentile(frames, 95)) },
    frameMaxMs: round(Math.max(...frames)),
    frames: frames.length,
    longTasks: long.length,
    longTaskMaxMs: round(Math.max(0, ...long)),
    droppedIn: check.differing,
  };
  console.log(JSON.stringify(results));
  writeFileSync("target/e2e/load.json", `${JSON.stringify(results, null, 2)}\n`);

  expect(input).toHaveLength(keys);
  expect(check.differing).toEqual([]);
  if (!cast) expect(check.tail).toBe("✻ Replay complete");
  expect(results.inputMs.p95).toBeLessThan(MAX_P95_INPUT_MS);
  expect(results.inputMs.p99).toBeLessThan(MAX_P99_INPUT_MS);
  expect(results.frameMs.p95).toBeLessThan(MAX_P95_FRAME_MS);
  expect(results.longTaskMaxMs).toBeLessThan(MAX_LONG_TASK_MS);
});
