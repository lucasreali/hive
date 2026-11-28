import type { Keymap } from "./protocol";
import { useHive } from "./store";
import { isMac } from "./window";

// The keymap (28.7, `docs/spike/keymap.md`): which chord runs which app command. The service
// owns the file and its grammar (`hive_protocol::keymap`: it normalises the valid entries and
// names the problems); this side keeps what is presentation (#37): each command's defaults
// (#35), the resolver that matches a key to a command, the conflicts and the recorder. The one
// duplicated piece, the grammar, is checked against the samples the Rust tests run
// (`crates/hive-protocol/tests/chords.json`).

/** Whose keys a chord is written for: `Cmd` exists only on macOS. */
export type Platform = "mac" | "windows";

/** The platform Hive runs on, asked on every call (`isMac`). */
export const platform = (): Platform => (isMac() ? "mac" : "windows");

/**
 * Where a command runs: anywhere, or only while a terminal (or a Markdown file) is shown, its
 * chord left to the terminal otherwise. `terminal` and `markdown` never meet.
 */
export type Context = "any" | "terminal" | "markdown";

/** What the keymap needs of an app command (`COMMANDS` in `src/shortcuts.ts`). */
export type Bindable = {
  id: string;
  label: string;
  context: Context;
  /** #35's chords on each platform, in the keymap's form; the first is the one shown. */
  defaults: { windows: string[]; mac: string[] };
};

/** One chord: its modifiers and one key, as `hive_protocol::keymap::Chord`. */
export type Chord = { ctrl: boolean; alt: boolean; shift: boolean; cmd: boolean; key: string };

/** As `hive_protocol::keymap`'s limits. */
export const CHORD_LIMIT = 64;
export const CHORDS_LIMIT = 8;

/** The named keys a chord may end with, as they are written, besides letters, digits, F1–F24. */
const NAMED = [
  "Tab",
  "Enter",
  "Esc",
  "Space",
  "Backspace",
  "Delete",
  "Insert",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Up",
  "Down",
  "Left",
  "Right",
];

/** The punctuation keys of a US layout, each with its physical key (`KeyboardEvent.code`). */
const PUNCTUATION: Record<string, string> = {
  "`": "Backquote",
  "-": "Minus",
  "=": "Equal",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  ";": "Semicolon",
  "'": "Quote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
};

/** Each character Shift types on a US layout, with the key that types it. */
const SHIFTED: Record<string, string> = {
  "~": "`",
  "!": "1",
  "@": "2",
  "#": "3",
  $: "4",
  "%": "5",
  "^": "6",
  "&": "7",
  "*": "8",
  "(": "9",
  ")": "0",
  _: "-",
  "{": "[",
  "}": "]",
  "|": "\\",
  ":": ";",
  '"': "'",
  "<": ",",
  ">": ".",
  "?": "/",
};

/** `event.key` of each named key. */
const EVENT_KEYS: Record<string, string> = {
  Tab: "Tab",
  Enter: "Enter",
  Esc: "Escape",
  Space: " ",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Up: "ArrowUp",
  Down: "ArrowDown",
  Left: "ArrowLeft",
  Right: "ArrowRight",
};

/**
 * A key the keymap never takes (the spike's section 6), in the keymap's form, with what it is;
 * `except`: the one command whose default it is. The same list as
 * `hive_protocol::keymap::FIXED`, which a test checks.
 */
export type Fixed = { platform: Platform; chord: string; what: string; except?: string };

