import { afterEach } from "bun:test";
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();
const storage = Object.getOwnPropertyDescriptor(window, "localStorage") ?? {};

// Every test starts with happy-dom's own (Linux) user agent, whatever `asMac` (mac.ts) set,
// and its own empty storage, whatever `stubStorage` (storage.ts) set (saved widths from a
// divider drag would leak into the next file).
afterEach(() => {
  delete (navigator as { userAgent?: string }).userAgent;
  Object.defineProperty(window, "localStorage", storage);
  localStorage.clear();
});
