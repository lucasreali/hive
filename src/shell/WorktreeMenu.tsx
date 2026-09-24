import { type KeyboardEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { findWorktree, openMenu, openModal, owner, setNotice, useHive } from "../store";
import { openTerminal } from "../terminals";
import { transport } from "../transport";
import { CloseIcon } from "./icons";

const closeMenu = () => openMenu(null);
const close = () => openModal(null);

/** Up/down move between the menu's enabled items, wrapping around. */
function moveInMenu(event: KeyboardEvent<HTMLElement>): void {
  const items = [
    ...event.currentTarget.querySelectorAll<HTMLElement>("[role=menuitem]:not(:disabled)"),
  ];
  const at = items.indexOf(document.activeElement as HTMLElement);
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  if (event.key === "Escape" || event.key === "Tab") closeMenu();
  else if (step) items[(at + step + items.length) % items.length]?.focus();
  else return;
  event.preventDefault();
}

/**
 * A worktree row's context menu (right click). What it does is the service's: deleting and
 * renaming go through their dialogs, "Open in Explorer" asks for the folder's Windows path.
 * It closes on a click outside, Esc, Tab, scrolling or the window losing focus.
 */
export function WorktreeMenu() {
  const menu = useHive((s) => s.menu);
  const w = useHive((s) => findWorktree(s.projects, s.menu?.worktree ?? null));
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const outside = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) closeMenu();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("scroll", closeMenu, true);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("resize", closeMenu);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("scroll", closeMenu, true);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("resize", closeMenu);
    };
  }, [menu]);

  // Kept inside the window, then focused on its first item for the keyboard.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!menu || !el) return;
    el.style.left = `${Math.max(0, Math.min(menu.x, window.innerWidth - el.offsetWidth))}px`;
    el.style.top = `${Math.max(0, Math.min(menu.y, window.innerHeight - el.offsetHeight))}px`;
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [menu]);

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
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      aria-label={`Worktree ${w.name}`}
      onKeyDown={moveInMenu}
    >
      <button type="button" role="menuitem" onClick={act(() => void openTerminal(w.path))}>
        New terminal here
      </button>
      <button type="button" role="menuitem" onClick={act(copy)}>
        Copy path
      </button>
      <button
        type="button"
        role="menuitem"
        onClick={act(() => void transport.openInEditor(w.path, ""))}
      >
        Open in Explorer
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
    </div>
  );
}

/** Opens a native modal dialog once and focuses `focus` inside it. */
const showModal = (focus: string) => (dialog: HTMLDialogElement | null) => {
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
