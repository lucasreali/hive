import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import {
  addTab,
  apply,
  initialState,
  select,
  setPanelView,
  setRightPanel,
  useHive,
} from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS, MOCK_SESSIONS } from "../transport/mock";
import { REFRESH_MS } from "./SessionsView";

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const [, checkout, untitled, refactor] = MOCK_SESSIONS;

/** The right panel's Sessions, for the worktree `worktree`. */
function show(worktree: string) {
  const listed = spyOn(transport, "listSessions").mockResolvedValue();
  spyOn(transport, "listChanges").mockResolvedValue();
  render(<App />);
  act(() => {
    apply({ type: "projects", projects: [shop, api] });
    select(worktree);
    setRightPanel("files");
    setPanelView("sessions");
  });
  return listed;
}

const list = () => screen.getByRole("list", { name: "Sessions" });
const titles = () =>
  [...list().querySelectorAll(".session-title .label")].map((t) => t.textContent);
const icons = () =>
  [...list().querySelectorAll(".session-title [role=img]")].map((i) =>
    i.getAttribute("aria-label"),
  );

test("Sessions lists the shown worktree's sessions only, searched", () => {
  const listed = show(shop.id);
  expect(listed).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Loading…")).toBeDefined();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null }));
  // shop's main worktree: its own sessions, one of them started in a subfolder.
  expect(titles()).toEqual(["Checkout totals", "Untitled session"]);
  // Every session has its state: here, one left mid-turn and one that ended.
  expect(icons()).toEqual(["waiting for you", "ended"]);
  expect(screen.getByText("2 shown")).toBeDefined();
  const first = list().querySelector(".session") as HTMLElement;
  expect(first.querySelector(".session-last")?.textContent).toBe(
    "You: [Request interrupted by user]",
  );
  expect(first.querySelector(".session-meta")?.textContent).toBe(
    "7 msgs · 14k ctx · 2k out · 55m ago · claude-opus-5-5",
  );

  act(() => select(shop.worktrees[1].id));
  expect(titles()).toEqual(["Fix the login redirect"]);
  expect(list().querySelector("b")?.textContent).toBe("Agent:");
  // Running outside Hive: its state from the log, and it is not resumed here.
  const outside = list().querySelector(".session") as HTMLElement;
  expect(outside.dataset.running).toBe("true");
  expect(icons()).toEqual(["waiting for you"]);
  const main = outside.querySelector(".session-main") as HTMLElement;
  expect(main.hasAttribute("title")).toBe(false);
  // A click does nothing.
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  fireEvent.click(main);
  expect([open.mock.calls.length, useHive.getState().notice]).toEqual([0, null]);

  const search = screen.getByRole("searchbox", { name: "Search sessions" });
  fireEvent.change(search, { target: { value: " REDIRECT " } });
  expect(titles()).toEqual(["Fix the login redirect"]);
  fireEvent.change(search, { target: { value: "nothing like it" } });
  expect(list().textContent).toBe("No Claude sessions in this worktree.");

  fireEvent.click(screen.getByRole("button", { name: "Refresh sessions" }));
  expect(listed).toHaveBeenCalledTimes(2);
  act(() => apply({ type: "sessions", sessions: [], error: "permission denied" }));
  expect(screen.getByText("permission denied")).toBeDefined();
});

test("the open tab asks for the sessions again every few seconds", () => {
  const every = spyOn(globalThis, "setInterval");
  const listed = show(shop.id);
  const [tick, ms] = every.mock.calls.at(-1) as [() => void, number];
  expect(ms).toBe(REFRESH_MS);
  tick();
  expect(listed).toHaveBeenCalledTimes(2);
});

test("a click resumes a session; a running one shows its state and its terminal", () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null }));
  fireEvent.click(screen.getAllByTitle("Resume in its worktree")[1] as HTMLElement);
  expect(open.mock.calls[0]?.[0]).toBe(untitled.cwd);

  const { cwd } = checkout;
  act(() => {
    apply({
      type: "agent_detected",
      channel: 5,
      id: checkout.id,
      project: null,
      worktree: null,
      cwd,
    });
    apply({
      type: "agent_state",
      id: checkout.id,
      state: "working",
      urgency: 2,
      pending: false,
      interrupted: false,
      subagents: [],
      activity: null,
      since_ms: 0,
    });
    select(shop.id);
  });
  const running = list().querySelector(".session") as HTMLElement;
  expect(running.dataset.live).toBe("true");
  expect(within(running).getByRole("img", { name: "working" })).toBeDefined();
  expect(within(running).getByTitle("Show its terminal")).toBeDefined();
});

test("a session running in a Hive chat shows its chat", () => {
  show(shop.id);
  const { cwd } = checkout;
  act(() => {
    apply({ type: "sessions", sessions: [{ ...checkout, running: true }], error: null });
    addTab(7, cwd, "chat");
    apply({
      type: "agent_detected",
      channel: 7,
      id: checkout.id,
      project: null,
      worktree: null,
      cwd,
    });
    select(shop.id);
  });
  const row = list().querySelector(".session") as HTMLElement;
  expect(row.dataset.live).toBe("true");
  fireEvent.click(within(row).getByTitle("Show its chat"));
  expect(useHive.getState().activeTab).toBe(7);
  fireEvent.click(screen.getByRole("button", { name: `Actions for ${checkout.title}` }));
  const shown = screen.getByRole("menuitem", { name: "Show Its Chat" }) as HTMLButtonElement;
  expect(shown.disabled).toBe(false);
  const remove = screen.getByRole("menuitem", { name: "Delete" }) as HTMLButtonElement;
  expect([remove.disabled, remove.title]).toEqual([true, "End the session before deleting it"]);
});

