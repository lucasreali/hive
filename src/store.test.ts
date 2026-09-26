import { beforeEach, expect, test } from "bun:test";
import { renderHook } from "@testing-library/react";
import {
  type AgentState,
  activateTab,
  addTab,
  agentWorkingIn,
  apply,
  CHAT_LIMIT,
  type ChatEntry,
  DEFAULT_SETTINGS,
  fileVisible,
  hideTranscript,
  initialState,
  mergeEntries,
  openFileDialog,
  openFileMenu,
  openModal,
  panelWorktree,
  removeTab,
  type ServiceMessage,
  type Subagent,
  select,
  setChat,
  setEdit,
  setEditing,
  setEditorNotice,
  setOpenFile,
  setRightPanel,
  showFile,
  showTranscript,
  TRANSCRIPT_LIMIT,
  type TranscriptEntry,
  tabPlace,
  tabsPlace,
  toggleCollapsed,
  useHive,
  useTerminal,
  visibleTabs,
} from "./store";
import { MOCK_REPOS, MOCK_SESSIONS } from "./transport/mock";
import { MOCK_CHAT_REQUESTS } from "./transport/mockChat";
import { type EditBuffer, toText } from "./viewer/buffer";

beforeEach(() => useHive.setState(initialState, true));

test("starts connecting with no data", () => {
  const s = useHive.getState();
  expect(s.connection).toEqual({ status: "connecting" });
  expect(s.terminals).toEqual({});
  expect(s.modal).toBeNull();
});

test("settings replace the defaults; a failure keeps them and says why", () => {
  expect(useHive.getState().settings).toEqual(DEFAULT_SETTINGS);
  apply({ type: "settings_failed", message: "Ignoring settings.json" });
  let s = useHive.getState();
  expect([s.settings, s.settingsError, s.notice]).toEqual([
    DEFAULT_SETTINGS,
    "Ignoring settings.json",
    "Ignoring settings.json",
  ]);
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.volume = 0;
  apply({ type: "settings", settings });
  s = useHive.getState();
  expect([s.settings, s.settingsError]).toEqual([settings, null]);
});

test("welcome and version_mismatch set the connection", () => {
  apply({ type: "welcome", version: "0.1.0", distro: "Ubuntu" });
  expect(useHive.getState().connection).toEqual({
    status: "connected",
    version: "0.1.0",
    distro: "Ubuntu",
  });
  const versions = { protocol: 2, version: "0.2.0", app_protocol: 1, app_version: "0.1.0" };
  apply({ type: "version_mismatch", ...versions });
  expect(useHive.getState().connection).toEqual({ status: "version_mismatch", ...versions });
});

test("disconnected keeps the reason; unknown messages change nothing", () => {
  apply({ type: "disconnected", reason: "the hive bridge exited" });
  const before = useHive.getState();
  expect(before.connection).toEqual({ status: "disconnected", reason: "the hive bridge exited" });
  apply({ type: "agent" } as unknown as ServiceMessage);
  expect(useHive.getState()).toEqual(before);
});

test("terminal messages update only their terminal", () => {
  apply({ type: "terminal_opened", channel: 1 });
  apply({ type: "terminal_opened", channel: 2 });
  apply({ type: "unhooked_agent", channel: 1 });
  apply({ type: "badge", channel: 1, text: "db" });
  expect(useHive.getState().terminals[1]?.badge).toBe("db");
  apply({ type: "terminal_exited", channel: 1, code: 3 });
  const { terminals } = useHive.getState();
  expect(terminals[1]).toEqual({ id: 1, exited: true, code: 3, unhooked: true, badge: "" });
  expect(terminals[2]).toEqual({ id: 2, exited: false, code: null, unhooked: false });
  apply({ type: "terminal_opened", channel: 1 });
  expect(useHive.getState().terminals[1]).toEqual({
    id: 1,
    exited: false,
    code: null,
    unhooked: false,
    badge: "",
  });
});

test("ui actions set ui state", () => {
  openModal("new-worktree");
  setRightPanel("files");
  select("wt-1");
  const s = useHive.getState();
  expect([s.modal, s.rightPanel, s.selection]).toEqual(["new-worktree", "files", "wt-1"]);
});

