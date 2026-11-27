import { afterEach, beforeAll, expect, jest, mock, setSystemTime, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { notice } from "../../test/notice";
import { expectClearButton } from "../../test/searchClear";
import { App } from "../App";
import { onMessage } from "../connect";
import { apply } from "../reduce";
import { onMoved } from "../sessions";
import { initialState, select, setPanelView, setRightPanel, showHost, useHive } from "../store";
import { closeTerminal } from "../terminals";
import { transport } from "../transport";
import { MOCK_REPOS, MOCK_SESSIONS } from "../transport/mock";

beforeAll(() => {
  // happy-dom has no layout: give the list and its rows a height so the virtualizer shows rows.
  for (const [key, cls, size] of [
    ["clientHeight", "sessions", 400],
    ["offsetHeight", "session", 72],
  ] as const) {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, key)?.get;
    Object.defineProperty(HTMLElement.prototype, key, {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains(cls) ? size : original?.call(this);
      },
    });
  }
});

afterEach(() => {
  jest.useRealTimers();
  setSystemTime();
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
  // The fake sessions' times are set when the mock loads, maybe minutes before this runs.
  setSystemTime(checkout.updated_ms + 55 * 60_000);
  const listed = show(shop.id);
  expect(listed).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Loading…")).toBeDefined();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
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
  expect([open.mock.calls.length, notice()]).toEqual([0, null]);

  const search = screen.getByRole("searchbox", { name: "Search sessions" });
  fireEvent.change(search, { target: { value: " REDIRECT " } });
  expect(titles()).toEqual(["Fix the login redirect"]);
  fireEvent.change(search, { target: { value: "nothing like it" } });
  expect(list().textContent).toBe("No Claude sessions in this worktree.");

  fireEvent.click(screen.getByRole("button", { name: "Refresh sessions" }));
  expect(listed).toHaveBeenCalledTimes(2);
  act(() =>
    apply({ type: "sessions", sessions: [], error: "permission denied", truncated: false }),
  );
  expect(screen.getByText("permission denied")).toBeDefined();
});

test("Search sessions has the app's clear button, which lists them all again", () => {
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  expectClearButton("Search sessions");
  expect(screen.getByText("2 shown")).toBeDefined();
});

test("a list cut short says so at its end", () => {
  show(shop.id);
  const line = () => screen.queryByText("Older sessions are not listed.");
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  expect(line()).toBeNull();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: true }));
  // After the rows, below the list.
  expect(line()?.previousElementSibling).toBe(list());
});

test("only the rows in view are rendered", () => {
  show(shop.id);
  const many = Array.from({ length: 300 }, (_, i) => ({
    ...checkout,
    id: `s${i}`,
    title: `S${i}`,
  }));
  act(() => apply({ type: "sessions", sessions: many, error: null, truncated: false }));
  const rows = list().querySelectorAll(".session");
  expect(rows.length).toBeGreaterThan(0);
  expect(rows.length).toBeLessThan(many.length);
  expect(titles()[0]).toBe("S0");
  expect(screen.getByText("300 shown")).toBeDefined();
});

test("the open tab asks once, then the service sends the changes (D19.4)", () => {
  jest.useFakeTimers();
  const listed = show(shop.id);
  expect(listed).toHaveBeenCalledTimes(1);
  act(() => jest.advanceTimersByTime(60_000));
  expect(listed).toHaveBeenCalledTimes(1);
  // A list the service pushes shows as it comes.
  act(() => apply({ type: "sessions", sessions: [checkout], error: null, truncated: false }));
  expect(titles()).toEqual([checkout.title as string]);
});

