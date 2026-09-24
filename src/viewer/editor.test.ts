import { afterEach, expect, test } from "bun:test";
import { syntaxTree } from "@codemirror/language";
import { getOriginalDoc } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { createViewer, selectedLines, type Viewer } from "./editor";

let viewer: Viewer | null = null;
afterEach(() => {
  viewer?.destroy();
  viewer = null;
  document.body.innerHTML = "";
});

const until = async (check: () => boolean) => {
  for (let i = 0; i < 200 && !check(); i++) await new Promise((r) => setTimeout(r, 5));
  expect(check()).toBe(true);
};

test("shows a file read-only, as a diff only when it has an original", () => {
  viewer = createViewer(document.body, "notes.unknown-extension");
  const { view } = viewer;
  expect(document.body.contains(view.dom)).toBe(true);

  viewer.show({ content: "a\nb\n", original: null });
  expect(view.state.doc.toString()).toBe("a\nb\n");
  expect(view.state.facet(EditorState.readOnly)).toBe(true);
  expect(view.dom.classList.contains("cm-merge-b")).toBe(false);

  view.scrollDOM.scrollTop = 40;
  viewer.show({ content: "a\nB\n", original: "a\nb\n" });
  expect(view.state.doc.toString()).toBe("a\nB\n");
  expect(getOriginalDoc(view.state).toString()).toBe("a\nb\n");
  expect(view.scrollDOM.scrollTop).toBe(40);
});

test("highlights by the file name's language once it is loaded", async () => {
  viewer = createViewer(document.body, "src/auth/session.ts");
  const { view } = viewer;
  viewer.show({ content: "const a = 1;\n", original: null });
  await until(() => syntaxTree(view.state).length > 0);
  // A later answer keeps the language.
  viewer.show({ content: "let b = 2;\n", original: "const a = 1;\n" });
  expect(syntaxTree(view.state).topNode.name).toBe("Script");
});

test("a viewer closed before its language loads is left alone", async () => {
  const closed = createViewer(document.body, "a.rs");
  closed.show({ content: "fn main() {}\n", original: null });
  closed.destroy();
  viewer = createViewer(document.body, "b.rs");
  viewer.show({ content: "fn main() {}\n", original: null });
  const { view } = viewer;
  await until(() => syntaxTree(view.state).length > 0);
  expect(syntaxTree(closed.view.state).length).toBe(0);
});

test("the selected lines: none when empty, a line ending the range only when entered", () => {
  const seen: unknown[] = [];
  viewer = createViewer(document.body, "a.txt", (lines) => seen.push(lines));
  const { view } = viewer;
  viewer.show({ content: "one\ntwo\nthree\n", original: "one\n" });
  expect(seen).toEqual([null]);
  // From inside line 1 to the start of line 3: lines 1-2.
  view.dispatch({ selection: { anchor: 1, head: 8 } });
  view.dispatch({ selection: { anchor: 9, head: 10 } });
  view.dispatch({ selection: { anchor: 5 } });
  expect(seen).toEqual([null, { from: 1, to: 2 }, { from: 3, to: 3 }, null]);
  expect(selectedLines(view.state)).toBeNull();
});