test("tabs open shown, switch with the selection and close to their neighbour", () => {
  const tabs = () => useHive.getState().tabs.map((t) => t.id);
  const active = () => useHive.getState().activeTab;
  addTab(1, "/a");
  addTab(2, "/b");
  addTab(3, "/c");
  expect([tabs(), active(), useHive.getState().selection]).toEqual([[1, 2, 3], 3, "/c"]);
  activateTab({ id: 1, cwd: "/a" });
  expect([active(), useHive.getState().selection]).toEqual([1, "/a"]);
  removeTab(2); // not shown: the shown tab stays
  expect([tabs(), active()]).toEqual([[1, 3], 1]);
  removeTab(1); // shown and alone in its place: none is shown
  expect([tabs(), active()]).toEqual([[3], null]);
  select(null); // every tab: the last one is shown
  expect(active()).toBe(3);
  removeTab(3);
  expect([tabs(), active()]).toEqual([[], null]);
});

test("tabs belong to the worktree they opened in; a project shows its main worktree's", () => {
  const [shop] = MOCK_REPOS;
  const [main, login, checkout] = shop.worktrees;
  apply({ type: "projects", projects: [shop] });
  const shown = () => visibleTabs(useHive.getState()).map((t) => t.id);
  const active = () => useHive.getState().activeTab;
  addTab(1, main.path);
  addTab(2, main.path);
  addTab(3, login.path);
  addTab(4, `${shop.path}/.claude/worktrees/gone`); // a removed worktree's: its project's
  addTab(5, "/outside");
  select(shop.id);
  expect([shown(), active()]).toEqual([[1, 2, 4], 4]);
  select(login.id);
  expect([shown(), active()]).toEqual([[3], 3]);
  activateTab({ id: 4, cwd: `${shop.path}/.claude/worktrees/gone` });
  expect([useHive.getState().selection, active()]).toEqual([shop.id, 4]);
  removeTab(4); // its right neighbour among the shown ones
  expect(active()).toBe(2);
  removeTab(2); // the last shown: the new last one
  expect(active()).toBe(1);
  select(checkout.id);
  expect([shown(), active()]).toEqual([[], null]);
  select("/outside");
  expect(shown()).toEqual([5]);

  // A selected agent shows its terminal's worktree, else the one it was placed in.
  apply({
    type: "agent_detected",
    channel: 3,
    id: "s",
    project: shop.id,
    worktree: main.id,
    cwd: null,
  });
  select("s");
  expect(tabsPlace(useHive.getState())).toBe(login.path);
  apply({
    type: "agent_detected",
    channel: 9,
    id: "t",
    project: shop.id,
    worktree: checkout.id,
    cwd: null,
  });
  select("t");
  expect(tabsPlace(useHive.getState())).toBe(checkout.id);

  // The open file's tab goes with its worktree.
  setOpenFile({ worktree: login.path, path: "a.ts" });
  expect(fileVisible(useHive.getState())).toBe(false);
  select(login.id);
  expect(fileVisible(useHive.getState())).toBe(true);
  select(null);
  expect(fileVisible(useHive.getState())).toBe(true);
  setOpenFile(null);
  expect(fileVisible(useHive.getState())).toBe(false);
});

const agent = (id: string, terminal = 1) => ({
  id,
  terminal,
  project: "/r",
  worktree: "/r/.claude/worktrees/x",
  cwd: "/r/.claude/worktrees/x/src",
});

test("agents are stored as the service places them and removed by id", () => {
  const { terminal: _, ...a } = agent("a", 2);
  apply({ type: "agent_detected", channel: 2, ...a });
  apply({ type: "agent_detected", channel: 3, ...agent("b"), project: null, worktree: null });
  expect(useHive.getState().agents).toEqual({
    a: agent("a", 2),
    b: { ...agent("b", 3), project: null, worktree: null },
  });
  apply({ type: "agent_removed", channel: 2, id: "a" });
  apply({ type: "agent_removed", channel: 2, id: "unknown" });
  expect(Object.keys(useHive.getState().agents)).toEqual(["b"]);
  // A lost service takes every agent with it.
  apply({ type: "disconnected", reason: "gone" });
  expect(useHive.getState().agents).toEqual({});
});

