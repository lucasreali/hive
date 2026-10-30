import { expect, test } from "bun:test";
import { Terminal } from "@xterm/xterm";
import { MAX_MARK, MAX_PROMPTS, Marks, parseMark, renderMark } from "./marks";

const write = (term: Terminal, data: string) =>
  new Promise<void>((resolve) => term.write(data, resolve));

const terminal = (rows = 5) => new Terminal({ rows, cols: 40, allowProposedApi: true });

const A = "\x1b]133;A\x07";
const C = "\x1b]133;C\x07";
const D = (exit = "") => `\x1b]133;D${exit && `;${exit}`}\x07`;

test("OSC 133 marks a prompt, a command's start and its end with its status", () => {
  expect(parseMark("A")).toEqual({ kind: "prompt" });
  expect(parseMark("A;special_key=1")).toEqual({ kind: "prompt" });
  expect(parseMark("C")).toEqual({ kind: "start" });
  expect(parseMark("D;0")).toEqual({ kind: "end", exit: 0 });
  expect(parseMark("D;-1;aid=2")).toEqual({ kind: "end", exit: -1 });
  expect(parseMark("D")).toEqual({ kind: "end", exit: null });
  // Broken statuses: a word, a fraction, too many digits.
  for (const exit of ["x", "1.5", "1e3", "", "1234567890"]) {
    expect(parseMark(`D;${exit}`)).toEqual({ kind: "end", exit: null });
  }
  // Other marks, nothing, and anything longer than the limit.
  for (const data of ["B", "", "AA", "P;k=v", "a"]) expect(parseMark(data)).toBeNull();
  expect(parseMark(`A;${"x".repeat(MAX_MARK - 2)}`)).toEqual({ kind: "prompt" });
  expect(parseMark(`A;${"x".repeat(MAX_MARK - 1)}`)).toBeNull();
});

test("a prompt's bar is coloured by how its command ended", () => {
  for (const status of ["ok", "failed", null] as const) {
    const el = document.createElement("div");
    renderMark(el, status);
    expect(el.className).toBe("shell-mark");
    expect(el.dataset.status).toBe(status ?? undefined);
  }
});

test("each prompt is marked, with its command's status once it ends", async () => {
  const term = terminal();
  const marks = new Marks(term);
  // An end with no prompt, or no command, marks nothing.
  await write(term, `${D("0")}${C}${D("0")}`);
  await write(term, `${A}$ ok\r\n${C}out\r\n${D("0")}`);
  await write(term, `${A}$ bad\r\n${C}${D("2")}`);
  await write(term, `${A}$ none\r\n${C}${D()}`);
  await write(term, `${A}$ \r\n${D("1")}${A}$ `);
  expect(marks.list()).toEqual([
    { line: 0, status: "ok" },
    { line: 2, status: "failed" },
    { line: 3, status: null },
    { line: 4, status: undefined },
    { line: 5, status: undefined },
  ]);
  // A mark the parser refuses changes nothing.
  await write(term, `\x1b]133;${"A".repeat(MAX_MARK + 1)}\x07`);
  expect(marks.list()).toHaveLength(5);
  term.dispose();
});

test("a command that clears its prompt off the screen ends without marking it (21.1)", async () => {
  const term = terminal();
  const marks = new Marks(term);
  const ended: string[] = [];
  // fish's `clear`: the command's prompt is erased between its start and its end. The end then
  // threw inside xterm's parser, and xterm drew nothing more.
  await write(term, `${A}hive on main\r\n❯ clear\r\n${C}\x1b[H\x1b[2J\x1b[3J${D("0")}`);
  await write(term, `${A}hive on main\r\n❯ `);
  term.write("x", () => ended.push("x"));
  await write(term, "");
  expect(ended).toEqual(["x"]);
  expect(marks.list()).toEqual([{ line: 0, status: undefined }]);
  expect(term.buffer.active.getLine(1)?.translateToString(true)).toBe("❯ x");
  // An older prompt left on the screen is not marked by that end either.
  await write(term, `\r\n${A}$ \r\n${C}\x1b[2;1H\x1b[J${D("1")}`);
  expect(marks.list()).toEqual([{ line: 0, status: undefined }]);
  term.dispose();
});

/** Every line of `term`'s buffer, scrollback first, trailing blank lines dropped. */
const lines = (term: Terminal) => {
  const buffer = term.buffer.active;
  const all: string[] = [];
  for (let i = 0; i < buffer.length; i++)
    all.push(buffer.getLine(i)?.translateToString(true) ?? "");
  return all.join("\n").trimEnd();
};

