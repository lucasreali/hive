import { ask, openModal, useHive } from "../store";
import { CloseIcon } from "./icons";
import { showModal } from "./WorktreeMenu";

/** Asks before dropping the unsaved edits of `path`; `then` runs once the user picks Discard. */
export const askDiscard = (path: string, then: () => void) =>
  ask({
    title: "Discard changes?",
    text: `Your unsaved changes to ${path} will be lost.`,
    action: "Discard",
    run: then,
  });

/**
 * The yes/no question of `ask` (8.20), in the app's look instead of the WebView's `confirm`. Its
 * action is destructive, so Cancel has the focus: Enter cancels, Esc too. Either answer goes back
 * to the dialog it was asked from, if any (`Question.back`).
 */
export function ConfirmDialog() {
  const question = useHive((s) => s.question);
  if (!question) return null;
  const cancel = () => openModal(question.back ?? null);
  return (
    <dialog
      className="dialog"
      aria-labelledby="confirm-title"
      ref={showModal(".secondary")}
      onClose={cancel}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          // Closed first, so `run` may ask again.
          cancel();
          question.run();
        }}
      >
        <header>
          <h2 id="confirm-title">{question.title}</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={cancel}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>{question.text}</p>
        </div>
        <footer>
          <button type="button" className="secondary" onClick={cancel}>
            Cancel
          </button>
          <button type="submit" className="primary danger">
            {question.action}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
