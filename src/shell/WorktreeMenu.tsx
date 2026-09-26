import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  findWorktree,
  openMenu,
  openModal,
  openProjectMenu,
  owner,
  scriptsOf,
  setNotice,
  useHive,
  type Worktree,
} from "../store";
import { openTerminal, openWith } from "../terminals";
import { transport } from "../transport";
import { isMac } from "../window";
import { CloseIcon } from "./icons";

const closeMenu = () => openMenu(null);
const close = () => openModal(null);

/** Up/down move between the menu's enabled items, wrapping around; Esc and Tab close it. */
function moveInMenu(event: KeyboardEvent<HTMLElement>, onClose: () => void): void {
  const items = [
    ...event.currentTarget.querySelectorAll<HTMLElement>("[role=menuitem]:not(:disabled)"),
  ];
  const at = items.indexOf(document.activeElement as HTMLElement);
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  if (event.key === "Escape" || event.key === "Tab") onClose();
  else if (step) items[(at + step + items.length) % items.length]?.focus();
  else return;
  event.preventDefault();
}

/**
 * A context menu at `at` (kept inside the window), focused on its first item. It closes
 * (`onClose`, which must not change between renders) on a click outside it and its `anchor`
 * (the button that toggles it, if any), Esc, Tab, scrolling outside it, resizing or the window
 * losing focus.
 */
export function ContextMenu(props: {
  at: { x: number; y: number };
  label: string;
  onClose: () => void;
  anchor?: HTMLElement | null;
  className?: string;
  children: ReactNode;
}) {
  const { at, onClose, anchor } = props;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const outside = (e: Event) => {
      const target = e.target as Node;
      if (!ref.current?.contains(target) && !anchor?.contains(target)) onClose();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("scroll", outside, true);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("scroll", outside, true);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose, anchor]);
  useLayoutEffect(() => {
    const el = ref.current as HTMLDivElement;
    el.style.left = `${Math.max(0, Math.min(at.x, window.innerWidth - el.offsetWidth))}px`;
    el.style.top = `${Math.max(0, Math.min(at.y, window.innerHeight - el.offsetHeight))}px`;
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [at]);
  return (
    <div
      ref={ref}
      className={props.className ? `context-menu ${props.className}` : "context-menu"}
      role="menu"
      aria-label={props.label}
      onKeyDown={(e) => moveInMenu(e, onClose)}
    >
      {props.children}
    </div>
  );
}

/**
 * A worktree row's context menu (right click). What it does is the service's: deleting and
 * renaming go through their dialogs, "Open in Explorer" ("Reveal in Finder" on macOS) asks for
 * the folder's path as the OS sees it.
 */
export function WorktreeMenu() {
  const menu = useHive((s) => s.menu);
  const w = useHive((s) => findWorktree(s.projects, s.menu?.worktree ?? null));
  const runs = useHive((s) => scriptsOf(s.settings, owner(s.projects, w?.id ?? null)?.id).run);

  if (!menu || !w) return null;
  const act = (action: () => void) => () => {
    closeMenu();
    action();
  };
  const copy = () =>
    navigator.clipboard
      .writeText(w.path)
      .then(() => setNotice(`Copied ${w.path}`))
      .catch((error) => setNotice(`Cannot copy the path: ${error}`));
  const renameWhy = w.main
    ? "The main worktree cannot be renamed"
    : !w.claude
      ? "Only worktrees under .claude/worktrees can be renamed"
      : undefined;
  return (
    <ContextMenu at={menu} label={`Worktree ${w.name}`} onClose={closeMenu}>
      <button type="button" role="menuitem" onClick={act(() => void openTerminal(w.path))}>
        New terminal here
      </button>
      {runs.map((run) => (
        <button
          type="button"
          role="menuitem"
          key={run.name}
          title={run.command}
          onClick={act(() => void openWith(w.path, run.command))}
        >
          Run: {run.name}
        </button>
      ))}
      <button type="button" role="menuitem" onClick={act(copy)}>
        Copy path
      </button>
      <button
        type="button"
        role="menuitem"
        onClick={act(() => void transport.openInEditor(w.path, ""))}
      >
        {isMac() ? "Reveal in Finder" : "Open in Explorer"}
      </button>
      <hr />
      <button
        type="button"
        role="menuitem"
        disabled={renameWhy !== undefined}
        title={renameWhy}
        onClick={act(() => openModal("rename-worktree", null, w.id))}
      >
        Rename…
      </button>
      <button
        type="button"
        role="menuitem"
        className="danger"
        disabled={w.main}
        title={w.main ? "The main worktree cannot be deleted" : undefined}
        onClick={act(() => openModal("remove-worktree", null, w.id))}
      >
        Delete…
      </button>
    </ContextMenu>
  );
}

const closeProjectMenu = () => openProjectMenu(null);

/** A project row's context menu (right click). */
export function ProjectMenu() {
  const menu = useHive((s) => s.projectMenu);
  if (!menu) return null;
  const removeMerged = () => {
    closeProjectMenu();
    openModal("remove-merged", menu.project);
  };
  return (
    <ContextMenu at={menu} label="Project" onClose={closeProjectMenu}>
      <button type="button" role="menuitem" onClick={removeMerged}>
        Remove merged worktrees…
      </button>
    </ContextMenu>
  );
}

/** A linked worktree the service reports merged into the main worktree's branch, with no changes. */
const removable = (w: Worktree) => !w.main && w.status?.merged && w.status.changes === 0;

/**
 * Removing a project's merged, clean worktrees at once: each checked one gets a plain
 * `remove_worktree` (never `--force`), and its own result. Nothing is ever merged (#12).
 */