test("clear keeps the whole prompt the cursor is at, at the top (21.1)", async () => {
  const term = terminal(4);
  const marks = new Marks(term);
  // A prompt of two lines (fish's), after a command with more output than the screen holds.
  await write(term, `${A}hive on main\r\n❯ seq\r\n${C}1\r\n2\r\n3\r\n4\r\n${D("0")}`);
  await write(term, `${A}hive on main\r\n❯ typed`);
  marks.clear();
  await write(term, "");
  expect(lines(term)).toBe("hive on main\n❯ typed");
  expect(term.buffer.active.cursorY).toBe(1);
  expect(term.buffer.active.cursorX).toBe(7);
  expect(marks.list()).toEqual([{ line: 0, status: undefined }]);
  // Already at the top: only the scrollback would go.
  marks.clear();
  await write(term, " more");
  expect(lines(term)).toBe("hive on main\n❯ typed more");
  // A command running: xterm's own clear, the cursor's line kept.
  await write(term, `\r\n${C}out\r\nlast`);
  marks.clear();
  await write(term, "");
  expect(lines(term)).toBe("last");
  term.dispose();
});

test("clear without a prompt mark keeps the cursor's line, as xterm does", async () => {
  const term = terminal(4);
  const marks = new Marks(term);
  await write(term, "one\r\ntwo\r\n$ ");
  marks.clear();
  await write(term, "");
  expect(lines(term)).toBe("$");
  // A prompt that left the screen (taller than it): the same.
  await write(term, `\r\n${A}a\r\nb\r\nc\r\nd\r\ne\r\n> `);
  marks.clear();
  await write(term, "");
  expect(lines(term)).toBe(">");
  term.dispose();
});

test("prompts trimmed from the scrollback, or past the limit, are dropped", async () => {
  const term = new Terminal({ rows: 2, cols: 20, scrollback: 3, allowProposedApi: true });
  const marks = new Marks(term);
  await write(term, `${A}one\r\n${A}two\r\n`);
  await write(term, "a\r\nb\r\nc\r\nd\r\n");
  expect(marks.list()).toEqual([]);
  const many = new Terminal({ rows: 2, cols: 20, scrollback: 5000, allowProposedApi: true });
  const kept = new Marks(many);
  await write(many, `${A}\r\n`.repeat(MAX_PROMPTS + 2));
  const lines = kept.list().map((p) => p.line);
  expect(lines).toHaveLength(MAX_PROMPTS);
  expect(lines[0]).toBe(2);
  term.dispose();
  many.dispose();
});

test("jumps go from prompt to prompt and past the last one back to the end", async () => {
  const term = terminal(4);
  const marks = new Marks(term);
  // Prompts at lines 0, 5 and 10, each with four lines of output, then a running command's.
  for (const name of ["a", "b"]) {
    await write(term, `${A}$ ${name}\r\n${C}1\r\n2\r\n3\r\n4\r\n${D("0")}`);
  }
  await write(term, `${A}$ c\r\n${C}1\r\n2\r\n3\r\n4\r\n5\r\n`);
  const buffer = term.buffer.active;
  expect(buffer.baseY).toBe(13);
  const top = () => buffer.viewportY;
  // From the end: the prompts above the view's top line, one by one.
  for (const line of [10, 5, 0]) {
    expect(marks.jump(-1)).toBe(true);
    expect(top()).toBe(line);
  }
  expect(marks.jump(-1)).toBe(false);
  expect(top()).toBe(0);
  // Down again, then past the last prompt: the end.
  for (const line of [5, 10, 13]) {
    expect(marks.jump(1)).toBe(true);
    expect(top()).toBe(line);
  }
  // At the end with no prompt below: nothing to do.
  expect(marks.jump(1)).toBe(false);
  // A prompt on the last screen: the end.
  await write(term, `${A}$ `);
  expect(marks.jump(-1)).toBe(true);
  expect(top()).toBe(10);
  expect(marks.jump(1)).toBe(true);
  expect(top()).toBe(13);
  // Scrolled by hand: from the view's top line.
  term.scrollToLine(3);
  expect(marks.jump(1)).toBe(true);
  expect(top()).toBe(5);
  term.scrollToLine(3);
  expect(marks.jump(-1)).toBe(true);
  expect(top()).toBe(0);
  term.dispose();
});

test("a terminal with no prompts has nowhere to jump", () => {
  const term = terminal();
  const marks = new Marks(term);
  expect(marks.jump(-1)).toBe(false);
  expect(marks.jump(1)).toBe(false);
  term.dispose();
});
