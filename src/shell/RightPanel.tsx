import {
  ClockCounterClockwiseIcon,
  FilesIcon,
  GitDiffIcon,
  MagnifyingGlassIcon,
} from "@phosphor-icons/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  type DragEvent,
  type KeyboardEvent,
  lazy,
  type MouseEvent,
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
  type FileStatus,
  type FileTarget,
  type OpenFile,
  openFileMenu,
  type PanelView,
  panelWorktree,
  type SearchMatch,
  setEditing,
  setOpenFile,
  setPanelView,
  setRightPanel,
  useHive,
  type Worktree,
} from "../store";
import { transport } from "../transport";
import { isDirty, isFor } from "../viewer/buffer";
import { CodeView, notice } from "../viewer/CodeView";
import { EditView, saveOpenFile } from "../viewer/EditView";
import { referenceTarget, sendReference } from "../viewer/reference";
import { CommentButton, CommentInput, ReviewList } from "../viewer/review";
import { isMac, keyText } from "../window";
import { askDiscard } from "./ConfirmDialog";
import { BranchIcon, ChevronIcon, CloseIcon, ExternalIcon, TerminalIcon } from "./icons";
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

type Folder = { folders: Map<string, Folder>; files: TreeFile[] };

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

/**
 * The visible rows of the files tree: `files` (sorted by the service) grouped into folders,
 * folders before files. A folder starts collapsed and is open only when
 * `collapsed["<tree>:<worktree>/<path>"]` is false: each tree (Files, Diff) opens its own folders. Grouping paths is presentation; statuses and counts are the service's.
 * `folders` (created from the tree) show even when git lists nothing in them.
 */