export const FIXED: readonly Fixed[] = [
  { platform: "mac", chord: "Cmd+C", what: "the terminal's Copy" },
  { platform: "mac", chord: "Cmd+V", what: "the terminal's Paste" },
  { platform: "mac", chord: "Cmd+K", what: "the terminal's Clear" },
  { platform: "mac", chord: "Cmd+Up", what: "the terminal's previous prompt" },
  { platform: "mac", chord: "Cmd+Down", what: "the terminal's next prompt" },
  { platform: "mac", chord: "Cmd+Left", what: "the terminal's start of the line" },
  { platform: "mac", chord: "Cmd+Right", what: "the terminal's end of the line" },
  { platform: "mac", chord: "Cmd+Backspace", what: "the terminal's Delete line" },
  { platform: "mac", chord: "Cmd+Q", what: "the menu's Quit" },
  { platform: "mac", chord: "Cmd+H", what: "the menu's Hide" },
  { platform: "mac", chord: "Alt+Cmd+H", what: "the menu's Hide Others" },
  { platform: "mac", chord: "Cmd+Z", what: "the menu's Undo" },
  { platform: "mac", chord: "Shift+Cmd+Z", what: "the menu's Redo" },
  { platform: "mac", chord: "Cmd+X", what: "the menu's Cut" },
  { platform: "mac", chord: "Cmd+A", what: "the menu's Select All" },
  { platform: "mac", chord: "Ctrl+Cmd+F", what: "the menu's Full Screen" },
  { platform: "mac", chord: "Cmd+M", what: "the menu's Minimize" },
  { platform: "windows", chord: "Ctrl+Shift+C", what: "the terminal's Copy" },
  {
    platform: "windows",
    chord: "Ctrl+Shift+V",
    what: "the terminal's Paste",
    except: "toggle-markdown",
  },
];

/** `text` with only its ASCII letters lowercased, as Rust's `to_ascii_lowercase`. */
const asciiLower = (text: string) => text.replace(/[A-Z]/g, (c) => c.toLowerCase());

/** Rust's `char::is_whitespace` (Unicode's White_Space), which `str::trim` takes away. */
const SPACE =
  "[\\t\\n\\v\\f\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000]";
const TRIM = new RegExp(`^${SPACE}+|${SPACE}+$`, "g");

/** `text` trimmed as Rust's `str::trim` trims it (JavaScript's `trim` also takes U+FEFF). */
const rustTrim = (text: string) => text.replace(TRIM, "");

/** The key `part` names, as it is written; null when it names none. */
function keyOf(part: string): string | null {
  if (/^[A-Za-z0-9]$/.test(part)) return part.toUpperCase();
  const f = /^[Ff]([1-9][0-9]?)$/.exec(part)?.[1];
  if (f && Number(f) <= 24) return `F${f}`;
  if (Object.hasOwn(PUNCTUATION, part)) return part;
  return NAMED.find((k) => asciiLower(k) === asciiLower(part)) ?? null;
}

/** `text` quoted as the service quotes it (cut at `CHORD_LIMIT` characters). */
const quote = (text: string) => JSON.stringify(text.slice(0, CHORD_LIMIT));

/** The chord written in the keymap's form: modifiers in macOS' order (⌃⌥⇧⌘), then the key. */
export function chordString(c: Chord): string {
  const names: [boolean, string][] = [
    [c.ctrl, "Ctrl+"],
    [c.alt, "Alt+"],
    [c.shift, "Shift+"],
    [c.cmd, "Cmd+"],
  ];
  return names.map(([on, name]) => (on ? name : "")).join("") + c.key;
}

export type Parsed = { chord: Chord; error?: undefined } | { chord?: undefined; error: string };

const MODIFIERS: Record<string, "ctrl" | "alt" | "shift" | "cmd"> = {
  ctrl: "ctrl",
  control: "ctrl",
  alt: "alt",
  option: "alt",
  opt: "alt",
  shift: "shift",
  cmd: "cmd",
  command: "cmd",
};
const MODIFIER_NAME = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", cmd: "Cmd" };

/** The modifier `part` names (any case, with its aliases), if it names one. */
function modifierOf(part: string) {
  const lower = asciiLower(part);
  return Object.hasOwn(MODIFIERS, lower) ? MODIFIERS[lower] : undefined;
}

/** A chord in the keymap's form (already parsed and normalised) taken apart. */
function split(chord: string): Chord {
  const parts = chord.split("+");
  const key = parts.pop() as string;
  const has = (name: string) => parts.includes(name);
  return { ctrl: has("Ctrl"), alt: has("Alt"), shift: has("Shift"), cmd: has("Cmd"), key };
}

/**
 * Parses `text` for the command `id` on `on`, as the service does (`Chord::parse` and the
 * fixed keys): the chord, or the problem with it as a sentence.
 */
