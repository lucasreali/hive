import { defineConfig } from "@playwright/test";

// The frontend baselines of 29.8 (`docs/review/performance.md`): measurements, not checks, so they
// are kept out of `bun run e2e` (whose config matches `*.e2e.ts` only). Run them on a quiet
// machine with `E2E_PORT=<port> bunx playwright test -c e2e/perf.config.ts`; results go to
// `target/perf/`. The dev server serves the mock-transport measurements; a production build on
// the next port serves the cold load, as the app's bundle loads it.
const port = Number(process.env.E2E_PORT ?? 1420);
const url = `http://localhost:${port}`;
const preview = `http://localhost:${port + 1}`;
const angle = process.platform === "darwin" ? "metal" : "gl";

export default defineConfig({
  testDir: ".",
  testMatch: "*.perf.ts",
  outputDir: "../target/perf/output",
  workers: 1,
  use: { baseURL: url, viewport: { width: 1440, height: 900 } },
  projects: [
    // WebGL on the system's GPU, as the load test (1.11) runs it: Metal on a Mac, GL elsewhere.
    {
      name: "chromium",
      use: {
        browserName: "chromium",
        launchOptions: {
          args: ["--ignore-gpu-blocklist", "--use-gl=angle", `--use-angle=${angle}`],
        },
      },
    },
    // WebKit is the engine of macOS' WKWebView (close to it, not identical).
    { name: "webkit", use: { browserName: "webkit" } },
  ],
  webServer: [
    { command: `bun run dev --port ${port}`, url, reuseExistingServer: true },
    {
      command: `bunx vite build && bunx vite preview --port ${port + 1} --strictPort`,
      cwd: "..",
      url: preview,
      reuseExistingServer: true,
      timeout: 600_000,
    },
  ],
});