test("agent states are stored as sent, before or after the agent, and go with it", () => {
  const sub = {
    id: "s1",
    agent_type: "Explore",
    state: "waiting_permission",
    worktree: null,
    activity: null,
    since_ms: 0,
  } as const;
  const doing = { activity: null, since_ms: 0 } as const;
  const permission = {
    state: "waiting_permission",
    urgency: 6,
    pending: true,
    interrupted: false,
    ...doing,
  } as const;
  const idle = { state: "idle", urgency: 1, pending: false, interrupted: false, ...doing } as const;
  apply({
    type: "agent_state",
    id: "a",
    state: "working",
    urgency: 2,
    pending: false,
    interrupted: false,
    subagents: [],
    activity: null,
    since_ms: 0,
  });
  apply({ type: "agent_state", id: "b", ...idle, subagents: [] });
  apply({ type: "agent_state", id: "a", ...permission, subagents: [sub] });
  expect(useHive.getState().agentStates).toEqual({
    a: { ...permission, subagents: [sub] },
    b: { ...idle, subagents: [] },
  });
  apply({ type: "agent_removed", channel: 1, id: "a" });
  expect(Object.keys(useHive.getState().agentStates)).toEqual(["b"]);
  apply({ type: "disconnected", reason: "gone" });
  expect(useHive.getState().agentStates).toEqual({});
});

test("agent usage is stored as sent and goes with the agent", () => {
  const usage = { context_tokens: 84_000, context_limit: 200_000, output_tokens: 12 };
  apply({ type: "agent_usage", id: "a", ...usage });
  apply({ type: "agent_usage", id: "b", ...usage, output_tokens: 1 });
  expect(useHive.getState().agentUsage.a).toEqual(usage);
  apply({ type: "agent_removed", channel: 1, id: "a" });
  expect(Object.keys(useHive.getState().agentUsage)).toEqual(["b"]);
});

test("useTerminal reads one terminal", () => {
  apply({ type: "terminal_opened", channel: 4 });
  const { result } = renderHook(() => useTerminal(4));
  expect(result.current?.id).toBe(4);
});

test("projects replace the list; an added project joins it and closes its dialog", () => {
  const [shop, api] = MOCK_REPOS;
  expect(useHive.getState().projects).toBeNull();
  apply({ type: "projects", projects: [shop] });
  expect(useHive.getState().projects).toEqual({ [shop.id]: shop });
  openModal("add-project");
  apply({ type: "add_project_failed", path: "x", error: "not_absolute", message: "m" });
  expect(useHive.getState().addProjectError).toBe("m");
  apply({ type: "project_added", project: api });
  const s = useHive.getState();
  expect(Object.keys(s.projects ?? {})).toEqual([shop.id, api.id]);
  expect([s.modal, s.addProjectError]).toEqual([null, null]);
  // Another dialog stays open.
  openModal("new-worktree");
  apply({ type: "project_added", project: shop });
  expect(useHive.getState().modal).toBe("new-worktree");
  apply({ type: "projects", projects: [] });
  expect(useHive.getState().projects).toEqual({});
});

test("a removed worktree leaves its project selected and its collapsed key goes", () => {
  const [shop, api] = MOCK_REPOS;
  const [main, fixLogin] = shop.worktrees;
  const without = { ...shop, worktrees: shop.worktrees.filter((w) => w !== fixLogin) };
  apply({ type: "projects", projects: [shop, api] });
  select(fixLogin.id);
  toggleCollapsed(`worktree:${fixLogin.id}`);
  toggleCollapsed(`worktree:${main.id}`);
  toggleCollapsed(shop.id);
  apply({ type: "projects", projects: [without, api] });
  let s = useHive.getState();
  expect(s.selection).toBe(shop.id);
  expect(s.collapsed).toEqual({ [`worktree:${main.id}`]: true, [shop.id]: true });
  // Anything else still listed, an agent, or nothing stays selected.
  for (const kept of [api.worktrees[1].id, "session-1", null]) {
    select(kept);
    apply({ type: "projects", projects: [without, api] });
    expect(useHive.getState().selection).toBe(kept);
  }
  // With its project gone too, nothing is selected.
  select(main.id);
  apply({ type: "projects", projects: [api] });
  s = useHive.getState();
  expect(s.selection).toBeNull();
});

