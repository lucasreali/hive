import { defineConfig } from "@playwright/test";

// Browser checks against the Vite dev server. Kept out of `bun test` (bunfig.toml root = src).
// E2E_PORT lets parallel agents (one per worktree) each test their own server.
const port = Number(process.env.E2E_PORT ?? 1420);
const url = `http://localhost:${port}`;

export default defineConfig({
  testDir: ".",
  testMatch: "*.e2e.ts",
  outputDir: "../target/e2e",
  use: { baseURL: url, viewport: { width: 1440, height: 900 } },
  // The load test (1.11) and the files tree's (9.23) measure timing, so each runs after the
  // other specs (both browsers'), alone, on Chromium.
  projects: [
    { name: "ui", testIgnore: ["load.e2e.ts", "tree-perf.e2e.ts"] },
    { name: "load", testMatch: "load.e2e.ts", dependencies: ["ui", "webkit"] },
    { name: "tree-perf", testMatch: "tree-perf.e2e.ts", dependencies: ["load"] },
    // WebKit (17.3), the engine of macOS' WKWebView (close to it, not identical). It tests the
    // engine with the same keys as Chromium: a non-Mac user agent keeps Ctrl as the command key
    // (window.ts `isMac`), on a Mac host and on CI's Linux alike.
    {
      name: "webkit",
      use: {
        browserName: "webkit",
        userAgent:
          "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
      },
      testIgnore: ["load.e2e.ts", "tree-perf.e2e.ts"],
    },
  ],
  webServer: { command: `bun run dev --port ${port}`, url, reuseExistingServer: true },
});