test("a click resumes a session; a running one shows its state and its terminal", () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
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
      alert: null,
      notify: false,
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

test("⋯ and a right click open a session's menu of actions", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  const write = spyOn(transport, "writeTerminal").mockResolvedValue();
  const located = spyOn(transport, "locateSession").mockResolvedValue();
  const deleted = spyOn(transport, "deleteSession").mockResolvedValue();
  const writeText = spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
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
  // None when Command Prompt cannot quote its folders.
  const unquotable = MOCK_SESSIONS.map((s) =>
    s === checkout ? { ...s, resume_command: null } : s,
  );
  act(() => apply({ type: "sessions", sessions: unquotable, error: null, truncated: false }));
  actions(name);
  const copyItem = screen.getByRole("menuitem", { name: "Copy Resume Command" });
  expect(copyItem.hasAttribute("disabled")).toBe(true);
  expect(copyItem.getAttribute("title")).toBe("Command Prompt cannot quote this session's folders");
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
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

  // Running outside Hive: neither resumed nor deleted here; a new session from it is fine.
  act(() => select(shop.worktrees[1].id));
  actions("Fix the login redirect");
  const resumeItem = screen.getByRole("menuitem", { name: "Resume in Worktree" });
  expect([resumeItem.hasAttribute("disabled"), resumeItem.title]).toEqual([true, ""]);
  const removeItem = screen.getByRole("menuitem", { name: "Delete" }) as HTMLButtonElement;
  expect([removeItem.disabled, removeItem.title]).toEqual([true, ""]);
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
  // The menu key has no pointer: under the row.
  fireEvent.keyDown(menu() as HTMLElement, { key: "Escape" });
  fireEvent.contextMenu(row, { clientX: 0, clientY: 0 });
  expect(useHive.getState().sessionMenu).toEqual({ session: untitled.id, x: 0, y: 0 });
  // A session that went away takes its menu with it.
  act(() => apply({ type: "session_deleted", id: untitled.id }));
  expect(menu()).toBeNull();
});

test("with accounts, a row names its account and the menu continues it in another (21.10)", async () => {
  const open = spyOn(transport, "openTerminal").mockResolvedValue(8);
  spyOn(transport, "writeTerminal").mockResolvedValue();
  const moved = spyOn(transport, "moveSession").mockImplementation(async (id, config_dir) => {
    queueMicrotask(() => onMoved({ type: "session_moved", id, config_dir }));
  });
  show(shop.id);
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  const name = "Checkout totals";
  const actions = () =>
    fireEvent.click(screen.getByRole("button", { name: `Actions for ${name}` }));
  const meta = () =>
    screen.getByText(name).closest(".session")?.querySelector(".session-meta")?.textContent;
  // Only the default account: no account named, no "Continue in".
  expect(meta()).not.toContain("Default");
  actions();
  expect(screen.queryByRole("menuitem", { name: "Continue in" })).toBeNull();
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });

  const accounts = [{ name: "Work", config_dir: "/w" }];
  const claude = { accounts, default_name: "Default", share_sessions: false };
  act(() => useHive.setState({ settings: { ...initialState.settings, claude } }));
  expect(meta()).toContain("Default");
  const worked = MOCK_SESSIONS.map((s) => (s === checkout ? { ...s, config_dir: "/w" } : s));
  act(() => apply({ type: "sessions", sessions: worked, error: null, truncated: false }));
  expect(meta()).toContain("Work");
  actions();
  const item = screen.getByRole("menuitem", { name: "Continue in" });
  expect(item.getAttribute("title")).toContain("rewind checkpoints from before the move are lost");
  fireEvent.keyDown(item, { key: "Enter" });
  expect(screen.queryByRole("menu", { name: "Continue in" })).toBeNull();
  fireEvent.keyDown(item, { key: "ArrowRight" });
  const sub = screen.getByRole("menu", { name: "Continue in" });
  // Every account but its own.
  expect(
    within(sub)
      .getAllByRole("menuitem")
      .map((i) => i.textContent),
  ).toEqual(["Default"]);
  fireEvent.click(item);
  expect(screen.queryByRole("menu", { name: "Continue in" })).toBeNull();
  fireEvent.click(item);
  fireEvent.click(within(screen.getByRole("menu", { name: "Continue in" })).getByRole("menuitem"));
  expect(screen.queryByRole("menu")).toBeNull();
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(moved).toHaveBeenCalledWith(checkout.id, null);
  expect(open.mock.calls.at(-1)?.[4]).toEqual({ config_dir: null });

  // Running: its log cannot move.
  act(() => select(shop.worktrees[1].id));
  fireEvent.click(screen.getByRole("button", { name: "Actions for Fix the login redirect" }));
  const running = screen.getByRole("menuitem", { name: "Continue in" });
  expect([running.hasAttribute("disabled"), running.title]).toEqual([
    true,
    "End the session before moving it",
  ]);
});