test("tree nodes toggle between collapsed and expanded", () => {
  toggleCollapsed("p");
  expect(useHive.getState().collapsed).toEqual({ p: true });
  toggleCollapsed("p");
  expect(useHive.getState().collapsed).toEqual({ p: false });
});

test("new-worktree answers are kept for the dialog until it opens again", () => {
  const [shop] = MOCK_REPOS;
  apply({ type: "projects", projects: [shop] });
  openModal("new-worktree", shop.id);
  expect(useHive.getState().modalProject).toBe(shop.id);
  const branches = { project: shop.id, local: ["main"], remote: [], current: "main", error: null };
  apply({ type: "branches", ...branches });
  const check = { project: shop.id, name: "x", folder: "f", branch: "b", error: null };
  apply({ type: "worktree_name_validated", ...check });
  const failure = { project: shop.id, name: "x", message: "m" };
  apply({ type: "create_worktree_failed", ...failure });
  expect(useHive.getState().worktreeDialog).toEqual({
    branches,
    nameChecks: { x: check },
    created: null,
    createFailure: failure,
    failure: null,
    removeFailures: {},
  });
  const path = `${shop.path}/.claude/worktrees/x`;
  const updated = { ...shop, worktrees: [...shop.worktrees, { ...shop.worktrees[1], id: path }] };
  apply({ type: "worktree_created", project: updated, path, notes: ["n"] });
  const s = useHive.getState();
  expect(s.projects?.[shop.id]).toBe(updated);
  expect(s.worktreeDialog.created).toEqual({ project: shop.id, path, notes: ["n"] });
  openModal("new-worktree");
  expect(useHive.getState().modalProject).toBeNull();
  expect(useHive.getState().worktreeDialog).toEqual(initialState.worktreeDialog);
});

test("files replace the watched worktree's list; a disconnect drops it", () => {
  apply({ type: "files", path: "/a", files: ["x"], truncated: false });
  apply({ type: "files", path: "/b", files: ["y", "z"], truncated: true });
  const files = { path: "/b", files: ["y", "z"], truncated: true };
  expect(useHive.getState().worktreeFiles).toEqual(files);
  apply({ type: "disconnected", reason: "gone" });
  expect(useHive.getState().worktreeFiles).toBeNull();
});

test("the files panel shows the selected worktree or the selected agent's", () => {
  const shop = MOCK_REPOS[0] as (typeof MOCK_REPOS)[number];
  const worktree = (shop.worktrees[1] as { id: string }).id;
  apply({ type: "projects", projects: MOCK_REPOS });
  const agent = { project: shop.id, worktree, cwd: `${worktree}/src` };
  apply({ type: "agent_detected", channel: 1, id: "s1", ...agent });
  const shown = () => panelWorktree(useHive.getState())?.worktree.id ?? null;
  select(worktree);
  expect(shown()).toBe(worktree);
  // A project is its main worktree.
  select(shop.id);
  expect(shown()).toBe(shop.id);
  select("s1");
  expect(shown()).toBe(worktree);
  select("unknown");
  expect(shown()).toBeNull();
  select(null);
  expect(shown()).toBeNull();
});

const fileAnswer = (content: string | null, path = "a.ts"): ServiceMessage => ({
  type: "file",
  worktree: "/w",
  path,
  content,
  base: null,
  version: content && `v:${content}`,
  binary: false,
  too_large: false,
  error: null,
});

