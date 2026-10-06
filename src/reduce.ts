import type { ServiceMessage, WorktreeFiles } from "./protocol";
import {
  type FileTab,
  type HiveState,
  inCurrentSpace,
  leaveSpace,
  type OpenFile,
  owner,
  type Question,
  runsKey,
  selected,
  spacePlace,
  type Terminal,
  useHive,
  type WorktreeDialog,
  within,
  withNotice,
} from "./store";
import { dropFile, editFor, fileKey, fileTabState, opened } from "./tabs";
import { type EditBuffer, failed, isDirty, isFor, saved } from "./viewer/buffer";

// How service data enters the store: `apply` stores what the service sent without deriving
// anything (#37).

function patchTerminal(s: HiveState, id: number, patch: Partial<Terminal>): Partial<HiveState> {
  const current = s.terminals[id] ?? {
    id,
    exited: false,
    code: null,
    unhooked: false,
    worktree: null,
  };
  return { terminals: { ...s.terminals, [id]: { ...current, ...patch } } };
}

function patchDialog(s: HiveState, patch: Partial<WorktreeDialog>): Partial<HiveState> {
  return { worktreeDialog: { ...s.worktreeDialog, ...patch } };
}

/**
 * What goes with a project the service stopped following (9.28): its rows and sessions, its
 * file tabs (their unsaved edits too: the question said so), the panel state of its worktrees,
 * and the places its worktrees and sessions held in the agents' and the tab bar's orders.
 */
function removedProject(s: HiveState, id: string): Partial<HiveState> {
  const project = s.projects?.[id];
  if (!project) return {};
  const places = [id, ...project.worktrees.map((w) => w.id)];
  const inside = (path: string | null) => places.includes(path as string);
  const files: Partial<HiveState> = {};
  for (const f of s.openFiles.filter((f) => inside(f.worktree))) {
    Object.assign(files, dropFile({ ...s, ...files }, f));
  }
  const gone = (s.sessions ?? []).filter((x) => x.project === id).map((x) => x.id);
  gone.push(...Object.values(s.agents).flatMap((a) => (inside(a.worktree) ? [a.id] : [])));
  const own = (key: string) =>
    key === id ||
    gone.some((g) => key === `session:${g}`) ||
    places.some(
      (p) =>
        key === `worktree:${p}` ||
        [`files:${p}/`, `changes:${p}/`, `file:${p}\n`].some((start) => key.startsWith(start)),
    );
  const keep = <T>(record: Record<string, T>) =>
    Object.fromEntries(Object.entries(record).filter(([key]) => !inside(key)));
  const { [id]: _, ...projects } = s.projects ?? {};
  return {
    ...files,
    projects,
    selection: inside(s.selection) || gone.includes(s.selection ?? "") ? null : s.selection,
    collapsed: Object.fromEntries(Object.entries(s.collapsed).filter(([key]) => !own(key))),
    tabOrder: (files.tabOrder ?? s.tabOrder).filter((key) => !own(key)),
    agentOrder: s.agentOrder.filter((a) => !gone.includes(a)),
    sessions: s.sessions?.filter((x) => x.project !== id) ?? null,
    worktreeFiles: inside(s.worktreeFiles?.path ?? null) ? null : s.worktreeFiles,
    changes: keep(s.changes),
    comments: keep(s.comments),
    commenting: inside(s.commenting?.worktree ?? null) ? null : s.commenting,
    newFolders: keep(s.newFolders),
    searchResults: inside(s.searchResults?.worktree ?? null) ? null : s.searchResults,
  };
}