test("a remote space's sessions: its running agent is live, and nothing opens its log here (26.18)", () => {
  spyOn(transport, "listSessions").mockResolvedValue();
  spyOn(transport, "listChanges").mockResolvedValue();
  const host = "gpu-box";
  render(<App />);
  act(() => {
    // With accounts, this Mac's sessions could continue in another; a remote's never do.
    const accounts = [{ name: "Work", config_dir: "/w" }];
    const claude = { accounts, default_name: "Default", share_sessions: false };
    useHive.setState({ settings: { ...initialState.settings, claude } });
    onMessage({ type: "host_link", host, state: "connected" });
    onMessage({ type: "projects", projects: [shop, api], host });
    onMessage({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false, host });
    // Its agent, the same session id as this Mac might have: keyed by its host.
    const { cwd } = checkout;
    onMessage({
      type: "agent_detected",
      channel: 5,
      id: checkout.id,
      project: null,
      worktree: null,
      cwd,
      host,
    });
    showHost(host);
    select(shop.id);
    setRightPanel("files");
    setPanelView("sessions");
  });
  const running = list().querySelector(".session") as HTMLElement;
  expect(running.dataset.live).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: `Actions for ${checkout.title}` }));
  const items = screen.getAllByRole("menuitem").map((i) => i.textContent);
  expect(items).not.toContain("Open Log");
  expect(items).not.toContain("Reveal Log");
  expect(items).not.toContain("Open Working Directory");
  expect(items).not.toContain("Continue in");
  expect(items).toContain("Copy Session ID");
});

const FIX_LOGIN = `${shop.id}/.claude/worktrees/fix-login`;
const fix = MOCK_SESSIONS[0];
const HOUR = 3_600_000;
const edit = {
  session: fix.id,
  title: fix.title,
  deleted: false,
  project: shop.id,
  worktree: FIX_LOGIN,
  agent: null,
  agent_type: null,
  tool: "Edit",
  tool_use_id: null,
  ms: Date.now() - 3 * HOUR,
  user_modified: true,
  start: 39,
  len: 3,
};
const changed = (truncated = false) => ({
  type: "session_files" as const,
  id: fix.id,
  files: [
    {
      worktree: FIX_LOGIN,
      path: "src/auth/session.ts",
      edits: 3,
      lines_now: 3,
      first_line: 39,
      last_ms: Date.now() - 3 * HOUR,
    },
    {
      worktree: shop.id,
      path: "src/auth/login.ts",
      edits: 1,
      lines_now: 1,
      first_line: null,
      last_ms: Date.now() - 4 * HOUR,
    },
  ],
  truncated,
});

test("a session's changed files: counted on its row, listed once opened, each opens at its line", () => {
  show(FIX_LOGIN);
  const asked = spyOn(transport, "listSessionFiles").mockResolvedValue();
  spyOn(transport, "openFile").mockResolvedValue();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  const toggle = screen.getByRole("button", { name: "4 files" });
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(toggle);
  expect(asked).toHaveBeenCalledWith(fix.id);
  expect(screen.getByText("Loading…", { selector: ".session-files" })).toBeDefined();
  act(() => apply(changed(true)));
  const files = screen.getByRole("list", { name: "Files changed" });
  const rows = within(files).getAllByRole("button");
  // One line each: the name, then the folder in two halves (cut in the middle), then the
  // numbers; the whole path only in the tooltip (28.22).
  const part = (r: HTMLElement, name: string) =>
    r.querySelector(`.session-file-${name}`)?.textContent;
  expect(rows.map((r) => [part(r, "name"), part(r, "head"), part(r, "tail")])).toEqual([
    ["session.ts", "src", "/auth"],
    ["login.ts", "src", "/auth"],
  ]);
  expect(rows.map((r) => r.title)).toEqual(["src/auth/session.ts", "src/auth/login.ts"]);
  // Another worktree's file names it.
  expect(rows.map((r) => r.querySelector(".session-meta")?.textContent)).toEqual([
    "3 edits · 3 lines · 3h ago",
    "main · 1 edit · 1 line · 4h ago",
  ]);
  expect(within(files).getByText("More files are not listed.")).toBeDefined();
  // No file is the one an edit was found in.
  expect(files.querySelector("[data-focused]")).toBeNull();

  fireEvent.click(rows[0]);
  const s = useHive.getState();
  expect(s.openFile).toEqual({ worktree: FIX_LOGIN, path: "src/auth/session.ts" });
  expect(s.gotoLine).toEqual({ worktree: FIX_LOGIN, path: "src/auth/session.ts", line: 39 });
  const again = within(screen.getByRole("list", { name: "Files changed" })).getAllByRole("button");
  fireEvent.click(again[1]);
  expect(useHive.getState().openFile).toEqual({ worktree: shop.id, path: "src/auth/login.ts" });
  expect(useHive.getState().gotoLine).toBeNull();

  fireEvent.click(screen.getByRole("button", { name: "4 files" }));
  expect(screen.queryByRole("list", { name: "Files changed" })).toBeNull();
  // One file says so.
  act(() => {
    const one = { ...fix, files_changed: 1 };
    apply({ type: "sessions", sessions: [one], error: null, truncated: false });
  });
  expect(screen.getByRole("button", { name: "1 file" })).toBeDefined();
});

