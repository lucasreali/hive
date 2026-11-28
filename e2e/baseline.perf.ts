import { mkdirSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, type Page, test } from "@playwright/test";

// The frontend baselines of 29.8 (`docs/review/performance.md`), against the mock transport:
// measurements printed and written to `target/perf/<name>-<browser>.json`, not checks. Run with
// `E2E_PORT=<port> bunx playwright test -c e2e/perf.config.ts` on a quiet machine.

const CWD = "/home/user/projects/shop/.claude/worktrees/fix-login";
const RUNS = 200;
// 10 s of keys, while every replay of `?mock=load` (19 s, started within 1.5 s) still runs.
const KEYS = 100;

const percentile = (values: number[], p: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] ?? NaN;
};
const round = (n: number) => Math.round(n * 10) / 10;
const stats = (values: number[]) => ({
  p50: round(percentile(values, 50)),
  p95: round(percentile(values, 95)),
  p99: round(percentile(values, 99)),
  max: round(Math.max(...values)),
});

function record(name: string, browser: string, results: object) {
  mkdirSync("target/perf", { recursive: true });
  const all = { name, browser, ...results };
  console.log(JSON.stringify(all));
  writeFileSync(`target/perf/${name}-${browser}.json`, `${JSON.stringify(all, null, 2)}\n`);
}

/** Opens `count` terminals in `CWD` (the last one shown and focused); their ids. */
const openTerminals = (page: Page, count: number) =>
  page.evaluate(
    async ({ count, cwd }) => {
      const url = "/src/terminals.ts";
      const { openTerminal } = await import(/* @vite-ignore */ url);
      const ids: number[] = [];
      for (let i = 0; i < count; i++) ids.push(await openTerminal(cwd));
      return ids;
    },
    { count, cwd: CWD },
  );

