import {
  ClockCounterClockwiseIcon,
  FilesIcon,
  GitDiffIcon,
  GitPullRequestIcon,
  MagnifyingGlassIcon,
  PlayCircleIcon,
} from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type DragEvent,
  type KeyboardEvent,
  lazy,
  type MouseEvent,
  memo,
  type ReactNode,
  Suspense,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useShallow } from "zustand/react/shallow";
import {
  agentWorkingIn,
  type ChangedFile,
  type Changes,
  type DiffBase,
  diffBase,
  dropFile,
  type FileStatus,
  type FileTarget,
  fileTabState,
  type OpenFile,
  openFileMenu,
  type PanelView,
  panelWorktree,
  type SearchMatch,
  setDiffBase,
  setEditing,
  setOpenFile,
  setPanelView,
  setRightPanel,
  useHive,
  type Worktree,
  within,
} from "../store";
import { transport } from "../transport";
import { isDirty } from "../viewer/buffer";
import { CodeView, notice } from "../viewer/CodeView";
import { EditView, saveOpenFile } from "../viewer/EditView";
import { openInEditor } from "../viewer/external";
import { referenceTarget, sendReference } from "../viewer/reference";
import { CommentButton, CommentInput, ReviewList } from "../viewer/review";
import { isMac, keyText } from "../window";
import { askDiscard } from "./ConfirmDialog";
import { askDelete } from "./FileMenu";
import { BranchIcon, ChevronIcon, CloseIcon, ExternalIcon, TerminalIcon } from "./icons";
import { PullsView } from "./PullsView";
import { RunsView } from "./RunsView";
import { ResizeHandle } from "./resize";
import { SessionsView } from "./SessionsView";

/**
 * How a git status shows (screen 1g): its letter (colored by CSS) and its weight when a
 * folder shows the strongest status inside. An untracked file shows as added.
 */
const STATUS: Record<FileStatus, { letter: string; rank: number }> = {
  modified: { letter: "M", rank: 1 },
  renamed: { letter: "R", rank: 2 },
  added: { letter: "A", rank: 3 },
  untracked: { letter: "A", rank: 3 },
  deleted: { letter: "D", rank: 4 },
};

/** A file of the tree: a changed one, or in "All" one git lists unchanged (`status` null). */
export type TreeFile = Omit<ChangedFile, "status"> & { status: FileStatus | null };

export type FileRow =
  | {
      kind: "folder";
      key: string;
      /** Relative to the worktree. */
      path: string;
      name: string;
      depth: number;
      open: boolean;
      status: FileStatus | null;
    }
  | { kind: "file"; key: string; name: string; depth: number; file: TreeFile };

/** A folder of the tree: its folders by name (sorted), its files, the strongest status inside. */
export type Folder = { folders: [string, Folder][]; files: TreeFile[]; status: FileStatus | null };

/**
 * "All": every file the service lists (`files`, sorted), each with its status from the
 * changes, plus the changed files it no longer lists (deleted ones), in path order.
 */
export function allFiles(listed: string[], changed: ChangedFile[]): TreeFile[] {
  const byPath = new Map(changed.map((f) => [f.path, f]));
  const listedPaths = new Set(listed);
  const unchanged = (path: string): TreeFile => ({
    path,
    status: null,
    old_path: null,
    added: null,
    removed: null,
  });
  return [
    ...listed.map((path) => byPath.get(path) ?? unchanged(path)),
    ...changed.filter((f) => !listedPaths.has(f.path)),
  ].sort((a, b) => (a.path < b.path ? -1 : 1));
}

/** The status of higher rank; null when neither changed. */
const stronger = (a: FileStatus | null, b: FileStatus | null) =>
  b && (!a || STATUS[b].rank > STATUS[a].rank) ? b : a;

/**
 * `files` (sorted by the service) grouped into folders, each folder's strongest status computed
 * while building. Built once per listing (9.23): moving in the tree or opening a folder only
 * walks it again (`fileRows`). Grouping paths is presentation; statuses and counts are the
 * service's. `folders` (created from the tree) show even when git lists nothing in them.
 */
