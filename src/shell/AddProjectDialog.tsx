import { useEffect, useRef, useState } from "react";
import { safeStorage } from "../persist";
import { clearAddProjectError, openModal, select, useHive } from "../store";
import { openClaude, showOpenFailure } from "../terminals";
import { transport } from "../transport";
import { homeText } from "../window";
import { FolderField, useFolderListing } from "./FolderField";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

/** Where the last Windows | WSL choice is remembered (a UI preference, not service data). */
const SIDE = "hive.folderSide";
/** Where the parent folder of the last new project is remembered (23.2), as it was typed. */
const PARENT = "hive.newProjectParent";

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

/** The parent folder of the last new project; empty (the home folder) when none was made. */
export const savedParent = (storage: Pick<Storage, "getItem"> | null = safeStorage()) => {
  try {
    return storage?.getItem(PARENT) ?? "";
  } catch {
    return "";
  }
};

const saveParent = (parent: string) => {
  try {
    safeStorage()?.setItem(PARENT, parent);
  } catch {
    // A full or blocked storage only loses the preference.
  }
};

type Mode = "add" | "new";

/**
 * Asks the service to follow a folder, typed or browsed in a `FolderField`, or (23.2, "New
 * project") to make one in a parent folder and follow it. With the service in WSL, a select picks
 * WSL or Windows paths; the service converts them. It answers with the project (the dialog closes;
 * a new project's terminal opens first, with `claude` typed when asked) or the reason it was
 * refused (shown under the field). Every rule, the name's too, is the service's. With the service
 * on Windows itself, every path is a Windows one (`\` or `/`, drive letters), listed by the
 * service as typed.
 */
export function AddProjectDialog() {
  const error = useHive((s) => s.addProjectError);
  const created = useHive((s) => s.projectCreated);
  const wsl = useHive((s) => s.connection.status === "connected" && s.connection.distro !== null);
  const [side, setSide] = useState(() => savedWindows());
  const windows = wsl && side;
  // Windows paths: the WSL service's Windows side, or every path of the service on Windows.
  const backslash = useHive((s) => windows || s.appMode?.mode === "native");
  const [mode, setMode] = useState<Mode>("add");
  const [path, setPath] = useState("");
  const [parent, setParent] = useState(() => savedParent());
  const [name, setName] = useState("");
  const [git, setGit] = useState(true);
  const [claude, setClaude] = useState(true);
  const creating = mode === "new";
  const folder = creating ? parent : path;
  const listing = useFolderListing(folder, windows);
  const typed = folder || listing?.path || "";
  const target = listing?.path === typed ? listing.linux_path : null;
  const canSubmit = target !== null && (!creating || name !== "");

  // The service made the project: remember where, select it, open its terminal, close.
  const handled = useRef(created);
  useEffect(() => {
    if (!created || handled.current === created) return;
    handled.current = created;
    saveParent(typed);
    select(created.path);
    if (claude) showOpenFailure(openClaude(created.path));
    close();
  }, [created, claude, typed]);

  const sideProps = {
    windows,
    onWindows: (next: boolean) => {
      setSide(next);
      saveWindows(next);
    },
    placeholder: backslash ? "C:\\Users\\you\\projects\\shop" : homeText("/home/you/projects/shop"),
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
          dialog.querySelector<HTMLInputElement>("input#add-project-path")?.focus();
        }
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!canSubmit) return;
          if (creating) void transport.createProject(target, name, git);
          else void transport.addProject(target);
        }}
      >
        <header>
          <h2 id="add-project-title">Add project</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <fieldset className="segmented add-project-mode" aria-label="Project">
            {(["add", "new"] as const).map((m) => (
              <button
                type="button"
                key={m}
                aria-pressed={mode === m}
                onClick={() => {
                  setMode(m);
                  clearAddProjectError();
                }}
              >
                {m === "add" ? "Existing folder" : "New project"}
              </button>
            ))}
          </fieldset>
          {creating ? (
            <>
              <div className="field">
                <label htmlFor="add-project-parent">Parent folder</label>
                <FolderField
                  id="add-project-parent"
                  value={parent}
                  onChange={(to) => {
                    setParent(to);
                    clearAddProjectError();
                  }}
                  {...sideProps}
                  home
                  newFolder
                />
              </div>
              <div className="field">
                <label htmlFor="add-project-name">Name</label>
                <input
                  id="add-project-name"
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    clearAddProjectError();
                  }}
                  placeholder="e.g. shop"
                  spellCheck={false}
                  autoComplete="off"
                  aria-invalid={error !== null}
                />
              </div>
            </>
          ) : (
            <div className="field">
              <label htmlFor="add-project-path">Folder</label>
              <FolderField
                id="add-project-path"
                value={path}
                onChange={(to) => {
                  setPath(to);
                  clearAddProjectError();
                }}
                {...sideProps}
                home
                newFolder
                invalid={error !== null}
              />
            </div>
          )}
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
          {creating ? (
            <>
              <label className="checkbox">
                <input type="checkbox" checked={git} onChange={(e) => setGit(e.target.checked)} />
                Initialize git
              </label>
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={claude}
                  onChange={(e) => setClaude(e.target.checked)}
                />
                Open a terminal running claude
              </label>
            </>
          ) : (
            <p className="field-help">
              A git repository, or any folder inside one: the whole repository is added.
            </p>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!canSubmit}>
            {creating ? "Create project" : "Add project"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