// Output → painted: bytes reach the terminal manager (the mock's echo of a key sent with
// `transport.writeTerminal`, delivered as the Tauri transport delivers a PTY frame) until the
// frame after xterm.js parsed them. Quiet, then with 10 other terminals replaying an agent's
// output (`?mock=load`, about 23 KB/s each).
for (const others of [0, 10]) {
  test(`output → painted, ${others} other terminals streaming`, async ({ page, browserName }) => {
    test.setTimeout(120_000);
    await page.goto(others ? "/?mock=load" : "/?mock");
    await expect(page.getByRole("navigation", { name: "Projects" })).toBeVisible();
    const ids = await openTerminals(page, others + 1);
    await page.waitForTimeout(3000);
    const samples: number[] = await page.evaluate(
      async ({ id, runs, marked }) => {
        const load = (url: string) => import(/* @vite-ignore */ url);
        const { terminal } = await load("/src/terminals.ts");
        const { transport } = await load("/src/transport/index.ts");
        const term = terminal(id);
        // With `?mock=load` the measured terminal replays too: its echo is told apart by the
        // mark the mock appends to it (OSC 7777); otherwise its echo is all it receives.
        let parsed = () => {};
        if (marked) {
          term.parser.registerOscHandler(7777, () => {
            parsed();
            return true;
          });
        }
        const out: number[] = [];
        for (let i = 0; i < runs; i++) {
          const painted = new Promise<number>((resolve) => {
            const done = () => requestAnimationFrame(() => resolve(performance.now()));
            if (marked) parsed = done;
            else {
              const sub = term.onWriteParsed(() => {
                sub.dispose();
                done();
              });
            }
          });
          const start = performance.now();
          await transport.writeTerminal(id, "k");
          out.push((await painted) - start);
          // Between keys, as a fast typist's 10 a second.
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
        return out;
      },
      { id: ids.at(-1) as number, runs: KEYS, marked: others > 0 },
    );
    record(`output-painted-${others}`, browserName, { runs: KEYS, others, ms: stats(samples) });
  });
}

// Hook event → state shown: an `agent_state` applied as the transport applies a service message,
// until the agent's row shows it (DOM committed) and the frame after that (painted).
test("agent state → shown", async ({ page, browserName }) => {
  test.setTimeout(120_000);
  await page.goto("/?mock=states");
  const tree = page.getByRole("navigation", { name: "Projects" });
  await expect(tree.locator(".tree-row.agent")).toHaveCount(7);
  const { committed, painted } = await page.evaluate(async (runs) => {
    const load = (url: string) => import(/* @vite-ignore */ url);
    const { apply } = await load("/src/reduce.ts");
    const { agentStatus } = await load("/src/transport/mock.ts");
    const nav = document.querySelector('nav[aria-label="Projects"]') as HTMLElement;
    const working = () =>
      nav.querySelectorAll('.tree-row.agent .state-icon[data-state="working"]').length;
    const base = working();
    const committed: number[] = [];
    const painted: number[] = [];
    for (let i = 0; i < runs; i++) {
      // The last mock agent (idle) goes working and back: one more working row, then one less.
      const state = i % 2 === 0 ? "working" : "idle";
      const want = i % 2 === 0 ? base + 1 : base;
      const shown = new Promise<void>((resolve) => {
        const start = performance.now();
        const observer = new MutationObserver(() => {
          if (working() !== want) return;
          observer.disconnect();
          committed.push(performance.now() - start);
          requestAnimationFrame(() => {
            painted.push(performance.now() - start);
            resolve();
          });
        });
        observer.observe(nav, { subtree: true, childList: true, attributes: true });
        apply({
          type: "agent_state",
          id: "mock-state-7",
          ...agentStatus(state, state === "working" ? "Run the tests" : null, Date.now()),
          subagents: [],
        });
      });
      await shown;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    return { committed, painted };
  }, RUNS);
  expect(committed).toHaveLength(RUNS);
  record("agent-state", browserName, {
    runs: RUNS,
    committedMs: stats(committed),
    paintedMs: stats(painted),
  });
});

// Cold load of the production bundle (`vite build`, served by `vite preview` on the next port):
// navigation until the first paint, and until the project list shows its rows. A fresh context
// each time: nothing cached. In the app the bundle comes from the app itself, not over HTTP.
test("cold load of the production bundle", async ({ browser, browserName, baseURL }) => {
  test.setTimeout(120_000);
  const port = Number(new URL(baseURL as string).port) + 1;
  const runs: { fcp: number; rows: number; dcl: number }[] = [];
  for (let i = 0; i < 6; i++) {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.addInitScript(() => {
      new MutationObserver((_, observer) => {
        if (!document.querySelector('nav[aria-label="Projects"] .tree-row')) return;
        observer.disconnect();
        requestAnimationFrame(() => Object.assign(window, { rowsAt: performance.now() }));
      }).observe(document, { subtree: true, childList: true });
    });
    await page.goto(`http://localhost:${port}/?mock`);
    await page.waitForFunction(() => "rowsAt" in window);
    runs.push(
      await page.evaluate(() => {
        const [nav] = performance.getEntriesByType("navigation") as PerformanceNavigationTiming[];
        // Headless Chromium may report no paint entries: NaN then.
        const paint = performance.getEntriesByType("paint");
        const fcp = (paint.find((p) => p.name === "first-contentful-paint") ?? paint[0])?.startTime;
        const rows = (window as unknown as { rowsAt: number }).rowsAt;
        return { fcp: fcp ?? NaN, rows, dcl: nav?.domContentLoadedEventEnd ?? NaN };
      }),
    );
    await context.close();
  }
  // The first run warms the server and the browser's own caches; the others are measured.
  const measured = runs.slice(1);
  const assets = join("dist", "assets");
  const sizes = readdirSync(assets).map((f) => ({ f, bytes: statSync(join(assets, f)).size }));
  const js = sizes.filter((s) => s.f.endsWith(".js"));
  record("cold-load", browserName, {
    runs: measured.length,
    firstPaintMs: stats(measured.map((r) => r.fcp)),
    domContentLoadedMs: stats(measured.map((r) => r.dcl)),
    projectRowsMs: stats(measured.map((r) => r.rows)),
    jsFiles: js.length,
    jsKiB: Math.round(js.reduce((n, s) => n + s.bytes, 0) / 1024),
    largestJs: js
      .sort((a, b) => b.bytes - a.bytes)
      .slice(0, 5)
      .map((s) => `${s.f} ${Math.round(s.bytes / 1024)} KiB`),
  });
});

// Memory with many tabs (Chromium only: WebKit has no heap metrics): the page's JS heap and DOM
// after a GC, empty, then with 30 terminals that each received an agent's ~19 s replay.
test("memory with 30 terminals", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "heap metrics through the Chrome DevTools Protocol");
  test.setTimeout(180_000);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const metrics = async () => {
    await cdp.send("HeapProfiler.collectGarbage");
    const { metrics } = await cdp.send("Performance.getMetrics");
    const get = (name: string) => metrics.find((m) => m.name === name)?.value ?? 0;
    return {
      heapMiB: round(get("JSHeapUsedSize") / 1048576),
      nodes: get("Nodes"),
      listeners: get("JSEventListeners"),
    };
  };
  await page.goto("/?mock=load");
  await expect(page.getByRole("navigation", { name: "Projects" })).toBeVisible();
  await page.waitForTimeout(2000);
  const empty = await metrics();
  await openTerminals(page, 30);
  // Every replay (19 s, the last one starting 3.5 s in) has ended.
  await page.waitForTimeout(30_000);
  const full = await metrics();
  record("memory-30-terminals", browserName, { empty, full });
});

