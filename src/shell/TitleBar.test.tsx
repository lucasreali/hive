import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "../App";
import { addToInbox, apply, initialState, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus } from "../transport/mock";

afterEach(() => {
  cleanup();
  useHive.setState(initialState, true);
});

const button = () => screen.queryByTitle("Restart Hive to finish the update");

function offered(agents: string[] = []) {
  const install = spyOn(transport, "installUpdate").mockResolvedValue();
  render(<App />);
  act(() => apply({ type: "welcome", version: "0.1.0", distro: null }));
  for (const id of agents) {
    act(() => {
      apply({ type: "agent_detected", channel: 1, id, project: null, worktree: null, cwd: null });
      apply({ type: "agent_state", id, ...agentStatus("working"), subagents: [] });
    });
  }
  act(() => apply({ type: "update_ready", version: "0.2.0" }));
  return install;
}

test("no update, no button", () => {
  render(<App />);
  expect(button()).toBeNull();
});

test("the update button installs at once when no agent would end", () => {
  const install = offered();
  expect(button()?.textContent).toBe("Restart to update to v0.2.0");
  fireEvent.click(button() as HTMLElement);
  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button()?.textContent).toBe("Restarting…");
  expect((button() as HTMLButtonElement).disabled).toBe(true);

  // A failure says why and lets the update be tried again.
  act(() => apply({ type: "update_failed", error: "signature mismatch" }));
  expect(useHive.getState().notice).toBe("Update failed: signature mismatch");
  expect((button() as HTMLButtonElement).disabled).toBe(false);
  install.mockRestore();
});

test("with agents running the update asks first, like closing", () => {
  const install = offered(["a"]);
  fireEvent.click(button() as HTMLElement);
  const dialog = screen.getByRole("dialog", { name: "Update Hive?" });
  expect(dialog.textContent).toContain("1 agent is running. Updating restarts Hive, which ends");
  expect(install).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Update and restart Enter" }));
  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button()?.textContent).toBe("Restarting…");
  install.mockRestore();
});

test("the bell's inbox: pending agents, then the alerts; unread dot; going to an agent", () => {
  render(<App />);
  const bell = () => document.querySelector(".pending-bell") as HTMLButtonElement;
  const dot = () => bell().querySelector(".unread-dot");
  const items = () => screen.getAllByRole("menuitem");
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    for (const [channel, id] of [
      [1, "a"],
      [2, "b"],
    ] as const) {
      apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
    }
    apply({ type: "agent_title", channel: 1, id: "a", title: "fix login" });
    apply({ type: "agent_state", id: "a", ...agentStatus("waiting_permission"), subagents: [] });
    apply({ type: "agent_state", id: "b", ...agentStatus("working"), subagents: [] });
  });
  // Nothing raised yet: no dot; the empty inbox says so. (Item texts start with the icon's name.)
  expect(dot()).toBeNull();
  act(() => useHive.setState({ agentStates: {} }));
  fireEvent.click(bell());
  expect(bell().getAttribute("aria-expanded")).toBe("true");
  expect(items().map((i) => [i.textContent, (i as HTMLButtonElement).disabled])).toEqual([
    ["No notifications", true],
  ]);
  // A second click closes it.
  fireEvent.pointerDown(bell());
  fireEvent.click(bell());
  expect(screen.queryByRole("menu")).toBeNull();

  const now = Date.now();
  act(() => {
    apply({ type: "agent_state", id: "a", ...agentStatus("waiting_permission"), subagents: [] });
    addToInbox({ agent: "gone", state: "error", at: now - 120_000, text: "old failed" });
    addToInbox({ agent: "b", state: "waiting_you", at: now - 5_000, text: "Claude finished" });
  });
  expect(dot()).not.toBeNull();
  fireEvent.click(bell());
  expect(dot()).toBeNull();
  const menu = screen.getByRole("menu", { name: "Notifications" });
  expect(items().map((i) => [i.textContent, (i as HTMLButtonElement).disabled])).toEqual([
    ["waiting for permissionfix loginwaiting for permission", false],
    ["waiting for youClaude finished5s ago", false],
    ["errorold failed2m ago", true],
  ]);
  expect(menu.querySelector("hr")).not.toBeNull();
  // Focus starts on the first item; arrows move; scrolling inside keeps it open.
  expect(document.activeElement).toBe(items()[0] as HTMLElement);
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(document.activeElement).toBe(items()[1] as HTMLElement);
  fireEvent.scroll(menu);
  expect(screen.queryByRole("menu")).not.toBeNull();
  // A history item goes to its agent and closes the inbox.
  fireEvent.click(items()[1] as HTMLElement);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(useHive.getState().selection).toBe("b");

  // Esc and a click outside close it; nothing new, no dot.
  fireEvent.click(bell());
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  expect(screen.queryByRole("menu")).toBeNull();
  fireEvent.click(bell());
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(dot()).toBeNull();
  // A pending agent goes to it.
  fireEvent.click(bell());
  fireEvent.click(items()[0] as HTMLElement);
  expect(useHive.getState().selection).toBe("a");
});

test("an update failure without an offered update only says why", () => {
  apply({ type: "update_failed", error: "no update to install" });
  expect(useHive.getState()).toMatchObject({
    update: null,
    notice: "Update failed: no update to install",
  });
});