export function parseChord(text: string, on: Platform, id = ""): Parsed {
  const quoted = quote(text);
  if (new TextEncoder().encode(text).length > CHORD_LIMIT) {
    return { error: `${quoted} is longer than ${CHORD_LIMIT} bytes` };
  }
  const bad = (why: string) => ({ error: `${quoted} is not a shortcut: ${why}` });
  const parts = text.split("+").map(rustTrim);
  if (parts.some((p) => p === "")) return bad("a key is missing after +");
  const chord: Chord = { ctrl: false, alt: false, shift: false, cmd: false, key: "" };
  const last = parts.pop() as string;
  for (const part of parts) {
    const modifier = modifierOf(part);
    if (!modifier) return bad(`${quote(part)} is not a modifier`);
    if (chord[modifier]) return bad(`${MODIFIER_NAME[modifier]} twice`);
    chord[modifier] = true;
  }
  const lastModifier = modifierOf(last);
  if (lastModifier) return bad(`a key is missing after ${MODIFIER_NAME[lastModifier]}`);
  const key = keyOf(last);
  if (!key) {
    const shifted = Object.hasOwn(SHIFTED, last) ? SHIFTED[last] : undefined;
    return bad(shifted ? `write it as Shift+${shifted}` : `${quote(last)} is not a key`);
  }
  chord.key = key;
  if (chord.cmd && on === "windows") return { error: `${quoted}: Cmd exists only on macOS` };
  const fn = key.length > 1 && key.startsWith("F");
  if (!chord.ctrl && !chord.alt && !chord.cmd && !fn) {
    const add = on === "mac" ? "Ctrl, Alt or Cmd" : "Ctrl or Alt";
    return { error: `${quoted} would take the key from the terminal: add ${add}` };
  }
  const written = chordString(chord);
  const fixed = FIXED.find((f) => f.platform === on && f.chord === written && f.except !== id);
  if (fixed) return { error: `${quoted} is ${fixed.what} and stays fixed` };
  return { chord };
}

/** A command's chords: the user's when the keymap has an entry for it, else its defaults. */
export type Binding = { chords: string[]; user: boolean };

/**
 * The entry's chords as written (at most `CHORDS_LIMIT`), or null when it is not a chord, a
 * list of chords or null (the service names that problem; the default applies).
 */
function written(value: unknown): string[] | null {
  if (value === null) return [];
  if (typeof value === "string") return [value];
  if (Array.isArray(value) && value.every((v) => typeof v === "string")) {
    return value.slice(0, CHORDS_LIMIT);
  }
  return null;
}

/** The keymap's entries, none when it is not an object (a hand edit may write anything). */
export const entries = (keymap: Keymap): Record<string, unknown> =>
  keymap !== null && typeof keymap === "object" && !Array.isArray(keymap)
    ? (keymap as Record<string, unknown>)
    : {};

/** Each command's effective chords on `on`: the user's valid ones (bad ones ignored), or its defaults. */
export function bindings(
  keymap: Keymap,
  commands: readonly Bindable[],
  on: Platform,
): Map<string, Binding> {
  const map = entries(keymap);
  return new Map<string, Binding>(
    commands.map((c): [string, Binding] => {
      const own = Object.hasOwn(map, c.id) ? written(map[c.id]) : null;
      if (own === null) return [c.id, { chords: c.defaults[on], user: false }];
      const valid = own.flatMap((t) => {
        const parsed = parseChord(t, on, c.id);
        return parsed.chord ? [chordString(parsed.chord)] : [];
      });
      // Every chord bad: the default applies, as for the service and the macOS menu.
      if (own.length > 0 && valid.length === 0) {
        return [c.id, { chords: c.defaults[on], user: false }];
      }
      return [c.id, { chords: [...new Set(valid)], user: true }];
    }),
  );
}