test("editing keeps a buffer for the open file, fed by its answers", () => {
  const edit = () => useHive.getState().edit;
  setOpenFile({ worktree: "/w", path: "a.ts" }, true);
  expect(useHive.getState().editing).toBe(true);
  apply(fileAnswer("one\n", "b.ts")); // Another file's answer starts nothing.
  expect(edit()).toBeNull();
  apply(fileAnswer("one\n"));
  expect(edit()?.doc.toString()).toBe("one\n");
  apply(fileAnswer("two\n"));
  expect([edit()?.doc.toString(), edit()?.version]).toEqual(["two\n", "v:two\n"]);

  // Saves: answers for another file change nothing.
  const sending = { ...(edit() as EditBuffer), saving: toText("three\n") };
  setEdit(sending);
  const at = { worktree: "/w", path: "b.ts" };
  apply({ type: "file_saved", ...at, version: "v" });
  apply({ type: "save_failed", ...at, error: "io", message: "m" });
  expect(edit()).toBe(sending);
  apply({ type: "file_saved", worktree: "/w", path: "a.ts", version: "v:three\n" });
  expect(edit()?.version).toBe("v:three\n");
  apply({ type: "save_failed", worktree: "/w", path: "a.ts", error: "conflict", message: "m" });
  expect([edit()?.error, edit()?.recheck]).toEqual(["m", 1]);

  // The same file again keeps the buffer; another file or closing drops it.
  setOpenFile({ worktree: "/w", path: "a.ts" }, false);
  expect(useHive.getState().editing).toBe(true);
  useHive.setState({ editorNotice: "n" });
  setOpenFile({ worktree: "/w", path: "b.ts" });
  expect(useHive.getState()).toMatchObject({ editing: false, edit: null, editorNotice: null });
  setOpenFile(null);
  expect(useHive.getState().openFile).toBeNull();
  // Not editing: answers keep no buffer, and nothing is stored for these.
  apply(fileAnswer("x\n", "b.ts"));
  apply({ type: "file_saved", worktree: "/w", path: "b.ts", version: "v" });
  apply({ type: "save_failed", worktree: "/w", path: "b.ts", error: "io", message: "m" });
  expect(edit()).toBeNull();
});

test("a created file opens as editable text unless unsaved edits would be lost", () => {
  const s = () => useHive.getState();
  openFileDialog({ worktree: "/w", folder: "", path: null });
  apply({ type: "file_created", worktree: "/w", path: "new.ts" });
  expect(s()).toMatchObject({ modal: null, fileDialog: null, editing: true, fileShown: true });
  expect(s().openFile).toEqual({ worktree: "/w", path: "new.ts" });
  // Unsaved edits of the open file stay; the dialog of another worktree stays open.
  apply(fileAnswer("one\n", "new.ts"));
  setEdit({ ...(s().edit as EditBuffer), doc: toText("mine\n") });
  openFileDialog({ worktree: "/x", folder: "", path: null });
  apply({ type: "file_created", worktree: "/w", path: "other.ts" });
  expect(s().openFile?.path).toBe("new.ts");
  expect(s()).toMatchObject({ modal: "file-name", fileDialog: { worktree: "/x" } });
});

test("a rename carries the open file, its text and its edits to the new path", () => {
  const s = () => useHive.getState();
  setOpenFile({ worktree: "/w", path: "a.ts" }, true);
  apply(fileAnswer("one\n"));
  openFileDialog({ worktree: "/w", folder: "", path: "a.ts" }, true);
  expect(s().fileDialog).toMatchObject({ renaming: true, error: null });
  apply({ type: "file_renamed", worktree: "/w", path: "b.ts", to: "c.ts" });
  expect([s().openFile?.path, s().modal]).toEqual(["a.ts", null]);
  apply({ type: "file_renamed", worktree: "/w", path: "a.ts", to: "d.ts" });
  expect([s().openFile?.path, s().file?.path, s().edit?.path]).toEqual(["d.ts", "d.ts", "d.ts"]);
  expect(s().edit?.doc.toString()).toBe("one\n");
  // Nothing open: nothing moves.
  setOpenFile(null);
  apply({ type: "file_renamed", worktree: "/w", path: "d.ts", to: "e.ts" });
  expect(s().openFile).toBeNull();
  // A refusal without a dialog for its worktree changes nothing.
  apply({ type: "file_op_failed", worktree: "/w", message: "no" });
  expect(s().fileDialog).toBeNull();
  openFileMenu({ worktree: "/w", folder: "", path: null, x: 1, y: 2 });
  expect(s().fileMenu).toMatchObject({ x: 1 });
});

