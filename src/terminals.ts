import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { type ITerminalOptions, type ITheme, Terminal } from "@xterm/xterm";
import LIGATURES from "./assets/fonts/ligatures.json";
import {
  addTab,
  focusPane,
  removeTab,
  type Settings,
  setSplit,
  shownSplit,
  tabPlace,
  useHive,
} from "./store";
import { transport } from "./transport";
import { commandKey, isMac } from "./window";

// xterm.js lives here, outside React (#30): one Terminal per terminal id, fed straight from
// the transport. React renders the host element and says which tab is shown; output never
// goes through React state. Only the shown terminals (one, or two side by side) are rendered,
// with WebGL (#28); hidden ones keep parsing output into their buffer and cost no rendering.

/** Colors from the design tokens in `src/styles.css`; cyan is One Dark's. */
const ONE_DARK: ITheme = {
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

/** Zed's One Light terminal colors, beside the `one-light` tokens in `src/styles.css`. */
const ONE_LIGHT: ITheme = {
  background: "#fafafa",
  foreground: "#2a2c33",
  cursor: "#5c78e2",
  cursorAccent: "#fafafa",
  selectionBackground: "#5c78e23d",
  scrollbarSliderBackground: "#383a414c",
  scrollbarSliderHoverBackground: "#383a4180",
  scrollbarSliderActiveBackground: "#383a4180",
  black: "#000000",
  red: "#de3e35",
  green: "#3f953a",
  yellow: "#d2b67c",
  blue: "#2f5af3",
  magenta: "#950095",
  cyan: "#0997b3",
  white: "#bbbbbb",
  brightBlack: "#000000",
  brightRed: "#de3e35",
  brightGreen: "#3f953a",
  brightYellow: "#d2b67c",
  brightBlue: "#2f5af3",
  brightMagenta: "#a00095",
  brightCyan: "#0bbcd6",
  brightWhite: "#ffffff",
};

/** The xterm options the settings give (6.2): used by new terminals and live by open ones. */
export function termOptions({ terminal: t, appearance }: Settings): ITerminalOptions {
  return {
    fontFamily: t.font_family,
    fontSize: t.font_size,
    scrollback: t.scrollback,
    cursorStyle: t.cursor_style,
    cursorBlink: t.cursor_blink,
    theme: appearance.theme === "one-light" ? ONE_LIGHT : ONE_DARK,
  };
}

/**
 * Ligatures (7.11): xterm draws each cell alone, so a font's ligatures never show unless a
 * character joiner hands the WebGL renderer the sequence as one unit. The list is the one the
 * bundled Hive Mono has ligatures for (scripts/build-terminal-font.sh); longest first.
 */
export function ligatureRanges(text: string): [number, number][] {
  const ranges: [number, number][] = [];
  let i = 0;
  while (i < text.length) {
    const found = LIGATURES.find((sequence) => text.startsWith(sequence, i));
    if (found) ranges.push([i, i + found.length]);
    i += found ? found.length : 1;
  }
  return ranges;
}

/**
 * The bundled faces load on first use, and xterm measures its cells and caches its glyphs
 * with whatever font is ready then: they are loaded before the first terminal is made. Null
 * once they are (or without a FontFaceSet): nothing to wait for.
 */
const BUNDLED_FACES = ['1em "Hive Mono"', 'bold 1em "Hive Mono"', '1em "Symbols Nerd Font"'];
let fontsReady = false;
let fontsLoading: Promise<void> | null = null;
function bundledFonts(): Promise<void> | null {
  if (fontsReady || !("fonts" in document)) return null;
  fontsLoading ??= Promise.all(
    BUNDLED_FACES.map((face) => document.fonts.load(face).catch(() => [])),
  ).then(() => {
    fontsReady = true;
  });
  return fontsLoading;
}

/** How long the host must keep its size before terminals are refitted and the PTY resized. */
export const RESIZE_DEBOUNCE_MS = 50;

type Entry = { term: Terminal; fit: FitAddon; el: HTMLDivElement; webgl: WebglAddon | null };

const entries = new Map<number, Entry>();
let host: HTMLElement | null = null;
let shown: number[] = [];
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
 * Pastes `text` into terminal `id` (one that was shown) as Ctrl+Shift+V does: bracketed when
 * the program in it asked for that, so its newlines do not submit a prompt.
 */
export const pasteToTerminal = (id: number, text: string) => terminal(id)?.paste(text);

/**
 * Opens a terminal in `cwd` (a worktree path) and adds its tab, shown. The Terminal exists
 * before the service is asked, so no output is lost before the tab appears.
 */
export async function openTerminal(cwd: string): Promise<number> {
  const fonts = bundledFonts();
  if (fonts) await fonts;
  const term = new Terminal({
    lineHeight: 1.2,
    allowProposedApi: true, // registerCharacterJoiner
    ...termOptions(useHive.getState().settings),
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
  term.onSelectionChange(() => {
    const copy = useHive.getState().settings.terminal.copy_on_select;
    if (copy && term.hasSelection()) void navigator.clipboard.writeText(term.getSelection());
  });
  const el = document.createElement("div");
  el.className = "terminal-pane";
  el.hidden = true;
  el.addEventListener("focusin", () => focusPane(id));
  entries.set(id, { term, fit, el, webgl: null });
  addTab(id, cwd);
  return id;
}

/**
 * A new "chat": a terminal in `cwd` with `claude` (and `args`, e.g. `--resume <id>`) typed
 * into it as the user would; from then on Hive only observes it.
 */
export const openClaude = (cwd: string, args = ""): Promise<number> =>
  openWith(cwd, `claude${args && ` ${args}`}`);

/** A terminal in `cwd` with `command` typed into it and Enter pressed, as the user would. */
export async function openWith(cwd: string, command: string): Promise<number> {
  const id = await openTerminal(cwd);
  await transport.writeTerminal(id, `${command}\r`);
  return id;
}

/**
 * Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes (#35); on macOS Cmd+C and Cmd+V, and
 * Ctrl+C stays the shell's. Every other key is the shell's.
 */
function keys(term: Terminal, event: KeyboardEvent): boolean {
  if (intercept(event)) return false;
  const chord = commandKey(event) && event.shiftKey !== isMac() && !event.altKey;
  const key = event.key.toUpperCase();
  if (event.type !== "keydown" || !chord || (key !== "C" && key !== "V")) return true;
  // The browser's own paste (Ctrl+Shift+V, Cmd+V) would paste a second time.
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
  shown = shown.filter((s) => s !== id);
  removeTab(id);
}

/** Shows terminal `id` alone in the host (hiding the others), or none. */
export const showTerminal = (id: number | null) => showTerminals(id === null ? [] : [id], id);

/**
 * Shows `ids` in the host, left to right (two make a split), hiding every other terminal, and
 * gives `focus` the keyboard.
 */
export function showTerminals(ids: number[], focus: number | null): void {
  for (const id of shown) {
    const entry = entries.get(id);
    if (entry && !ids.includes(id)) hide(entry);
  }
  shown = ids;
  if (!host) return;
  for (const [i, id] of ids.entries()) {
    const entry = entries.get(id);
    if (!entry) continue;
    if (entry.el.parentElement !== host) host.append(entry.el);
    entry.el.dataset.pane = ids.length > 1 ? (i === 0 ? "left" : "right") : "";
    entry.el.hidden = false;
    if (!entry.term.element) {
      entry.term.open(entry.el);
      entry.term.registerCharacterJoiner(ligatureRanges);
    }
    if (!entry.webgl) entry.webgl = webgl(entry);
    entry.fit.fit();
  }
  if (focus !== null) entries.get(focus)?.term.focus();
}

/** Refits every shown terminal to its pane. */
function fitShown(): void {
  for (const id of shown) entries.get(id)?.fit.fit();
}

/**
 * Ctrl+Shift+D and the tab menu's "Split right" (6.11): tab `id` beside the next tab of its
 * worktree, or beside a new terminal there when it has no other; when `id` is already a
 * shown pane, the split ends instead.
 */
export async function splitTerminal(id: number | null): Promise<void> {
  const s = useHive.getState();
  const split = shownSplit(s);
  if (split && (split.left === id || split.right === id)) return setSplit(null);
  // Chats (7.3) are not terminals: they take no part in a split.
  const terminals = s.tabs.filter((t) => t.kind !== "chat");
  const tab = terminals.find((t) => t.id === id);
  if (!tab) return;
  const place = tabPlace(s, tab.cwd);
  const same = terminals.filter((t) => tabPlace(s, t.cwd) === place);
  const next = same[(same.indexOf(tab) + 1) % same.length] as typeof tab;
  const right = next === tab ? await openTerminal(tab.cwd) : next.id;
  setSplit({ left: tab.id, right });
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

/** New settings apply to every open terminal at once; the shown ones refit to the new font. */
function applySettings(settings: Settings): void {
  const options = termOptions(settings);
  for (const entry of entries.values()) entry.term.options = options;
  fitShown();
}

/**
 * Makes `element` the place terminals render in, follows its size (and the split's divider)
 * and applies new settings. Returns the cleanup.
 */
export function mountTerminals(element: HTMLElement): () => void {
  host = element;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const refit = () => {
    clearTimeout(timer);
    timer = setTimeout(fitShown, RESIZE_DEBOUNCE_MS);
  };
  const unsubscribe = useHive.subscribe((s, prev) => {
    if (s.settings !== prev.settings) applySettings(s.settings);
    if (s.splitPercent !== prev.splitPercent) refit();
  });
  const observer = new ResizeObserver(refit);
  observer.observe(element);
  showTerminals(shown, null);
  return () => {
    clearTimeout(timer);
    observer.disconnect();
    unsubscribe();
    if (host === element) host = null;
  };
}
