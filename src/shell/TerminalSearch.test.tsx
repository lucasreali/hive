import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SearchAddon } from "@xterm/addon-search";
import type { Terminal } from "@xterm/xterm";
import { asMac } from "../../test/mac";
import { App } from "../App";
import { apply } from "../reduce";
import { shortcut } from "../shortcuts";
import { addTab, initialState, setOpenFile, useHive } from "../store";
import { closeTerminal, openTerminal, terminal } from "../terminals";
import { MOCK_REPOS } from "../transport/mock";
import { findInTerminal, MATCH_LIMIT, resultText } from "./TerminalSearch";

const [shop] = MOCK_REPOS;
const [, fixLogin] = shop.worktrees;

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
});

const print = (term: Terminal, text: string) =>
  new Promise<void>((resolve) => term.write(text, resolve));

/** The app with a terminal in fix-login whose scrollback holds `lines`, then a prompt. */
async function shown(lines: string[]) {
  render(<App />);
  act(() => apply({ type: "projects", projects: [shop] }));
  const id = await act(() => openTerminal(fixLogin.path));
  const term = terminal(id) as Terminal;
  // After the mock's prompt (printed on a later tick), which this replaces.
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  term.reset();
  await print(term, `${lines.join("\r\n")}\r\n$ `);
  return { id, term };
}

const field = () =>
  screen.getByRole("textbox", { name: "Search the terminal" }) as HTMLInputElement;
const count = () => (document.querySelector(".terminal-search-count") as HTMLElement).textContent;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const type = (text: string) => act(() => fireEvent.change(field(), { target: { value: text } }));
const key = (key: string, shiftKey = false) =>
  act(() => fireEvent.keyDown(field(), { key, shiftKey }));
/** The row of the scrollback the current match is on. */
const matchRow = (term: Terminal) => term.getSelectionPosition()?.start.y;

test("the count names the current match, a total past the limit, or none", () => {
  expect(resultText({ index: 2, count: 12 })).toBe("3 of 12");
  expect(resultText({ index: -1, count: 12 })).toBe("12 matches");
  expect(resultText({ index: 0, count: 0 })).toBe("No results");
  expect(resultText({ index: 4, count: MATCH_LIMIT })).toBe(`5 of ${MATCH_LIMIT}+`);
  expect(resultText({ index: -1, count: MATCH_LIMIT })).toBe(`${MATCH_LIMIT}+ matches`);
});

test("Ctrl+Shift+F searches up from the newest output; Enter goes older, Shift+Enter newer", async () => {
  const filler = Array.from({ length: 200 }, (_, i) => `filler ${i}`);
  const { id, term } = await shown(["Error one", ...filler, "error two", "ERROR three"]);
  const press = new KeyboardEvent("keydown", { key: "F", ctrlKey: true, shiftKey: true });
  act(() => shortcut(press)?.());
  expect(document.activeElement).toBe(field());
  // Over the pane of the terminal it searches.
  expect(field().closest(".terminal-pane")).toBe(term.element?.parentElement as HTMLElement);
  expect(button("Next match").disabled).toBe(true);
  expect(count()).toBe("");

  type("error");
  // The newest match first, as terminals search.
  await waitFor(() => expect(count()).toBe("3 of 3"));
  expect(matchRow(term)).toBe(202);
  key("Enter");
  expect(count()).toBe("2 of 3");
  expect(matchRow(term)).toBe(201);
  key("Enter");
  // Far up the scrollback (the view following it is checked in e2e/terminal-search).
  expect(count()).toBe("1 of 3");
  expect(matchRow(term)).toBe(0);
  key("Enter", true);
  expect(count()).toBe("2 of 3");
  act(() => fireEvent.click(button("Previous match")));
  expect(count()).toBe("3 of 3");
  act(() => fireEvent.click(button("Next match")));
  expect(count()).toBe("2 of 3");

  // Match case: only the lower-case one, found again from the bottom.
  act(() => fireEvent.click(button("Match case")));
  expect(button("Match case").getAttribute("aria-pressed")).toBe("true");
  expect(count()).toBe("1 of 1");
  type("nowhere");
  expect(count()).toBe("No results");
  // Enter with nothing typed does nothing.
  type("");
  expect(count()).toBe("");
  expect(term.hasSelection()).toBe(false);
  key("Enter");
  expect(count()).toBe("");

  // Esc closes it and gives the keyboard back to the terminal.
  type("filler 1");
  key("Escape");
  expect(screen.queryByRole("search")).toBeNull();
  expect(useHive.getState().terminalSearch).toBeNull();
  expect(term.textarea).toBe(document.activeElement as HTMLTextAreaElement);
  expect(id).toBeGreaterThan(0);
});

test("⌘F on macOS; pressed again it selects the text to type a new search", async () => {
  asMac();
  await shown(["one", "two"]);
  const cmdF = new KeyboardEvent("keydown", { key: "f", metaKey: true });
  // Ctrl+F stays the shell's (forward a character).
  expect(shortcut(new KeyboardEvent("keydown", { key: "f", ctrlKey: true }))).toBeNull();
  act(() => shortcut(cmdF)?.());
  type("two");
  await waitFor(() => expect(count()).toBe("1 of 1"));
  const other = document.createElement("button");
  document.body.append(other);
  other.focus();
  act(() => shortcut(cmdF)?.());
  expect(document.activeElement).toBe(field());
  expect([field().selectionStart, field().selectionEnd]).toEqual([0, 3]);
  // The close button does what Esc does.
  act(() => fireEvent.click(button("Close search")));
  expect(screen.queryByRole("search")).toBeNull();
  other.remove();
});

test("only over a shown terminal; it closes with its terminal out of view", async () => {
  const { id, term } = await shown(["needle"]);
  // A file's tab over the terminals: the key is free, and the command does nothing.
  act(() => setOpenFile({ worktree: fixLogin.id, path: "README.md" }));
  const press = new KeyboardEvent("keydown", { key: "F", ctrlKey: true, shiftKey: true });
  expect(shortcut(press)).toBeNull();
  act(() => findInTerminal());
  expect(useHive.getState().terminalSearch).toBeNull();
  // Back on the terminal it opens; another tab hides it and clears its highlights.
  act(() => useHive.setState({ fileShown: false }));
  act(() => shortcut(press)?.());
  type("needle");
  await waitFor(() => expect(count()).toBe("1 of 1"));
  const clear = spyOn(SearchAddon.prototype, "clearDecorations");
  act(() => addTab(id + 100, fixLogin.path));
  expect(screen.queryByRole("search")).toBeNull();
  expect(clear).toHaveBeenCalledTimes(1);
  clear.mockRestore();
  // The match found stays selected, to copy.
  expect(term.getSelection()).toBe("needle");
  // Its terminal shown again: the bar is back, empty.
  act(() => useHive.setState({ activeTab: id }));
  expect(field().value).toBe("");
  act(() => useHive.setState({ tabs: useHive.getState().tabs.filter((t) => t.id === id) }));
});
