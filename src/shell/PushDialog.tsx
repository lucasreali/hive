import { push } from "../git";
import { findWorktree, openModal, useHive } from "../store";
import { CloseIcon } from "./icons";
import { showModal } from "./WorktreeMenu";

const close = () => openModal(null);

/**
 * "Push…" (27.6, D27.3) for the dialog's worktree: where its branch goes, as the service says
 * (its upstream, or `origin/<branch>` set as its upstream), and Push. Never forced: there is no
 * option for it. Closes once pushed; git's refusal (a rejected push) shows here as is.
 */
export function PushDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const target = useHive((s) => (w ? s.pushTargets[w.path] : undefined));
  const git = useHive((s) => (w ? s.git[w.path] : undefined));
  if (!w) return null;
  const busy = git?.busy === "push";
  const failed = git?.error?.action === "push" ? git.error.message : null;
  const ahead = target?.ahead;
  return (
    <dialog
      className="dialog"
      aria-labelledby="push-title"
      ref={showModal("button.primary")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          push(w.path);
        }}
      >
        <header>
          <h2 id="push-title">Push</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {!target && <p className="hint">Looking for where {w.branch ?? "it"} goes…</p>}
          {target?.error && (
            <span className="field-error" role="alert">
              {target.error}
            </span>
          )}
          {target && !target.error && (
            <p>
              Push <b>{target.branch}</b> to <b>{target.target}</b>
              {target.set_upstream ? ", and make it its upstream" : ""}?
              {ahead ? ` ${ahead} ${ahead === 1 ? "commit" : "commits"} to send.` : ""}
              {ahead === 0 ? " It has no commit to send." : ""}
            </p>
          )}
          {target && !target.error && (
            <p className="hint">
              The repository's hooks run as git runs them; credentials are git's own.
            </p>
          )}
          {failed && (
            <pre className="field-error git-output" role="alert">
              {failed}
            </pre>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button
            type="submit"
            className="primary"
            disabled={busy || !target || target.error !== null}
          >
            {busy ? "Pushing…" : "Push"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
