import { ArrowUpIcon, GitBranchIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { clearAddProjectError, openModal, safeStorage, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon, FolderIcon } from "./icons";

const close = () => openModal(null);

/** How long typing must pause before the folder is listed. */
export const LIST_DELAY_MS = 150;
/** Where the last Windows | WSL choice is remembered (a UI preference, not service data). */
const SIDE = "hive.folderSide";

const savedWindows = () => {
  try {
    return safeStorage()?.getItem(SIDE) === "windows";
  } catch {
    return false;
  }
};

const saveWindows = (windows: boolean) => {
  try {
    safeStorage()?.setItem(SIDE, windows ? "windows" : "wsl");
  } catch {
    // A full or blocked storage only loses the preference.
  }
};

/**
 * Asks the service to follow a folder. Under the field, the subfolders of the typed folder
 * (listed by the service once typing pauses) filtered by the text after its last separator:
 * a click enters one, ↑ goes up. With the service in WSL, a select picks WSL or Windows paths;
 * the service converts them. It answers the add with the project (the dialog closes) or the
 * reason it was refused (shown under the field).
 */
export function AddProjectDialog() {
  const error = useHive((s) => s.addProjectError);
  const wsl = useHive((s) => s.connection.status === "connected" && s.connection.distro !== null);
  const [side, setSide] = useState(savedWindows);
  const windows = wsl && side;
  const [path, setPath] = useState("");
  // The field shows the home folder once it is listed, until the user types.
  const [fill, setFill] = useState(true);
  const input = useRef<HTMLInputElement>(null);
  const listing = useHive((s) =>
    s.dirs?.windows === windows && (s.dirs.path === path || path === "") ? s.dirs : null,
  );
  useEffect(() => {
    const later = setTimeout(() => void transport.listDirs(path, windows), LIST_DELAY_MS);
    return () => clearTimeout(later);
  }, [path, windows]);
  useEffect(() => {
    if (fill && listing && path === "") {
      setPath(listing.path);
      setFill(false);
    }
  }, [fill, listing, path]);

  const typed = path || listing?.path || "";
  const cut = Math.max(typed.lastIndexOf("/"), windows ? typed.lastIndexOf("\\") : -1) + 1;
  const filter = typed.slice(cut).toLowerCase();
  const shown = listing?.dirs.filter((d) => d.name.toLowerCase().includes(filter)) ?? [];
  const target = listing?.path === typed ? listing.linux_path : null;
  const go = (to: string) => {
    setPath(to);
    setFill(false);
    clearAddProjectError();
    input.current?.focus();
  };
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog"
      aria-labelledby="add-project-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          // showModal focuses the first control; the field is what the user needs.
          dialog.querySelector("input")?.focus();
        }
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (target) void transport.addProject(target);
        }}
      >
        <header>
          <h2 id="add-project-title">Add project</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <div className="field">
            <label htmlFor="add-project-path">Folder</label>
            <div className="folder-picker">
              <div className="folder-path">
                {wsl && (
                  <select
                    aria-label="Folder kind"
                    value={windows ? "windows" : "wsl"}
                    onChange={(e) => {
                      const next = e.target.value === "windows";
                      setSide(next);
                      saveWindows(next);
                      setPath("");
                      setFill(true);
                      clearAddProjectError();
                    }}
                  >
                    <option value="windows">Windows</option>
                    <option value="wsl">WSL</option>
                  </select>
                )}
                <input
                  id="add-project-path"
                  ref={input}
                  value={path}
                  onChange={(e) => {
                    setPath(e.target.value);
                    setFill(false);
                    clearAddProjectError();
                  }}
                  placeholder={windows ? "C:\\Users\\you\\projects\\shop" : "/home/you/projects/shop"}
                  spellCheck={false}
                  autoComplete="off"
                  aria-invalid={error !== null}
                />
              </div>
              <ul className="folder-list" aria-label="Folders">
                {listing?.parent && (
                  <li>
                    <button
                      type="button"
                      tabIndex={-1}
                      className="folder-row"
                      title="Up a level"
                      onClick={() => go(listing.parent as string)}
                    >
                      <ArrowUpIcon />
                      <span className="folder-name">{listing.parent}</span>
                    </button>
                  </li>
                )}
                {shown.map((d) => (
                  <li key={d.name}>
                    <button
                      type="button"
                      tabIndex={-1}
                      className="folder-row"
                      onClick={() => go(`${typed.slice(0, cut)}${d.name}${windows ? "\\" : "/"}`)}
                    >
                      {d.git ? <GitBranchIcon aria-label="Repository" /> : <FolderIcon />}
                      <span className="folder-name">{d.name}</span>
                    </button>
                  </li>
                ))}
                {listing && shown.length === 0 && (
                  <li className="folder-empty">{listing.error ?? "No folders here"}</li>
                )}
              </ul>
            </div>
          </div>
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          {windows && (
            <p className="field-help">
              Windows folders are slower and don't update live; WSL folders are recommended.
            </p>
          )}
          <p className="field-help">
            A git repository, or any folder inside one: the whole repository is added.
          </p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary" disabled={!target}>
            Add project <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
