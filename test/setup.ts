import { afterEach } from "bun:test";
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();

// Every test starts with happy-dom's own (Linux) user agent, whatever `asMac` (mac.ts) set.
afterEach(() => {
  delete (navigator as { userAgent?: string }).userAgent;
});