test("a live answer: the session's row selected, its edit told and its file highlighted", () => {
  show(FIX_LOGIN);
  spyOn(transport, "listSessionFiles").mockResolvedValue();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  const path = "src/auth/session.ts";
  act(() => {
    const ask = { worktree: FIX_LOGIN, path, line: 40, side: "worktree" as const, x: 0, y: 0 };
    useHive.setState({ provenance: { ...ask, origin: null } });
    apply({
      type: "line_author",
      worktree: FIX_LOGIN,
      path,
      line: 40,
      side: "worktree",
      origin: { kind: "session", edit, commit: null },
    });
  });
  const row = list().querySelector("[data-focused]") as HTMLElement;
  expect(row.querySelector(".session-title .label")?.textContent).toBe(fix.title as string);
  // The edit is a line of the row, apart from the button that resumes the session (28.22).
  const told = within(row).getByRole("button", {
    name: "Edited lines 39–41 · 3h ago · you changed it before accepting",
  });
  expect(told.closest(".session-main")).toBeNull();
  act(() => apply(changed()));
  const marked = row.querySelectorAll(".session-file[aria-current]");
  expect([...marked].map((r) => (r as HTMLElement).title)).toEqual([path]);
  expect(marked[0].hasAttribute("data-focused")).toBe(true);
  // The edit opens its file at its first line.
  spyOn(transport, "openFile").mockResolvedValue();
  fireEvent.click(told);
  const s = useHive.getState();
  expect(s.openFile).toEqual({ worktree: FIX_LOGIN, path });
  expect(s.gotoLine).toEqual({ worktree: FIX_LOGIN, path, line: 39 });
  // Listed here: nothing shows on top.
  expect(screen.queryByRole("region", { name: /^From / })).toBeNull();
});

test("a session of another worktree shows on top, under From, without switching", () => {
  show(shop.id);
  const asked = spyOn(transport, "listSessionFiles").mockResolvedValue();
  act(() => apply({ type: "sessions", sessions: MOCK_SESSIONS, error: null, truncated: false }));
  act(() =>
    useHive.setState({
      sessionFocus: { edit, path: "src/auth/session.ts" },
      openSessionFiles: [fix.id],
    }),
  );
  const from = screen.getByRole("region", { name: "From fix-login" });
  expect(within(from).getByText("From fix-login")).toBeDefined();
  expect(from.querySelector(".session-title .label")?.textContent).toBe(fix.title as string);
  expect(asked).toHaveBeenCalledWith(fix.id);
  // The shown worktree's list is unchanged.
  expect(titles()).toEqual(["Checkout totals", "Untitled session"]);
  expect(useHive.getState().selection).toBe(shop.id);

  // Deleted (its record stays): by its name, under the worktree its edit was in.
  act(() => {
    apply({ type: "session_deleted", id: fix.id });
    useHive.setState({ sessionFocus: { edit: { ...edit, deleted: true }, path: "x" } });
  });
  const gone = screen.getByRole("region", { name: "From fix-login" });
  expect(gone.querySelector(".session-title .label")?.textContent).toBe(fix.title as string);
  expect(gone.querySelector(".session-edit")?.textContent).toBe(
    "Edited lines 39–41 · 3h ago · you changed it before accepting · session deleted",
  );
  // A worktree no longer followed is named by its path.
  act(() =>
    useHive.setState({ sessionFocus: { edit: { ...edit, worktree: "/gone" }, path: "x" } }),
  );
  expect(screen.getByRole("region", { name: "From /gone" })).toBeDefined();
});
