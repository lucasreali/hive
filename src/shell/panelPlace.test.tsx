import { afterEach, beforeAll, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useRef } from "react";
import { PLACES_LIMIT } from "../persist";
import type { ChangedFile } from "../protocol";
import { apply } from "../reduce";
import { initialState, select, setPanelView, useHive } from "../store";
import { transport } from "../transport";
import { MOCK_REPOS } from "../transport/mock";
import { keepViewPlace, useKeptScroll, viewKey, viewPlace } from "./panelPlace";
import { RightPanel } from "./RightPanel";

// 28.19: each right-panel view of each worktree keeps its scroll and a tree its active row.

beforeAll(() => {
  // happy-dom has no layout: give the tree its CSS size so the virtualizer shows rows.
  const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight")?.get;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("files-tree") ? 400 : original?.call(this);
    },
  });
});
afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

function List({ place, ready }: { place: string; ready: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const onScroll = useKeptScroll(place, ref, ready);
  return <div data-testid="list" ref={ref} onScroll={onScroll} />;
}

test("a view's scroll is kept by its key and put back once it is ready", () => {
  const { rerender } = render(<List place="a" ready />);
  const list = screen.getByTestId("list");
  list.scrollTop = 120;
  fireEvent.scroll(list);
  expect(viewPlace("a")?.top).toBe(120);
  // Another worktree, not listed yet: its scroll is not taken from the first one's.
  rerender(<List place="b" ready={false} />);
  fireEvent.scroll(list);
  expect(viewPlace("b")).toBeUndefined();
  // Listed: it starts at the top.
  rerender(<List place="b" ready />);
  expect(list.scrollTop).toBe(0);
  list.scrollTop = 30;
  fireEvent.scroll(list);
  // Back to the first: its scroll.
  rerender(<List place="a" ready />);
  expect(list.scrollTop).toBe(120);
  rerender(<List place="b" ready />);
  expect(list.scrollTop).toBe(30);
  expect(viewKey("files", "gpu-box", "/w")).toBe("files\ngpu-box\n/w");
  expect(viewKey("files", null, "/w")).toBe("files\n\n/w");
});

test("the oldest views' places are forgotten past the limit", () => {
  for (let i = 0; i <= PLACES_LIMIT; i++) keepViewPlace(`k${i}`, { top: i });
  expect(viewPlace("k0")).toBeUndefined();
  expect(viewPlace("k1")?.top).toBe(1);
  // A place changed again is the newest: it outlives older ones.
  keepViewPlace("k1", { row: "a.ts" });
  keepViewPlace("new", { top: 5 });
  expect([viewPlace("k1"), viewPlace("k2")]).toEqual([{ top: 1, row: "a.ts" }, undefined]);
});

const [shop, api] = MOCK_REPOS;
const refactor = api.worktrees[1];
const fixLogin = shop.worktrees[1];
const changed = (path: string): ChangedFile => ({
  path,
  status: "modified",
  staged: null,
  old_path: null,
  added: 1,
  removed: 0,
});

test("the Diff tree keeps each worktree's active row and scroll", () => {
  spyOn(transport, "listChanges").mockImplementation(async () => {});
  spyOn(transport, "listComments").mockImplementation(async () => {});
  apply({ type: "projects", projects: [shop, api] });
  setPanelView("changes");
  for (const [w, files] of [
    [refactor.path, ["a.ts", "b.ts", "c.ts"]],
    [fixLogin.path, ["x.ts", "y.ts"]],
  ] as const) {
    apply({
      type: "changes",
      path: w,
      base: "head",
      branch: null,
      base_error: null,
      files: files.map(changed),
      added: files.length,
      removed: 0,
      error: null,
    });
  }
  act(() => select(refactor.id));
  render(<RightPanel />);
  const tree = () => screen.getByRole("tree", { name: "Files" });
  const active = () =>
    document
      .getElementById(tree().getAttribute("aria-activedescendant") ?? "")
      ?.querySelector(".name")?.textContent;
  const scroller = () => document.querySelector(".files-tree") as HTMLElement;
  fireEvent.keyDown(tree(), { key: "ArrowDown" });
  fireEvent.keyDown(tree(), { key: "ArrowDown" });
  expect(active()).toBe("c.ts");
  scroller().scrollTop = 40;
  fireEvent.scroll(scroller());
  // Another worktree starts on its first row, at the top.
  act(() => select(fixLogin.id));
  expect([active(), scroller().scrollTop]).toEqual(["x.ts", 0]);
  // Back: the row and the scroll it was left with, also once another view showed meanwhile.
  act(() => select(refactor.id));
  expect([active(), scroller().scrollTop]).toEqual(["c.ts", 40]);
  act(() => setPanelView("files"));
  act(() => setPanelView("changes"));
  expect([active(), scroller().scrollTop]).toEqual(["c.ts", 40]);
  // A row gone (c.ts committed): the same place in the list.
  act(() =>
    apply({
      type: "changes",
      path: refactor.path,
      base: "head",
      branch: null,
      base_error: null,
      files: ["a.ts", "b.ts"].map(changed),
      added: 2,
      removed: 0,
      error: null,
    }),
  );
  expect(active()).toBe("b.ts");
});