/** Whether `event` presses `chord` (in the keymap's form): its modifiers exactly, and its key. */
export function presses(chord: string, event: KeyboardEvent): boolean {
  const c = split(chord);
  const modifiers =
    event.ctrlKey === c.ctrl &&
    event.altKey === c.alt &&
    event.shiftKey === c.shift &&
    event.metaKey === c.cmd;
  if (!modifiers) return false;
  if (Object.hasOwn(EVENT_KEYS, c.key)) return event.key === EVENT_KEYS[c.key];
  if (event.key.toUpperCase() === c.key) return true;
  // Shift or the layout changes punctuation and digits ("{" for ⇧[), Option a letter (π for
  // ⌥P): those also match by their physical key.
  const code = /^[0-9]$/.test(c.key) ? `Digit${c.key}` : PUNCTUATION[c.key];
  return event.code === (code ?? (c.alt ? `Key${c.key}` : undefined));
}

/** The key a physical key (`event.code`) is on a US layout: a letter, a digit or punctuation. */
const physicalKey = (code: string): string | undefined =>
  /^Key([A-Z])$/.exec(code)?.[1] ??
  /^Digit([0-9])$/.exec(code)?.[1] ??
  Object.keys(PUNCTUATION).find((k) => PUNCTUATION[k] === code);

/**
 * The chord a key press makes (the recorder): its modifiers, and the key: a letter as the
 * layout types it (`event.key`, AZERTY's A is `A`, as `presses` matches it), else by its
 * physical place (Option+P is `Alt+P`, not π; ⇧[ is `Shift+[`), else by name. Null for a lone
 * modifier or a key the keymap has no name for.
 */
export function chordOf(event: KeyboardEvent): string | null {
  const letter = /^[A-Za-z]$/.test(event.key) ? event.key.toUpperCase() : undefined;
  const physical = letter ?? physicalKey(event.code);
  const named = Object.keys(EVENT_KEYS).find((k) => EVENT_KEYS[k] === event.key);
  const fn = /^F[0-9]+$/.test(event.key) ? keyOf(event.key) : null;
  const key = physical ?? named ?? fn;
  if (!key) return null;
  return chordString({
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    cmd: event.metaKey,
    key,
  });
}

/**
 * Whether `event` types a character with AltGr (Windows, where AltGr is Ctrl+Alt): `@` on a
 * German Q, `€` on E. Such a key is the field's or the terminal's, never a shortcut: a printable
 * key that is not the one its physical key names (Ctrl+Alt+K types k). Not `AltGraph`: Windows
 * may report it for a plain Ctrl+Alt too.
 */
export function altGr(event: KeyboardEvent): boolean {
  if (isMac() || !event.ctrlKey || !event.altKey || event.metaKey) return false;
  if ([...event.key].length !== 1) return false;
  const key = event.key.toUpperCase();
  const physical = physicalKey(event.code);
  if (Object.values(EVENT_KEYS).includes(event.key)) return false;
  return physical === undefined || (key !== physical && SHIFTED[key] !== physical);
}

/** Each key's symbol on macOS. */
const MAC_KEYS: Record<string, string> = {
  Up: "↑",
  Down: "↓",
  Left: "←",
  Right: "→",
  Enter: "↩",
  Backspace: "⌫",
  Delete: "⌦",
  Esc: "esc",
};

/** A chord as this platform shows it: `Shift+Cmd+P` is ⇧⌘P on macOS, as written elsewhere. */
export function chordText(chord: string): string {
  if (!isMac()) return chord;
  const c = split(chord);
  const symbols = `${c.ctrl ? "⌃" : ""}${c.alt ? "⌥" : ""}${c.shift ? "⇧" : ""}${c.cmd ? "⌘" : ""}`;
  return symbols + (MAC_KEYS[c.key] ?? c.key);
}

/** Whether two contexts can hold at once. */
export const overlap = (a: Context, b: Context) => a === "any" || b === "any" || a === b;

/**
 * Which of two commands sharing a chord in overlapping contexts runs: a user's chord beats a
 * default (the user just chose it), then table order.
 */
export function wins(a: Bindable, b: Bindable, of: Map<string, Binding>, order: Bindable[]) {
  const [ua, ub] = [of.get(a.id)?.user ?? false, of.get(b.id)?.user ?? false];
  if (ua !== ub) return ua;
  return order.indexOf(a) < order.indexOf(b);
}

/** A conflict on one command's row: the chord, the other command, and whether this one runs. */
export type Conflict = { chord: string; other: Bindable; runs: boolean };

