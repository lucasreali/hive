import { useVirtualizer } from "@tanstack/react-virtual";
import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import {
  type ChangedFile,
  type FileStatus,
  panelWorktree,
  setChangedOnly,
  setOpenFile,
  setRightPanel,
  toggleCollapsed,
  useHive,
} from "../store";
import { transport } from "../transport";
import { CodeView, notice } from "../viewer/CodeView";
import { BranchIcon, ChevronIcon, CloseIcon, FileIcon, FolderIcon } from "./icons";

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

export type FileRow =
  | { kind: "folder"; key: string; name: string; depth: number; open: boolean; status: FileStatus }
  | { kind: "file"; key: string; name: string; depth: number; file: ChangedFile };

type Folder = { folders: Map<string, Folder>; files: ChangedFile[] };

/**
 * The visible rows of the files tree: `files` (sorted by the service) grouped into folders,
 * folders before files, a collapsed folder (`collapsed["folder:<worktree>/<path>"]`) hiding
 * what is inside it. Grouping paths is presentation; statuses and counts are the service's.
 */
export function fileRows(
  worktree: string,
  files: ChangedFile[],
  collapsed: Record<string, boolean>,
): FileRow[] {
  const root: Folder = { folders: new Map(), files: [] };
  for (const file of files) {
    const parts = file.path.split("/");
    let folder = root;
    for (const part of parts.slice(0, -1)) {
      const next = folder.folders.get(part) ?? { folders: new Map(), files: [] };
      folder.folders.set(part, next);
      folder = next;
    }
    folder.files.push(file);
  }
  const rows: FileRow[] = [];
  const walk = (folder: Folder, prefix: string, depth: number) => {
    for (const [name, inner] of [...folder.folders].sort(([a], [b]) => (a < b ? -1 : 1))) {
      const key = `folder:${worktree}/${prefix}${name}`;
      const open = !collapsed[key];
      rows.push({ kind: "folder", key, name, depth, open, status: strongest(inner) });
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

/** The status of highest rank inside a folder (every folder holds at least one file). */
function strongest(folder: Folder): FileStatus {
  const inside = [
    ...folder.files.map((f) => f.status),
    ...[...folder.folders.values()].map(strongest),
  ];
  return inside.reduce((a, b) => (STATUS[b].rank > STATUS[a].rank ? b : a));
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
 * Files and diff (screen 1g), toggled by Ctrl+Shift+B. It shows the selected worktree (or the
 * selected agent's, or the shown terminal's) and asks the service for its changes when it
 * opens and whenever that worktree changes.
 */
export function RightPanel() {
  const target = useHive(useShallow(panelWorktree));
  const changedOnly = useHive((s) => s.changedOnly);
  const path = target?.worktree.path;
  useEffect(() => {
    if (path) void transport.listChanges(path);
  }, [path]);
  const mode = (label: string, title: string, changed: boolean) => (
    <button
      type="button"
      title={title}
      aria-pressed={changedOnly === changed}
      onClick={() => setChangedOnly(changed)}
    >
      {label}
    </button>
  );
  return (
    <aside className="right-panel" aria-label="Files and diff">
      <div className="bar">
        <span>Files and diff</span>
        <div className="segmented">
          {mode("All", "Show all files", false)}
          {mode("Changed", "Show only changed files", true)}
        </div>
        <button
          type="button"
          className="ghost"
          title="Collapse (Ctrl+Shift+B)"
          onClick={() => setRightPanel(null)}
        >
          <CloseIcon />
        </button>
      </div>
      {target ? (
        <>
          <div className="files-info">
            <div className="files-worktree">
              <BranchIcon />
              <span className="name">{target.worktree.name}</span>
              <span className="project">{target.project.name}</span>
            </div>
            <Summary worktree={target.worktree.path} />
          </div>
          <FileTree worktree={target.worktree.path} />
          <FileView worktree={target.worktree.path} />
        </>
      ) : (
        <div className="right-panel-empty">Select a project or agent to see its files.</div>
      )}
    </aside>
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
 * same. Until the whole file list arrives (task 3.1), "All" shows the changed files too.
 */
function FileTree({ worktree }: { worktree: string }) {
  const changes = useHive((s) => s.changes[worktree]);
  const collapsed = useHive((s) => s.collapsed);
  const open = useHive((s) => (s.openFile?.worktree === worktree ? s.openFile.path : null));
  const rows = fileRows(worktree, changes?.files ?? [], collapsed);
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
  const pick = (row: FileRow) =>
    row.kind === "folder" ? toggleCollapsed(row.key) : setOpenFile({ worktree, path: row.key });
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
    <div className="files-tree hive-scroll" ref={scroller}>
      {changes && rows.length === 0 && <div className="hint">No changes in this worktree.</div>}
      <div
        role="tree"
        aria-label="Files"
        tabIndex={0}
        aria-activedescendant={rows[at] ? `${id}-${at}` : undefined}
        onKeyDown={onKeyDown}
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
              data-status={STATUS[status].letter}
              data-deleted={status === "deleted"}
              title={row.kind === "file" ? row.key : undefined}
              style={{ transform: `translateY(${item.start}px)`, paddingLeft: 8 + row.depth * 14 }}
              onClick={() => {
                setActive(item.index);
                pick(row);
              }}
            >
              {row.kind === "folder" ? (
                <>
                  <ChevronIcon open={row.open} />
                  <FolderIcon />
                  <span className="name">{row.name}</span>
                  {!row.open && <span className="status-dot" />}
                </>
              ) : (
                <>
                  <span className="chevron-space" />
                  <FileIcon />
                  <span className="name">{row.name}</span>
                  <Counts added={row.file.added} removed={row.file.removed} />
                  <span className="status-letter" title={row.file.status}>
                    {STATUS[status].letter}
                  </span>
                </>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * The open file's header and, under it, its diff against HEAD when it is among the changes,
 * else its text (CodeMirror, `src/viewer/`). Shown while `openFile` is in this worktree;
 * "Close diff" clears it.
 */
export function FileView({ worktree }: { worktree: string }) {
  const openFile = useHive((s) => s.openFile);
  const file = useHive((s) => s.changes[worktree]?.files.find((f) => f.path === openFile?.path));
  const text = useHive((s) =>
    s.file?.worktree === worktree && s.file.path === openFile?.path ? s.file : null,
  );
  if (openFile?.worktree !== worktree) return null;
  const why = text && notice(text);
  return (
    <section className="file-view" aria-label={openFile.path}>
      <div className="file-view-bar">
        <span className="status-letter" data-status={file && STATUS[file.status].letter}>
          {file && STATUS[file.status].letter}
        </span>
        <span className="path">{openFile.path}</span>
        {file && <Counts added={file.added} removed={file.removed} />}
        <button
          type="button"
          className="ghost"
          title="Close diff"
          onClick={() => setOpenFile(null)}
        >
          <CloseIcon />
        </button>
      </div>
      <div className="file-view-body">
        {!file && <div className="hint">No changes in this file.</div>}
        {why && <div className="hint">{why}</div>}
        {text && !why && (
          <CodeView key={`${worktree}\n${openFile.path}`} text={text} diff={!!file} />
        )}
      </div>
    </section>
  );
}