export function RemoveMergedDialog() {
  const project = useHive((s) => s.projects?.[s.modalProject ?? ""]);
  const failures = useHive((s) => s.worktreeDialog.removeFailures);
  // As listed when the dialog opened: a removed worktree stays, with its result.
  const [listed] = useState(() => project?.worktrees.filter(removable) ?? []);
  const [unchecked, setUnchecked] = useState<Set<string>>(new Set());
  const [sent, setSent] = useState<Set<string>>(new Set());
  const present = new Set(project?.worktrees.map((w) => w.path));
  const chosen = listed.filter((w) => !unchecked.has(w.path) && !sent.has(w.path));
  const toggle = (path: string) =>
    setUnchecked((was) => {
      const next = new Set(was);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  const result = (path: string) => failures[path] ?? (present.has(path) ? "Removing…" : "Removed");
  return (
    <dialog
      className="dialog"
      aria-labelledby="remove-merged-title"
      ref={showModal("button[type=submit]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          for (const w of chosen) void transport.removeWorktree(w.path, false);
          setSent(new Set([...sent, ...chosen.map((w) => w.path)]));
        }}
      >
        <header>
          <h2 id="remove-merged-title">Remove merged worktrees</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {listed.length === 0 ? (
            <p>No merged worktrees without changes.</p>
          ) : (
            <>
              <p className="field-help">
                Every commit of these is on the main worktree's branch and they have no changes.
                Their branches are kept.
              </p>
              <ul className="merged-list">
                {listed.map((w) => (
                  <li key={w.path} className="merged-item">
                    <label>
                      <input
                        type="checkbox"
                        checked={!unchecked.has(w.path)}
                        disabled={sent.has(w.path)}
                        onChange={() => toggle(w.path)}
                      />
                      <span className="label">{w.name}</span>
                      <span className="field-help">
                        last commit {new Date(w.status?.last_commit_ms ?? 0).toLocaleDateString()}
                      </span>
                    </label>
                    {sent.has(w.path) && (
                      <span
                        className={failures[w.path] ? "field-error" : "field-help"}
                        role="status"
                      >
                        {result(w.path)}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Close <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary danger" disabled={chosen.length === 0}>
            Remove ({chosen.length}) <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}

/** Opens a native modal dialog once and focuses `focus` inside it. */
export const showModal = (focus: string) => (dialog: HTMLDialogElement | null) => {
  if (dialog && !dialog.open) {
    dialog.showModal();
    dialog.querySelector<HTMLElement>(focus)?.focus();
  }
};

/**
 * Deleting a worktree from its menu. The first attempt is plain `git worktree remove`; when the
 * service refuses it (changes, or a process working there) the reason shows and the button
 * becomes "Delete anyway" (`--force`). The branch is always kept.
 */
export function RemoveWorktreeDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const failure = useHive((s) => s.worktreeDialog.failure);
  if (!w) return null;
  const failed = failure?.path === w.path && failure.name === null ? failure.message : null;
  return (
    <dialog
      className="dialog"
      aria-labelledby="remove-worktree-title"
      ref={showModal("button[type=submit]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void transport.removeWorktree(w.path, failed !== null);
        }}
      >
        <header>
          <h2 id="remove-worktree-title">Delete worktree</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            Delete <strong>{w.name}</strong> and its folder <code>{w.path}</code>?
          </p>
          {w.branch && (
            <p className="field-help">
              The branch <code>{w.branch}</code> is kept.
            </p>
          )}
          {failed && (
            <p className="field-error" role="alert">
              {failed}
            </p>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary danger">
            {failed ? "Delete anyway" : "Delete"} <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}

/**
 * Renaming a Claude worktree from its menu: the name is checked by the service as the user
 * types (#33), and the folder and its `worktree-<name>` branch move together.
 */
export function RenameWorktreeDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const project = useHive((s) => owner(s.projects, s.modalWorktree)?.id ?? "");
  const { nameChecks, failure } = useHive((s) => s.worktreeDialog);
  const [name, setName] = useState(w?.name ?? "");
  useEffect(() => void transport.validateWorktreeName(project, name), [project, name]);
  if (!w) return null;

  const check = nameChecks[name]?.project === project ? nameChecks[name] : null;
  const failed = failure?.path === w.path && failure.name === name ? failure.message : null;
  const same = name === w.name;
  const error = failed ?? (name && !same ? check?.error : null);
  const canRename = !same && check?.error === null && !failed;
  // The branch follows only while the worktree is still on its own `worktree-<name>`.
  const ownBranch = w.branch === `worktree-${w.name}`;
  return (
    <dialog
      className="dialog"
      aria-labelledby="rename-worktree-title"
      ref={showModal("input[name=name]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (canRename) void transport.renameWorktree(w.path, name);
        }}
      >
        <header>
          <h2 id="rename-worktree-title">Rename worktree</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <div className="field">
            <label htmlFor="rename-worktree-name">New name</label>
            <input
              id="rename-worktree-name"
              name="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              spellCheck={false}
              autoComplete="off"
              aria-invalid={Boolean(error)}
            />
            {error && (
              <span className="field-error" role="alert">
                {error}
              </span>
            )}
          </div>
          <div className="plan">
            <div>
              <span className="plan-label">Folder: </span>
              {check?.folder}
            </div>
            <div>
              <span className="plan-label">Branch: </span>
              {ownBranch ? check?.branch : `${w.branch ?? "detached"} (unchanged)`}
            </div>
          </div>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary" disabled={!canRename}>
            Rename <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
