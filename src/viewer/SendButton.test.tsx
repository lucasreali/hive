import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { AgentStatus } from "../protocol";
import { type HiveState, initialState, useHive } from "../store";
import { SendButton } from "./SendButton";
import { savedPick } from "./target";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  useHive.setState(initialState, true);
});

const wt = "/home/u/shop";
const agent = (id: string, terminal: number) => ({
  id,
  terminal,
  project: wt,
  worktree: wt,
  cwd: wt,
});
const status = (state: AgentStatus["state"], activity: string | null) =>
  ({ state, activity, since_ms: 0 }) as AgentStatus;
// A dev server's terminal (1, active) and two agents' (2 and 3).
const ready: Partial<HiveState> = {
  tabs: [1, 2, 3].map((id) => ({ id, cwd: wt })),
  activeTab: 1,
};

function button(why: string | null = null) {
  const send = mock((_: number) => {});
  render(
    <SendButton worktree={wt} why={why} className="ghost text" title="Paste it" onSend={send}>
      Send
    </SendButton>,
  );
  return { send, button: screen.getByRole("button", { name: "Send" }) as HTMLButtonElement };
}

test("with no agent in the worktree it is disabled and says why; its own reason comes first", () => {
  useHive.setState(ready);
  const { button: b } = button();
  expect([b.disabled, b.title]).toEqual([true, "No agent in this worktree"]);
  cleanup();
  useHive.setState({ agents: { a: agent("a", 2) } });
  const { button: other } = button("No open comments to send");
  expect([other.disabled, other.title]).toEqual([true, "No open comments to send"]);
});

test("with one agent it sends there at once, never to the active plain terminal", () => {
  useHive.setState({ ...ready, agents: { a: agent("a", 2) } });
  const { send, button: b } = button();
  expect([b.disabled, b.title, b.getAttribute("aria-haspopup")]).toEqual([false, "Paste it", null]);
  fireEvent.click(b);
  expect(send).toHaveBeenCalledWith(2);
  expect(screen.queryByRole("menu")).toBeNull();
});

test("with several a menu lists them, the last picked first, and the pick is remembered", () => {
  useHive.setState({
    ...ready,
    agents: { a: agent("a", 2), b: agent("b", 3) },
    agentTitles: { a: "fix the login" },
    agentStates: { a: status("working", "Edit src/a.ts"), b: status("waiting_you", null) },
  });
  const { send, button: b } = button();
  expect(b.getAttribute("aria-haspopup")).toBe("menu");
  expect(b.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(b);
  const menu = screen.getByRole("menu", { name: "Send to agent" });
  const items = () =>
    screen
      .getAllByRole("menuitem")
      .map((i) => [".label", ".inbox-meta"].map((c) => i.querySelector(c)?.textContent));
  // The tab's title (untitled, as its tab would show it), the state and the activity.
  expect(items()).toEqual([
    ["fix the login", "working · Edit src/a.ts"],
    ["Claude", "waiting for you"],
  ]);
  expect(menu.querySelectorAll(".state-icon")).toHaveLength(2);
  expect(b.getAttribute("aria-expanded")).toBe("true");
  expect(send).not.toHaveBeenCalled();

  fireEvent.click(screen.getAllByRole("menuitem")[1] as HTMLElement);
  expect(send).toHaveBeenCalledWith(3);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(savedPick(wt)).toBe("b");

  // Next time the one picked comes first; the button again closes the menu.
  fireEvent.click(b);
  expect(items().map(([label]) => label)).toEqual(["Claude", "fix the login"]);
  fireEvent.click(b);
  expect(screen.queryByRole("menu")).toBeNull();
  // Open, each entry follows its agent's state.
  fireEvent.click(b);
  act(() => useHive.setState({ agentStates: { a: status("idle", null) } }));
  expect(items()).toEqual([
    ["Claude", "idle"],
    ["fix the login", "idle"],
  ]);
});
