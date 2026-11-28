import { expect, test } from "bun:test";
import samples from "../crates/hive-protocol/tests/chords.json";
import { asMac } from "../test/mac";
import {
  type Bindable,
  bindings,
  COMMAND_IDS,
  chordOf,
  chordString,
  chordText,
  conflicts,
  entries,
  FIXED,
  keymapProblems,
  overlap,
  parseChord,
  platform,
  presses,
  withChords,
} from "./keymap";
import { COMMANDS } from "./shortcuts";

const RUST = `${import.meta.dir}/../crates/hive-protocol/src/keymap.rs`;

/** `text` parsed for `id` on each platform, as the keymap writes it; null when refused. */
const parsed = (text: string, on: "mac" | "windows", id?: string) => {
  const p = parseChord(text, on, id);
  return p.chord ? chordString(p.chord) : null;
};

test("the shared samples parse as the service parses them", () => {
  expect(samples.length).toBeGreaterThan(20);
  for (const s of samples as { text: string; id?: string; mac: string; windows: string }[]) {
    expect([s.text, parsed(s.text, "mac", s.id), parsed(s.text, "windows", s.id)]).toEqual([
      s.text,
      s.mac,
      s.windows,
    ]);
  }
});

test("the fixed keys and the command ids are the service's", async () => {
  const rust = await Bun.file(RUST).text();
  const fixed = [
    ...(rust.split("pub const FIXED")[1]?.split("\n];")[0] ?? "").matchAll(
      /fixed\(\s*(Mac|Windows),\s*"([^"]+)",\s*"([^"]+)",\s*(None|Some\("([^"]+)"\)),?\s*\)/g,
    ),
  ].map(([, on, chord, what, , except]) => ({
    platform: on === "Mac" ? "mac" : "windows",
    chord,
    what: what?.replaceAll("\\'", "'"),
    ...(except ? { except } : {}),
  }));
  expect(fixed).toEqual([...FIXED]);
  const list = /pub const COMMAND_IDS: &\[&str\] = &\[([^\]]*)\];/.exec(rust)?.[1] ?? "";
  const ids = [...list.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  expect(ids).toEqual([...COMMAND_IDS].sort());
  expect(COMMANDS.map((c) => c.id)).toEqual([...COMMAND_IDS]);
});

test("every default is a valid chord of its platform, written as the keymap writes it", () => {
  for (const c of COMMANDS) {
    for (const on of ["mac", "windows"] as const) {
      for (const chord of c.defaults[on])
        expect([c.id, parsed(chord, on, c.id)]).toEqual([c.id, chord]);
    }
  }
});

test("a bad chord says what is wrong, as the service does", () => {
  const error = (text: string, on: "mac" | "windows" = "mac") => parseChord(text, on).error;
  expect(error("Cmd+Shift+")).toBe('"Cmd+Shift+" is not a shortcut: a key is missing after +');
  expect(error("Hyper+P")).toBe('"Hyper+P" is not a shortcut: "Hyper" is not a modifier');
  // Not one of the object's own names either.
  expect(error("constructor+P")).toBe(
    '"constructor+P" is not a shortcut: "constructor" is not a modifier',
  );
  expect(error("Ctrl+constructor")).toBe(
    '"Ctrl+constructor" is not a shortcut: "constructor" is not a key',
  );
  expect(error("Cmd+Cmd+P")).toBe('"Cmd+Cmd+P" is not a shortcut: Cmd twice');
  expect(error("Cmd+Shift")).toBe('"Cmd+Shift" is not a shortcut: a key is missing after Shift');
  expect(error("Cmd+{")).toBe('"Cmd+{" is not a shortcut: write it as Shift+[');
  expect(error("Cmd+P", "windows")).toBe('"Cmd+P": Cmd exists only on macOS');
  expect(error("M")).toBe('"M" would take the key from the terminal: add Ctrl, Alt or Cmd');
  expect(error("M", "windows")).toBe('"M" would take the key from the terminal: add Ctrl or Alt');
  expect(error("Cmd+C")).toBe(`"Cmd+C" is the terminal's Copy and stays fixed`);
  expect(error(`Cmd+${"x".repeat(64)}`)).toBe(`"Cmd+${"x".repeat(60)}" is longer than 64 bytes`);
  expect(parsed("Ctrl+Shift+V", "windows", "toggle-markdown")).toBe("Ctrl+Shift+V");
  expect(parsed("Ctrl+Shift+V", "windows", "palette")).toBeNull();
});

const command = (id: string, context: Bindable["context"], windows: string[]): Bindable => ({
  id,
  label: id,
  context,
  defaults: { windows, mac: [] },
});
const A = command("a", "any", ["Ctrl+Shift+A"]);
const B = command("b", "any", ["F8", "Ctrl+Alt+B"]);
const T = command("t", "terminal", ["Ctrl+Shift+T"]);
const M = command("m", "markdown", ["Ctrl+Shift+T"]);

test("each command's chords are the user's valid ones, else its defaults", () => {
  const of = (keymap: unknown) => bindings(keymap, [A, B], "windows");
  expect(of({}).get("a")).toEqual({ chords: ["Ctrl+Shift+A"], user: false });
  expect(of({ a: "ctrl+k" }).get("a")).toEqual({ chords: ["Ctrl+K"], user: true });
  expect(of({ a: ["Ctrl+K", "ctrl+k", "Hyper+P", "Ctrl+Shift+C"] }).get("a")).toEqual({
    chords: ["Ctrl+K"],
    user: true,
  });
  expect(of({ a: [] }).get("a")).toEqual({ chords: [], user: true });
  expect(of({ a: null }).get("a")).toEqual({ chords: [], user: true });
  // A value of the wrong type is the service's problem: the default applies.
  expect(of({ a: 3, b: ["F9", 1] }).get("a")).toEqual({ chords: ["Ctrl+Shift+A"], user: false });
  expect(of({ b: ["F9", 1] }).get("b")).toEqual({ chords: ["F8", "Ctrl+Alt+B"], user: false });
  // At most 8 chords are read.
  const nine = [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => `Ctrl+${n}`);
  expect(of({ a: nine }).get("a")?.chords).toEqual(nine.slice(0, 8));
  // A keymap that is not an object has no entries.
  for (const odd of [null, "x", [1], 3]) expect(of(odd).get("a")?.user).toBe(false);
  expect(entries({ a: 1 })).toEqual({ a: 1 });
  expect(bindings({}, [A], "mac").get("a")?.chords).toEqual([]);
});

const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

test("a chord is pressed with exactly its modifiers and its key", () => {
  expect(presses("Ctrl+Shift+P", key({ key: "P", ctrlKey: true, shiftKey: true }))).toBe(true);
  expect(presses("Ctrl+Shift+P", key({ key: "p", ctrlKey: true, shiftKey: true }))).toBe(true);
  for (const extra of [{ altKey: true }, { metaKey: true }, { shiftKey: false }]) {
    const init = { key: "P", ctrlKey: true, shiftKey: true, ...extra };
    expect(presses("Ctrl+Shift+P", key(init))).toBe(false);
  }
  expect(presses("Shift+Cmd+P", key({ key: "p", metaKey: true, shiftKey: true }))).toBe(true);
  // Shift's character, or the physical key, for punctuation and digits.
  const brace = key({ key: "{", code: "BracketLeft", metaKey: true, shiftKey: true });
  expect(presses("Shift+Cmd+[", brace)).toBe(true);
  expect(presses("Shift+Cmd+]", brace)).toBe(false);
  expect(
    presses("Ctrl+Shift+/", key({ key: "?", code: "Slash", ctrlKey: true, shiftKey: true })),
  ).toBe(true);
  expect(presses("Cmd+1", key({ key: "!", code: "Digit1", metaKey: true }))).toBe(true);
  expect(presses("Cmd+=", key({ key: "=", code: "Equal", metaKey: true }))).toBe(true);
  // Option's character is matched by its key only with Alt.
  expect(presses("Alt+P", key({ key: "π", code: "KeyP", altKey: true }))).toBe(true);
  expect(presses("Ctrl+P", key({ key: "x", code: "KeyP", ctrlKey: true }))).toBe(false);
  // Named keys by their name.
  expect(presses("Ctrl+Tab", key({ key: "Tab", ctrlKey: true }))).toBe(true);
  expect(presses("Ctrl+Esc", key({ key: "Escape", ctrlKey: true }))).toBe(true);
  expect(presses("Ctrl+Space", key({ key: " ", code: "Space", ctrlKey: true }))).toBe(true);
  expect(presses("Ctrl+Up", key({ key: "ArrowDown", ctrlKey: true }))).toBe(false);
  expect(presses("F8", key({ key: "F8" }))).toBe(true);
  expect(presses("F8", key({ key: "F9" }))).toBe(false);
});

test("a key press is recorded by its physical key, a lone modifier is nothing yet", () => {
  expect(chordOf(key({ key: "π", code: "KeyP", altKey: true }))).toBe("Alt+P");
  expect(chordOf(key({ key: "K", code: "KeyK", metaKey: true, shiftKey: true }))).toBe(
    "Shift+Cmd+K",
  );
  expect(chordOf(key({ key: "{", code: "BracketLeft", ctrlKey: true, shiftKey: true }))).toBe(
    "Ctrl+Shift+[",
  );
  expect(chordOf(key({ key: "@", code: "Digit2", ctrlKey: true, shiftKey: true }))).toBe(
    "Ctrl+Shift+2",
  );
  expect(chordOf(key({ key: "Escape", code: "Escape", ctrlKey: true }))).toBe("Ctrl+Esc");
  expect(chordOf(key({ key: " ", code: "Space", altKey: true }))).toBe("Alt+Space");
  expect(chordOf(key({ key: "F5", code: "F5" }))).toBe("F5");
  expect(chordOf(key({ key: "F25", code: "F25" }))).toBeNull();
  expect(chordOf(key({ key: "Shift", code: "ShiftLeft", shiftKey: true }))).toBeNull();
  expect(chordOf(key({ key: "Meta", code: "MetaLeft", metaKey: true }))).toBeNull();
  expect(chordOf(key({ key: "Unidentified", code: "" }))).toBeNull();
});

test("a chord is shown with macOS' symbols there, as written elsewhere", () => {
  expect(platform()).toBe("windows");
  expect(chordText("Ctrl+Shift+P")).toBe("Ctrl+Shift+P");
  asMac();
  expect(platform()).toBe("mac");
  expect(chordText("Shift+Cmd+P")).toBe("⇧⌘P");
  expect(chordText("Ctrl+Alt+Shift+Cmd+Up")).toBe("⌃⌥⇧⌘↑");
  expect(chordText("Ctrl+Shift+Tab")).toBe("⌃⇧Tab");
  expect(chordText("Cmd+Down")).toBe("⌘↓");
  expect(chordText("Alt+Left")).toBe("⌥←");
  expect(chordText("Alt+Right")).toBe("⌥→");
  expect(chordText("Cmd+Enter")).toBe("⌘↩");
  expect(chordText("Cmd+Backspace")).toBe("⌘⌫");
  expect(chordText("Cmd+Delete")).toBe("⌘⌦");
  expect(chordText("Ctrl+Esc")).toBe("⌃esc");
  expect(chordText("F8")).toBe("F8");
});

test("conflicts are equal chords in contexts that meet; the user's chord runs", () => {
  expect([
    overlap("any", "terminal"),
    overlap("markdown", "any"),
    overlap("terminal", "terminal"),
  ]).toEqual([true, true, true]);
  expect(overlap("terminal", "markdown")).toBe(false);
  const commands = [A, B, T, M];
  // A terminal's and a Markdown file's chord never meet.
  expect(conflicts(commands, bindings({}, commands, "windows")).size).toBe(0);
  // B takes A's chord: the user's wins, on both rows.
  const of = bindings({ b: ["Ctrl+Shift+A"] }, commands, "windows");
  const found = conflicts(commands, of);
  expect(found.get("a")).toEqual([{ chord: "Ctrl+Shift+A", other: B, runs: false }]);
  expect(found.get("b")).toEqual([{ chord: "Ctrl+Shift+A", other: A, runs: true }]);
  // Both the user's (or both defaults): table order.
  const both = conflicts(commands, bindings({ a: "F8" }, commands, "windows"));
  expect(both.get("a")).toEqual([{ chord: "F8", other: B, runs: true }]);
  expect(both.get("b")).toEqual([{ chord: "F8", other: A, runs: false }]);
  const defaults = conflicts([T, A], bindings({ a: "Ctrl+Shift+T" }, [T, A], "windows"));
  expect(defaults.get("t")?.[0]?.runs).toBe(false);
  const users = conflicts(
    [T, A],
    bindings({ a: "Ctrl+Shift+T", t: "Ctrl+Shift+T" }, [T, A], "windows"),
  );
  expect(users.get("t")?.[0]?.runs).toBe(true);
});

test("only changes are kept: the defaults again drop the entry", () => {
  expect(withChords({ b: "F9" }, A, ["Ctrl+K"], "windows")).toEqual({ b: "F9", a: "Ctrl+K" });
  expect(withChords({}, A, ["Ctrl+K", "F2"], "windows")).toEqual({ a: ["Ctrl+K", "F2"] });
  expect(withChords({}, A, [], "windows")).toEqual({ a: [] });
  expect(withChords({ a: "Ctrl+K" }, A, ["Ctrl+Shift+A"], "windows")).toEqual({});
  expect(withChords({ b: "F9" }, B, ["Ctrl+Alt+B", "F8"], "windows")).toEqual({
    b: ["Ctrl+Alt+B", "F8"],
  });
  expect(withChords({ b: "F9" }, B, ["F8"], "windows")).toEqual({ b: "F8" });
  expect(withChords("odd", A, ["Ctrl+K"], "windows")).toEqual({ a: "Ctrl+K" });
});

test("the mock's problems: valid entries say nothing", () => {
  expect(
    keymapProblems({ palette: ["Ctrl+K", "F2"], comment: null, settings: [] }, "windows"),
  ).toEqual([]);
  expect(keymapProblems({ palette: ["Ctrl+K", "Cmd+K"] }, "windows")).toEqual([
    'keymap.palette: "Cmd+K": Cmd exists only on macOS',
  ]);
  expect(keymapProblems([1], "mac")).toEqual([]);
});