test("Edit starts the buffer from the last answer; Diff drops it", () => {
  setOpenFile({ worktree: "/w", path: "a.ts" });
  setEditing(true); // No answer yet: the buffer starts with the first one.
  expect(useHive.getState().edit).toBeNull();
  apply(fileAnswer("one\n"));
  setEditing(false);
  expect(useHive.getState().edit).toBeNull();
  setEditing(true);
  expect(useHive.getState().edit?.doc.toString()).toBe("one\n");
  setEditorNotice("why");
  expect(useHive.getState().editorNotice).toBe("why");
});

test("an agent is working in a worktree while it (or its subagent there) may write", () => {
  const worktree = "/w";
  const place = (id: string, at: string | null) =>
    apply({ type: "agent_detected", channel: 1, id, project: "/p", worktree: at, cwd: at });
  const none = { activity: null, since_ms: 0 };
  const state = (id: string, state: AgentState, subagents: Subagent[] = []) =>
    apply({
      type: "agent_state",
      id,
      state,
      urgency: 0,
      pending: false,
      interrupted: false,
      subagents,
      ...none,
    });
  const working = () => agentWorkingIn(useHive.getState(), worktree);
  place("a", worktree);
  expect(working()).toBe(false); // No state yet.
  for (const [s, expected] of [
    ["idle", false],
    ["waiting_you", false],
    ["error", false],
    ["ended", false],
    ["working", true],
    ["with_subagents", true],
    ["waiting_permission", true],
  ] as const) {
    state("a", s);
    expect(working()).toBe(expected);
  }
  state("a", "idle");
  place("b", "/elsewhere");
  state("b", "working", [
    { id: "s", agent_type: null, state: "working", worktree: "/other", ...none },
  ]);
  expect(working()).toBe(false);
  state("b", "with_subagents", [{ id: "s", agent_type: null, state: "idle", worktree, ...none }]);
  expect(working()).toBe(false);
  state("b", "with_subagents", [
    { id: "s", agent_type: null, state: "working", worktree, ...none },
  ]);
  expect(working()).toBe(true);
});

test("sessions are stored as listed; a deleted one leaves, a refused delete says why", () => {
  const [a, b] = MOCK_SESSIONS;
  apply({ type: "session_deleted", id: a.id });
  expect(useHive.getState().sessions).toBeNull();
  apply({ type: "sessions", sessions: [a, b], error: null });
  apply({ type: "session_deleted", id: a.id });
  expect(useHive.getState().sessions).toEqual([b]);
  apply({ type: "delete_session_failed", id: b.id, message: "busy" });
  expect(useHive.getState().notice).toBe("Cannot delete the session: busy");
});

test("a tab belongs to the deepest worktree holding its folder", () => {
  const [shop] = MOCK_REPOS;
  const [main, login] = shop.worktrees;
  apply({ type: "projects", projects: [shop] });
  const s = useHive.getState();
  expect(tabPlace(s, `${login.path}/src/auth`)).toBe(login.id);
  expect(tabPlace(s, `${shop.path}/src`)).toBe(main.id);
  expect(tabPlace(s, login.path)).toBe(login.id);
  // A sibling folder whose name starts the same is not inside.
  expect(tabPlace(s, `${login.path}-copy`)).toBe(main.id);
  expect(tabPlace(s, "/elsewhere")).toBe("/elsewhere");
});

test("a worktree status replaces only that worktree's; before any projects it is dropped", () => {
  const [shop, api] = MOCK_REPOS;
  const [, login] = shop.worktrees;
  const status = { changes: 0, ahead: 0, behind: 4, merged: true, last_commit_ms: 1 };
  apply({ type: "worktree_status", path: login.path, status });
  expect(useHive.getState().projects).toBeNull();
  apply({ type: "projects", projects: [shop, api] });
  apply({ type: "worktree_status", path: login.path, status });
  const { projects } = useHive.getState();
  expect(projects?.[shop.id].worktrees[1]).toEqual({ ...login, status });
  expect(projects?.[shop.id].worktrees[2]).toBe(shop.worktrees[2]);
  expect(projects?.[api.id]).toEqual(api);
  expect(Object.keys(projects ?? {})).toEqual([shop.id, api.id]);
});

