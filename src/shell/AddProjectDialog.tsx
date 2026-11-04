import { useState } from "react";
import { safeStorage } from "../persist";
import { clearAddProjectError, openModal, useHive } from "../store";
import { transport } from "../transport";
import { homeText } from "../window";
import { FolderField, useFolderListing } from "./FolderField";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

/** Where the last Windows | WSL choice is remembered (a UI preference, not service data). */
const SIDE = "hive.folderSide";

/** Whether the last choice was Windows. */
export const savedWindows = (storage: Pick<Storage, "getItem"> | null = safeStorage()) => {
  try {
    return storage?.getItem(SIDE) === "windows";
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
 * Asks the service to follow a folder, typed or browsed in a `FolderField`. With the service in
 * WSL, a select picks WSL or Windows paths; the service converts them. It answers the add with the project (the dialog closes) or the
 * reason it was refused (shown under the field). With the service on Windows itself, every path
 * is a Windows one (`\` or `/`, drive letters), listed by the service as typed.
 */
export function AddProjectDialog() {
  const error = useHive((s) => s.addProjectError);
  const wsl = useHive((s) => s.connection.status === "connected" && s.connection.distro !== null);
  const [side, setSide] = useState(() => savedWindows());
  const windows = wsl && side;
  // Windows paths: the WSL service's Windows side, or every path of the service on Windows.
  const backslash = useHive((s) => windows || s.appMode?.mode === "native");
  const [path, setPath] = useState("");
  const listing = useFolderListing(path, windows);
  const typed = path || listing?.path || "";
  const target = listing?.path === typed ? listing.linux_path : null;
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
            <FolderField
              id="add-project-path"
              value={path}
              onChange={(to) => {
                setPath(to);
                clearAddProjectError();
              }}
              windows={windows}
              onWindows={(next) => {
                setSide(next);
                saveWindows(next);
              }}
              home
              placeholder={
                backslash ? "C:\\Users\\you\\projects\\shop" : homeText("/home/you/projects/shop")
              }
              invalid={error !== null}
            />
          </div>
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          {/* A network or cloud folder (D18.9): it can still be added. */}
          {target && listing?.warning && (
            <p className="field-warning" role="status">
              {listing.warning}
            </p>
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
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!target}>
            Add project
          </button>
        </footer>
      </form>
    </dialog>
  );
}
