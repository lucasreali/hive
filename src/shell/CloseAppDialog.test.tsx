import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import type { AgentState } from "../protocol";
import { apply } from "../reduce";
import { DEFAULT_SETTINGS, initialState, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus } from "../transport/mock";
import { remoteAgents, requestQuit } from "./CloseAppDialog";

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
const confirm = () => screen.getByRole("button", { name: "Close Hive" });

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

test("with confirm_close off the app closes at once, agents or not", () => {
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.agents.confirm_close = false;
  apply({ type: "settings", settings });
  closeWith(["a"]);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(closed()).toBe(true);
});

test("cancel, the header button and Esc keep the app open", () => {
  closeWith(["a"]);
  expect(document.querySelector("button kbd")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
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
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  act(() => apply({ type: "agent_removed", channel: 1, id: "a" }));
  fireEvent.click(screen.getByTitle("Close"));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(closed()).toBe(true);
});

test("only working, with-subagents and waiting agents ask; a state change counts at once (#18)", () => {
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
    for (const state of ["idle", "error", "ended"] as const) agent(state, state);
  });
  fireEvent.click(screen.getByTitle("Close"));
  expect(closed()).toBe(true);
  delete document.documentElement.dataset.closed;
  act(() => {
    agent("idle", "waiting_permission");
    agent("error", "waiting_you");
    agent("ended", "with_subagents");
  });
  fireEvent.click(screen.getByTitle("Close"));
  expect(closed()).toBe(false);
  expect(dialog().textContent).toContain("3 agents are running.");
});

test("a quit request (⌘Q) shows the same dialog while agents run; confirming quits", () => {
  render(<App />);
  act(() => agent("a"));
  act(() => requestQuit());
  expect(dialog().open).toBe(true);
  expect(closed()).toBe(false);
  fireEvent.click(confirm());
  expect(closed()).toBe(true);
});

/** Agents of remote host `host`, linked (26.18). */
function remote(host: string, ids: string[], state: AgentState = "working") {
  apply({ type: "host_link", host, state: "connected" });
  for (const id of ids) {
    apply({
      type: "agent_detected",
      channel: 9,
      id,
      project: null,
      worktree: null,
      cwd: null,
      host,
    });
    apply({ type: "agent_state", id, ...agentStatus(state), subagents: [], host });
  }
}

test("quitting asks whether each host's agents keep running, keep first (26.18, Q2)", async () => {
  const bye = spyOn(transport, "byeHosts").mockResolvedValue();
  render(<App />);
  act(() => {
    remote("gpu-box", ["a"]);
    remote("lab", ["b", "c"]);
    remote("idle", ["d"], "idle");
  });
  expect(remoteAgents(useHive.getState())).toEqual(["gpu-box\n1", "lab\n2"]);
  // No local agent, yet it asks: they would keep running.
  act(() => requestQuit());
  expect(dialog().textContent).not.toContain("running. Closing Hive ends");
  const gpu = screen.getByRole("checkbox", { name: "Keep 1 agent running on gpu-box?" });
  const lab = screen.getByRole("checkbox", { name: "Keep 2 agents running on lab?" });
  expect([(gpu as HTMLInputElement).checked, (lab as HTMLInputElement).checked]).toEqual([
    true,
    true,
  ]);
  expect(dialog().textContent).toContain("Settings → Hosts");
  fireEvent.click(lab);
  fireEvent.click(lab);
  fireEvent.click(gpu);
  fireEvent.click(confirm());
  await act(async () => {});
  expect(bye.mock.calls).toEqual([
    [
      [
        ["gpu-box", true],
        ["lab", false],
      ],
    ],
  ]);
  expect(closed()).toBe(true);
  bye.mockRestore();
});

test("a host whose link is down is not asked about", () => {
  remote("gpu-box", ["a"]);
  apply({ type: "host_link", host: "gpu-box", state: "reconnecting", reason: "x", retry_secs: 1 });
  expect(remoteAgents(useHive.getState())).toEqual([]);
});
