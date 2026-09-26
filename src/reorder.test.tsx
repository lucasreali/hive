import { afterEach, expect, spyOn, test } from "bun:test";
import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { App } from "./App";
import { moveNextTo } from "./reorder";
import {
  apply,
  initialState,
  moveAgent,
  pendingAgents,
  savedAgentOrder,
  stepAgent,
  treeAgents,
  useHive,
} from "./store";
import { agentStatus, MOCK_REPOS } from "./transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
  localStorage.clear();
});

const [shop] = MOCK_REPOS;
const [main, fixLogin] = shop.worktrees;
const at = (w: typeof main) => ({ project: shop.id, worktree: w.id, cwd: w.path });
const ids = () => treeAgents(useHive.getState()).map((a) => a.id);

/** Agents s1, s2, s3 in fix-login and s4 in main, each with its own titled tab. */
function agents() {
  act(() => {
    apply({ type: "projects", projects: [shop] });
    useHive.setState({ tabs: [1, 2, 3, 4].map((id) => ({ id, cwd: shop.path })) });
    for (const [n, w] of [fixLogin, fixLogin, fixLogin, main].entries()) {
      const id = `s${n + 1}`;
      apply({ type: "agent_detected", channel: n + 1, id, ...at(w) });
      apply({ type: "agent_title", channel: n + 1, id, title: id });
    }
  });
}

const inTree = (name: RegExp) =>
  within(screen.getByRole("navigation", { name: "Projects" })).getByRole("button", { name });
const rowTitles = () =>
  [...document.querySelectorAll(".tree-row.agent")].map((r) => r.textContent?.slice(4, 6));

test("moveNextTo places an item before or after another, and ignores a missing one", () => {
  expect(moveNextTo(["a", "b", "c"], "a", "c", true)).toEqual(["b", "c", "a"]);
  expect(moveNextTo(["a", "b", "c"], "c", "a", false)).toEqual(["c", "a", "b"]);
  expect(moveNextTo(["a", "b", "c"], "a", "b", false)).toEqual(["a", "b", "c"]);
  const list = ["a", "b"];
  expect(moveNextTo(list, "a", "a", true)).toBe(list);
  expect(moveNextTo(list, "x", "a", true)).toBe(list);
  expect(moveNextTo(list, "a", "x", true)).toBe(list);
});

test("the order follows moves within a worktree, is remembered, and new agents go last", () => {
  agents();
  expect(ids()).toEqual(["s4", "s1", "s2", "s3"]);
  moveAgent("s3", "s1", false);
  expect(ids()).toEqual(["s4", "s3", "s1", "s2"]);
  expect(savedAgentOrder()).toEqual(["s3", "s1", "s2"]);
  // F8's cycle follows the same order.
  act(() => {
    for (const id of ["s1", "s3"]) {
      apply({ type: "agent_state", id, ...agentStatus("waiting_permission"), subagents: [] });
    }
  });
  expect(pendingAgents(useHive.getState()).map((a) => a.id)).toEqual(["s3", "s1"]);
  // A reload: the order comes back from storage before the agents do; a new one goes last.
  useHive.setState({ ...initialState, agentOrder: savedAgentOrder() }, true);
  agents();
  act(() => apply({ type: "agent_detected", channel: 5, id: "s5", ...at(fixLogin) }));
  expect(ids()).toEqual(["s4", "s3", "s1", "s2", "s5"]);
  moveAgent("s5", "s3", false);
  expect(savedAgentOrder()).toEqual(["s5", "s3", "s1", "s2"]);
});

test("an agent is not moved into another worktree, nor next to itself or an unknown one", () => {
  agents();
  const setItem = spyOn(Storage.prototype, "setItem");
  moveAgent("s4", "s1", false);
  moveAgent("s1", "s1", true);
  moveAgent("gone", "s1", true);
  moveAgent("s1", "gone", true);
  expect(ids()).toEqual(["s4", "s1", "s2", "s3"]);
  expect(setItem).not.toHaveBeenCalled();
  setItem.mockRestore();
});

test("a saved order that cannot be read is empty; a failing storage keeps the move", () => {
  localStorage.setItem("hive.agentOrder", "not json");
  expect(savedAgentOrder()).toEqual([]);
  localStorage.setItem("hive.agentOrder", JSON.stringify({ s1: 1 }));
  expect(savedAgentOrder()).toEqual([]);
  localStorage.setItem("hive.agentOrder", JSON.stringify(["s1", 2, "s2"]));
  expect(savedAgentOrder()).toEqual(["s1", "s2"]);
  expect(savedAgentOrder(null)).toEqual([]);
  agents();
  const setItem = spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("full");
  });
  moveAgent("s2", "s1", false);
  expect(ids()).toEqual(["s4", "s2", "s1", "s3"]);
  setItem.mockRestore();
});

