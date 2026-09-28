import { beforeEach, expect, test } from "bun:test";
import { renderHook } from "@testing-library/react";
import { notice, noticeKind } from "../test/notice";
import type { ServiceMessage, Subagent } from "./protocol";
import { apply } from "./reduce";
import {
  activateTab,
  addTab,
  agentWorkingIn,
  DEFAULT_SETTINGS,
  dismissNotice,
  initialState,
  NOTICE_LIMIT,
  openFileDialog,
  openFileMenu,
  openModal,
  panelWorktree,
  removeTab,
  select,
  setEdit,
  setEditing,
  setEditorNotice,
  setOpenFile,
  setRightPanel,
  showFailure,
  showNotice,
  toggleCollapsed,
  useHive,
  useTerminal,
} from "./store";
import { fileVisible, tabsPlace, visibleTabs } from "./tabs";
import { MOCK_REPOS, MOCK_SESSIONS } from "./transport/mock";
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
  expect([s.settings, s.settingsError, s.notices.at(-1)]).toMatchObject([
    DEFAULT_SETTINGS,
    "Ignoring settings.json",
    { kind: "error", text: "Ignoring settings.json" },
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
  apply({ type: "terminal_opened", channel: 1, worktree: null });
  apply({ type: "terminal_opened", channel: 2, worktree: "/w" });
  apply({ type: "unhooked_agent", channel: 1 });
  apply({ type: "badge", channel: 1, text: "db" });
  expect(useHive.getState().terminals[1]?.badge).toBe("db");
  apply({ type: "terminal_exited", channel: 1, code: 3 });
  const { terminals } = useHive.getState();
  const closed = { id: 1, exited: true, code: 3, unhooked: true, badge: "", worktree: null };
  expect(terminals[1]).toEqual(closed);
  const open = { id: 2, exited: false, code: null, unhooked: false, worktree: "/w" };
  expect(terminals[2]).toEqual(open);
  apply({ type: "terminal_opened", channel: 1, worktree: null });
  expect(useHive.getState().terminals[1]).toEqual({
    id: 1,
    exited: false,
    code: null,
    unhooked: false,
    worktree: null,
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
  addTab(4, `${shop.path}/src`); // placed by the service in its main worktree
  apply({ type: "terminal_opened", channel: 4, worktree: main.id });
  addTab(5, "/outside");
  select(shop.id);
  expect([shown(), active()]).toEqual([[1, 2, 4], 4]);
  select(login.id);
  expect([shown(), active()]).toEqual([[3], 3]);
  activateTab({ id: 4, cwd: `${shop.path}/src` });
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

  // A worktree that goes away leaves its tabs under its project; other places stay.
  apply({ type: "terminal_opened", channel: 3, worktree: login.id });
  apply({ type: "terminal_opened", channel: 5, worktree: "/unknown" });
  apply({ type: "projects", projects: [{ ...shop, worktrees: [main, checkout] }] });
  select(shop.id);
  expect(shown()).toEqual([1, 3]);
  expect(useHive.getState().terminals[5]?.worktree).toBe("/unknown");
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
    writing: true,
  } as const;
  const doing = { activity: null, since_ms: 0 } as const;
  const permission = {
    state: "waiting_permission",
    urgency: 6,
    pending: true,
    interrupted: false,
    alert: "waiting",
    writing: true,
    ...doing,
  } as const;
  const idle = {
    state: "idle",
    urgency: 1,
    pending: false,
    interrupted: false,
    alert: null,
    writing: false,
    ...doing,
  } as const;
  apply({
    type: "agent_state",
    id: "a",
    state: "working",
    urgency: 2,
    pending: false,
    interrupted: false,
    alert: null,
    writing: true,
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
  apply({ type: "terminal_opened", channel: 4, worktree: null });
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

  // The same file again keeps the buffer; another file shows in its own tab (8.21), and
  // closing it shows a.ts again with its buffer; closing that one drops it.
  setOpenFile({ worktree: "/w", path: "a.ts" }, false);
  expect(useHive.getState().editing).toBe(true);
  useHive.setState({ editorNotice: "n" });
  setOpenFile({ worktree: "/w", path: "b.ts" });
  expect(useHive.getState()).toMatchObject({ editing: false, edit: null, editorNotice: null });
  setOpenFile(null);
  expect(useHive.getState()).toMatchObject({ openFile: { path: "a.ts" }, editing: true });
  expect(edit()?.version).toBe("v:three\n");
  setOpenFile(null);
  expect(useHive.getState()).toMatchObject({ openFile: null, openFiles: [], edit: null });
  // Not editing: answers keep no buffer, and nothing is stored for these.
  apply(fileAnswer("x\n", "b.ts"));
  apply({ type: "file_saved", worktree: "/w", path: "b.ts", version: "v" });
  apply({ type: "save_failed", worktree: "/w", path: "b.ts", error: "io", message: "m" });
  expect(edit()).toBeNull();
});

test("a created file opens as editable text in its own tab", () => {
  const s = () => useHive.getState();
  openFileDialog({ worktree: "/w", folder: "", path: null });
  apply({ type: "file_created", worktree: "/w", path: "new.ts" });
  expect(s()).toMatchObject({ modal: null, fileDialog: null, editing: true, fileShown: true });
  expect(s().openFile).toEqual({ worktree: "/w", path: "new.ts" });
  // Unsaved edits of the open file stay in its tab; the dialog of another worktree stays open.
  apply(fileAnswer("one\n", "new.ts"));
  setEdit({ ...(s().edit as EditBuffer), doc: toText("mine\n") });
  openFileDialog({ worktree: "/x", folder: "", path: null });
  apply({ type: "file_created", worktree: "/w", path: "other.ts" });
  expect(s().openFile?.path).toBe("other.ts");
  expect(s().openFiles[0]?.edit?.doc.toString()).toBe("mine\n");
  expect(s()).toMatchObject({ modal: "file-name", fileDialog: { worktree: "/x" } });
});

test("a rename carries the open file, its text and its edits to the new path", () => {
  const s = () => useHive.getState();
  setOpenFile({ worktree: "/w", path: "a.ts" }, true);
  apply(fileAnswer("one\n"));
  openFileDialog({ worktree: "/w", folder: "", path: "a.ts" }, "rename");
  expect(s().fileDialog).toMatchObject({ kind: "rename", error: null });
  apply({ type: "file_renamed", worktree: "/w", path: "b.ts", to: "c.ts" });
  expect([s().openFile?.path, s().modal]).toEqual(["a.ts", null]);
  apply({ type: "file_renamed", worktree: "/w", path: "a.ts", to: "d.ts" });
  expect([s().openFile?.path, s().file?.path, s().edit?.path]).toEqual(["d.ts", "d.ts", "d.ts"]);
  expect(s().edit?.doc.toString()).toBe("one\n");
  // Nothing open: nothing moves.
  setOpenFile(null);
  apply({ type: "file_renamed", worktree: "/w", path: "d.ts", to: "e.ts" });
  expect(s().openFile).toBeNull();
  // A refusal without a dialog for its worktree (a drag) shows as an error toast.
  apply({ type: "file_op_failed", worktree: "/w", message: "no" });
  expect([s().fileDialog, notice(), noticeKind()]).toEqual([null, "no", "error"]);
  openFileMenu({ worktree: "/w", folder: "", path: null, x: 1, y: 2 });
  expect(s().fileMenu).toMatchObject({ x: 1 });
});

test("a folder's rename or move carries the open files under it and its folders' state", () => {
  const s = () => useHive.getState();
  setOpenFile({ worktree: "/w", path: "src/a.ts" }, true);
  apply(fileAnswer("one\n", "src/a.ts"));
  setEdit({ ...(s().edit as EditBuffer), doc: toText("mine\n") });
  setOpenFile({ worktree: "/w", path: "src/lib/b.ts" });
  setOpenFile({ worktree: "/w", path: "srcx/c.ts" });
  setOpenFile({ worktree: "/x", path: "src/a.ts" });
  useHive.setState({
    collapsed: {
      "files:/w/src": false,
      "files:/w/src/lib": false,
      "changes:/w/src/lib": false,
      "files:/w/srcx": false,
      "files:/x/src": false,
      "/w": true,
    },
    newFolders: { "/w": ["src/empty", "other"] },
  });
  apply({ type: "file_renamed", worktree: "/w", path: "src", to: "lib/core" });
  const paths = s().openFiles.map((f) => `${f.worktree}:${f.path}`);
  expect(paths).toEqual([
    "/w:lib/core/a.ts",
    "/w:lib/core/lib/b.ts",
    "/w:srcx/c.ts",
    "/x:src/a.ts",
  ]);
  // Unsaved edits are kept.
  expect(s().openFiles[0]?.edit).toMatchObject({ path: "lib/core/a.ts" });
  expect(s().openFiles[0]?.edit?.doc.toString()).toBe("mine\n");
  expect(s().tabOrder.filter((k) => k.startsWith("file:"))).toEqual([
    "file:/w\nlib/core/a.ts",
    "file:/w\nlib/core/lib/b.ts",
    "file:/w\nsrcx/c.ts",
    "file:/x\nsrc/a.ts",
  ]);
  expect(s().collapsed).toEqual({
    "files:/w/lib/core": false,
    "files:/w/lib/core/lib": false,
    "changes:/w/lib/core/lib": false,
    "files:/w/srcx": false,
    "files:/x/src": false,
    "/w": true,
  });
  expect(s().newFolders).toEqual({ "/w": ["lib/core/empty", "other"] });
  expect(s().movedRow).toEqual({ worktree: "/w", path: "lib/core" });
  // No folder was created in another worktree: none is kept for it.
  apply({ type: "file_renamed", worktree: "/y", path: "a", to: "b" });
  expect(s().newFolders).toEqual({ "/w": ["lib/core/empty", "other"] });
});

test("a created folder is kept for the tree, with the folders around it open", () => {
  const s = () => useHive.getState();
  openFileDialog({ worktree: "/w", folder: "src/lib", path: null }, "folder");
  apply({ type: "folder_created", worktree: "/w", path: "src/lib/new" });
  expect(s()).toMatchObject({ modal: null, fileDialog: null });
  expect(s().newFolders).toEqual({ "/w": ["src/lib/new"] });
  expect(s().collapsed).toEqual({ "files:/w/src": false, "files:/w/src/lib": false });
  apply({ type: "folder_created", worktree: "/w", path: "top" });
  apply({ type: "folder_created", worktree: "/x", path: "top" });
  expect(s().newFolders).toEqual({ "/w": ["src/lib/new", "top"], "/x": ["top"] });
  expect(Object.keys(s().collapsed)).toHaveLength(2);
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
  // The service says whether each may be writing; the state does not matter here.
  const state = (id: string, writing: boolean, subagents: Subagent[] = []) =>
    apply({
      type: "agent_state",
      id,
      state: "idle",
      urgency: 0,
      pending: false,
      interrupted: false,
      alert: null,
      writing,
      subagents,
      ...none,
    });
  const sub = (at: string, writing: boolean): Subagent => ({
    id: "s",
    agent_type: null,
    state: "idle",
    worktree: at,
    writing,
    ...none,
  });
  const working = () => agentWorkingIn(useHive.getState(), worktree);
  place("a", worktree);
  expect(working()).toBe(false); // No state yet.
  state("a", false);
  expect(working()).toBe(false);
  state("a", true);
  expect(working()).toBe(true);
  state("a", false);
  place("b", "/elsewhere");
  state("b", true, [sub("/other", true)]);
  expect(working()).toBe(false);
  state("b", true, [sub(worktree, false)]);
  expect(working()).toBe(false);
  state("b", false, [sub(worktree, true)]);
  expect(working()).toBe(true);
});

test("sessions are stored as listed; a deleted one leaves, a refused delete says why", () => {
  const [a, b] = MOCK_SESSIONS;
  apply({ type: "session_deleted", id: a.id });
  expect(useHive.getState().sessions).toBeNull();
  apply({ type: "sessions", sessions: [a, b], error: null, truncated: false });
  apply({ type: "session_deleted", id: a.id });
  expect(useHive.getState().sessions).toEqual([b]);
  apply({ type: "delete_session_failed", id: b.id, message: "busy" });
  expect([notice(), noticeKind()]).toEqual(["Cannot delete the session: busy", "error"]);
});

test("a notice from the service is an error toast: it stays until dismissed", () => {
  apply({ type: "notice", message: "No GitHub token for me" });
  expect([notice(), noticeKind()]).toEqual(["No GitHub token for me", "error"]);
});

test("toasts: each has a kind; at most 3 show, the newest last; one is dismissed by its id", () => {
  const shown = () => useHive.getState().notices.map((n) => `${n.kind}:${n.text}`);
  showNotice("error", "a");
  showNotice("info", "b");
  showNotice("error", "c");
  expect(shown()).toEqual(["error:a", "info:b", "error:c"]);
  // A fourth replaces the oldest.
  showNotice("info", "d");
  expect(shown()).toEqual(["info:b", "error:c", "info:d"]);
  const ids = useHive.getState().notices.map((n) => n.id);
  expect(new Set(ids).size).toBe(NOTICE_LIMIT);
  dismissNotice(ids[1] as number);
  expect(shown()).toEqual(["info:b", "info:d"]);
  // An unknown id changes nothing; errors never go by themselves (only `Toasts` fades infos).
  dismissNotice(-1);
  expect(shown()).toEqual(["info:b", "info:d"]);
});

test("a failed action shows why as an error toast, after what it was", async () => {
  await showFailure(Promise.reject("gone"), "Cannot open a terminal").catch(() => {});
  expect([notice(), noticeKind()]).toEqual(["Cannot open a terminal: gone", "error"]);
  await showFailure(Promise.reject("down")).catch(() => {});
  expect(notice()).toBe("down");
  await showFailure(Promise.resolve(1));
  expect(useHive.getState().notices).toHaveLength(2);
});

test("a tab opened in a subfolder or through a link shows under the worktree the service placed", () => {
  const [shop] = MOCK_REPOS;
  const [, login] = shop.worktrees;
  apply({ type: "projects", projects: [shop] });
  const at = () => {
    const s = useHive.getState();
    const panel = panelWorktree(s)?.worktree.id ?? null;
    return [s.selection, visibleTabs(s).map((t) => t.id), panel];
  };
  // Until the service answers, the tab stands at its own folder.
  addTab(1, `${login.path}/src`);
  expect(at()).toEqual([`${login.path}/src`, [1], null]);
  apply({ type: "terminal_opened", channel: 1, worktree: login.id });
  expect(at()).toEqual([login.id, [1], login.id]);
  // Answered before its tab is added (a link to the worktree).
  apply({ type: "terminal_opened", channel: 2, worktree: login.id });
  addTab(2, "/home/user/link-to-login");
  expect(at()).toEqual([login.id, [1, 2], login.id]);
  // A selection made meanwhile stays.
  addTab(3, `${login.path}/docs`);
  select(shop.id);
  apply({ type: "terminal_opened", channel: 3, worktree: login.id });
  expect(useHive.getState().selection).toBe(shop.id);
  // Outside every project, its place is its folder.
  addTab(4, "/outside");
  apply({ type: "terminal_opened", channel: 4, worktree: null });
  expect(at()).toEqual(["/outside", [4], null]);
  // With nothing selected, the files panel shows the active terminal's worktree.
  activateTab({ id: 3, cwd: `${login.path}/docs` });
  select(null);
  expect(at()).toEqual([null, [1, 2, 3, 4], login.id]);
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
  // Other projects keep their objects, so their rows do not render again (9.23).
  expect(projects?.[api.id]).toBe(api);
  expect(Object.keys(projects ?? {})).toEqual([shop.id, api.id]);
  apply({ type: "worktree_status", path: "/nowhere", status });
  expect(useHive.getState().projects).toBe(projects);
});
