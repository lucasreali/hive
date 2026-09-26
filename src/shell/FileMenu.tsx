import { FilePlusIcon, FolderPlusIcon, type Icon, PencilSimpleIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { type FileDialogKind, openFileDialog, openFileMenu, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon, ICON } from "./icons";
import { ContextMenu, showModal } from "./WorktreeMenu";

const closeMenu = () => openFileMenu(null);
const close = () => useHive.setState({ modal: null, fileDialog: null });

/** The file tree's menu (right click on a file, a folder or the tree's background). */
export function FileMenu() {
  const menu = useHive((s) => s.fileMenu);
  if (!menu) return null;
  const { x, y, ...target } = menu;
  const item = (Shape: Icon, label: string, kind: FileDialogKind) => (
    <button
      type="button"
      role="menuitem"
      onClick={() => {
        closeMenu();
        openFileDialog(target, kind);
      }}
    >
      <Shape {...ICON} />
      {label}
    </button>
  );
  return (
    <ContextMenu at={menu} label={target.path ?? "Files"} onClose={closeMenu}>
      {item(FilePlusIcon, "New File…", "file")}
      {item(FolderPlusIcon, "New Folder…", "folder")}
      {target.path !== null && item(PencilSimpleIcon, "Rename…", "rename")}
    </ContextMenu>
  );
}

const TITLES: Record<FileDialogKind, [title: string, submit: string]> = {
  file: ["New file", "Create"],
  folder: ["New folder", "Create"],
  rename: ["Rename file", "Rename"],
};

/**
 * "New file" or "New folder" (in the menu's folder) or "Rename file": the service checks the
 * name, creates or renames without ever overwriting, and its refusal shows under the field.
 */
export function FileNameDialog() {
  const dialog = useHive((s) => s.fileDialog);
  const path = dialog?.kind === "rename" ? (dialog.path ?? "") : "";
  const current = path.slice(path.lastIndexOf("/") + 1);
  const [name, setName] = useState(current);
  if (!dialog) return null;
  const { worktree, folder, kind, error } = dialog;
  const canSubmit = name !== "" && name !== current;
  const [title, action] = TITLES[kind];
  return (
    <dialog
      className="dialog"
      aria-labelledby="file-name-title"
      ref={showModal("input[name=name]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!canSubmit) return;
          if (kind === "rename") void transport.renameFile(worktree, path, name);
          else if (kind === "folder") void transport.createFolder(worktree, folder, name);
          else void transport.createFile(worktree, folder, name);
        }}
      >
        <header>
          <h2 id="file-name-title">{title}</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <div className="field">
            <label htmlFor="file-name">Name</label>
            <input
              id="file-name"
              name="name"
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                useHive.setState({ fileDialog: { ...dialog, error: null } });
              }}
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
            <span className="plan-label">Folder: </span>
            {folder || "(worktree root)"}
          </div>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary" disabled={!canSubmit}>
            {action} <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