function reduce(s: HiveState, m: ServiceMessage): Partial<HiveState> {
  switch (m.type) {
    case "welcome":
      return { connection: { status: "connected", version: m.version, distro: m.distro } };
    case "app_mode":
      // First on every connect (12.5.4): a new connection is on its way, once there is a mode.
      return { appMode: { mode: m.mode, wsl: m.wsl }, connection: { status: "connecting" } };
    case "settings":
      return { settings: m.settings, settingsError: null, settingsPending: false };
    case "settings_failed":
      return {
        settingsError: m.message,
        ...withNotice(s, "error", m.message),
        settingsPending: false,
      };
    case "diagnostics": {
      const { type: _, ...diagnostics } = m;
      return { diagnostics };
    }
    case "update_ready":
      return { update: { version: m.version, installing: false } };
    case "update_failed":
      return {
        update: s.update && { ...s.update, installing: false },
        ...withNotice(s, "error", `Update failed: ${m.error}`),
      };
    case "version_mismatch": {
      const { type: _, ...versions } = m;
      return { connection: { status: "version_mismatch", ...versions } };
    }
    case "terminal_opened": {
      const opened = { exited: false, code: null, unhooked: false, worktree: m.worktree };
      // Its tab selected its cwd; the place is the worktree holding it (a subfolder, a link).
      const tab = s.tabs.find((t) => t.id === m.channel);
      const moved = tab && s.selection === tab.cwd ? { selection: m.worktree ?? tab.cwd } : {};
      return { ...patchTerminal(s, m.channel, opened), ...moved };
    }
    case "terminal_exited":
      return patchTerminal(s, m.channel, { exited: true, code: m.code, badge: "" });
    case "unhooked_agent":
      return patchTerminal(s, m.channel, { unhooked: true });
    case "badge":
      return patchTerminal(s, m.channel, { badge: m.text });
    case "agent_detected": {
      const { type: _, channel, ...agent } = m;
      return { agents: { ...s.agents, [m.id]: { ...agent, terminal: channel } } };
    }
    case "agent_removed": {
      const { [m.id]: _, ...agents } = s.agents;
      const { [m.id]: __, ...agentStates } = s.agentStates;
      const { [m.id]: ___, ...agentTitles } = s.agentTitles;
      const { [m.id]: ____, ...agentUsage } = s.agentUsage;
      const { [m.id]: _____, ...pendingSeen } = s.pendingSeen;
      return { agents, agentStates, agentTitles, agentUsage, pendingSeen };
    }
    case "agent_title":
      return { agentTitles: { ...s.agentTitles, [m.id]: m.title } };
    case "session_usage":
      return { sessionUsage: m.usage };
    case "agent_usage": {
      const { type: _, id, ...usage } = m;
      return { agentUsage: { ...s.agentUsage, [id]: usage } };
    }
    case "subagent_worktrees":
      return { subagentWorktrees: m.worktrees };
    case "agent_state": {
      const { type: _, id, ...status } = m;
      const agentStates = { ...s.agentStates, [id]: status };
      if (status.pending && s.pendingSeen[id] === status.state) return { agentStates };
      const { [id]: __, ...pendingSeen } = s.pendingSeen;
      return { agentStates, pendingSeen };
    }
    case "projects": {
      const projects = Object.fromEntries(m.projects.map((p) => [p.id, p]));
      const ids = new Set(m.projects.flatMap((p) => [p.id, ...p.worktrees.map((w) => w.id)]));
      // A selected worktree that went away (e.g. `WorktreeRemove`) leaves its project selected.
      const was = owner(s.projects, s.selection);
      const gone = was && !ids.has(s.selection as string);
      const selection = gone ? (projects[was.id] ? was.id : null) : s.selection;
      // So does a terminal's: its tab shows under the project (its main worktree's id).
      const terminals = Object.fromEntries(
        Object.values(s.terminals).map((t) => {
          const left = t.worktree !== null && !ids.has(t.worktree);
          const project = left ? owner(s.projects, t.worktree)?.id : undefined;
          return [t.id, project && projects[project] ? { ...t, worktree: project } : t];
        }),
      );
      const collapsed = Object.fromEntries(
        Object.entries(s.collapsed).filter(
          ([key]) => !key.startsWith("worktree:") || ids.has(key.slice("worktree:".length)),
        ),
      );
      return { projects, selection, terminals, collapsed };
    }
    case "project_added":
      // Only the add-project dialog asks for this, so it has done its job.
      return {
        projects: { ...s.projects, [m.project.id]: m.project },
        addProjectError: null,
        modal: s.modal === "add-project" ? null : s.modal,
      };
    case "add_project_failed":
      return { addProjectError: m.message };
    case "project_removed":
      return removedProject(s, m.id);
    case "remove_project_failed":
      return withNotice(s, "error", m.message);
    case "spaces": {
      // The answer to the space dialog's request: it has done its job.
      const spaces = {
        spaces: m.spaces,
        currentSpace: m.current,
        spaceError: null,
        modal: s.modal === "new-space" || s.modal === "edit-space" ? null : s.modal,
      };
      // Another space shows the place last selected there (11.5), unless the selection is
      // already in it: going to an agent there switched the space, and its agent stays.
      const next = { ...s, ...spaces };
      const switched = s.currentSpace !== null && m.current !== s.currentSpace;
      if (!switched || inCurrentSpace(next, s.selection)) return spaces;
      const left = { ...next, ...leaveSpace(s) };
      return { ...spaces, spacePlaces: left.spacePlaces, ...selected(left, spacePlace(left)) };
    }
    case "space_failed":
      return { spaceError: m.message };
    case "notice":
      // A warning (a terminal opened without its space's GitHub account): it stays.
      return withNotice(s, "error", m.message);
    case "pulls": {
      const { type: _, ...pulls } = m;
      return { pulls: { ...s.pulls, [m.project]: pulls } };
    }
    case "pull": {
      const shown = s.openPull;
      if (shown?.project !== m.project || shown.number !== m.number) return {};
      return { openPull: { ...shown, detail: m.pull, error: m.error } };
    }
    case "pull_done":
      return {
        pullBusy: null,
        pullError: null,
        ...withNotice(s, "info", m.message),
        modal: s.modal === "new-pull" ? null : s.modal,
      };
    case "pull_failed": {
      const { type: _, ...pullError } = m;
      return { pullBusy: null, pullError };
    }
    case "runs": {
      const { type: _, ...runs } = m;
      return { runs: { ...s.runs, [runsKey(m.project, m.branch)]: runs } };
    }
    case "run": {
      const shown = s.openRun;
      if (shown?.project !== m.project || shown.run !== m.run) return {};
      return { openRun: { ...shown, detail: m.detail, error: m.error } };
    }
    case "job_log": {
      const shown = s.jobLog;
      if (shown?.project !== m.project || shown.job !== m.job) return {};
      return { jobLog: { ...shown, log: m.log, error: m.error } };
    }
    case "run_done":
      return { runBusy: null, runError: null, ...withNotice(s, "info", m.message) };
    case "run_failed": {
      const { type: _, ...runError } = m;
      return { runBusy: null, runError };
    }
    case "gh_accounts": {
      const { type: _, ...accounts } = m;
      return { ghAccounts: accounts };
    }
    case "branches": {
      const { type: _, ...branches } = m;
      return patchDialog(s, { branches });
    }
    case "worktree_name_validated": {
      const { type: _, ...check } = m;
      return patchDialog(s, { nameChecks: { ...s.worktreeDialog.nameChecks, [m.name]: check } });
    }
    case "create_worktree_failed": {
      const { type: _, ...createFailure } = m;
      return patchDialog(s, { createFailure });
    }
    case "worktree_created": {
      const { project, path, notes } = m;
      return {
        projects: { ...s.projects, [project.id]: project },
        ...patchDialog(s, { created: { project: project.id, path, notes } }),
      };
    }
    case "worktree_removed":
    case "worktree_renamed": {
      // As a new list: a worktree that went away is dropped as `projects` drops it.
      const list = Object.values(s.projects ?? {}).map((p) =>
        p.id === m.project.id ? m.project : p,
      );
      const next = reduce(s, { type: "projects", projects: list });
      const renamed = m.type === "worktree_renamed" && s.selection === m.from;
      const dialog = m.type === "worktree_removed" ? "remove-worktree" : "rename-worktree";
      const gone = m.type === "worktree_removed" ? m.path : m.from;
      return {
        ...next,
        selection: renamed ? m.path : next.selection,
        // Only the dialog opened for that worktree has done its job.
        modal: s.modal === dialog && s.modalWorktree === gone ? null : s.modal,
      };
    }
    case "remove_worktree_failed":
      return patchDialog(s, {
        failure: { path: m.path, name: null, message: m.message },
        removeFailures: { ...s.worktreeDialog.removeFailures, [m.path]: m.message },
      });
    case "worktree_status": {
      // Only the project that owns the path changes: every other row keeps its objects (9.23).
      const p = Object.values(s.projects ?? {}).find((p) =>
        p.worktrees.some((w) => w.path === m.path),
      );
      if (!p) return {};
      const worktrees = p.worktrees.map((w) =>
        w.path === m.path ? { ...w, status: m.status } : w,
      );
      return { projects: { ...s.projects, [p.id]: { ...p, worktrees } } };
    }
    case "rename_worktree_failed": {
      const { type: _, ...failure } = m;
      return patchDialog(s, { failure });
    }
    case "files": {
      const { type: _, ...worktreeFiles } = m;
      return deletedFiles({ ...s, worktreeFiles }, s.worktreeFiles);
    }
    case "changes": {
      const { type: _, ...changes } = m;
      return { changes: { ...s.changes, [m.path]: changes } };
    }
    case "sessions":
      return { sessions: m.sessions, sessionsError: m.error, sessionsTruncated: m.truncated };
    case "session_deleted":
      return { sessions: s.sessions?.filter((x) => x.id !== m.id) ?? null };
    case "delete_session_failed":
      return withNotice(s, "error", `Cannot delete the session: ${m.message}`);
    case "search_results": {
      const { type: _, ...searchResults } = m;
      return { searchResults };
    }
    case "dirs": {
      const { type: _, ...dirs } = m;
      return { dirs };
    }
    case "file": {
      const { type: _, ...file } = m;
      return { file, edit: editFor(s, file) };
    }
    case "file_saved":
      return patchEdits(s, m, (b) => saved(b, m.version));
    case "save_failed":
      return patchEdits(s, m, (b) => failed(b, m.error, m.message));
    case "file_created":
      // The new file opens in its own tab as editable text.
      return {
        ...opened(s, { worktree: m.worktree, path: m.path }, true),
        ...fileDialogDone(s, m.worktree),
      };
    case "file_renamed": {
      // Its tab, its place in the bar, its text and its edits follow the rename (or move); a
      // folder's `to` moves every path under it, with its folders' open state.
      const to = (path: string) => (within(path, m.path) ? m.to + path.slice(m.path.length) : null);
      const moved = <T extends OpenFile>(f: T | null) => {
        const path = f?.worktree === m.worktree ? to(f.path) : null;
        return f && path !== null ? { ...f, path } : f;
      };
      // Keys that end in a path of the worktree: bar keys and the trees' folders.
      const prefixes = [fileKey({ worktree: m.worktree, path: "" })].concat(
        ["files", "changes"].map((tree) => `${tree}:${m.worktree}/`),
      );
      const rekey = (key: string) => {
        const prefix = prefixes.find((p) => key.startsWith(p));
        const path = prefix === undefined ? null : to(key.slice(prefix.length));
        return path === null ? key : `${prefix}${path}`;
      };
      const shown = s.newFolders[m.worktree];
      return {
        openFile: moved(s.openFile),
        openFiles: s.openFiles.map((f) => ({ ...(moved(f) as FileTab), edit: moved(f.edit) })),
        tabOrder: s.tabOrder.map(rekey),
        file: moved(s.file),
        edit: moved(s.edit),
        collapsed: Object.fromEntries(Object.entries(s.collapsed).map(([k, v]) => [rekey(k), v])),
        newFolders: shown
          ? { ...s.newFolders, [m.worktree]: shown.map((p) => to(p) ?? p) }
          : s.newFolders,
        movedRow: { worktree: m.worktree, path: m.to },
        ...fileDialogDone(s, m.worktree),
      };
    }
    case "folder_created": {
      // It shows at once, even empty, with the folders around it open.
      const open = m.path
        .split("/")
        .slice(0, -1)
        .map((_, i, parts) => [`files:${m.worktree}/${parts.slice(0, i + 1).join("/")}`, false]);
      const shown = s.newFolders[m.worktree] ?? [];
      return {
        newFolders: { ...s.newFolders, [m.worktree]: [...shown, m.path] },
        collapsed: { ...s.collapsed, ...Object.fromEntries(open) },
        ...fileDialogDone(s, m.worktree),
      };
    }
    case "file_deleted": {
      // The entry leaves the tree at once (the next listing confirms it), and the tabs of the
      // files it held close.
      const gone = (path: string) => within(path, m.path);
      const listing = s.worktreeFiles;
      const shown = s.newFolders[m.worktree];
      const tree = {
        ...s,
        worktreeFiles:
          listing?.path === m.worktree
            ? {
                ...listing,
                files: listing.files.filter((p) => !gone(p)),
                ignored: listing.ignored.filter((p) => !gone(p)),
              }
            : listing,
        newFolders: shown
          ? { ...s.newFolders, [m.worktree]: shown.filter((p) => !gone(p)) }
          : s.newFolders,
      };
      return closeDeleted(
        tree,
        s.openFiles.filter((f) => f.worktree === m.worktree && gone(f.path)),
      );
    }
    case "file_op_failed":
      // Under the dialog's field, else (a drag) as a toast.
      return s.fileDialog?.worktree === m.worktree
        ? { fileDialog: { ...s.fileDialog, error: m.message } }
        : withNotice(s, "error", m.message);
    case "disconnected":
      // The service is gone, and every agent and the watches with it.
      return {
        connection: { status: "disconnected", reason: m.reason, bundled: m.bundled },
        sessionUsage: null,
        agents: {},
        agentStates: {},
        subagentWorktrees: [],
        worktreeFiles: null,
      };
    case "error":
    case "session_located":
    case "restore_sessions":
    case "editor_target":
    case "notification_clicked":
      // Answered where they were asked (see `ServiceMessage`), not stored.
      return {};
    default:
      // Every known type has its case: a new one fails the typecheck here, and a type the app
      // does not know (a newer service) changes nothing.
      m satisfies never;
      return {};
  }
}