/** Every conflict of each command (by id): an equal chord of another one whose context overlaps. */
export function conflicts(
  commands: readonly Bindable[],
  of: Map<string, Binding>,
): Map<string, Conflict[]> {
  const order = [...commands];
  const found = new Map<string, Conflict[]>();
  for (const a of commands) {
    for (const b of commands) {
      if (a === b || !overlap(a.context, b.context)) continue;
      const theirs = of.get(b.id)?.chords ?? [];
      for (const chord of of.get(a.id)?.chords ?? []) {
        if (!theirs.includes(chord)) continue;
        const list = found.get(a.id) ?? [];
        list.push({ chord, other: b, runs: wins(a, b, of, order) });
        found.set(a.id, list);
      }
    }
  }
  return found;
}

/**
 * The keymap with `id` given `chords`: its entry removed when they are its defaults again
 * (only changes are kept), written otherwise (one chord as a text).
 */
export function withChords(
  keymap: Keymap,
  command: Bindable,
  chords: string[],
  on: Platform,
): Record<string, unknown> {
  const { [command.id]: _, ...rest } = entries(keymap);
  const same =
    chords.length === command.defaults[on].length &&
    chords.every((c, i) => c === command.defaults[on][i]);
  if (same) return rest;
  return { ...rest, [command.id]: chords.length === 1 ? chords[0] : chords };
}

/**
 * The keymap's problems as the service words them (`hive_protocol::keymap::check`), kept to what
 * the mock service needs: unknown commands, values of the wrong type and refused chords.
 */
export function keymapProblems(keymap: Keymap, on: Platform): string[] {
  return Object.entries(entries(keymap)).flatMap(([id, value]) => {
    if (!COMMAND_IDS.includes(id)) return [`keymap: Hive has no command ${quote(id)}`];
    const texts = written(value);
    if (texts === null) {
      return [`keymap.${id}: a shortcut is a text like "Ctrl+Alt+M" or a list of them`];
    }
    return texts.flatMap((t) => parseChord(t, on, id).error ?? []).map((p) => `keymap.${id}: ${p}`);
  });
}

/** No default on one platform. */
const none: string[] = [];

/**
 * Every app command the keymap knows, in table order (the order Settings → Keyboard and the
 * palette list them): `COMMANDS` (`src/shortcuts.ts`) gives each its action. `shortcut()` runs
 * them by their chords (the keymap's, else these defaults, #35). Each id is one of
 * `hive_protocol::keymap::COMMAND_IDS`, which a test checks. On macOS the defaults are Cmd's
 * (D17.2, 17.14, 18.34, D18.10); the commands that had keys only there have none on Windows,
 * where those keys are the shell's.
 */
