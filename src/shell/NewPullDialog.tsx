import { useState } from "react";
import { setPullBusy } from "../pulls";
import { findWorktree, openModal, owner, useHive } from "../store";
import { transport } from "../transport";
import { CloseIcon } from "./icons";
import { showModal } from "./WorktreeMenu";

const close = () => openModal(null);

/**
 * "Create pull request" (9.31) from the dialog's worktree's branch: title, description, base
 * (the main worktree's branch at first) and draft. The service checks them and runs
 * `gh pr create`; the branch must be on GitHub already (Hive does not push). Closes when the
 * pull request is opened; `gh`'s refusal shows here.
 */
export function NewPullDialog() {
  const w = useHive((s) => findWorktree(s.projects, s.modalWorktree));
  const project = useHive((s) => owner(s.projects, s.modalWorktree));
  const busy = useHive((s) => s.pullBusy?.action === "create");
  const error = useHive((s) => (s.pullError?.number === null ? s.pullError.message : null));
  const main = project?.worktrees.find((x) => x.main)?.branch ?? "main";
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [base, setBase] = useState(main);
  const [draft, setDraft] = useState(false);
  if (!w || !project) return null;
  return (
    <dialog
      className="dialog"
      aria-labelledby="new-pull-title"
      ref={showModal("input[name=title]")}
      onClose={close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          setPullBusy({ project: project.id, number: null, action: "create" });
          void transport.createPull(w.path, title, body, base, draft);
        }}
      >
        <header>
          <h2 id="new-pull-title">Create pull request</h2>
          <button type="button" className="ghost" title="Close (Esc)" onClick={close}>
            <CloseIcon />
          </button>
        </header>
        <div className="dialog-body">
          <p className="hint">
            From <b>{w.branch}</b>, which must be pushed to GitHub first.
          </p>
          <div className="field">
            <label htmlFor="new-pull-title-input">Title</label>
            <input
              id="new-pull-title-input"
              name="title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              maxLength={256}
              autoComplete="off"
            />
          </div>
          <div className="field">
            <label htmlFor="new-pull-base">Into</label>
            <input
              id="new-pull-base"
              value={base}
              onChange={(e) => setBase(e.target.value)}
              spellCheck={false}
              autoComplete="off"
            />
          </div>
          <div className="field">
            <label htmlFor="new-pull-body">Description</label>
            <textarea
              id="new-pull-body"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              rows={8}
            />
          </div>
          <label className="checkbox">
            <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} />
            Draft
          </label>
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
        </div>
        <footer>
          <button type="button" className="secondary" onClick={close}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={busy || !title.trim()}>
            {busy ? "Creating…" : "Create"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
