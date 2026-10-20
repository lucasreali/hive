import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { asMac } from "../../test/mac";
import { notice } from "../../test/notice";
import { LINK_DOWN, unsent } from "../../test/unsent";
import { App } from "../App";
import { apply } from "../reduce";
import { addToInbox, initialState, pendingAgents, useHive } from "../store";
import { transport } from "../transport";
import { agentStatus, MOCK_REPOS } from "../transport/mock";
import { TitleBar } from "./TitleBar";
import { RemoveWorktreeDialog } from "./WorktreeMenu";

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

test("an install that cannot be sent is shown as an error toast; the button comes back (9.21)", async () => {
  offered().mockRestore();
  const restore = unsent("installUpdate");
  fireEvent.click(button() as HTMLElement);
  expect(button()?.textContent).toBe("Restarting…");
  await waitFor(() => expect(button()?.textContent).toBe("Restart to update to v0.2.0"));
  restore();
  expect(notice()).toBe(LINK_DOWN);
  expect((button() as HTMLButtonElement).disabled).toBe(false);
});

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
  expect(notice()).toBe("Update failed: signature mismatch");
  expect((button() as HTMLButtonElement).disabled).toBe(false);
  install.mockRestore();
});

test("with agents running the update asks first, like closing", () => {
  const install = offered(["a"]);
  fireEvent.click(button() as HTMLElement);
  const dialog = screen.getByRole("dialog", { name: "Update Hive?" });
  expect(dialog.textContent).toContain("1 agent is running. Updating restarts Hive, which ends");
  expect(install).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Update and restart" }));
  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button()?.textContent).toBe("Restarting…");
  install.mockRestore();
});

test("a usage alert in the inbox is about no agent: choosing it only closes the inbox (17.23)", () => {
  render(<App />);
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    useHive.setState({ selection: "x" });
    addToInbox({ agent: null, state: null, at: Date.now() - 5_000, text: "Session usage at 80%" });
  });
  fireEvent.click(document.querySelector(".pending-bell") as HTMLButtonElement);
  const item = screen.getByRole("menuitem", { name: /Session usage at 80%/ });
  expect(item.textContent).toBe("Session usage at 80%5s ago");
  expect((item as HTMLButtonElement).disabled).toBe(false);
  expect(item.title).toBe("");
  expect(item.querySelector(".usage-alert-icon")?.getAttribute("aria-label")).toBe("Usage");
  fireEvent.click(item);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(useHive.getState().selection).toBe("x");
});

test("the bell's inbox: pending agents, then the alerts; going to an agent", () => {
  render(<App />);
  const bell = () => document.querySelector(".pending-bell") as HTMLButtonElement;
  const dot = () => bell().querySelector(".pending-count");
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
  // Seen once the bell opens: no badge; the empty inbox says so. (Item texts start with the icon's name.)
  act(() => useHive.setState({ agentStates: {} }));
  fireEvent.click(bell());
  expect(dot()).toBeNull();
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
  // A re-render (here a new title) keeps the focus where it is.
  act(() => apply({ type: "agent_title", channel: 1, id: "a", title: "fix login 2" }));
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

test("a terminal's inbox item shows that terminal while it is open (17.25)", () => {
  render(<App />);
  const items = () => screen.getAllByRole("menuitem");
  const now = Date.now();
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    useHive.setState({
      tabs: [
        { id: 3, cwd: "/w" },
        { id: 4, cwd: "/v" },
      ],
      activeTab: 4,
    });
    addToInbox({ terminal: 9, failed: false, at: now - 60_000, text: "make: done" });
    addToInbox({ terminal: 3, failed: true, at: now - 5_000, text: "Command failed (exit 2)" });
  });
  fireEvent.click(document.querySelector(".pending-bell") as HTMLButtonElement);
  expect(items().map((i) => [i.textContent, (i as HTMLButtonElement).disabled])).toEqual([
    ["Command failed (exit 2)5s ago", false],
    ["make: done1m ago", true],
  ]);
  expect(items()[1]?.title).toBe("This terminal is closed");
  const icons = document.querySelectorAll(".inbox-terminal");
  expect([...icons].map((i) => i.getAttribute("data-failed"))).toEqual(["true", null]);
  fireEvent.click(items()[0] as HTMLElement);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(useHive.getState().activeTab).toBe(3);
});