/** `change` applied to the edit buffers of file `f`: the open one's and its tab's. */
function patchEdits(
  s: HiveState,
  f: OpenFile,
  change: (b: EditBuffer) => EditBuffer,
): Partial<HiveState> {
  const mine = (b: EditBuffer | null): b is EditBuffer => !!b && isFor(b, f);
  return {
    edit: mine(s.edit) ? change(s.edit) : s.edit,
    openFiles: s.openFiles.map((t) => (mine(t.edit) ? { ...t, edit: change(t.edit) } : t)),
  };
}

/**
 * After a new listing of a worktree: the tabs of its files that the last listing held and this
 * one does not (deleted) close; one with unsaved edits asks first. A listing cut short proves
 * nothing, nor does one that lists an ignored folder holding the file: it may only be closed.
 */
function deletedFiles(s: HiveState, before: WorktreeFiles | null): HiveState {
  const now = s.worktreeFiles as WorktreeFiles;
  if (before?.path !== now.path || before.truncated || now.truncated) return s;
  const was = new Set([...before.files, ...before.ignored]);
  const is = new Set([...now.files, ...now.ignored]);
  const folders = now.ignored.filter((p) => p.endsWith("/"));
  const gone = s.openFiles.filter(
    (f) =>
      f.worktree === now.path &&
      was.has(f.path) &&
      !is.has(f.path) &&
      !folders.some((folder) => f.path.startsWith(folder)),
  );
  return closeDeleted(s, gone);
}

