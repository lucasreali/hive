import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "../App";
import { apply, initialState, select, setPanelView, setRightPanel, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS, MOCK_SESSIONS } from "../transport/mock";

afterEach(() => {
  for (const tab of useHive.getState().tabs) closeTerminal(tab.id);
  mock.restore();
  cleanup();
  useHive.setState(initialState, true);
});

const [shop, api] = MOCK_REPOS;
const [login, checkout, untitled, refactor] = MOCK_SESSIONS;

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
const titles = () => [...list().querySelectorAll(".session-title")].map((t) => t.textContent);

test("Sessions lists the shown worktree's sessions only, searched", () => {
  const listed = show(shop.id);
  expect(listed).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Loading…")).toBeDefined();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null }));
  // shop's main worktree: its own sessions, one of them started in a subfolder.
  expect(titles()).toEqual(["Checkout totals", "Untitled session"]);
  expect(screen.getByText("2 shown")).toBeDefined();
  const first = list().querySelector(".session") as HTMLElement;
  expect(first.querySelector(".session-last")?.textContent).toBe(
    "You: [Request interrupted by user]",
  );
  expect(first.querySelector(".session-meta")?.textContent).toBe(
    "7 msgs · 55m ago · claude-opus-5-5",
  );

  act(() => select(shop.worktrees[1].id));
  expect(titles()).toEqual(["Fix the login redirect"]);
  expect(list().querySelector("b")?.textContent).toBe("Agent:");

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
      subagents: [],
    });
    select(shop.id);
  });
  const running = list().querySelector(".session") as HTMLElement;
  expect(running.dataset.live).toBe("true");
  expect(within(running).getByRole("img", { name: "working" })).toBeDefined();
  expect(within(running).getByTitle("Show its terminal")).toBeDefined();
});

test("⋯ and a right click open a session's menu of actions", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const located = spyOn(transport, "locateSession").mockResolvedValue();
  const deleted = spyOn(transport, "deleteSession").mockResolvedValue();
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  spyOn(window, "confirm").mockReturnValue(true);
  show(shop.worktrees[1].id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null }));
  const menu = () => screen.queryByRole("menu");
  const pick = (name: string, from: string) => {
    fireEvent.click(screen.getByRole("button", { name: `Actions for ${from}` }));
    fireEvent.click(screen.getByRole("menuitem", { name }));
    expect(menu()).toBeNull();
  };
  fireEvent.click(screen.getByRole("button", { name: "Actions for Fix the login redirect" }));
  expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual([
    "Resume in Worktree",
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

  const name = "Fix the login redirect";
  pick("Copy Resume Command", name);
  expect(writeText).toHaveBeenLastCalledWith(`cd '${login.cwd}' && claude --resume ${login.id}`);
  pick("Copy Session ID", name);
  expect(writeText).toHaveBeenLastCalledWith(login.id);
  pick("Copy Log Path", name);
  expect(writeText).toHaveBeenLastCalledWith(login.log);
  pick("Open Log", name);
  pick("Reveal Log", name);
  pick("Open Working Directory", name);
  expect(located.mock.calls).toEqual([
    [login.id, "log"],
    [login.id, "log"],
    [login.id, "folder"],
  ]);
  pick("Delete", name);
  expect(deleted).toHaveBeenCalledWith(login.id);

  act(() => select(api.worktrees[1].id));
  pick("Continue in New Session", "Refactor auth middleware");
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(write).toHaveBeenLastCalledWith(8, `claude --resume ${refactor.id} --fork-session\r`);
  act(() => select(shop.id));
  pick("Resume in Worktree", untitled.id);
  expect(open.mock.calls.at(-1)?.[0]).toBe(untitled.cwd);

  // A right click opens it at the pointer; a running session cannot be deleted.
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
    select(shop.id);
  });
  const row = screen.getByText("Checkout totals").closest("button") as HTMLElement;
  fireEvent.contextMenu(row, { clientX: 30, clientY: 40 });
  expect(useHive.getState().sessionMenu).toEqual({ session: checkout.id, x: 30, y: 40 });
  expect(screen.getByRole("menuitem", { name: "Show Its Terminal" })).toBeDefined();
  const remove = screen.getByRole("menuitem", { name: "Delete" }) as HTMLButtonElement;
  expect([remove.disabled, remove.title]).toEqual([true, "End the session before deleting it"]);
  // The menu key has no pointer: under the row.
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });
  fireEvent.contextMenu(row, { clientX: 0, clientY: 0 });
  expect(useHive.getState().sessionMenu).toEqual({ session: checkout.id, x: 0, y: 0 });
  // A session that went away takes its menu with it.
  act(() => apply({ type: "session_deleted", id: checkout.id }));
  expect(menu()).toBeNull();
});