test("Alt+↑/↓ moves the focused agent among its worktree's, keeping the focus", async () => {
  render(<App />);
  agents();
  expect(rowTitles()).toEqual(["s4", "s1", "s2", "s3"]);
  const button = () => inTree(/s1/);
  button().focus();
  act(() => void fireEvent.keyDown(button(), { key: "ArrowDown", altKey: true }));
  expect(rowTitles()).toEqual(["s4", "s2", "s1", "s3"]);
  await act(() => new Promise((done) => setTimeout(done)));
  expect(document.activeElement).toBe(button());
  act(() => void fireEvent.keyDown(button(), { key: "ArrowUp", altKey: true }));
  act(() => void fireEvent.keyDown(button(), { key: "ArrowUp", altKey: true }));
  // The first one stays first; the tree's own arrow keys did not move the focus.
  expect(rowTitles()).toEqual(["s4", "s1", "s2", "s3"]);
  expect(document.activeElement).toBe(button());
  // Without Alt, arrows still move between rows; other keys do nothing here.
  fireEvent.keyDown(button(), { key: "ArrowDown" });
  expect(document.activeElement).toBe(inTree(/s2/));
  fireEvent.keyDown(button(), { key: "Enter", altKey: true });
  expect(rowTitles()).toEqual(["s4", "s1", "s2", "s3"]);
  stepAgent("gone", 1);
  expect(ids()).toEqual(["s4", "s1", "s2", "s3"]);
});

test("dragging an agent shows a drop line and drops it there; another worktree refuses it", () => {
  render(<App />);
  agents();
  const row = (name: string) => inTree(new RegExp(name)).parentElement as HTMLElement;
  const data = { setData: () => {} };
  const box = { top: 100, height: 20, left: 0, width: 200 } as DOMRect;
  for (const n of ["s1", "s2", "s3", "s4"]) row(n).getBoundingClientRect = () => box;
  // happy-dom has no DragEvent: the pointer's fields are set on a plain event.
  const fire = (el: HTMLElement, type: "dragOver" | "drop" | "dragLeave", fields: object) => {
    const event = createEvent[type](el, { dataTransfer: data });
    for (const [key, value] of Object.entries(fields)) Object.defineProperty(event, key, { value });
    return fireEvent(el, event);
  };

  fireEvent.dragStart(row("s1"), { dataTransfer: data });
  // Over itself: no line, no drop.
  expect(fire(row("s1"), "dragOver", { clientY: 115 })).toBe(true);
  // Over another worktree's agent: refused.
  expect(fire(row("s4"), "dragOver", { clientY: 115 })).toBe(true);
  expect(row("s4").dataset.drop).toBeUndefined();
  // Over s3's lower half: the line shows after it.
  expect(fire(row("s3"), "dragOver", { clientY: 115 })).toBe(false);
  expect(row("s3").dataset.drop).toBe("after");
  fire(row("s3"), "dragOver", { clientY: 115 });
  fire(row("s3"), "dragOver", { clientY: 105 });
  expect(row("s3").dataset.drop).toBe("before");
  // Onto a child of the row is not leaving it; leaving clears the line.
  fire(row("s3"), "dragLeave", { relatedTarget: row("s3").firstChild });
  expect(row("s3").dataset.drop).toBe("before");
  fire(row("s3"), "dragLeave", { relatedTarget: null });
  expect(row("s3").dataset.drop).toBeUndefined();
  // Dropping on another worktree does nothing.
  fire(row("s4"), "drop", { clientY: 115 });
  expect(rowTitles()).toEqual(["s4", "s1", "s2", "s3"]);
  fire(row("s3"), "drop", { clientY: 115 });
  expect(rowTitles()).toEqual(["s4", "s2", "s3", "s1"]);
  expect(row("s1").dataset.drop).toBeUndefined();
  fireEvent.dragEnd(row("s1"), { dataTransfer: data });
  // With no drag going on, nothing takes a drop.
  fire(row("s2"), "drop", { clientY: 115 });
  expect(rowTitles()).toEqual(["s4", "s2", "s3", "s1"]);
});