/**
 * The tabs of the deleted files `gone` close; those with unsaved edits once the user agrees,
 * in one question.
 */
function closeDeleted(s: HiveState, gone: OpenFile[]): HiveState {
  const dirty = gone.filter((f) => {
    const { edit } = fileTabState(s, f);
    return edit && isDirty(edit);
  });
  const next = dropAll(
    s,
    gone.filter((f) => !dirty.includes(f)),
  );
  if (dirty.length === 0) return next;
  // Files deleted earlier may still wait for their answer: one question names them all (9.24),
  // over the same dialog underneath.
  const open = s.modal === "confirm" ? s.question : null;
  const all = [...(open?.deleted ?? []), ...dirty];
  const [one] = all;
  const text =
    all.length === 1
      ? `${one?.path} was deleted. Your unsaved changes to it will be lost.`
      : `${all.map((f) => f.path).join(", ")} were deleted. Your unsaved changes to them will be lost.`;
  const question: Question = {
    title: "Discard changes?",
    text,
    action: "Discard",
    run: () => useHive.setState((s) => dropAll(s, all)),
    back: open ? open.back : s.modal,
    deleted: all,
  };
  return { ...next, modal: "confirm", question };
}

/** `s` with the tabs of `files` closed, one after the other. */
function dropAll(s: HiveState, files: OpenFile[]): HiveState {
  let next = s;
  for (const f of files) next = { ...next, ...dropFile(next, f) };
  return next;
}

/** Closes the file dialog when the answer is for its worktree. */
function fileDialogDone(s: HiveState, worktree: string): Partial<HiveState> {
  return s.fileDialog?.worktree === worktree ? { fileDialog: null, modal: null } : {};
}

/** The only way service data enters the store. */
export function apply(message: ServiceMessage): void {
  useHive.setState((s) => reduce(s, message));
}
