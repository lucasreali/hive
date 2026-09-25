import { afterEach } from "bun:test";
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();

// Every test starts with happy-dom's own (Linux) user agent, whatever `asMac` (mac.ts) set,
// and an empty storage (saved widths from a divider drag would leak into the next file).
afterEach(() => {
  delete (navigator as { userAgent?: string }).userAgent;
  localStorage.clear();
});