export function fileRows(
  worktree: string,
  files: TreeFile[],
  collapsed: Record<string, boolean>,
  tree: "files" | "changes" = "files",
  folders: string[] = [],
): FileRow[] {
  const root: Folder = { folders: new Map(), files: [] };
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
  const rows: FileRow[] = [];
  const walk = (folder: Folder, prefix: string, depth: number) => {
    for (const [name, inner] of [...folder.folders].sort(([a], [b]) => (a < b ? -1 : 1))) {
      const key = `${tree}:${worktree}/${prefix}${name}`;
      const open = collapsed[key] === false;
      const path = `${prefix}${name}`;
      rows.push({ kind: "folder", key, path, name, depth, open, status: strongest(inner) });
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
 * the root (no row: the tree's background); only a file still on disk can be renamed.
 */
export function fileTarget(worktree: string, row: FileRow | undefined): FileTarget {
  if (!row) return { worktree, folder: "", path: null };
  if (row.kind === "folder") return { worktree, folder: row.path, path: null };
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

/** The status of highest rank inside a folder; null when nothing inside changed. */
function strongest(folder: Folder): FileStatus | null {
  const inside = [
    ...folder.files.map((f) => f.status),
    ...[...folder.folders.values()].map(strongest),
  ];
  return inside.reduce<FileStatus | null>(
    (a, b) => (b && (!a || STATUS[b].rank > STATUS[a].rank) ? b : a),
    null,
  );
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
 * shown terminal's); its changes are asked for when it is shown and whenever it changes.
 */
function useShownWorktree() {
  const target = useHive(useShallow(panelWorktree));
  const path = target?.worktree.path;
  useEffect(() => {
    if (path) void transport.listChanges(path);
  }, [path]);
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
  if (!results) return <div className="hint search-hint">Searching…</div>;
  if (results.error) return <div className="files-error">{results.error}</div>;
  const byPath = new Map<string, SearchMatch[]>();
  for (const m of results.matches) byPath.set(m.path, [...(byPath.get(m.path) ?? []), m]);
  const fileOf = (path: string): TreeFile =>
    files.find((f) => f.path === path) ?? {
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

/** "N files changed +a −d", or why the service could not list them. */
function Summary({ worktree }: { worktree: string }) {
  const changes = useHive((s) => s.changes[worktree]);
  if (!changes) return null;
  const n = changes.files.length;
  return (
    <>
      <div className="files-summary">
        <span>{n === 0 ? "No changes" : `${n} ${n === 1 ? "file" : "files"} changed`}</span>
        {n > 0 && <Counts added={changes.added} removed={changes.removed} />}
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
  const files = useMemo(
    () => (all ? allFiles(all.files, changes?.files ?? []) : (changes?.files ?? [])),
    [all, changes],
  );
  const rows = fileRows(worktree, files, collapsed, changedOnly ? "changes" : "files", newFolders);
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
        // On the tree itself: a click below the rows (the root), or the Menu key (the active row).
        onContextMenu={(e) => {
          const key = !(e.clientX || e.clientY);
          const row = key ? document.getElementById(`${id}-${at}`) : null;
          openTreeMenu(worktree, key ? rows[at] : undefined, row)(e);
        }}
        style={{ height: virtual.getTotalSize(), position: "relative" }}
      >
        {virtual.getVirtualItems().map((item) => {
          const row = rows[item.index];
          const status = row.kind === "folder" ? row.status : row.file.status;
          return (
            // biome-ignore lint/a11y/useKeyWithClickEvents: the tree handles the keys.
            <div
              key={row.key}
              id={`${id}-${item.index}`}
              role="treeitem"
              tabIndex={-1}
              aria-expanded={row.kind === "folder" ? row.open : undefined}
              aria-selected={row.kind === "file" && row.key === open}
              className="file-row"
              data-active={item.index === at}
              data-status={status ? STATUS[status].letter : undefined}
              data-deleted={status === "deleted"}
              data-index={item.index}
              data-file-drop={row.kind === "folder" && row.path === drag.over}
              draggable={!changedOnly && row.kind === "file" && status !== "deleted"}
              onDragStart={(e) => drag.start(e, row)}
              title={row.kind === "file" ? row.key : undefined}
              style={{ transform: `translateY(${item.start}px)`, paddingLeft: 8 + row.depth * 14 }}
              onClick={() => {
                setActive(item.index);
                pick(row);
              }}
              onContextMenu={(e) => {
                setActive(item.index);
                openTreeMenu(worktree, row)(e);
              }}
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
        })}
      </div>
    </div>
  );
}

/** How long a closed folder must be hovered while dragging a file before it opens. */
export const HOVER_OPEN_MS = 600;

/**
 * Dragging a file of the Files tree onto a folder, a file (its folder) or the tree below the
 * rows (the root) asks the service to move it there; its own folder does nothing. A closed
 * folder hovered for [`HOVER_OPEN_MS`] opens. `over` is the folder a drop would go to.
 */
function useFileDrag(worktree: string, rows: FileRow[]) {
  const [dragged, setDragged] = useState<FileTarget | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const hover = useRef<{ key: string; timer: ReturnType<typeof setTimeout> } | null>(null);
  const unhover = () => {
    if (hover.current) clearTimeout(hover.current.timer);
    hover.current = null;
  };
  const end = () => {
    unhover();
    setDragged(null);
    setOver(null);
  };
  // A folder does not open after the tree is gone.
  useEffect(() => () => clearTimeout(hover.current?.timer), []);
  const rowAt = (e: DragEvent) => {
    const at = (e.target as Element).closest("[data-index]")?.getAttribute("data-index");
    return at == null ? undefined : rows[Number(at)];
  };
  const start = (e: DragEvent, row: FileRow) => {
    const target = fileTarget(worktree, row);
    if (target.path === null) return;
    e.dataTransfer.setData("text/plain", target.path);
    e.dataTransfer.effectAllowed = "move";
    setDragged(target);
  };
  const handlers = {
    onDragOver: (e: DragEvent) => {
      if (!dragged) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const row = rowAt(e);
      setOver(fileTarget(worktree, row).folder);
      if (hover.current?.key === row?.key) return;
      unhover();
      if (row?.kind !== "folder" || row.open) return;
      const timer = setTimeout(
        () => useHive.setState((s) => ({ collapsed: { ...s.collapsed, [row.key]: false } })),
        HOVER_OPEN_MS,
      );
      hover.current = { key: row.key, timer };
    },
    onDragLeave: (e: DragEvent) => {
      if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
      unhover();
      setOver(null);
    },
    onDrop: (e: DragEvent) => {
      if (!dragged?.path) return;
      e.preventDefault();
      const folder = fileTarget(worktree, rowAt(e)).folder;
      if (folder !== dragged.folder) void transport.moveFile(worktree, dragged.path, folder);
      end();
    },
    onDragEnd: end,
  };
  return { over, start, handlers };
}

/**
 * Opens `next` (null closes the file), at `line` when given, once the user agrees to drop the unsaved edits of the
 * file open now, if any. The file already open switches to editable text or its diff as asked, unless it has unsaved edits.
 */
export function leaveFile(next: OpenFile | null, editing = false, line?: number): void {
  const { edit } = useHive.getState();
  const losing = edit && isDirty(edit) && !(next && isFor(next, edit));
  const go = () => {
    setOpenFile(next, editing, line);
    const s = useHive.getState();
    const same = next && s.openFile && isFor(next, s.openFile);
    if (same && s.editing !== editing && !(s.edit && isDirty(s.edit))) setEditing(editing);
  };
  if (losing) askDiscard(edit.path, go);
  else go();
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
              title={dirty ? "Save or reload the file first" : "Show the diff against HEAD"}
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
          onClick={() => void transport.openInEditor(worktree, openFile.path)}
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
