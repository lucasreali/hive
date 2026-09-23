import { defineConfig } from "@playwright/test";

// Browser checks against the Vite dev server. Kept out of `bun test` (bunfig.toml root = src).
export default defineConfig({
  testDir: ".",
  testMatch: "*.e2e.ts",
  outputDir: "../target/e2e",
  use: { baseURL: "http://localhost:1420", viewport: { width: 1440, height: 900 } },
  webServer: { command: "bun run dev", url: "http://localhost:1420", reuseExistingServer: true },
});