export function fileTree(files: TreeFile[], folders: string[] = []): Folder {
  type Building = { folders: Map<string, Building>; files: TreeFile[] };
  const root: Building = { folders: new Map(), files: [] };
  const folderOf = (parts: string[]) => {
    let folder = root;
    for (const part of parts) {
      const next = folder.folders.get(part) ?? { folders: new Map(), files: [] };
      folder.folders.set(part, next);
      folder = next;
    }
    return folder;
  };
  for (const path of folders) folderOf(path.split("/"));
  for (const file of files) folderOf(file.path.split("/").slice(0, -1)).files.push(file);
  const finish = (folder: Building): Folder => {
    const inner = [...folder.folders]
      .sort(([a], [b]) => (a < b ? -1 : 1))
      .map(([name, f]): [string, Folder] => [name, finish(f)]);
    const status = inner.reduce(
      (a, [, f]) => stronger(a, f.status),
      folder.files.reduce<FileStatus | null>((a, f) => stronger(a, f.status), null),
    );
    return { folders: inner, files: folder.files, status };
  };
  return finish(root);
}

/**
 * The visible rows of `root` (from `fileTree`), folders before files. A folder starts collapsed
 * and is open only when `collapsed["<tree>:<worktree>/<path>"]` is false: each tree (Files,
 * Diff) opens its own folders.
 */
export function fileRows(
  worktree: string,
  root: Folder,
  collapsed: Record<string, boolean>,
  tree: "files" | "changes" = "files",
): FileRow[] {
  const rows: FileRow[] = [];
  const walk = (folder: Folder, prefix: string, depth: number) => {
    for (const [name, inner] of folder.folders) {
      const key = `${tree}:${worktree}/${prefix}${name}`;
      const open = collapsed[key] === false;
      const path = `${prefix}${name}`;
      rows.push({ kind: "folder", key, path, name, depth, open, status: inner.status });
      if (open) walk(inner, `${prefix}${name}/`, depth + 1);
    }
    for (const file of folder.files) {
      rows.push({
        kind: "file",
        key: file.path,
        name: file.path.slice(prefix.length),
        depth,
        file,
      });
    }
  };
  walk(root, "", 0);
  return rows;
}

/**
 * What the tree's menu acts on for `row`: new files go in the folder, or the file's folder, or
 * the root (no row: the tree's background); a folder or a file still on disk can be renamed
 * (a folder is its own `folder`).
 */
export function fileTarget(worktree: string, row: FileRow | undefined): FileTarget {
  if (!row) return { worktree, folder: "", path: null };
  if (row.kind === "folder") return { worktree, folder: row.path, path: row.path };
  const folder = row.key.slice(0, Math.max(row.key.lastIndexOf("/"), 0));
  return { worktree, folder, path: row.file.status === "deleted" ? null : row.key };
}

/** Opens the tree's menu for `row` at the pointer, or under `at` (the Menu key or Shift+F10). */
function openTreeMenu(worktree: string, row: FileRow | undefined, at?: Element | null) {
  return (event: MouseEvent<HTMLElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const box = (at ?? event.currentTarget).getBoundingClientRect();
    const pointer = event.clientX || event.clientY;
    const x = pointer ? event.clientX : box.left;
    const y = pointer ? event.clientY : box.bottom;
    openFileMenu({ ...fileTarget(worktree, row), x, y });
  };
}

const plus = (n: number | null) => (n ? `+${n}` : "");
const minus = (n: number | null) => (n ? `−${n}` : "");

/** The "+a −d" line counts; a binary file (null) shows none. */
function Counts({ added, removed }: { added: number | null; removed: number | null }) {
  return (
    <>
      <span className="count-added">{plus(added)}</span>
      <span className="count-removed">{minus(removed)}</span>
    </>
  );
}

/**
 * The shown worktree's name (not its project's: the panel always shows the worktree of the
 * project you are in), and under it `children` (e.g. its change totals).
 */
function WorktreeInfo(props: { worktree: Worktree; children?: ReactNode }) {
  return (
    <div className="files-info">
      <div className="files-worktree">
        <BranchIcon />
        <span className="name">{props.worktree.name}</span>
      </div>
      {props.children}
    </div>
  );
}

/**
 * The worktree the files views show (the selected worktree, or the selected agent's, or the
 * shown terminal's); its changes are asked for when it is shown and whenever it or its base
 * changes.
 */
function useShownWorktree() {
  const target = useHive(useShallow(panelWorktree));
  const path = target?.worktree.path;
  const base = useHive((s) => (path ? diffBase(s, path) : null));
  useEffect(() => {
    if (path && base) void transport.listChanges(path, base);
  }, [path, base]);
  return target;
}

const NOTHING_SHOWN = "Select a project or agent to see its files.";

