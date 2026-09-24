import { useState } from "react";
import { clearAddProjectError, openModal, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";

const close = () => openModal(null);

/**
 * Asks the service to follow a folder inside WSL. The service checks it and answers with the
 * project (the dialog closes) or the reason it was refused (shown under the field).
 * The prototype has no screen for this; it follows the new-worktree dialog (1c).
 */
export function AddProjectDialog() {
  const error = useHive((s) => s.addProjectError);
  const [path, setPath] = useState("");
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog"
      aria-labelledby="add-project-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          // showModal focuses the first button; the field is what the user needs.
          dialog.querySelector("input")?.focus();
        }
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void transport.addProject(path);
        }}
      >
        <header>
          <h2 id="add-project-title">Add project</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <label className="field">
            <span>Folder in WSL</span>
            <input
              value={path}
              onChange={(e) => {
                setPath(e.target.value);
                clearAddProjectError();
              }}
              placeholder="/home/user/projects/shop"
              spellCheck={false}
              aria-invalid={error !== null}
            />
          </label>
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
          <p className="field-help">
            A git repository, or any folder inside one: the whole repository is added.
          </p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button type="submit" className="primary" disabled={path.trim() === ""}>
            Add project <kbd>Enter</kbd>
          </button>
        </footer>
      </form>
    </dialog>
  );
}
