import { ask, fileTabState, openModal, useHive } from "../store";
import { transport } from "../transport";
import { isDirty } from "../viewer/buffer";
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

/** Asks before the service stops following the project `id` (9.28); its files stay on disk. */
export function askRemoveProject(id: string): void {
  const s = useHive.getState();
  const project = s.projects?.[id];
  if (!project) return;
  const places = [id, ...project.worktrees.map((w) => w.id)];
  const unsaved = s.openFiles.some((f) => {
    const { edit } = fileTabState(s, f);
    return places.includes(f.worktree) && !!edit && isDirty(edit);
  });
  const lost = unsaved ? " Unsaved changes in its open files will be lost." : "";
  ask({
    title: "Remove project?",
    text: `Remove ${project.name} from Hive? Its files stay on disk: the repository and its worktrees are not deleted.${lost}`,
    action: "Remove",
    run: () => void transport.removeProject(id),
  });
}

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