// Where the main thread goes with 20 terminals replaying (Chromium's CPU profiler, as the
// performance panel records it): self time by function, the top 25, and the whole profile in
// `target/perf/replay-20.cpuprofile` (open it in Chrome's DevTools, Performance panel).
test("CPU profile, 20 terminals replaying", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "the profiler of the Chrome DevTools Protocol");
  test.setTimeout(120_000);
  await page.goto("/?mock=load");
  await expect(page.getByRole("navigation", { name: "Projects" })).toBeVisible();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Profiler.enable");
  await cdp.send("Profiler.setSamplingInterval", { interval: 200 });
  await openTerminals(page, 20);
  await page.waitForTimeout(2000);
  await cdp.send("Profiler.start");
  await page.waitForTimeout(10_000);
  const { profile } = await cdp.send("Profiler.stop");
  mkdirSync("target/perf", { recursive: true });
  writeFileSync("target/perf/replay-20.cpuprofile", JSON.stringify(profile));
  const self = new Map<number, number>();
  const deltas = profile.timeDeltas ?? [];
  (profile.samples ?? []).forEach((node, i) => {
    self.set(node, (self.get(node) ?? 0) + (deltas[i] ?? 0));
  });
  const total = [...self.values()].reduce((a, b) => a + b, 0);
  const byFunction = new Map<string, number>();
  for (const node of profile.nodes) {
    const { functionName, url, lineNumber } = node.callFrame;
    const file = url.replace(/^.*\/(node_modules\/\.vite\/deps|src)\//, "$1/").replace(/\?.*$/, "");
    const key = `${functionName || "(anonymous)"} ${file}:${lineNumber + 1}`;
    byFunction.set(key, (byFunction.get(key) ?? 0) + (self.get(node.id) ?? 0));
  }
  // Shares of the busy time: the idle samples left out.
  const idle = byFunction.get("(idle) :0") ?? 0;
  byFunction.delete("(idle) :0");
  const busy = total - idle;
  const top = [...byFunction.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 25)
    .map(([fn, us]) => `${((us / busy) * 100).toFixed(1)}% ${fn}`);
  record("profile-replay-20", browserName, {
    sampledMs: round(total / 1000),
    busyPct: round((busy / total) * 100),
    top,
  });
});
