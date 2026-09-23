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
  webServer: { command: `bun run dev --port ${port}`, url, reuseExistingServer: true },
});
