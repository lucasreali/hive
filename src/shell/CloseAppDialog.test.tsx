import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { type AgentState, apply, initialState, useHive } from "../store";
import { agentStatus } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
  delete document.documentElement.dataset.closed;
});

const closed = () => document.documentElement.dataset.closed !== undefined;
const agent = (id: string, state: AgentState = "working") => {
  apply({ type: "agent_detected", channel: 1, id, project: null, worktree: null, cwd: null });
  apply({ type: "agent_state", id, ...agentStatus(state), subagents: [] });
};

function closeWith(agents: string[]) {
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  for (const id of agents) act(() => agent(id));
  fireEvent.click(screen.getByTitle("Close"));
}

const dialog = () => screen.getByRole("dialog", { name: "Close Hive?" }) as HTMLDialogElement;
const confirm = () => screen.getByRole("button", { name: "Close Hive Enter" });

test("with no agent the app closes at once", () => {
  closeWith([]);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(closed()).toBe(true);
});

test("with agents the close waits for a confirmation, focused on it", () => {
  closeWith(["a"]);
  expect(closed()).toBe(false);
  expect(dialog().open).toBe(true);
  expect(dialog().textContent).toContain("1 agent is running.");
  expect(document.activeElement).toBe(confirm());
  act(() => agent("b"));
  expect(dialog().textContent).toContain("2 agents are running.");
  fireEvent.click(confirm());
  expect(closed()).toBe(true);
});

test("cancel, the header button and Esc keep the app open", () => {
  closeWith(["a"]);
  fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByTitle("Close"));
  fireEvent.click(screen.getByTitle("Close (Esc)"));
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByTitle("Close"));
  act(() => dialog().close());
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(closed()).toBe(false);
});

test("a removed agent no longer asks", () => {
  closeWith(["a"]);
  fireEvent.click(screen.getByRole("button", { name: "Cancel Esc" }));
  act(() => apply({ type: "agent_removed", channel: 1, id: "a" }));
  fireEvent.click(screen.getByTitle("Close"));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(closed()).toBe(true);
});

test("only working and waiting agents ask; a state change counts at once (#18)", () => {
  render(<App />);
  act(() => {
    apply({
      type: "agent_detected",
      channel: 1,
      id: "new",
      project: null,
      worktree: null,
      cwd: null,
    });
    for (const state of ["idle", "error", "with_subagents", "ended"] as const) agent(state, state);
  });
  fireEvent.click(screen.getByTitle("Close"));
  expect(closed()).toBe(true);
  delete document.documentElement.dataset.closed;
  act(() => {
    agent("idle", "waiting_permission");
    agent("error", "waiting_you");
  });
  fireEvent.click(screen.getByTitle("Close"));
  expect(closed()).toBe(false);
  expect(dialog().textContent).toContain("2 agents are running.");
});
