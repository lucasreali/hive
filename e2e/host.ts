import { platform } from "node:process";

// Host-aware keys (17.14). The pages report the host's platform (`playwright.config.ts` pins a
// non-Mac user agent only off macOS), so Hive's command key is the host's: Cmd on a Mac, Ctrl
// elsewhere, as Playwright's own editing keys (Select All, line start…) are.

/** Whether the specs run on a Mac, where Hive takes macOS' keys and labels (`isMac`). */
export const mac = platform === "darwin";

/** Hive's command key and the fields' and the editor's own: Cmd on macOS, Ctrl elsewhere. */
export const mod = mac ? "Meta" : "Control";

/** A label as Hive shows it on this platform (`keyText`): "Ctrl+Shift+O" is "⇧⌘O" on macOS. */
export const keyLabel = (text: string) =>
  mac ? text.replaceAll("Ctrl+Shift+", "⇧⌘").replaceAll("Ctrl+", "⌘") : text;

/** The line start key of a field and the editor: ⌘← on macOS, where Home is the document's. */
export const HOME = mac ? "Meta+ArrowLeft" : "Home";

/** The line end key of a field and the editor: ⌘→ on macOS, where End is the document's. */
export const END = mac ? "Meta+ArrowRight" : "End";

/** The status bar's connection: the service's on macOS, where it runs natively, else WSL's. */
export const CONNECTION = mac ? "Service connection" : "WSL connection";
/** Where the service runs while it says nothing else (no `welcome` yet). */
export const PLACE = mac ? "macOS" : "WSL";

/** The terminal's copy and paste keys (#35): ⌘C / ⌘V on macOS. */
export const COPY = mac ? "Meta+C" : "Control+Shift+C";
export const PASTE = mac ? "Meta+V" : "Control+Shift+V";
