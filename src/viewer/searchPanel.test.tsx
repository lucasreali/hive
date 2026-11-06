import { afterEach, expect, test } from "bun:test";
import { getSearchQuery, openSearchPanel, search, searchPanelOpen } from "@codemirror/search";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { act, fireEvent, screen } from "@testing-library/react";
import { asMac } from "../../test/mac";
import { editorSearchKeymap } from "./editor";
import { createSearchPanel } from "./searchPanel";

let view: EditorView | null = null;
afterEach(async () => {
  // The panel's React root unmounts on a microtask.
  await act(async () => {
    view?.destroy();
    await Promise.resolve();
  });
  view = null;
  document.body.innerHTML = "";
});

/** A view of `doc` with the find bar open (as ⌘F opens it), the cursor at the start. */
function opened(doc: string, readOnly = false) {
  view = new EditorView({
    parent: document.body,
    state: EditorState.create({
      doc,
      extensions: [
        keymap.of(editorSearchKeymap),
        search({ top: true, createPanel: createSearchPanel }),
        EditorState.readOnly.of(readOnly),
      ],
    }),
  });
  act(() => {
    openSearchPanel(view as EditorView);
  });
  return view;
}

const field = (name: string) => screen.getByRole("textbox", { name }) as HTMLInputElement;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const type = (name: string, text: string) =>
  act(() => fireEvent.change(field(name), { target: { value: text } }));
const click = (name: string) => act(() => fireEvent.click(button(name)));
const key = (name: string, init: KeyboardEventInit) =>
  act(() => fireEvent.keyDown(field(name), init));
const selected = (v: EditorView) => {
  const { from, to } = v.state.selection.main;
  return [from, to];
};

test("the bar: a find field with its toggles, arrows, close, and a replace row", () => {
  const v = opened("foo\n");
  const bar = screen.getByRole("search", { name: "Find in the file" });
  // CodeMirror's panel class (the editor's own styles and its focus lookup) and ours.
  expect([...bar.classList]).toEqual(["cm-search", "find-bar", "cm-panel"]);
  expect(bar.parentElement?.classList.contains("cm-panels-top")).toBe(true);
  // The find field has the keyboard, marked as the panel's main field.
  expect(document.activeElement).toBe(field("Find"));
  expect(field("Find").getAttribute("main-field")).toBe("true");
  const labels = [...bar.querySelectorAll("input, button")].map(
    (e) => e.getAttribute("aria-label") ?? e.textContent,
  );
  expect(labels).toEqual([
    "Find",
    "Match case",
    "Use regular expression",
    "Match whole word",
    "Previous match",
    "Next match",
    "Close search",
    "Replace",
    "Replace",
    "Replace all",
  ]);
  expect([...bar.querySelectorAll(".find-toggle")].map((b) => b.textContent)).toEqual([
    "Aa",
    ".*",
    "ab",
  ]);
  // Tooltips name each control and its key; every icon is decoration only.
  expect(button("Previous match").title).toBe("Previous match (Shift+Enter)");
  expect(button("Next match").title).toBe("Next match (Enter)");
  expect(button("Close search").title).toBe("Close (Esc)");
  expect(button("Match case").title).toBe("Match case");
  expect(button("Replace").title).toBe("Replace the current match (Enter)");
  expect(button("Replace all").title).toBe("Replace every match");
  for (const svg of bar.querySelectorAll("svg"))
    expect(svg.getAttribute("aria-hidden")).toBe("true");
  // Plain buttons, not submit buttons: each one reachable by Tab.
  for (const b of bar.querySelectorAll("button")) expect(b.type).toBe("button");
  expect(searchPanelOpen(v.state)).toBe(true);
});

test("the tooltips use the Mac's key glyphs on macOS", () => {
  asMac();
  opened("foo\n");
  expect(button("Previous match").title).toBe("Previous match (⇧↩)");
  expect(button("Next match").title).toBe("Next match (↩)");
  expect(button("Close search").title).toBe("Close (esc)");
});