test("a subagent's conversation is kept as sent, grown by what is appended, and capped", () => {
  const entry = (text: string): TranscriptEntry => ({ role: "user", text, tool: null });
  const at = { agent: "s", subagent: "a" };
  apply({ type: "transcript", ...at, entries: [entry("1")], truncated: false });
  // Another subagent's entries are not this conversation's.
  apply({ type: "transcript_appended", agent: "s", subagent: "b", entries: [entry("x")] });
  apply({ type: "transcript_appended", agent: "t", subagent: "a", entries: [entry("x")] });
  apply({ type: "transcript_appended", ...at, entries: [entry("2")] });
  expect(useHive.getState().transcript).toEqual({
    ...at,
    entries: [entry("1"), entry("2")],
    truncated: false,
  });
  // Past the cap the oldest go, and the conversation says so.
  const many = Array.from({ length: TRANSCRIPT_LIMIT - 2 }, (_, i) => entry(`n${i}`));
  apply({ type: "transcript_appended", ...at, entries: many });
  expect(useHive.getState().transcript?.truncated).toBe(false);
  apply({ type: "transcript_appended", ...at, entries: [entry("last")] });
  const t = useHive.getState().transcript;
  expect([t?.entries.length, t?.entries[0]?.text, t?.entries.at(-1)?.text, t?.truncated]).toEqual([
    TRANSCRIPT_LIMIT,
    "2",
    "last",
    true,
  ]);
  apply({ type: "disconnected", reason: "gone" });
  expect(useHive.getState().transcript).toBeNull();
});

test("a shown conversation selects its agent and gives way to any terminal or file shown", () => {
  addTab(1, "/w");
  addTab(2, "/w");
  apply({ type: "agent_detected", channel: 1, id: "s", project: null, worktree: "/w", cwd: "/w" });
  showTranscript("s", "a");
  const s = useHive.getState();
  expect([s.transcriptShown, s.selection, s.activeTab]).toEqual([
    { agent: "s", subagent: "a" },
    "s",
    1,
  ]);
  hideTranscript();
  expect(useHive.getState().transcriptShown).toBeNull();
  // An agent without a tab keeps the shown one.
  activateTab({ id: 2, cwd: "/w" });
  apply({ type: "agent_detected", channel: 5, id: "u", project: null, worktree: "/w", cwd: "/w" });
  showTranscript("u", "a");
  expect(useHive.getState().activeTab).toBe(2);
  const shown = { agent: "s", subagent: "a" };
  const hides = [
    () => select("/w"),
    () => activateTab({ id: 1, cwd: "/w" }),
    () => addTab(3, "/w"),
    showFile,
    () => setOpenFile({ worktree: "/w", path: "a" }),
    () => setOpenFile({ worktree: "/w", path: "a" }),
  ];
  for (const hide of hides) {
    useHive.setState({ transcriptShown: shown });
    hide();
    expect(useHive.getState().transcriptShown).toBeNull();
  }
  // Closing the file leaves a shown conversation.
  useHive.setState({ transcriptShown: shown });
  setOpenFile(null);
  expect(useHive.getState().transcriptShown).toEqual(shown);
  // Its agent leaving takes it away; another agent leaving does not.
  apply({ type: "agent_removed", channel: 9, id: "other" });
  expect(useHive.getState().transcriptShown).toEqual(shown);
  apply({ type: "agent_removed", channel: 1, id: "s" });
  expect(useHive.getState().transcriptShown).toBeNull();
  useHive.setState({ transcriptShown: shown });
  apply({ type: "disconnected", reason: "gone" });
  expect(useHive.getState().transcriptShown).toBeNull();
});