test("⋯ and a right click open a session's menu of actions", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const located = spyOn(transport, "locateSession").mockResolvedValue();
  const deleted = spyOn(transport, "deleteSession").mockResolvedValue();
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null }));
  const menu = () => screen.queryByRole("menu");
  const actions = (from: string) =>
    fireEvent.click(screen.getByRole("button", { name: `Actions for ${from}` }));
  const pick = (name: string, from: string) => {
    actions(from);
    fireEvent.click(screen.getByRole("menuitem", { name }));
    expect(menu()).toBeNull();
  };
  const name = "Checkout totals";
  actions(name);
  expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
    "Resume in Worktree",
    "Open as Chat",
    "Continue in New Session",
    "Copy Resume Command",
    "Open Log",
    "Reveal Log",
    "Open Working Directory",
    "Copy Session ID",
    "Copy Log Path",
    "Delete",
  ]);
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });

  pick("Copy Resume Command", name);
  expect(writeText).toHaveBeenLastCalledWith(
    `cd '${checkout.cwd}' && claude --resume ${checkout.id}`,
  );
  pick("Copy Session ID", name);
  expect(writeText).toHaveBeenLastCalledWith(checkout.id);
  pick("Copy Log Path", name);
  expect(writeText).toHaveBeenLastCalledWith(checkout.log);
  pick("Open Log", name);
  pick("Reveal Log", name);
  pick("Open Working Directory", name);
  expect(located.mock.calls).toEqual([
    [checkout.id, "log"],
    [checkout.id, "log"],
    [checkout.id, "folder"],
  ]);
  pick("Delete", name);
  expect(deleted).not.toHaveBeenCalled(); // Asked first (sessions.test.ts).
  fireEvent.click(screen.getByRole("button", { name: "Delete" })); // The Hive dialog.
  expect(deleted).toHaveBeenCalledWith(checkout.id);

  act(() => select(api.worktrees[1].id));
  pick("Continue in New Session", "Refactor auth middleware");
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(write).toHaveBeenLastCalledWith(8, `claude --resume ${refactor.id} --fork-session\r`);
  act(() => select(shop.id));
  pick("Resume in Worktree", untitled.id);
  expect(open.mock.calls.at(-1)?.[0]).toBe(untitled.cwd);
  const chat = spyOn(transport, "openChat").mockResolvedValue(9);
  pick("Open as Chat", untitled.id);
  expect(chat.mock.calls).toEqual([[untitled.cwd, untitled.id, null]]);

  // Running outside Hive: neither resumed nor deleted here; a new session from it is fine.
  act(() => select(shop.worktrees[1].id));
  actions("Fix the login redirect");
  const resumeItem = screen.getByRole("menuitem", { name: "Resume in Worktree" });
  expect([resumeItem.hasAttribute("disabled"), resumeItem.title]).toEqual([true, ""]);
  const removeItem = screen.getByRole("menuitem", { name: "Delete" }) as HTMLButtonElement;
  expect([removeItem.disabled, removeItem.title]).toEqual([true, ""]);
  const chatItem = screen.getByRole("menuitem", { name: "Open as Chat" }) as HTMLButtonElement;
  expect([chatItem.disabled, chatItem.title]).toEqual([true, ""]);
  const fork = screen.getByRole("menuitem", { name: "Continue in New Session" });
  expect(fork.hasAttribute("disabled")).toBe(false);
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });

  // A right click opens it at the pointer; a session running in Hive cannot be deleted.
  const { cwd } = untitled;
  act(() => {
    apply({
      type: "agent_detected",
      channel: 5,
      id: untitled.id,
      project: null,
      worktree: null,
      cwd,
    });
    select(shop.id);
  });
  const row = screen.getByText("Untitled session").closest("button") as HTMLElement;
  fireEvent.contextMenu(row, { clientX: 30, clientY: 40 });
  expect(useHive.getState().sessionMenu).toEqual({ session: untitled.id, x: 30, y: 40 });
  expect(screen.getByRole("menuitem", { name: "Show Its Terminal" })).toBeDefined();
  const remove = screen.getByRole("menuitem", { name: "Delete" }) as HTMLButtonElement;
  expect([remove.disabled, remove.title]).toEqual([true, "End the session before deleting it"]);
  const asChat = screen.getByRole("menuitem", { name: "Open as Chat" }) as HTMLButtonElement;
  expect(asChat.disabled).toBe(true);
  // The menu key has no pointer: under the row.
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });
  fireEvent.contextMenu(row, { clientX: 0, clientY: 0 });
  expect(useHive.getState().sessionMenu).toEqual({ session: untitled.id, x: 0, y: 0 });
  // A session that went away takes its menu with it.
  act(() => apply({ type: "session_deleted", id: untitled.id }));
  expect(menu()).toBeNull();
});
