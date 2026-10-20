import { copy } from "../sessions";
import { openModal } from "../store";
import { CloseIcon } from "./icons";

/** Lifts the quarantine macOS put on the downloaded app, so it no longer runs translocated. */
export const UNQUARANTINE = "xattr -dr com.apple.quarantine /Applications/Hive.app";

const close = () => openModal(null);

/**
 * macOS runs Hive translocated (17.10): opened where it was downloaded, from a read-only copy
 * the updater cannot replace. Opened once per run by the app side (`translocated`), and by the
 * update button while translocated.
 */
export function TranslocatedDialog() {
  return (
    // A native modal dialog: the page behind is inert and Esc closes it.
    <dialog
      className="dialog"
      aria-labelledby="translocated-title"
      ref={(dialog) => {
        if (dialog && !dialog.open) {
          dialog.showModal();
          dialog.querySelector<HTMLButtonElement>(".primary")?.focus();
        }
      }}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          close();
        }}
      >
        <header>
          <h2 id="translocated-title">Move Hive to Applications</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            macOS runs Hive from a temporary read-only copy, because it was opened where it was
            downloaded. Hive works, but updates cannot install there.
          </p>
          <ol>
            <li>Quit Hive.</li>
            <li>In Finder, drag Hive into the Applications folder.</li>
            <li>
              In Terminal, run <code>{UNQUARANTINE}</code>
            </li>
            <li>Open Hive from Applications.</li>
          </ol>
        </div>
        <footer>
          <button
            type="button"
            className="secondary"
            onClick={() => void copy(UNQUARANTINE, "command")}
          >
            Copy command
          </button>
          <button type="submit" className="primary">
            OK
          </button>
        </footer>
      </form>
    </dialog>
  );
}
