import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import { addTab, removeTab, useHive } from "./store";
import { transport } from "./transport";

// xterm.js lives here, outside React (#30): one Terminal per terminal id, fed straight from
// the transport. React renders the host element and says which tab is shown; output never
// goes through React state. Only the shown terminal is rendered, with WebGL (#28); hidden
// ones keep parsing output into their buffer and cost no rendering.

/** Colors from the design tokens in `src/styles.css`; cyan is One Dark's. */
const THEME: ITheme = {
  background: "#282c33",
  foreground: "#dce0e5",
  cursor: "#dce0e5",
  cursorAccent: "#282c33",
  selectionBackground: "#454a56",
  scrollbarSliderBackground: "#454a56",
  scrollbarSliderHoverBackground: "#4f5563",
  scrollbarSliderActiveBackground: "#4f5563",
  black: "#3b414d",
  red: "#d07277",
  green: "#a1c181",
  yellow: "#dec184",
  blue: "#74ade8",
  magenta: "#b477cf",
  cyan: "#6eb4bf",
  white: "#a9afbc",
  brightBlack: "#5d636f",
  brightRed: "#d07277",
  brightGreen: "#a1c181",
  brightYellow: "#dec184",
  brightBlue: "#74ade8",
  brightMagenta: "#b477cf",
  brightCyan: "#6eb4bf",
  brightWhite: "#dce0e5",
};

/** How long the host must keep its size before terminals are refitted and the PTY resized. */
export const RESIZE_DEBOUNCE_MS = 50;

type Entry = { term: Terminal; fit: FitAddon; el: HTMLDivElement; webgl: WebglAddon | null };

const entries = new Map<number, Entry>();
let host: HTMLElement | null = null;
let shown: number | null = null;
let intercept: (event: KeyboardEvent) => boolean = () => false;

/**
 * App shortcuts (1.9): `handler` sees every key event of a focused terminal first; returning
 * true keeps the event from the terminal.
 */
export function interceptKeys(handler: (event: KeyboardEvent) => boolean): void {
  intercept = handler;
}

/** The xterm instance of a terminal, e.g. to read its buffer. */
export const terminal = (id: number): Terminal | undefined => entries.get(id)?.term;

/**
 * Opens a terminal in `cwd` (a worktree path) and adds its tab, shown. The Terminal exists
 * before the service is asked, so no output is lost before the tab appears.
 */
export async function openTerminal(cwd: string): Promise<number> {
  const term = new Terminal({
    scrollback: useHive.getState().scrollback,
    fontFamily: '"IBM Plex Mono", monospace',
    fontSize: 13,
    lineHeight: 1.2,
    theme: THEME,
  });
  let id: number;
  try {
    id = await transport.openTerminal(cwd, term.cols, term.rows, (bytes) => term.write(bytes));
  } catch (error) {
    term.dispose();
    throw error;
  }
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.onData((data) => {
    if (!useHive.getState().terminals[id]?.exited) void transport.writeTerminal(id, data);
  });
  term.onResize(({ cols, rows }) => void transport.resizeTerminal(id, cols, rows));
  term.attachCustomKeyEventHandler((event) => keys(term, event));
  const el = document.createElement("div");
  el.className = "terminal-pane";
  el.hidden = true;
  entries.set(id, { term, fit, el, webgl: null });
  addTab(id, cwd);
  return id;
}

/**
 * A new "chat": a terminal in `cwd` with `claude` (and `args`, e.g. `--resume <id>`) typed
 * into it as the user would; from then on Hive only observes it.
 */
export async function openClaude(cwd: string, args = ""): Promise<number> {
  const id = await openTerminal(cwd);
  await transport.writeTerminal(id, `claude${args && ` ${args}`}\r`);
  return id;
}

/** Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes (#35); every other key is the shell's. */
function keys(term: Terminal, event: KeyboardEvent): boolean {
  if (intercept(event)) return false;
  const chord = event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey;
  const key = event.key.toUpperCase();
  if (event.type !== "keydown" || !chord || (key !== "C" && key !== "V")) return true;
  // The browser's own Ctrl+Shift+V would paste a second time.
  event.preventDefault();
  if (key === "C") {
    if (term.hasSelection()) void navigator.clipboard.writeText(term.getSelection());
  } else {
    void navigator.clipboard.readText().then((text) => term.paste(text));
  }
  return false;
}

/** Ends the terminal (unless it already exited), drops its xterm instance and its tab. */
export function closeTerminal(id: number): void {
  if (!useHive.getState().terminals[id]?.exited) void transport.closeTerminal(id);
  const entry = entries.get(id);
  entry?.term.dispose();
  entry?.el.remove();
  entries.delete(id);
  if (shown === id) shown = null;
  removeTab(id);
}

/** Shows terminal `id` in the host (hiding the previous one), or none. */
export function showTerminal(id: number | null): void {
  const previous = shown === null ? undefined : entries.get(shown);
  if (previous && shown !== id) hide(previous);
  shown = id;
  const entry = id === null ? undefined : entries.get(id);
  if (!entry || !host) return;
  if (entry.el.parentElement !== host) host.append(entry.el);
  entry.el.hidden = false;
  if (!entry.term.element) entry.term.open(entry.el);
  if (!entry.webgl) entry.webgl = webgl(entry);
  entry.fit.fit();
  entry.term.focus();
}

function hide(entry: Entry): void {
  entry.webgl?.dispose();
  entry.webgl = null;
  entry.el.hidden = true;
}

/** The WebGL renderer, or null when WebGL is unavailable; a lost context falls back to DOM. */
function webgl(entry: Entry): WebglAddon | null {
  try {
    const addon = new WebglAddon();
    addon.onContextLoss(() => {
      addon.dispose();
      if (entry.webgl === addon) entry.webgl = null;
    });
    entry.term.loadAddon(addon);
    return addon;
  } catch {
    return null;
  }
}

/**
 * Makes `element` the place terminals render in and follows its size. Returns the cleanup.
 */
export function mountTerminals(element: HTMLElement): () => void {
  host = element;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const observer = new ResizeObserver(() => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const entry = shown === null ? undefined : entries.get(shown);
      entry?.fit.fit();
    }, RESIZE_DEBOUNCE_MS);
  });
  observer.observe(element);
  showTerminal(shown);
  return () => {
    clearTimeout(timer);
    observer.disconnect();
    if (host === element) host = null;
  };
}