export const COMMAND_TABLE: readonly Bindable[] = [
  {
    id: "palette",
    label: "Command palette",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+P"], mac: ["Shift+Cmd+P"] },
  },
  {
    id: "settings",
    label: "Open settings",
    context: "any",
    defaults: { windows: ["Ctrl+,"], mac: ["Cmd+,"] },
  },
  {
    // Delta's ⌘/ (28.7); ⇧⌘/ is macOS' Help search.
    id: "keyboard-settings",
    label: "Keyboard shortcuts",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+/"], mac: ["Cmd+/"] },
  },
  {
    id: "worktree-picker",
    label: "New terminal in a worktree",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+T"], mac: ["Shift+Cmd+T"] },
  },
  {
    id: "new-terminal",
    label: "New terminal in this worktree",
    context: "any",
    defaults: { windows: none, mac: ["Cmd+T"] },
  },
  {
    id: "new-worktree",
    label: "New worktree",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+N"], mac: ["Cmd+N", "Shift+Cmd+N"] },
  },
  {
    id: "split-terminal",
    label: "Split terminal",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+D"], mac: ["Shift+Cmd+D"] },
  },
  {
    id: "toggle-panel",
    label: "Show or hide the side panel's view",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+B"], mac: ["Shift+Cmd+B"] },
  },
  {
    id: "add-project",
    label: "Add project",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+O"], mac: ["Shift+Cmd+O"] },
  },
  {
    id: "send-reference",
    label: "Send the selected lines' reference to the agent",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+L"], mac: ["Shift+Cmd+L"] },
  },
  {
    id: "comment",
    label: "Comment on the selected lines",
    context: "any",
    defaults: { windows: ["Ctrl+Shift+M"], mac: ["Shift+Cmd+M"] },
  },
  {
    // F8 is Play/Pause on a Mac's keyboard unless Fn is held (D17.2).
    id: "next-pending",
    label: "Go to the next pending agent",
    context: "any",
    defaults: { windows: ["F8"], mac: ["Shift+Cmd+J", "F8"] },
  },
  {
    // 17.21: ⌘F on macOS (D17.2); elsewhere Ctrl+F is the shell's, so #35's Ctrl+Shift+F.
    id: "find-in-terminal",
    label: "Search the terminal",
    context: "terminal",
    defaults: { windows: ["Ctrl+Shift+F"], mac: ["Cmd+F", "Shift+Cmd+F"] },
  },
  {
    // 11.2: VS Code's key, the terminal's paste everywhere but on a Markdown file's tab.
    id: "toggle-markdown",
    label: "Show a Markdown file rendered or as text",
    context: "markdown",
    defaults: { windows: ["Ctrl+Shift+V"], mac: ["Shift+Cmd+V"] },
  },
  {
    id: "close-tab",
    label: "Close the tab",
    context: "any",
    defaults: { windows: none, mac: ["Cmd+W"] },
  },
  {
    id: "next-tab",
    label: "Show the next tab",
    context: "any",
    defaults: { windows: none, mac: ["Ctrl+Tab", "Shift+Cmd+]"] },
  },
  {
    id: "previous-tab",
    label: "Show the previous tab",
    context: "any",
    defaults: { windows: none, mac: ["Ctrl+Shift+Tab", "Shift+Cmd+["] },
  },
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map(
    (n): Bindable => ({
      id: `tab-${n}`,
      label: `Show tab ${n}`,
      context: "any",
      defaults: { windows: none, mac: [`Cmd+${n}`] },
    }),
  ),
  {
    id: "font-bigger",
    label: "Make the terminal font bigger",
    context: "any",
    defaults: { windows: none, mac: ["Cmd+="] },
  },
  {
    id: "font-smaller",
    label: "Make the terminal font smaller",
    context: "any",
    defaults: { windows: none, mac: ["Cmd+-"] },
  },
  {
    id: "font-reset",
    label: "Terminal font at the settings' size",
    context: "any",
    defaults: { windows: none, mac: ["Cmd+0"] },
  },
];

/** Every command id, as the service checks the keymap's. */
export const COMMAND_IDS: readonly string[] = COMMAND_TABLE.map((c) => c.id);

/** The last bindings worked out, for the keymap and platform they came from. */
let cached: { keymap: Keymap; on: Platform; of: Map<string, Binding> } | null = null;

/** Each command's effective chords now (`bindings`), worked out again only when they change. */
export function effective(): Map<string, Binding> {
  const keymap = useHive.getState().settings.keymap;
  const on = platform();
  if (cached === null || cached.keymap !== keymap || cached.on !== on) {
    cached = { keymap, on, of: bindings(keymap, COMMAND_TABLE, on) };
  }
  return cached.of;
}

/** The command `id`'s effective chords, the first one shown. */
export const commandChords = (id: string): string[] => effective().get(id)?.chords ?? [];

/** The text of the command `id`'s first chord as this platform names it; "" with none. */
export function shortcutText(id: string): string {
  const first = commandChords(id)[0];
  return first ? chordText(first) : "";
}

/** `text` with the command `id`'s chord after it in brackets, when it has one (tooltips). */
export function withKeys(text: string, id: string): string {
  const keys = shortcutText(id);
  return keys ? `${text} (${keys})` : text;
}

/** Shows the calling component again when the keymap changes (its `withKeys` texts). */
export function useKeymap(): void {
  useHive((s) => s.settings.keymap);
}

/** `shortcutText` in a component, shown again when the keymap changes. */
export function useShortcut(id: string): string {
  useHive((s) => s.settings.keymap);
  return shortcutText(id);
}
