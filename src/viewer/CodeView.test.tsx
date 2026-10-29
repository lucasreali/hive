import { afterEach, expect, spyOn, test } from "bun:test";
import { EditorSelection } from "@codemirror/state";
import { act, cleanup, render } from "@testing-library/react";
import type { FileText } from "../protocol";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { CodeView } from "./CodeView";
import * as editor from "./editor";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

/** The open file as the panel shows it: the store's last answer. */
function Shown() {
  const text = useHive((s) => s.file);
  return text && <CodeView text={text} diff />;
}

const answer: FileText = {
  worktree: "/w",
  path: "a.ts",
  content: "one\ntwo\nthree\n",
  base: "one\n",
  version: "v1",
  binary: false,
  too_large: false,
  error: null,
};

test("an equal answer is not shown again, so the selected lines stay (19.12)", () => {
  const shows: number[] = [];
  const views: editor.Viewer["view"][] = [];
  const create = editor.createViewer;
  const spy = spyOn(editor, "createViewer").mockImplementation((...args) => {
    const viewer = create(...args);
    const show = viewer.show;
    views.push(viewer.view);
    viewer.show = (doc) => {
      shows.push(1);
      show(doc);
    };
    return viewer;
  });
  act(() => apply({ type: "file", ...answer }));
  render(<Shown />);
  expect(shows).toHaveLength(1);
  act(() => views[0]?.dispatch({ selection: EditorSelection.range(0, 5) }));
  expect(useHive.getState().selectedLines).toEqual({ from: 1, to: 2 });

  act(() => apply({ type: "file", ...answer }));
  expect(shows).toHaveLength(1);
  expect(useHive.getState().selectedLines).toEqual({ from: 1, to: 2 });
  // A new version on disk is shown, which clears the selection.
  act(() => apply({ type: "file", ...answer, content: "one\n", version: "v2" }));
  expect(shows).toHaveLength(2);
  expect(useHive.getState().selectedLines).toBeNull();
  spy.mockRestore();
});