const VIEWS: { view: PanelView; label: string; icon: ReactNode }[] = [
  { view: "files", label: "Files", icon: <FilesIcon size={14} aria-hidden="true" /> },
  { view: "changes", label: "Diff", icon: <GitDiffIcon size={14} aria-hidden="true" /> },
  {
    view: "sessions",
    label: "Sessions",
    icon: <ClockCounterClockwiseIcon size={14} aria-hidden="true" />,
  },
  // 9.31 and 9.32.
  { view: "pulls", label: "PRs", icon: <GitPullRequestIcon size={14} aria-hidden="true" /> },
  { view: "actions", label: "Actions", icon: <PlayCircleIcon size={14} aria-hidden="true" /> },
];

/**
 * The right panel (screen 1g, grown), toggled by Ctrl+Shift+B: for the shown worktree, every
 * file with a search ("Files"), the changed files with their totals ("Changes"), or its Claude
 * sessions ("Sessions").
 */
export function RightPanel() {
  const target = useShownWorktree();
  const view = useHive((s) => s.panelView);
  const width = useHive((s) => s.panelWidth);
  const shown = VIEWS.find((v) => v.view === view) as (typeof VIEWS)[number];
  return (
    <aside className="right-panel" aria-label="Side panel" style={{ width }}>
      <ResizeHandle side="panel" />
      <div className="bar">
        <div className="panel-views" role="tablist" aria-label="Panel">
          {VIEWS.map((v) => (
            <button
              key={v.view}
              type="button"
              role="tab"
              aria-selected={view === v.view}
              onClick={() => setPanelView(v.view)}
            >
              {v.icon}
              {v.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="ghost"
          title={keyText("Collapse (Ctrl+Shift+B)")}
          onClick={() => setRightPanel(null)}
        >
          <CloseIcon />
        </button>
      </div>
      {target ? (
        <section className="panel-view" aria-label={shown.label}>
          <WorktreeInfo worktree={target.worktree}>
            {view === "changes" && <Summary worktree={target.worktree.path} />}
          </WorktreeInfo>
          {view === "files" && <FilesView worktree={target.worktree.path} />}
          {view === "changes" && <FileTree worktree={target.worktree.path} changedOnly />}
          {view === "sessions" && <SessionsView worktree={target.worktree.id} />}
          {view === "pulls" && <PullsView project={target.project} worktree={target.worktree} />}
          {view === "actions" && (
            <RunsView
              key={target.worktree.id}
              project={target.project}
              worktree={target.worktree}
            />
          )}
        </section>
      ) : (
        <div className="right-panel-empty">{NOTHING_SHOWN}</div>
      )}
    </aside>
  );
}

/** How long typing must pause before the service is asked to search the contents. */
export const SEARCH_DELAY_MS = 250;
/** Most file names listed for a name search. */
export const NAME_LIMIT = 500;

type SearchMode = "names" | "contents";

/**
 * Files: every file of the shown worktree, with the changes' statuses. Typing in "Find files"
 * lists the files whose path holds the text ("Names"), or the lines holding it, searched by
 * the service ("Contents"); a line opens its file there.
 */
export function FilesView({ worktree }: { worktree: string }) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("names");
  const q = query.trim();
  const tab = (value: SearchMode, label: string) => (
    <button type="button" aria-pressed={mode === value} onClick={() => setMode(value)}>
      {label}
    </button>
  );
  return (
    <>
      <div className="files-search">
        <label className="files-search-field">
          <MagnifyingGlassIcon size={14} aria-hidden="true" />
          <input
            type="search"
            aria-label="Find files"
            placeholder={mode === "names" ? "Find files" : "Search in files"}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <fieldset className="segmented" aria-label="Search in">
          {tab("names", "Names")}
          {tab("contents", "Contents")}
        </fieldset>
      </div>
      {q === "" ? (
        <FileTree worktree={worktree} changedOnly={false} />
      ) : mode === "names" ? (
        <NameResults worktree={worktree} query={q} />
      ) : (
        <ContentResults key={worktree} worktree={worktree} query={q} />
      )}
    </>
  );
}

/** The tree's icons: monochrome through CSS (`.tree-icon`), at the size of the panel's other icons. */
const TREE_ICON = { className: "tree-icon", width: 14, height: 14, "aria-hidden": true } as const;

/**
 * The icon library maps every name it knows, so it cannot be tree-shaken: it loads in its own chunk the first time
 * the tree shows, and each row keeps the icon's place until then.
 */
const symbols = () => import("@react-symbols/icons/utils");

/** A file's icon by its name, from the library's own name and extension mapping (its default for unknown ones). */
const LazyFileIcon = lazy(async () => {
  const { getIconForFile } = await symbols();
  return {
    default: ({ name }: { name: string }) =>
      getIconForFile({ fileName: name, autoAssign: true, ...TREE_ICON }),
  };
});

/**
 * A folder's icon by its name; the library has an open variant only for its default folder, so a named folder keeps
 * its icon open or closed.
 */
const LazyFolderIcon = lazy(async () => {
  const { DefaultFolderIcon, DefaultFolderOpenedIcon, getIconForFolder } = await symbols();
  return {
    default: ({ name, open }: { name: string; open: boolean }) => {
      const icon = getIconForFolder({ folderName: name, ...TREE_ICON });
      return open && icon.type === DefaultFolderIcon ? (
        <DefaultFolderOpenedIcon {...TREE_ICON} />
      ) : (
        icon
      );
    },
  };
});

const iconSpace = <span className="tree-icon-space" />;

const TreeFileIcon = ({ name }: { name: string }) => (
  <Suspense fallback={iconSpace}>
    <LazyFileIcon name={name} />
  </Suspense>
);

const TreeFolderIcon = ({ name, open }: { name: string; open: boolean }) => (
  <Suspense fallback={iconSpace}>
    <LazyFolderIcon name={name} open={open} />
  </Suspense>
);

/** A file of the search results: its name, then its folder, then its status letter. */
function ResultFile({ file }: { file: TreeFile }) {
  const slash = file.path.lastIndexOf("/");
  return (
    <>
      <TreeFileIcon name={file.path.slice(slash + 1)} />
      <span className="result-name">{file.path.slice(slash + 1)}</span>
      <span className="result-folder">{file.path.slice(0, Math.max(slash, 0))}</span>
      {file.status && (
        <span className="status-letter" data-status={STATUS[file.status].letter}>
          {STATUS[file.status].letter}
        </span>
      )}
    </>
  );
}

/** The worktree's files (listed and changed) as the tree shows them, by path. */
function useTreeFiles(worktree: string): TreeFile[] {
  const listing = useHive((s) => (s.worktreeFiles?.path === worktree ? s.worktreeFiles : null));
  const changes = useHive((s) => s.changes[worktree]);
  return useMemo(() => allFiles(listing?.files ?? [], changes?.files ?? []), [listing, changes]);
}

/** "Names": the files whose path holds `query`, in any case. */
function NameResults({ worktree, query }: { worktree: string; query: string }) {
  const files = useTreeFiles(worktree);
  const q = query.toLowerCase();
  const found = files.filter((f) => f.path.toLowerCase().includes(q));
  return (
    <ul className="search-results hive-scroll" aria-label="Matching files">
      {found.length === 0 && <li className="hint">No file name holds “{query}”.</li>}
      {found.slice(0, NAME_LIMIT).map((file) => (
        <li key={file.path}>
          <button
            type="button"
            className="result-file"
            title={file.path}
            data-status={file.status ? STATUS[file.status].letter : undefined}
            onClick={() => leaveFile({ worktree, path: file.path }, true)}
          >
            <ResultFile file={file} />
          </button>
        </li>
      ))}
      {found.length > NAME_LIMIT && (
        <li className="hint">
          Showing {NAME_LIMIT} of {found.length} files: type more to narrow them.
        </li>
      )}
    </ul>
  );
}

/** `text` with every occurrence of `query` (any case) marked. */
function Marked({ text, query }: { text: string; query: string }) {
  const parts: ReactNode[] = [];
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  let at = 0;
  for (let i = lower.indexOf(q); i >= 0; i = lower.indexOf(q, at)) {
    parts.push(text.slice(at, i), <mark key={i}>{text.slice(i, i + q.length)}</mark>);
    at = i + q.length;
  }
  parts.push(text.slice(at));
  return <>{parts}</>;
}

/**
 * "Contents": the lines holding `query`, found by the service once typing pauses, grouped by
 * file. A line opens its file with that line shown.
 */
function ContentResults({ worktree, query }: { worktree: string; query: string }) {
  useEffect(() => {
    const later = setTimeout(() => void transport.searchFiles(worktree, query), SEARCH_DELAY_MS);
    return () => clearTimeout(later);
  }, [worktree, query]);
  const results = useHive((s) =>
    s.searchResults?.worktree === worktree && s.searchResults.query === query
      ? s.searchResults
      : null,
  );
  const files = useTreeFiles(worktree);
  const byFile = useMemo(() => new Map(files.map((f) => [f.path, f])), [files]);
  if (!results) return <div className="hint search-hint">Searching…</div>;
  if (results.error) return <div className="files-error">{results.error}</div>;
  const byPath = new Map<string, SearchMatch[]>();
  for (const m of results.matches) byPath.set(m.path, [...(byPath.get(m.path) ?? []), m]);
  const fileOf = (path: string): TreeFile =>
    byFile.get(path) ?? {
      path,
      status: null,
      old_path: null,
      added: null,
      removed: null,
    };
  return (
    <ul className="search-results hive-scroll" aria-label="Matching lines">
      {byPath.size === 0 && <li className="hint">No file holds “{query}”.</li>}
      {[...byPath].map(([path, matches]) => {
        const file = fileOf(path);
        return (
          <li key={path}>
            <div className="result-file" title={path}>
              <ResultFile file={file} />
              <span className="result-count">{matches.length}</span>
            </div>
            <ul>
              {matches.map((m) => (
                <li key={m.line}>
                  <button
                    type="button"
                    className="result-line"
                    title={`${path}:${m.line}`}
                    onClick={() => leaveFile({ worktree, path }, true, m.line)}
                  >
                    <span className="line-number">{m.line}</span>
                    <span className="line-text">
                      <Marked text={m.text} query={query} />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </li>
        );
      })}
      {results.truncated && (
        <li className="hint">Too many matches: only the first {results.matches.length} show.</li>
      )}
    </ul>
  );
}

/**
 * What the changes are compared with (9.11): HEAD (what is not committed yet) or the main
 * worktree's branch (the merge-base with it: all the work of this branch). Only where there is
 * a branch to compare with, or a branch base fell back to HEAD: then the branch is disabled and
 * its tooltip says why.
 */
function BaseToggle({ changes }: { changes: Changes }) {
  if (!changes.branch && !changes.base_error) return null;
  const branch = changes.branch ?? "Branch";
  const option = (base: DiffBase, label: string, title: string, disabled = false) => (
    <button
      type="button"
      aria-pressed={changes.base === base}
      title={title}
      disabled={disabled}
      onClick={() => setDiffBase(changes.path, base)}
    >
      {label}
    </button>
  );
  return (
    <fieldset className="segmented diff-base" aria-label="Compare with">
      {option("head", "HEAD", "Compare with HEAD: what is not committed yet")}
      {option(
        "branch",
        branch,
        changes.base_error ?? `Compare with where this branch left ${branch}: all its work`,
        !!changes.base_error,
      )}
    </fieldset>
  );
}

/** "N files changed +a −d" and the base toggle, or why the service could not list them. */
function Summary({ worktree }: { worktree: string }) {
  const changes = useHive((s) => s.changes[worktree]);
  if (!changes) return null;
  const n = changes.files.length;
  return (
    <>
      <div className="files-summary">
        <span>{n === 0 ? "No changes" : `${n} ${n === 1 ? "file" : "files"} changed`}</span>
        {n > 0 && <Counts added={changes.added} removed={changes.removed} />}
        <BaseToggle changes={changes} />
      </div>
      {changes.error && <div className="files-error">{changes.error}</div>}
    </>
  );
}

/**
 * The tree, virtualized (#30). It is one focusable element (#35): ↑/↓ move the active row,
 * ←/→ collapse and expand a folder, Enter opens a file or toggles a folder; a click does the
 * same. Unless `changedOnly`, it shows every file the service lists for the watched worktree
 * with the changes' statuses; until that list arrives, the changed files.
 */
function FileTree({ worktree, changedOnly }: { worktree: string; changedOnly: boolean }) {
  const changes = useHive((s) => s.changes[worktree]);
  const listing = useHive((s) => (s.worktreeFiles?.path === worktree ? s.worktreeFiles : null));
  const all = changedOnly ? null : listing;
  const collapsed = useHive((s) => s.collapsed);
  const open = useHive((s) => (s.openFile?.worktree === worktree ? s.openFile.path : null));
  const newFolders = useHive((s) => (changedOnly ? undefined : s.newFolders[worktree]));
  const root = useMemo(
    () =>
      fileTree(
        all ? allFiles(all.files, changes?.files ?? []) : (changes?.files ?? []),
        newFolders,
      ),
    [all, changes, newFolders],
  );
  // Only opening or closing a folder walks the tree again, not moving in it or dragging (9.23).
  const rows = useMemo(
    () => fileRows(worktree, root, collapsed, changedOnly ? "changes" : "files"),
    [worktree, root, collapsed, changedOnly],
  );
  const [active, setActive] = useState(0);
  const scroller = useRef<HTMLDivElement>(null);
  const id = useId();
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 22,
    overscan: 8,
  });
  const at = Math.min(active, rows.length - 1);
  // The Diff tab opens a file as its diff; the Files tab as editable text (#31).
  const pick = (row: FileRow) =>
    row.kind === "folder"
      ? useHive.setState((s) => ({ collapsed: { ...s.collapsed, [row.key]: row.open } }))
      : leaveFile({ worktree, path: row.key }, !changedOnly);
  const drag = useFileDrag(worktree, rows);
  // The entry just renamed or moved becomes the active row once the listing shows it.
  const moved = useHive((s) =>
    !changedOnly && s.movedRow?.worktree === worktree ? s.movedRow.path : null,
  );
  const movedAt =
    moved === null ? -1 : rows.findIndex((r) => (r.kind === "folder" ? r.path : r.key) === moved);
  useEffect(() => {
    if (movedAt < 0) return;
    setActive(movedAt);
    virtual.scrollToIndex(movedAt);
    useHive.setState({ movedRow: null });
  }, [movedAt, virtual]);
  const onKeyDown = (event: KeyboardEvent) => {
    const row = rows[at];
    if (!row) return;
    const move = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (move) {
      const next = Math.max(0, Math.min(rows.length - 1, at + move));
      setActive(next);
      virtual.scrollToIndex(next);
    } else if (event.key === "Enter" || event.key === " ") {
      pick(row);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      if (row.kind === "folder" && row.open === (event.key === "ArrowLeft")) pick(row);
    } else if (event.key === "Delete") {
      askDelete(fileTarget(worktree, row));
    } else {
      return;
    }
    event.preventDefault();
  };
  return (
    <div
      className="files-tree hive-scroll"
      ref={scroller}
      data-file-drop={drag.over === ""}
      {...(changedOnly ? {} : drag.handlers)}
    >
      {changes && rows.length === 0 && <div className="hint">No changes in this worktree.</div>}
      {all?.truncated && <div className="hint">Too many files: the list is cut short.</div>}
      <div
        role="tree"
        aria-label="Files"
        tabIndex={0}
        aria-activedescendant={rows[at] ? `${id}-${at}` : undefined}
        onKeyDown={onKeyDown}
        // The rows' clicks, menus and drags are handled here, so a row renders again only when
        // what it shows changes, not on every ↑/↓ (9.23).
        onClick={(e) => {
          const index = indexAt(e);
          if (index < 0) return;
          setActive(index);
          pick(rows[index] as FileRow);
        }}
        // On a row, its menu; on the tree itself: a click below the rows (the root), or the Menu
        // key (the active row).
        onContextMenu={(e) => {
          const index = indexAt(e);
          if (index >= 0) setActive(index);
          const on = index >= 0 ? index : e.clientX || e.clientY ? -1 : at;
          openTreeMenu(worktree, rows[on], document.getElementById(`${id}-${on}`))(e);
        }}
        onDragStart={(e) => {
          const row = rows[indexAt(e)];
          if (row) drag.start(e, row);
        }}
        style={{ height: virtual.getTotalSize(), position: "relative" }}
      >
        {virtual.getVirtualItems().map((item) => {
          const row = rows[item.index] as FileRow;
          return (
            <TreeRow
              key={row.key}
              row={row}
              index={item.index}
              start={item.start}
              id={id}
              active={item.index === at}
              selected={row.kind === "file" && row.key === open}
              dropTarget={row.kind === "folder" && row.path === drag.over}
              movable={!changedOnly}
            />
          );
        })}
      </div>
    </div>
  );
}

/** Which row of the tree an event happened on; -1 for the tree itself. */
function indexAt(e: { target: EventTarget }): number {
  const at = (e.target as Element).closest("[data-index]")?.getAttribute("data-index");
  return at == null ? -1 : Number(at);
}

/** A row of the files tree; the tree handles its keys, clicks, menu and drag. */
const TreeRow = memo(function TreeRow(props: {
  row: FileRow;
  index: number;
  start: number;
  id: string;
  active: boolean;
  selected: boolean;
  dropTarget: boolean;
  movable: boolean;
}) {
  const { row, index } = props;
  const status = row.kind === "folder" ? row.status : row.file.status;
  return (
    <div
      id={`${props.id}-${index}`}
      role="treeitem"
      tabIndex={-1}
      aria-expanded={row.kind === "folder" ? row.open : undefined}
      aria-selected={props.selected}
      className="file-row"
      data-active={props.active}
      data-status={status ? STATUS[status].letter : undefined}
      data-deleted={status === "deleted"}
      data-index={index}
      data-file-drop={props.dropTarget}
      draggable={props.movable && (row.kind === "folder" || status !== "deleted")}
      title={row.kind === "file" ? row.key : undefined}
      style={{ transform: `translateY(${props.start}px)`, paddingLeft: 8 + row.depth * 14 }}
    >
      {row.kind === "folder" ? (
        <>
          <ChevronIcon open={row.open} />
          <TreeFolderIcon name={row.name} open={row.open} />
          <span className="name">{row.name}</span>
          {!row.open && status && <span className="status-dot" />}
        </>
      ) : (
        <>
          <span className="chevron-space" />
          <TreeFileIcon name={row.name} />
          <span className="name">{row.name}</span>
          <Counts added={row.file.added} removed={row.file.removed} />
          {status && (
            <span className="status-letter" title={status}>
              {STATUS[status].letter}
            </span>
          )}
        </>
      )}
    </div>
  );
});

/** How long a closed folder must be hovered while dragging before it opens. */
export const HOVER_OPEN_MS = 600;

/** The folder holding `path` ("" for the root). */
const parentOf = (path: string) => path.slice(0, Math.max(path.lastIndexOf("/"), 0));

/**
 * Dragging a file or a folder of the Files tree onto a folder, a file (its folder) or the tree
 * below the rows (the root) asks the service to move it there; its own folder does nothing, and
 * a folder never goes into itself (no drop line). A closed folder hovered for
 * [`HOVER_OPEN_MS`] opens; it closes again when the drag leaves it (its row and the rows inside
 * it) or ends without a drop. Folders open before the drag stay open, and the folder a drop
 * goes to opens with its parents, so the moved entry shows. `over` is the folder a drop would
 * go to.
 */
function useFileDrag(worktree: string, rows: FileRow[]) {
  const [dragged, setDragged] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const hover = useRef<{ key: string; timer: ReturnType<typeof setTimeout> } | null>(null);
  // The folders this drag opened.
  const opened = useRef<string[]>([]);
  const setOpen = (folders: string[], open: boolean) => {
    if (folders.length === 0) return;
    const keys = folders.map((f) => [`files:${worktree}/${f}`, !open]);
    useHive.setState((s) => ({ collapsed: { ...s.collapsed, ...Object.fromEntries(keys) } }));
  };
  // Closes the folders the drag opened, except those holding `folder`.
  const closeOutside = (folder: string | null) => {
    const holds = (f: string) => folder !== null && within(folder, f);
    setOpen(
      opened.current.filter((f) => !holds(f)),
      false,
    );
    opened.current = opened.current.filter(holds);
  };
  const unhover = () => {
    if (hover.current) clearTimeout(hover.current.timer);
    hover.current = null;
  };
  const end = () => {
    unhover();
    closeOutside(null);
    setDragged(null);
    setOver(null);
  };
  // A folder does not open after the tree is gone.
  useEffect(() => () => clearTimeout(hover.current?.timer), []);
  const rowAt = (e: DragEvent) => rows[indexAt(e)];
  const start = (e: DragEvent, row: FileRow) => {
    const { path } = fileTarget(worktree, row);
    if (path === null) return;
    e.dataTransfer.setData("text/plain", path);
    e.dataTransfer.effectAllowed = "move";
    setDragged(path);
  };
  const handlers = {
    onDragOver: (e: DragEvent) => {
      if (dragged === null) return;
      const row = rowAt(e);
      const folder = fileTarget(worktree, row).folder;
      closeOutside(folder);
      if (hover.current?.key !== row?.key) unhover();
      // A folder never goes into itself: no drop there.
      if (within(folder, dragged)) return setOver(null);
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      setOver(folder);
      if (hover.current || row?.kind !== "folder" || row.open) return;
      const timer = setTimeout(() => {
        opened.current.push(row.path);
        setOpen([row.path], true);
      }, HOVER_OPEN_MS);
      hover.current = { key: row.key, timer };
    },
    onDragLeave: (e: DragEvent) => {
      if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
      unhover();
      closeOutside(null);
      setOver(null);
    },
    onDrop: (e: DragEvent) => {
      if (dragged === null) return;
      const folder = fileTarget(worktree, rowAt(e)).folder;
      if (within(folder, dragged)) return;
      e.preventDefault();
      if (folder !== parentOf(dragged)) {
        void transport.moveFile(worktree, dragged, folder);
        const parts = folder.split("/");
        setOpen(folder ? parts.map((_, i) => parts.slice(0, i + 1).join("/")) : [], true);
      }
      // The folders the drag opened stay open.
      opened.current = [];
      end();
    },
    onDragEnd: end,
  };
  return { over, start, handlers };
}

/**
 * Opens `next` in its tab (8.21), at `line` when given. A file already open switches to editable
 * text or its diff as asked, unless it has unsaved edits.
 */
export function leaveFile(next: OpenFile, editing = false, line?: number): void {
  setOpenFile(next, editing, line);
  const s = useHive.getState();
  if (s.editing !== editing && !(s.edit && isDirty(s.edit))) setEditing(editing);
}

/** Closes file `f`'s tab, once the user agrees to drop its unsaved edits, if any. */
export function closeFile(f: OpenFile): void {
  const { edit } = fileTabState(useHive.getState(), f);
  const close = () => useHive.setState((s) => dropFile(s, f));
  if (edit && isDirty(edit)) askDiscard(f.path, close);
  else close();
}

/**
 * The open file's header and, under it, its diff against HEAD when it is among the changes,
 * else its text (CodeMirror, `src/viewer/`), shown in the open file's tab of the terminal area
 * (whose × closes it). While `editing` it is editable text (the diff stays read-only, #31):
 * a changed file switches with Edit / Diff, the header has Save
 * (Ctrl+S in the editor; its tab marks unsaved edits), "Agent working here" warns that an agent may write it meanwhile, and
 * "Open in external editor" hands it to Windows.
 */
export function FileView({ worktree }: { worktree: string }) {
  const openFile = useHive((s) => s.openFile);
  const file = useHive((s) => s.changes[worktree]?.files.find((f) => f.path === openFile?.path));
  const text = useHive((s) =>
    s.file?.worktree === worktree && s.file.path === openFile?.path ? s.file : null,
  );
  const unsendable = useHive((s) => {
    const target = referenceTarget(s);
    return "why" in target ? target.why : null;
  });
  const editing = useHive((s) => s.editing);
  const edit = useHive((s) => s.edit);
  const editorNotice = useHive((s) => s.editorNotice);
  const working = useHive((s) => agentWorkingIn(s, worktree));
  if (openFile?.worktree !== worktree) return null;
  const why = text && notice(text);
  const dirty = !!edit && isDirty(edit);
  return (
    <section className="file-view" aria-label={openFile.path}>
      <div className="file-view-bar">
        <span className="status-letter" data-status={file && STATUS[file.status].letter}>
          {file && STATUS[file.status].letter}
        </span>
        <span className="path">{openFile.path}</span>
        {file && <Counts added={file.added} removed={file.removed} />}
        {working && (
          <span className="tab-badge working" title="An agent in this worktree may write this file">
            Agent working here
          </span>
        )}
        {edit && (
          <button
            type="button"
            className="ghost text"
            title={keyText("Save (Ctrl+S)")}
            disabled={!dirty || !!edit.saving}
            onClick={saveOpenFile}
          >
            Save
          </button>
        )}
        {file &&
          (editing ? (
            <button
              type="button"
              className="ghost text"
              title={dirty ? "Save or reload the file first" : "Show the diff"}
              disabled={dirty}
              onClick={() => setEditing(false)}
            >
              Diff
            </button>
          ) : (
            <button
              type="button"
              className="ghost text"
              title="Edit the file"
              disabled={text?.content == null || !!why}
              onClick={() => setEditing(true)}
            >
              Edit
            </button>
          ))}
        <CommentButton />
        <button
          type="button"
          className="ghost"
          aria-label="Open in external editor"
          title={`Open in external editor (the ${isMac() ? "" : "Windows "}default app for the file)`}
          onClick={() => openInEditor(worktree, openFile.path)}
        >
          <ExternalIcon />
        </button>
        <button
          type="button"
          className="ghost"
          aria-label="Send to terminal"
          title={
            unsendable ??
            keyText("Send the selected lines' reference to the terminal (Ctrl+Shift+L)")
          }
          disabled={unsendable !== null}
          onClick={sendReference}
        >
          <TerminalIcon />
        </button>
      </div>
      <CommentInput worktree={worktree} path={openFile.path} />
      {editorNotice && <div className="files-error">{editorNotice}</div>}
      <div className="file-view-body">
        {edit ? (
          <EditView key={`${worktree}\n${openFile.path}`} edit={edit} />
        ) : (
          <>
            {!file && <div className="hint">No changes in this file.</div>}
            {why && <div className="hint">{why}</div>}
            {text && !why && (
              <CodeView key={`${worktree}\n${openFile.path}`} text={text} diff={!!file} />
            )}
          </>
        )}
      </div>
      <ReviewList worktree={worktree} />
    </section>
  );
}
