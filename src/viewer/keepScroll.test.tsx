import { afterEach, beforeEach, expect, test } from "bun:test";
import { EditorView } from "@codemirror/view";
import { act, cleanup, render } from "@testing-library/react";
import type { Space } from "../protocol";
import { apply } from "../reduce";
import { TerminalArea } from "../shell/TerminalArea";
import {
  activateTab,
  addTab,
  initialState,
  type OpenFile,
  pinFile,
  select,
  setEditing,
  setOpenFile,
  setRendered,
  setSplit,
  useHive,
} from "../store";
import { closeTerminal } from "../terminals";
import { MOCK_REPOS } from "../transport/mock";
import { snapshot, type ViewSnapshot } from "./editor";

// 15.4: a file shown again is at the line it was left at, with its selection, however it was left.

const [shop, api] = MOCK_REPOS;
const [, fixLogin, checkout] = shop.worktrees;
const W = fixLogin.path;
const LONG = Array.from({ length: 300 }, (_, i) => `line ${i + 1}`).join("\n");
const NO_ENV = { git_name: null, git_email: null, gh_config_dir: null, gh_account: null };
const spaces = (current: string) =>
  apply({
    type: "spaces",
    spaces: [
      { id: "home", name: "Home", projects: [shop.id], env: NO_ENV },
      { id: "work", name: "Work", projects: [api.id], env: NO_ENV },
    ] satisfies Space[],
    current,
  });

const file = (path: string): OpenFile => ({ worktree: W, path });
const answer = (path: string, content = LONG) =>
  apply({
    type: "file",
    worktree: W,
    path,
    content,
    base: content,
    version: `v:${content}`,
    binary: false,
    too_large: false,
    error: null,
  });
/** Opens `path` in a tab of its own (a double click, 11.1), with the service's answer. */
function open(path: string, editing = true) {
  setOpenFile(file(path), editing);
  pinFile(file(path));
  answer(path);
}

/** The view of the file shown. */
const shown = () =>
  EditorView.findFromDOM(
    document.querySelector(".file-view .cm-editor") as HTMLElement,
  ) as EditorView;

/** Selects some text far down and scrolls there, as the user does; what the view then shows. */
function scrollDown(): ViewSnapshot {
  const view = shown();
  view.dispatch({ selection: { anchor: 2000, head: 2010 } });
  view.scrollDOM.scrollTop = 3000;
  view.scrollDOM.dispatchEvent(new Event("scroll"));
  return snapshot(view);
}

/** The line at the top of a snapshot's scroll. */
const topLine = (saved: ViewSnapshot) =>
  shown().state.doc.lineAt((saved.scroll.value as { range: { head: number } }).range.head).number;

beforeEach(() => {
  useHive.setState(initialState, true);
  localStorage.clear();
  apply({ type: "projects", projects: [shop, api] });
  spaces("home");
  select(fixLogin.id);
});
afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  cleanup();
  useHive.setState(initialState, true);
  localStorage.clear();
});

const ways: [string, () => void, () => void][] = [
  ["another file's tab", () => open("other.ts"), () => setOpenFile(file("long.ts"))],
  [
    "a terminal's tab",
    () => {
      addTab(1, W);
      activateTab({ id: 1, cwd: W });
    },
    () => setOpenFile(file("long.ts")),
  ],
  [
    "another worktree",
    () => select(checkout.id),
    () => {
      select(fixLogin.id);
      setOpenFile(file("long.ts"));
    },
  ],
  [
    "another space (#48)",
    () => spaces("work"),
    () => {
      spaces("home");
      setOpenFile(file("long.ts"));
    },
  ],
  [
    "a split",
    () => {
      addTab(1, W);
      addTab(2, W);
      setSplit({ left: 1, right: 2 });
    },
    () => {
      setSplit(null);
      setOpenFile(file("long.ts"));
    },
  ],
  ["the diff (the read-only view)", () => setEditing(false), () => setEditing(true)],
  ["the Markdown preview", () => setRendered(true), () => setRendered(false)],
];

for (const [way, leave, back] of ways) {
  test(`a file left for ${way} shows again at the same scroll and selection`, () => {
    render(<TerminalArea />);
    act(() => open("long.ts"));
    const left = scrollDown();
    expect(topLine(left)).toBeGreaterThan(1);
    act(leave);
    act(back);
    const again = snapshot(shown());
    expect(again.selection.eq(left.selection)).toBe(true);
    expect(topLine(again)).toBe(topLine(left));
    // Left again without a scroll meanwhile: still there.
    act(leave);
    act(back);
    expect(topLine(snapshot(shown()))).toBe(topLine(left));
  });
}

test("the read-only view keeps its scroll too, and a later answer does not reset it", () => {
  render(<TerminalArea />);
  act(() => open("long.ts", false));
  const left = scrollDown();
  act(() => open("other.ts", false));
  act(() => {
    setOpenFile(file("long.ts"));
    answer("long.ts");
  });
  expect(topLine(snapshot(shown()))).toBe(topLine(left));
  // The service answers again (`followOpenFile` asks once more): the same line stays on top.
  act(() => answer("long.ts", `${LONG}\nmore`));
  expect(topLine(snapshot(shown()))).toBe(topLine(left));
});
