import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { CommitInfo } from "../protocol";
import { apply } from "../reduce";
import { initialState, useHive } from "../store";
import { transport } from "../transport";
import { CommitPicker } from "./CommitPicker";

afterEach(() => {
  cleanup();
  mock.restore();
  useHive.setState(initialState, true);
});

const PATH = "/r/w";

const commit = (n: number, subject: string, author: string, on_branch: boolean): CommitInfo => ({
  id: `${n}`.repeat(40),
  short: `${n}`.repeat(7),
  author,
  time_ms: Date.UTC(2026, 10, n),
  subject,
  on_branch,
});

const COMMITS = [
  commit(1, "Fix the login", "Ana", true),
  commit(2, "Add tests", "Lucas", true),
  commit(3, "Initial commit", "Ana", false),
];

function open() {
  const asked = spyOn(transport, "listCommits").mockImplementation(async () => {});
  const onClose = mock(() => {});
  render(<CommitPicker path={PATH} onClose={onClose} />);
  const dialog = screen.getByRole("dialog", { name: "Compare with a commit" }) as HTMLDialogElement;
  return { asked, onClose, dialog };
}

const listed = (commits = COMMITS, truncated = false, error: string | null = null) =>
  act(() => apply({ type: "commits", path: PATH, commits, truncated, error }));
const field = () => screen.getByRole("textbox", { name: "Filter commits" });
const key = (key: string) => fireEvent.keyDown(field(), { key });
const rows = () =>
  [...document.querySelectorAll(".commit-row")].map((r) => [
    r.querySelector(".commit-short")?.textContent,
    r.getAttribute("aria-pressed"),
  ]);
const headings = () => [...document.querySelectorAll(".picker-heading")].map((h) => h.textContent);

test("asks for the commits and lists them, the branch's first under their heading", () => {
  const { asked, dialog } = open();
  expect(dialog.open).toBe(true);
  expect(asked).toHaveBeenCalledWith(PATH);
  expect(screen.getByText("Loading commits…")).toBeDefined();
  listed();
  expect(rows()).toEqual([
    ["1111111", "true"],
    ["2222222", "false"],
    ["3333333", "false"],
  ]);
  expect(headings()).toEqual(["This branch", "Older commits"]);
  expect(document.querySelector(".commit-row .picker-path")?.textContent).toBe("Ana · 2026-11-01");
  expect(dialog.querySelector("footer")?.textContent).toBe("↑↓navigate");
  // A list cut short says so.
  listed(COMMITS, true);
  expect(dialog.querySelector("footer")?.textContent).toBe("↑↓navigateOnly the last 3 commits");
});

test("filters by subject, short id or author; arrows move and Enter compares with it", () => {
  const { onClose } = open();
  listed();
  fireEvent.change(field(), { target: { value: "ana" } });
  expect(rows().map((r) => r[0])).toEqual(["1111111", "3333333"]);
  fireEvent.change(field(), { target: { value: "2222" } });
  expect(rows()).toEqual([["2222222", "true"]]);
  fireEvent.change(field(), { target: { value: "INITIAL" } });
  expect(headings()).toEqual(["Older commits"]);
  fireEvent.change(field(), { target: { value: "" } });
  key("ArrowDown");
  key("ArrowDown");
  key("ArrowDown");
  expect(rows().map((r) => r[1])).toEqual(["false", "false", "true"]);
  key("ArrowUp");
  key("ArrowUp");
  key("ArrowUp");
  expect(rows().map((r) => r[1])).toEqual(["true", "false", "false"]);
  // Another key is the field's.
  key("a");
  fireEvent.mouseEnter(document.querySelectorAll(".commit-row")[1]);
  key("Enter");
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(useHive.getState().diffBases[PATH]).toEqual({ commit: { id: "2".repeat(40) } });
});

test("nothing to pick: the service's reason, or none found; a click outside closes", () => {
  const { onClose, dialog } = open();
  listed([], false, "/r/w is not a worktree of a followed project");
  expect(screen.getByText("/r/w is not a worktree of a followed project")).toBeDefined();
  key("Enter");
  expect(onClose).not.toHaveBeenCalled();
  listed();
  fireEvent.change(field(), { target: { value: "zzz" } });
  expect(screen.getByText("No commits found")).toBeDefined();
  fireEvent.click(field());
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.click(dialog);
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(useHive.getState().diffBases).toEqual({});
});
