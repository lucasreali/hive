import { afterEach, expect, mock, setSystemTime, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { LineCommit, LineEdit, LineOrigin } from "../protocol";
import { initialState, type LineAsk, useHive } from "../store";
import { transport } from "../transport";
import { LineMenu, LinePopover } from "./LineAuthor";

afterEach(() => {
  mock.restore();
  setSystemTime();
  cleanup();
  useHive.setState(initialState, true);
});

const NOW = Date.UTC(2026, 10, 26, 12);
const ask: LineAsk = { worktree: "/r", path: "a.ts", line: 4, side: "worktree", x: 30, y: 40 };
const commit: LineCommit = {
  id: "abc1234def",
  author: "Ada",
  ms: NOW - 48 * 3_600_000,
  summary: "Keep sessions",
};
const edit: LineEdit = {
  session: "s",
  title: "Fix the login",
  deleted: false,
  project: "/r",
  worktree: "/r",
  agent: "a1",
  agent_type: "general-purpose",
  tool: "Edit",
  tool_use_id: null,
  ms: NOW,
  user_modified: false,
  start: 4,
  len: 1,
};

/** The popover showing `origin` for `ask`'s line. */
function answer(origin: LineOrigin | null) {
  setSystemTime(NOW);
  render(<LinePopover />);
  act(() => useHive.setState({ provenance: { ...ask, origin } }));
  return screen.queryByRole("dialog", { name: "Who wrote this line?" });
}

test("the line number menu asks who wrote its line", () => {
  const sent = spyOn(transport, "whoWrote").mockResolvedValue();
  render(<LineMenu />);
  expect(screen.queryByRole("menu")).toBeNull();
  act(() => useHive.setState({ lineMenu: ask }));
  const menu = screen.getByRole("menu", { name: "Line 4" });
  fireEvent.click(screen.getByRole("menuitem", { name: "Who wrote this line?" }));
  expect(sent).toHaveBeenCalledWith("/r", "a.ts", 4, "worktree", "head");
  expect(menu.isConnected).toBe(false);
  // Esc closes it unasked.
  act(() => useHive.setState({ lineMenu: ask }));
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  expect(useHive.getState().lineMenu).toBeNull();
});

test("nothing shows before the answer, nor for a live session's (the Sessions view has it)", () => {
  expect(answer(null)).toBeNull();
  act(() =>
    useHive.setState({ provenance: { ...ask, origin: { kind: "session", edit, commit: null } } }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("the user in Hive's editor", () => {
  const popover = answer({ kind: "editor", ms: NOW - 10 * 60_000 }) as HTMLElement;
  expect(popover.textContent).toBe("You, in Hive's editor · 10m ago");
  expect(popover.style.left).toBe("30px");
  expect(popover.style.top).toBe("40px");
});

test("unknown: why, and the commit git blame gives when there is one", () => {
  const popover = answer({ kind: "unknown", commit: null }) as HTMLElement;
  expect(popover.textContent).toBe(
    "No session recorded for this line. It was changed outside an agent's Edit or Write (a shell command, another editor, git), or before Hive recorded the file.",
  );
  act(() => useHive.setState({ provenance: { ...ask, origin: { kind: "unknown", commit } } }));
  expect(screen.getByText("abc1234 · Ada · 2d ago · Keep sessions")).toBeDefined();
});

test("probable: says so, and Show session goes to the session's row", () => {
  const popover = answer({ kind: "session", edit, commit }) as HTMLElement;
  expect(popover.querySelector("p")?.textContent).toBe(
    "Probably general-purpose subagent of Fix the login, committed in abc1234 · 2d ago",
  );
  expect(screen.queryByText("Session deleted")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Show session" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  const s = useHive.getState();
  expect(s.panelView).toBe("sessions");
  expect(s.sessionFocus).toEqual({ edit, path: "a.ts" });

  const gone = { ...edit, deleted: true };
  act(() =>
    useHive.setState({ provenance: { ...ask, origin: { kind: "session", edit: gone, commit } } }),
  );
  expect(screen.getByText("Session deleted")).toBeDefined();
});

test("Esc or a click outside closes it; a click inside does not", () => {
  answer({ kind: "editor", ms: NOW });
  const popover = screen.getByRole("dialog");
  fireEvent.pointerDown(popover);
  fireEvent.keyDown(document, { key: "Enter" });
  expect(useHive.getState().provenance).not.toBeNull();
  fireEvent.pointerDown(document.body);
  expect(useHive.getState().provenance).toBeNull();
  act(() => useHive.setState({ provenance: { ...ask, origin: { kind: "editor", ms: NOW } } }));
  fireEvent.keyDown(document, { key: "Escape" });
  expect(useHive.getState().provenance).toBeNull();
});
