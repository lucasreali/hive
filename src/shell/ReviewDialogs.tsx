import { useEffect, useRef } from "react";
import { findWorktree, openModal, select, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";
import { showModal } from "./WorktreeMenu";

const close = () => openModal(null);

/**
 * "Review in a new worktree…" (27.4): what the service will make, then `create_review`. The
 * review is selected once made; with notes from the service the dialog stays to show them.
 */
export function ReviewWorktreeDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const { created, createFailure } = useHive((s) => s.worktreeDialog);
  const handled = useRef(created);
  useEffect(() => {
    if (!created || handled.current === created) return;
    handled.current = created;
    select(created.path);
    if (created.notes.length === 0) close();
  }, [created]);
  if (!w) return null;
  const failed = createFailure?.name === w.path ? createFailure.message : null;
  return (
    <dialog
      className="dialog"
      aria-labelledby="review-worktree-title"
      ref={showModal("button[type=submit]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (created) close();
          else void transport.createReview(w.path);
        }}
      >
        <header>
          <h2 id="review-worktree-title">Review in a new worktree</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {created ? (
            <>
              <p className="created">
                Created <code>{created.path}</code>
              </p>
              <ul className="notes">
                {created.notes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            </>
          ) : (
            <>
              <p>
                A new worktree on a new branch from the tip of <code>{w.branch}</code>, shown under{" "}
                <strong>{w.name}</strong>: try and fix things there without touching it.
              </p>
              <p className="field-help">
                Only committed work is in it. Its changes come back to {w.name} only when you choose
                "Bring changes back".
              </p>
            </>
          )}
          {failed && (
            <p className="field-error" role="alert">
              {failed}
            </p>
          )}
        </div>
        <footer>
          {!created && (
            <button type="button" className="secondary" onClick={close}>
              Cancel
            </button>
          )}
          <button type="submit" className="primary">
            {created ? "Done" : "Create review"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}

const commits = (n: number) => `${n} commit${n === 1 ? "" : "s"}`;

/**
 * "Bring changes back…" (27.4): the service's plan (the review's commits missing from the
 * reviewed branch), then `bring_back` with the tip shown, only on the click (#12). The service
 * merges; its refusal or git's message shows here.
 */
export function BringBackDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const { plan: answer, bringFailure } = useHive((s) => s.worktreeDialog);
  const path = w?.path;
  useEffect(() => {
    if (path) void transport.planBringBack(path);
  }, [path]);
  if (!w) return null;
  const shown = answer?.path === w.path ? answer : null;
  const plan = shown?.plan ?? null;
  const failed = bringFailure?.path === w.path ? bringFailure.message : null;
  const more = plan ? plan.total - plan.commits.length : 0;
  return (
    <dialog
      className="dialog"
      aria-labelledby="bring-back-title"
      ref={showModal("button[type=submit]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (plan && plan.total > 0)
            void transport.bringBack(w.path, plan.into, plan.base, plan.tip);
        }}
      >
        <header>
          <h2 id="bring-back-title">Bring changes back</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          {!shown && <p className="field-help">Looking at the review's commits…</p>}
          {shown?.error && (
            <p className="field-error" role="alert">
              {shown.error}
            </p>
          )}
          {plan && plan.total === 0 && (
            <p>
              Nothing to bring back: every commit of <code>{plan.branch}</code> is on{" "}
              <code>{plan.into}</code>.
            </p>
          )}
          {plan && plan.total > 0 && (
            <>
              <p>
                {commits(plan.total)} of <code>{plan.branch}</code> go into <code>{plan.into}</code>
                , {plan.fast_forward ? "as a fast-forward" : "with a merge commit"}:
              </p>
              <ul className="merged-list review-commits">
                {plan.commits.map((c) => (
                  <li key={c.id} className="merged-item">
                    <code>{c.id.slice(0, 7)}</code>
                    <span className="label">{c.summary}</span>
                  </li>
                ))}
              </ul>
              {more > 0 && <p className="field-help">and {commits(more)} more</p>}
              <p className="field-help">
                Both worktrees must have no uncommitted changes. A conflict stops it and changes
                nothing.
              </p>
            </>
          )}
          {failed && (
            <p className="field-error said" role="alert">
              {failed}
            </p>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!plan || plan.total === 0}>
            Bring back
          </button>
        </footer>
      </form>
    </dialog>
  );
}

/**
 * "Discard review…" (27.4): the delete dialog's checks (4.4), then "Discard anyway" (`force`)
 * after a refusal. Its branch goes only when every commit of it was brought back.
 */
export function DiscardReviewDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const failure = useHive((s) => s.worktreeDialog.failure);
  if (!w) return null;
  const failed = failure?.path === w.path && failure.name === null ? failure.message : null;
  return (
    <dialog
      className="dialog"
      aria-labelledby="discard-review-title"
      ref={showModal("button[type=submit]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void transport.discardReview(w.path, failed !== null);
        }}
      >
        <header>
          <h2 id="discard-review-title">Discard review</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p>
            Delete the review <strong>{w.name}</strong> and its folder <code>{w.path}</code>?
          </p>
          {w.branch && (
            <p className="field-help">
              Its branch <code>{w.branch}</code> is deleted too if every commit of it was brought
              back; otherwise it is kept.
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
            Cancel
          </button>
          <button type="submit" className="primary danger">
            {failed ? "Discard anyway" : "Discard"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