test("typing sets the query; the toggles set its options, pressed while on", () => {
  const v = opened("foo\n");
  type("Find", "fo+");
  expect(getSearchQuery(v.state).search).toBe("fo+");
  for (const [name, option] of [
    ["Match case", "caseSensitive"],
    ["Use regular expression", "regexp"],
    ["Match whole word", "wholeWord"],
  ] as const) {
    expect(button(name).getAttribute("aria-pressed")).toBe("false");
    click(name);
    expect(getSearchQuery(v.state)[option]).toBe(true);
    expect(button(name).getAttribute("aria-pressed")).toBe("true");
  }
  // Each toggle kept the others and the text.
  expect(getSearchQuery(v.state)).toMatchObject({
    search: "fo+",
    caseSensitive: true,
    regexp: true,
    wholeWord: true,
  });
  click("Match case");
  expect(getSearchQuery(v.state).caseSensitive).toBe(false);
  expect(button("Match case").getAttribute("aria-pressed")).toBe("false");
});

test("a query set elsewhere shows in the bar", () => {
  const v = opened("foo bar\n");
  // ⌘F again with "bar" selected takes it as the query, as CodeMirror's panel does.
  act(() => v.dispatch({ selection: { anchor: 4, head: 7 } }));
  act(() => {
    field("Find").blur();
    openSearchPanel(v);
  });
  expect(field("Find").value).toBe("bar");
  expect(document.activeElement).toBe(field("Find"));
});

test("next and previous run CodeMirror's commands, by button and by key", () => {
  const v = opened("foo\nbar foo\nfoo\n");
  type("Find", "foo");
  click("Next match");
  expect(selected(v)).toEqual([0, 3]);
  click("Next match");
  expect(selected(v)).toEqual([8, 11]);
  click("Previous match");
  expect(selected(v)).toEqual([0, 3]);
  key("Find", { key: "Enter" });
  expect(selected(v)).toEqual([8, 11]);
  key("Find", { key: "Enter", shiftKey: true });
  expect(selected(v)).toEqual([0, 3]);
  // The editor's search keys still work in the bar: ⌘G (Ctrl+G) and F3.
  key("Find", { key: "g", ctrlKey: true });
  expect(selected(v)).toEqual([8, 11]);
  // Enter while composing (an IME) is the composition's.
  key("Find", { key: "Enter", isComposing: true });
  expect(selected(v)).toEqual([8, 11]);
  // Another key in the field is just typing.
  key("Find", { key: "a" });
  expect(selected(v)).toEqual([8, 11]);
});

test("replace replaces the current match, by button or Enter in its field; replace all, every one", () => {
  const v = opened("foo foo foo\n");
  type("Find", "foo");
  type("Replace", "x");
  expect(getSearchQuery(v.state).replace).toBe("x");
  click("Next match");
  click("Replace");
  expect(v.state.doc.toString()).toBe("x foo foo\n");
  key("Replace", { key: "Enter" });
  expect(v.state.doc.toString()).toBe("x x foo\n");
  // Not while composing, nor another key.
  key("Replace", { key: "Enter", isComposing: true });
  key("Replace", { key: "a" });
  expect(v.state.doc.toString()).toBe("x x foo\n");
  click("Replace all");
  expect(v.state.doc.toString()).toBe("x x x\n");
});

test("a read-only text has no replace row", () => {
  opened("foo\n", true);
  expect(screen.queryByRole("textbox", { name: "Replace" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Replace all" })).toBeNull();
});

test("Esc in the bar and its close button close it", async () => {
  const v = opened("foo\n");
  key("Find", { key: "Escape" });
  expect(searchPanelOpen(v.state)).toBe(false);
  expect(document.querySelector(".find-bar")).toBeNull();
  act(() => {
    openSearchPanel(v);
  });
  click("Close search");
  expect(searchPanelOpen(v.state)).toBe(false);
  expect(document.querySelector(".find-bar")).toBeNull();
  // Its React root is gone after the click.
  await act(() => Promise.resolve());
});
