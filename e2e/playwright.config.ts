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
  // other specs, alone.
  projects: [
    { name: "ui", testIgnore: ["load.e2e.ts", "tree-perf.e2e.ts"] },
    { name: "load", testMatch: "load.e2e.ts", dependencies: ["ui"] },
    { name: "tree-perf", testMatch: "tree-perf.e2e.ts", dependencies: ["load"] },
  ],
  webServer: { command: `bun run dev --port ${port}`, url, reuseExistingServer: true },
});