test("the badge counts pending agents not seen yet; opening the bell clears it (8.5)", () => {
  render(<App />);
  const bell = () => document.querySelector(".pending-bell") as HTMLButtonElement;
  const badge = () => bell().querySelector(".pending-count")?.textContent ?? null;
  const filled = () => bell().getAttribute("data-pending");
  const state = (id: string, s: Parameters<typeof agentStatus>[0]) =>
    act(() => apply({ type: "agent_state", id, ...agentStatus(s), subagents: [] }));
  const open = () => {
    fireEvent.click(bell());
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  };
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    for (const [channel, id] of [
      [1, "a"],
      [2, "b"],
      [3, "c"],
    ] as const) {
      apply({ type: "agent_detected", channel, id, project: null, worktree: null, cwd: null });
    }
  });
  state("a", "waiting_permission");
  state("b", "waiting_you");
  state("c", "working");
  expect([badge(), filled(), bell().title]).toEqual([
    "2",
    "true",
    "2 pending: notifications (F8: next pending)",
  ]);
  // Opening marks them seen: no badge, a plain bell; no navigation.
  open();
  expect([badge(), filled(), bell().title]).toEqual([
    null,
    "false",
    "Notifications (F8: next pending)",
  ]);
  expect(useHive.getState().selection).toBeNull();
  // They stay pending: the inbox still lists them.
  expect(pendingAgents(useHive.getState()).map((a) => a.id)).toEqual(["a", "b"]);
  // Another update in the same pending state is not a new alert.
  state("a", "waiting_permission");
  expect(badge()).toBeNull();
  // A new pending agent counts; so does a new pending reason.
  state("c", "error");
  expect(badge()).toBe("1");
  state("b", "waiting_permission");
  expect(badge()).toBe("2");
  open();
  expect(badge()).toBeNull();
  // Pending again after working counts again, even in the same state.
  state("a", "working");
  expect(badge()).toBeNull();
  state("a", "waiting_permission");
  expect(badge()).toBe("1");
  // An agent that goes away forgets it was seen.
  open();
  act(() => apply({ type: "agent_removed", channel: 2, id: "b" }));
  expect(useHive.getState().pendingSeen).toEqual({ a: "waiting_permission", c: "error" });
});

test("an offer to remove a worktree opens its remove dialog; never removes by itself (17.27)", () => {
  const [shop] = MOCK_REPOS;
  const fix = shop.worktrees[1];
  const remove = spyOn(transport, "removeWorktree").mockResolvedValue();
  // Without the rest of the app, which would ask the mock service for the project's data.
  render(
    <>
      <TitleBar />
      <RemoveWorktreeDialog />
    </>,
  );
  const bell = () => document.querySelector(".pending-bell") as HTMLButtonElement;
  const items = () => screen.getAllByRole("menuitem") as HTMLButtonElement[];
  const offer = (worktree: string, state: "merged" | "closed", text: string) =>
    addToInbox({
      offer: { worktree, number: 12, title: "Fix the login", state },
      at: Date.now() - 60_000,
      text,
      space: "shop",
    });
  act(() => {
    apply({ type: "welcome", version: "0.1.0", distro: null });
    apply({ type: "projects", projects: [shop] });
    offer("/gone", "closed", "#12 closed: remove its worktree?");
    offer(fix.id, "merged", `#12 merged: remove ${fix.name}?`);
  });
  fireEvent.click(bell());
  expect(items().map((i) => [i.textContent, i.disabled, i.title])).toEqual([
    [`#12 merged: remove ${fix.name}?shop · 1m ago`, false, "Fix the login\nRemove the worktree…"],
    ["#12 closed: remove its worktree?shop · 1m ago", true, "This worktree is gone"],
  ]);
  fireEvent.click(items()[0] as HTMLElement);
  expect(screen.queryByRole("menu")).toBeNull();
  const dialog = screen.getByRole("dialog", { name: "Delete worktree" });
  expect(dialog.textContent).toContain(fix.path);
  expect(remove).not.toHaveBeenCalled();
  remove.mockRestore();
});

test("an update failure without an offered update only says why", () => {
  apply({ type: "update_failed", error: "no update to install" });
  expect(useHive.getState()).toMatchObject({
    update: null,
    notices: [{ kind: "error", text: "Update failed: no update to install" }],
  });
});

test("on macOS the bell names ⇧⌘J for the next pending agent (F8 is Play/Pause there)", () => {
  asMac();
  render(<App />);
  const bell = document.querySelector(".pending-bell") as HTMLButtonElement;
  expect(bell.title).toBe("Notifications (⇧⌘J: next pending)");
});