const chatEntry = (id: number, text: string, more: Partial<ChatEntry> = {}): ChatEntry => ({
  id,
  kind: "assistant",
  text,
  tool: null,
  parent: null,
  status: null,
  output: null,
  images: [],
  ...more,
});

test("chat entries are added, replace theirs by id, or the last one for live text; capped", () => {
  const [a, b] = [chatEntry(1, "a"), chatEntry(2, "b", { status: "running" })];
  expect(mergeEntries([], [a, b], false)).toEqual([a, b]);
  // A tool's result re-sends its entry.
  const done = { ...b, status: "ok" as const, output: "out" };
  expect(mergeEntries([a, b], [done], false)).toEqual([a, done]);
  // Live text: the first entry replaces the last, whatever its id; the rest follow.
  const grown = chatEntry(3, "b grown");
  const c = chatEntry(4, "c");
  const live = mergeEntries([a, b], [grown, c], true);
  expect(live).toEqual([a, grown, c]);
  // The other entries stay the same objects, so their rows do not render again.
  expect(live[0]).toBe(a);
  expect(mergeEntries([], [a], true)).toEqual([a]);
  const many = Array.from({ length: CHAT_LIMIT }, (_, i) => chatEntry(i + 10, "n"));
  const merged = mergeEntries([a], many, false);
  expect([merged.length, merged[0]?.id, merged.at(-1)?.id]).toEqual([
    CHAT_LIMIT,
    10,
    CHAT_LIMIT + 9,
  ]);
});

test("a chat's messages fill its data; a chat tab is a tab of kind chat", () => {
  const chat = (id = 7) => useHive.getState().chats[id];
  // The service may speak before the tab exists.
  apply({ type: "confirm_chat_folder", channel: 7, chat: 7, cwd: "/w" });
  expect(chat()).toEqual({
    cwd: "/w",
    opened: null,
    status: null,
    entries: [],
    requests: [],
    confirm: true,
    closed: null,
  });
  setChat(7, "/w");
  addTab(7, "/w", "chat");
  expect(useHive.getState().tabs).toEqual([{ id: 7, cwd: "/w", kind: "chat" }]);
  expect(chat()?.confirm).toBe(true);
  const opened = {
    chat: 7,
    cwd: "/w",
    session: "s",
    model: "m",
    mode: "default" as const,
    commands: ["compact"],
    api_key_source: null,
  };
  apply({ type: "chat_opened", channel: 7, ...opened });
  expect([chat()?.opened, chat()?.confirm]).toEqual([opened, false]);
  const status = {
    chat: 7,
    busy: true,
    mode: "plan" as const,
    model: "m",
    retry: null,
    compacting: false,
    api_key_source: null,
    session: "s",
  };
  apply({ type: "chat_status", channel: 7, ...status });
  expect(chat()?.status).toEqual(status);
  const entries = (list: ChatEntry[], replace_last: boolean): ServiceMessage => ({
    type: "chat_entries",
    channel: 7,
    chat: 7,
    entries: list,
    replace_last,
  });
  apply(entries([chatEntry(1, "hi")], false));
  apply(entries([chatEntry(2, "hi!")], true));
  expect(chat()?.entries).toEqual([chatEntry(2, "hi!")]);
  const request = { id: "r1", ...MOCK_CHAT_REQUESTS.permission };
  apply({ type: "chat_request", channel: 7, chat: 7, request });
  apply({ type: "chat_request", channel: 7, chat: 7, request: { ...request, id: "r2" } });
  apply({ type: "chat_request_gone", channel: 7, chat: 7, request: "r1" });
  expect(chat()?.requests.map((r) => r.id)).toEqual(["r2"]);
  apply({ type: "chat_closed", channel: 7, chat: 7, error: "boom" });
  expect([chat()?.closed, chat()?.status?.busy, chat()?.requests]).toEqual([
    { error: "boom" },
    false,
    [],
  ]);
  // Closed before any status, it has none.
  apply({ type: "chat_closed", channel: 8, chat: 8, error: null });
  expect([chat(8)?.closed, chat(8)?.status]).toEqual([{ error: null }, null]);
  setChat(7, null);
  expect(Object.keys(useHive.getState().chats)).toEqual(["8"]);
});
